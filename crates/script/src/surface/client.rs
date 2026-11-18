//! Surface-neutral adapter half of the slot's native seat protocol.
//!
//! A run owns this client. It owns request identity, bounded outgoing rows and
//! exactly one awaited completion, but neither a VM nor a game machine. The
//! slot remains the only authority that admits a batch to the gameplay seat.

use crate::api_combat::CombatEnd;
use crate::api_gather::GatherEnd;
use crate::api_progress::ProgressPage;
use crate::interact::InteractReq;
use crate::native::request::NativeKind;
use crate::native::seat::{SeatResult, SeatTerminal};
use crate::native::seat_rows::{
    DecodedSeatBatch, HeldWaitFacts, QueuedSeatBatch, SeatCancelDecoded, SeatStartArgs,
    SeatStartInput, MAX_SEAT_CANCELS, MAX_SEAT_GAMEPLAY_ROWS, MAX_SEAT_STARTS,
};
use crate::native::snapshot_source::NativeFactsInput;
use api::identity::{ExecutionId, RequestId};
use api::script_contract::{AbortReason, CallbackCancelReason};
use std::collections::VecDeque;

/// An error returned before the client queues any part of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("a request is already awaited")]
    AlreadyWaiting,
    #[error("the seat's pending-start capacity is exhausted")]
    StartsFull,
    #[error("the seat's pending-cancel capacity is exhausted")]
    CancelsFull,
    #[error("the seat's gameplay-row capacity is exhausted")]
    RowsFull,
    #[error("the isolate request namespace is exhausted")]
    IdExhausted,
}

/// One awaited operation; only one tick coroutine exists in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {
    Seat { id: RequestId, kind: NativeKind },
    Walk { id: RequestId, after_seq: u64 },
    NextTick,
}

/// The exact walk outcome carried by a lent delivery, not a boolean inferred
/// from a bare seat terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkCompletion {
    pub id: RequestId,
    pub generation: u64,
    pub sequence: u64,
    pub failed: bool,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub radius: i32,
    pub blocked: bool,
    pub no_path: bool,
}

/// A completion copied while its borrowed snapshot page is still alive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatCompletion {
    Terminal(SeatResult),
    Gather(GatherEnd),
    Combat(CombatEnd),
    Progress(ProgressPage),
    Walk(WalkCompletion),
    NextTick,
    Aborted(AbortReason),
}

#[derive(Debug, Clone)]
struct StoredCompletion {
    value: SeatCompletion,
    terminal_seq: u64,
}

/// Bounded adapter state for one running non-JS surface.
#[derive(Debug)]
pub struct SeatClient {
    execution: ExecutionId,
    published_terminal_seq: u64,
    next_id: u64,
    consumed_terminal_seq: u64,
    observed_walk_outcome_seq: u64,
    awaiting: Option<Awaited>,
    stored: Option<StoredCompletion>,
    starts: Vec<SeatStartInput>,
    cancels: Vec<SeatCancelDecoded>,
    lifecycle: Vec<InteractReq>,
    restored: VecDeque<QueuedSeatBatch>,
}

impl SeatClient {
    pub fn new(execution: ExecutionId) -> Self {
        Self {
            execution,
            next_id: RequestId::ISOLATE_FIRST,
            consumed_terminal_seq: 0,
            observed_walk_outcome_seq: 0,
            awaiting: None,
            stored: None,
            published_terminal_seq: 0,
            starts: Vec::new(),
            cancels: Vec::new(),
            lifecycle: Vec::new(),
            restored: VecDeque::new(),
        }
    }

    pub fn execution(&self) -> ExecutionId {
        self.execution
    }

    pub fn align_execution(&mut self, execution: ExecutionId) {
        if self.execution != execution {
            self.reset(Some(execution));
        }
    }

    /// A session boundary terminates the old wait locally, without publishing
    /// a Reset terminal on the seat wire. The returned generation is the one
    /// the slot uses to fence any batches already drained.
    pub fn reset(&mut self, execution: Option<ExecutionId>) -> u64 {
        if self.awaiting.is_some() {
            self.stored = Some(StoredCompletion {
                value: SeatCompletion::Aborted(AbortReason::Reset),
                terminal_seq: 0,
            });
        }
        self.published_terminal_seq = 0;
        self.starts.clear();
        self.cancels.clear();
        self.lifecycle.clear();
        self.restored.clear();
        self.consumed_terminal_seq = 0;
        self.observed_walk_outcome_seq = 0;
        if let Some(execution) = execution {
            self.execution = execution;
        }
        self.execution.generation().get()
    }

    pub fn awaiting(&self) -> Option<Awaited> {
        self.awaiting
    }

    fn mint(&mut self) -> Result<RequestId, ClientError> {
        let id = RequestId::isolate(self.next_id).map_err(|_| ClientError::IdExhausted)?;
        self.next_id = self.next_id.checked_add(1).unwrap_or(RequestId::HOST_FIRST);
        Ok(id)
    }

    /// Queue one typed start with the current run/session stamp and begin its
    /// wait. No machine and no script callback runs here.
    pub fn start_awaited(
        &mut self,
        kind: NativeKind,
        args: SeatStartArgs,
        parent: Option<RequestId>,
    ) -> Result<RequestId, ClientError> {
        if self.awaiting.is_some() {
            return Err(ClientError::AlreadyWaiting);
        }
        if self.pending_starts() >= MAX_SEAT_STARTS {
            return Err(ClientError::StartsFull);
        }
        if self.lifecycle.len() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        let id = self.mint()?;
        let run = self.execution.run();
        self.starts.push(SeatStartInput {
            id,
            parent,
            run: run.run,
            session: run.session,
            kind,
            args,
        });
        self.awaiting = Some(Awaited::Seat { id, kind });
        self.lifecycle.push(InteractReq::WaitEnqueued);
        Ok(id)
    }

    /// A walk is an already-typed interaction queued by a binding. Remember
    /// only its identity and the sequence preceding its dispatch.
    pub fn await_walk(&mut self, id: RequestId) -> Result<(), ClientError> {
        if self.awaiting.is_some() {
            return Err(ClientError::AlreadyWaiting);
        }
        if self.lifecycle.len() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        self.awaiting = Some(Awaited::Walk {
            id,
            after_seq: self.observed_walk_outcome_seq,
        });
        self.lifecycle.push(InteractReq::WaitEnqueued);
        Ok(())
    }

    pub fn await_next_tick(&mut self) -> Result<(), ClientError> {
        if self.awaiting.is_some() {
            return Err(ClientError::AlreadyWaiting);
        }
        if self.lifecycle.len() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        self.awaiting = Some(Awaited::NextTick);
        self.lifecycle.push(InteractReq::WaitEnqueued);
        Ok(())
    }

    /// The control stays bounded even when a host holds the seat. A pending
    /// start cancelled before admission travels with that start in one batch.
    pub fn cancel(
        &mut self,
        of: RequestId,
        reason: CallbackCancelReason,
    ) -> Result<(), ClientError> {
        if self.cancels.len()
            + self
                .restored
                .iter()
                .map(|batch| batch.batch.cancels.len())
                .sum::<usize>()
            >= MAX_SEAT_CANCELS
        {
            return Err(ClientError::CancelsFull);
        }
        self.cancels.push(SeatCancelDecoded { of, reason });
        Ok(())
    }

    /// Copy the one matching terminal while the lent page is borrowed. A
    /// family Done/Failed cannot settle with a bare terminal: its typed page
    /// must match the token. Refused/Aborted do not require a family page.
    /// `release_floor` excludes results already seen before this SeatPage.
    pub fn accept_delivery(
        &mut self,
        native: NativeFactsInput<'_>,
        release_floor: u64,
        tick: bool,
    ) {
        if self.stored.is_some() {
            return;
        }
        if native.walk_outcome_seq > self.observed_walk_outcome_seq {
            self.observed_walk_outcome_seq = native.walk_outcome_seq;
        }
        match self.awaiting {
            Some(Awaited::NextTick) if tick => {
                self.stored = Some(StoredCompletion {
                    value: SeatCompletion::NextTick,
                    terminal_seq: 0,
                });
            }
            Some(Awaited::Walk { id, after_seq })
                if native.walk_outcome_seq > after_seq
                    && native.walk_outcome_request_id == id.get() =>
            {
                self.stored = Some(StoredCompletion {
                    value: SeatCompletion::Walk(WalkCompletion {
                        id,
                        generation: native.walk_outcome_generation,
                        sequence: native.walk_outcome_seq,
                        failed: native.walk_outcome_failed,
                        x: native.walk_outcome_x,
                        z: native.walk_outcome_z,
                        level: native.walk_outcome_level,
                        radius: native.walk_outcome_radius,
                        blocked: native.walk_outcome_blocked,
                        no_path: native.walk_outcome_no_path,
                    }),
                    terminal_seq: 0,
                });
            }
            Some(Awaited::Seat { id, kind }) => {
                let Some(page) = native.seat_page else { return };
                let Some(result) = page.results().iter().find(|result| {
                    result.id == id
                        && result.seq > self.consumed_terminal_seq
                        && result.seq > release_floor
                }) else {
                    return;
                };
                let value = match (kind, result.outcome) {
                    (NativeKind::Gather, SeatTerminal::Done | SeatTerminal::Failed) => native
                        .api_gather_outcome
                        .filter(|end| end.token() == id.get())
                        .cloned()
                        .map(SeatCompletion::Gather),
                    (NativeKind::Combat, SeatTerminal::Done | SeatTerminal::Failed) => native
                        .api_combat_outcome
                        .filter(|end| end.token() == id.get())
                        .cloned()
                        .map(SeatCompletion::Combat),
                    (NativeKind::QuestProgress, SeatTerminal::Done | SeatTerminal::Failed) => {
                        native
                            .api_progress
                            .filter(|page| {
                                page.token() == id.get()
                                    && !matches!(page, ProgressPage::Reading { .. })
                            })
                            .cloned()
                            .map(SeatCompletion::Progress)
                    }
                    (NativeKind::QuestProgress, SeatTerminal::Refused(_)) => native
                        .api_progress
                        .filter(|page| {
                            page.token() == id.get() && matches!(page, ProgressPage::Refused { .. })
                        })
                        .cloned()
                        .map(SeatCompletion::Progress)
                        .or(Some(SeatCompletion::Terminal(*result))),
                    _ => Some(SeatCompletion::Terminal(*result)),
                };
                if let Some(value) = value {
                    self.stored = Some(StoredCompletion {
                        value,
                        terminal_seq: result.seq,
                    });
                }
            }
            _ => {}
        }
    }

    pub fn completion_ready(&self) -> bool {
        self.stored.is_some()
    }

    /// A completion is consumed only when the coroutine actually resumes.
    pub fn take_completion(&mut self) -> Option<SeatCompletion> {
        if self.lifecycle.len() >= MAX_SEAT_GAMEPLAY_ROWS {
            return None;
        }
        let stored = self.stored.take()?;
        self.awaiting.take()?;
        self.consumed_terminal_seq = self.consumed_terminal_seq.max(stored.terminal_seq);
        self.lifecycle.push(InteractReq::WaitSettled);
        Some(stored.value)
    }

    pub fn loop_settled(&mut self) -> Result<(), ClientError> {
        if self.lifecycle.len() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        self.lifecycle.push(InteractReq::LoopSettled);
        Ok(())
    }

    pub fn drain_lifecycle(&mut self) -> Vec<InteractReq> {
        std::mem::take(&mut self.lifecycle)
    }

    pub fn pending_starts(&self) -> usize {
        self.starts.len()
            + self
                .restored
                .iter()
                .map(|batch| batch.batch.starts.len())
                .sum::<usize>()
    }

    pub fn drain_seat_batches(&mut self) -> Vec<QueuedSeatBatch> {
        let mut batches: Vec<_> = self.restored.drain(..).collect();
        if !self.starts.is_empty()
            || !self.cancels.is_empty()
            || self.consumed_terminal_seq > self.published_terminal_seq
        {
            batches.push(QueuedSeatBatch {
                generation: self.execution.generation().get(),
                observed_walk_outcome_seq: self.observed_walk_outcome_seq,
                batch: DecodedSeatBatch {
                    consumed_terminal_seq: self.consumed_terminal_seq,
                    starts: std::mem::take(&mut self.starts),
                    decisions: Vec::new(),
                    cancels: std::mem::take(&mut self.cancels),
                    mirror_revokes: Vec::new(),
                    reqs: Vec::new(),
                },
                machine_events: Vec::new(),
                held_waits: HeldWaitFacts::default(),
                held_fold: false,
            });
        }
        self.published_terminal_seq = self.consumed_terminal_seq;
        batches
    }

    /// Preserve the original order when the slot cannot admit a batch yet.
    pub fn restore_seat_batches(&mut self, batches: Vec<QueuedSeatBatch>) {
        for batch in batches.into_iter().rev() {
            self.restored.push_front(batch);
        }
    }
}

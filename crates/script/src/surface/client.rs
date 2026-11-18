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
    DecodedSeatBatch, HeldWaitFacts, QueuedSeatBatch, SeatCancelDecoded, SeatInteractDecoded,
    SeatStartArgs, SeatStartInput, MAX_SEAT_CANCELS, MAX_SEAT_GAMEPLAY_ROWS, MAX_SEAT_STARTS,
};
use crate::native::snapshot_source::{NativeFactsInput, ReachViewInput, SnapshotInput};
use crate::native::walk_wait::{HostOutcome, Observation, WalkKey, WalkSlot};
use crate::native::WalkCancelReason;
use api::identity::{ExecutionId, RequestId};
use api::query::{ArrivalProbe, SceneReachOptions};
use api::script_contract::{AbortReason, CallbackCancelReason};
use api::snapshot::WorldTile;
use std::collections::VecDeque;
use std::num::NonZeroU64;

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
    #[error("only walk and walk-near requests can start a walk wait")]
    NotWalk,
}

/// One awaited operation; only one tick coroutine exists in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {
    Seat { id: RequestId, kind: NativeKind },
    Walk { id: RequestId },
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
    pub allow_teleports: bool,
    pub cancel_reason: WalkCancelReason,
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
pub struct SeatClient {
    execution: ExecutionId,
    consumed_terminal_seq: u64,
    /// The slot fences drained batches with this work epoch, not the seat run key.
    work_generation: u64,
    acknowledged_terminal_seq: u64,
    sent_terminal_seq: u64,
    observed_walk_outcome_seq: u64,
    next_id: u64,
    awaiting: Option<Awaited>,
    stored: Option<StoredCompletion>,
    awaited_start: Option<SeatStartInput>,
    reconnect_held: bool,
    rearm_walk_on_tick: bool,
    walk: WalkSlot,
    walk_generation: u64,
    walk_intent_baseline: u64,
    manual_takeover_seq: u64,
    walk_request: Option<InteractReq>,
    starts: Vec<SeatStartInput>,
    cancels: Vec<SeatCancelDecoded>,
    reqs: Vec<SeatInteractDecoded>,
    lifecycle: Vec<InteractReq>,
    restored: VecDeque<QueuedSeatBatch>,
}

impl SeatClient {
    pub fn new(execution: ExecutionId) -> Self {
        Self {
            work_generation: execution.generation().get(),
            execution,
            next_id: RequestId::ISOLATE_FIRST,
            consumed_terminal_seq: 0,
            acknowledged_terminal_seq: 0,
            sent_terminal_seq: 0,
            observed_walk_outcome_seq: 0,
            awaiting: None,
            stored: None,
            awaited_start: None,
            reconnect_held: false,
            rearm_walk_on_tick: false,
            walk: WalkSlot::new(),
            walk_generation: 0,
            walk_intent_baseline: 0,
            manual_takeover_seq: 0,
            walk_request: None,
            starts: Vec::new(),
            cancels: Vec::new(),
            reqs: Vec::new(),
            lifecycle: Vec::new(),
            restored: VecDeque::new(),
        }
    }

    pub fn execution(&self) -> ExecutionId {
        self.execution
    }

    pub fn align_execution(&mut self, execution: ExecutionId) {
        if self.execution == execution {
            return;
        }
        self.execution = execution;
        for start in &mut self.starts {
            start.run = execution.run().run;
            start.session = execution.run().session;
        }
        if let Some(start) = &mut self.awaited_start {
            start.run = execution.run().run;
            start.session = execution.run().session;
        }
        for batch in &mut self.restored {
            batch.generation = self.work_generation;
            for start in &mut batch.batch.starts {
                start.run = execution.run().run;
                start.session = execution.run().session;
            }
        }
    }

    /// A session reset discards unacknowledged gameplay rows and aborts only
    /// an unresolved wait. An already-copied completion belongs to the script,
    /// even while Pause or a host hold delays its coroutine.
    pub fn reset(&mut self, execution: Option<ExecutionId>) -> u64 {
        self.clear_session_work(execution);
        if self.awaiting.is_some() && self.stored.is_none() {
            self.stored = Some(StoredCompletion {
                value: SeatCompletion::Aborted(AbortReason::Reset),
                terminal_seq: 0,
            });
        }
        self.work_generation
    }

    /// An unexpected disconnect holds the pending wait and its intent, but
    /// never carries the old session's outbound gameplay rows into the new one.
    /// The first new tick re-arms an awaited start under its current run key.
    pub fn reconnect(&mut self, execution: Option<ExecutionId>) -> u64 {
        let retained_start = self
            .stored
            .is_none()
            .then(|| self.awaited_start.take())
            .flatten();
        let pending_walk = match (self.awaiting, self.stored.is_none()) {
            (Some(Awaited::Walk { id }), true) => Some(id),
            _ => None,
        };
        let unsent_walk = pending_walk.is_some_and(|id| {
            self.reqs
                .iter()
                .chain(
                    self.restored
                        .iter()
                        .flat_map(|batch| batch.batch.reqs.iter()),
                )
                .any(|row| {
                    matches!(&row.request,
                    InteractReq::Walk { request_id, .. } | InteractReq::WalkNear { request_id, .. }
                    if *request_id == id.get())
                })
        });
        let retained_walk =
            pending_walk.map(|_| std::mem::replace(&mut self.walk, WalkSlot::new()));
        let retained_walk_request = self.walk_request.take();
        self.clear_session_work(execution);
        self.awaited_start = retained_start;
        if let Some(start) = &mut self.awaited_start {
            let run = self.execution.run();
            start.run = run.run;
            start.session = run.session;
        }
        if let Some(walk) = retained_walk {
            self.walk = walk;
            self.walk_request = retained_walk_request;
            self.rearm_walk_on_tick = unsent_walk;
        }
        self.reconnect_held = true;
        self.work_generation
    }

    fn clear_session_work(&mut self, execution: Option<ExecutionId>) {
        // Design 4.4 item 1: a reset mints a fresh work generation as JS
        // does, independent of the ExecutionId's stable initial generation.
        // The slot uses this value to refuse drained batches from the old work.
        self.work_generation = self.work_generation.wrapping_add(1).max(1);
        self.starts.clear();
        self.cancels.clear();
        self.awaited_start = None;
        self.reqs.clear();
        self.lifecycle.clear();
        self.restored.clear();
        self.consumed_terminal_seq = 0;
        self.acknowledged_terminal_seq = 0;
        self.sent_terminal_seq = 0;
        // A copied completion stays with the script, but its old session's
        // terminal sequence must never acknowledge a result in the new seat.
        if let Some(stored) = &mut self.stored {
            stored.terminal_seq = 0;
        }
        self.observed_walk_outcome_seq = 0;
        self.walk.reset();
        self.walk_generation = 0;
        self.walk_request = None;
        self.rearm_walk_on_tick = false;
        self.reconnect_held = false;
        if let Some(execution) = execution {
            self.execution = execution;
        }
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
        if self.pending_rows() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        let id = self.mint()?;
        let run = self.execution.run();
        let start = SeatStartInput {
            id,
            parent,
            run: run.run,
            session: run.session,
            kind,
            args,
        };
        self.awaited_start = Some(start.clone());
        self.starts.push(start);
        self.awaiting = Some(Awaited::Seat { id, kind });
        self.lifecycle.push(InteractReq::WaitEnqueued);
        Ok(id)
    }

    /// Queue a walk with its current session's route generation and the
    /// game input at the request's start. The request and its wait share one
    /// minted token; the slot remains the only authority that admits it.
    pub fn queue_walk(
        &mut self,
        mut request: InteractReq,
        expected_generation: u64,
        input: &SnapshotInput<'_>,
    ) -> Result<RequestId, ClientError> {
        if self.awaiting.is_some() {
            return Err(ClientError::AlreadyWaiting);
        }
        if self.pending_rows() > MAX_SEAT_GAMEPLAY_ROWS.saturating_sub(2) {
            return Err(ClientError::RowsFull);
        }
        let key = match &request {
            InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports,
                ..
            } => WalkKey {
                tile: WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                },
                radius: 0,
                allow_teleports: Some(*allow_teleports),
            },
            InteractReq::WalkNear {
                x,
                z,
                level,
                radius,
                allow_teleports,
                ..
            } => WalkKey {
                tile: WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                },
                radius: *radius,
                allow_teleports: Some(*allow_teleports),
            },
            _ => return Err(ClientError::NotWalk),
        };
        let id = self.mint()?;
        match &mut request {
            InteractReq::Walk { request_id, .. } | InteractReq::WalkNear { request_id, .. } => {
                *request_id = id.get()
            }
            _ => unreachable!("request was checked as Walk or WalkNear"),
        }
        self.walk.begin(
            NonZeroU64::new(id.get()).expect("RequestId is nonzero"),
            key,
            HostOutcome {
                seq: self.observed_walk_outcome_seq,
                ..HostOutcome::empty()
            },
        );
        self.walk_generation = expected_generation;
        self.walk_intent_baseline = input.user_move_intent_seq;
        self.walk_request = Some(request.clone());
        self.awaiting = Some(Awaited::Walk { id });
        self.reqs.push(SeatInteractDecoded {
            lineage: None,
            mirror_owner: None,
            request,
        });
        self.lifecycle.push(InteractReq::WaitEnqueued);
        Ok(id)
    }

    /// Abort only the walk whose token this client owns. A later walk cannot
    /// be cut by a stale cancellation.
    pub fn abort_walk(&mut self, id: RequestId) -> Result<(), ClientError> {
        if !self.walk.owns(id.get()) {
            return Err(ClientError::NotWalk);
        }
        if self.pending_rows() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        self.reqs.push(SeatInteractDecoded {
            lineage: None,
            mirror_owner: None,
            request: InteractReq::AbortWalk {
                request_id: id.get(),
            },
        });
        Ok(())
    }

    pub fn rearm_host_discarded_walk(&mut self, token: u64) -> bool {
        if !self.walk.owns(token) || self.pending_rows() >= MAX_SEAT_GAMEPLAY_ROWS {
            return false;
        }
        if self.reconnect_held {
            self.rearm_walk_on_tick = self.walk_request.is_some();
            return self.rearm_walk_on_tick;
        }
        let Some(request) = self.walk_request.clone() else {
            return false;
        };
        self.reqs.push(SeatInteractDecoded {
            lineage: None,
            mirror_owner: None,
            request,
        });
        true
    }

    pub fn live_walking_operation(&self) -> bool {
        self.awaiting
            .is_some_and(|wait| matches!(wait, Awaited::Walk { id } if self.walk.pending(id.get())))
    }

    pub fn note_manual_walk_takeover(&mut self, sequence: u64) {
        self.manual_takeover_seq = self.manual_takeover_seq.max(sequence);
    }

    pub fn await_next_tick(&mut self) -> Result<(), ClientError> {
        if self.awaiting.is_some() {
            return Err(ClientError::AlreadyWaiting);
        }
        if self.pending_rows() >= MAX_SEAT_GAMEPLAY_ROWS {
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
        if let Some(existing) = self.cancels.iter_mut().find(|cancel| cancel.of == of) {
            if reason == CallbackCancelReason::ParentAborted {
                existing.reason = reason;
            }
            return Ok(());
        }
        if self.cancels.len() >= MAX_SEAT_CANCELS {
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
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        release_floor: u64,
        tick: bool,
    ) {
        if self.reconnect_held && !tick {
            return;
        }
        if let Some(page) = native.seat_page {
            let live = page.live();
            let results = page.results();
            self.starts.retain(|start| {
                !live.iter().any(|row| row.id == start.id)
                    && !results.iter().any(|row| row.id == start.id)
            });
            self.cancels
                .retain(|cancel| !results.iter().any(|row| row.id == cancel.of));
            if self.sent_terminal_seq > self.acknowledged_terminal_seq
                && !results.iter().any(|row| row.seq <= self.sent_terminal_seq)
            {
                self.acknowledged_terminal_seq = self.sent_terminal_seq;
            }
            for row in results {
                self.consumed_terminal_seq = self.consumed_terminal_seq.max(row.seq);
            }
        }
        if native.walk_outcome_seq > self.observed_walk_outcome_seq {
            self.observed_walk_outcome_seq = native.walk_outcome_seq;
        }
        if self.reconnect_held {
            self.reconnect_held = false;
            if self.stored.is_none() {
                match self.awaiting {
                    Some(Awaited::Seat { .. }) => {
                        if let Some(start) = &self.awaited_start {
                            self.starts.push(start.clone());
                        }
                        self.lifecycle.push(InteractReq::WaitEnqueued);
                    }
                    Some(Awaited::Walk { .. }) => {
                        if let Some((token, key)) = self.walk.request() {
                            self.walk.begin(
                                NonZeroU64::new(token).expect("pending Walk has nonzero token"),
                                key,
                                HostOutcome {
                                    seq: native.walk_outcome_seq,
                                    ..HostOutcome::empty()
                                },
                            );
                        }
                        self.walk_generation = native.walk_outcome_generation;
                        self.walk_intent_baseline = input.user_move_intent_seq;
                        if self.rearm_walk_on_tick && self.reqs.is_empty() {
                            if let Some(request) = &self.walk_request {
                                self.reqs.push(SeatInteractDecoded {
                                    lineage: None,
                                    mirror_owner: None,
                                    request: request.clone(),
                                });
                            }
                        }
                        self.rearm_walk_on_tick = false;
                        self.lifecycle.push(InteractReq::WaitEnqueued);
                    }
                    _ => {}
                }
            }
        }
        if self.stored.is_some() {
            return;
        }
        match self.awaiting {
            Some(Awaited::NextTick) if tick => {
                self.stored = Some(StoredCompletion {
                    value: SeatCompletion::NextTick,
                    terminal_seq: 0,
                });
            }
            Some(Awaited::Walk { id }) => {
                let observation = WalkObservation {
                    input,
                    native,
                    token: id,
                    generation: self.walk_generation,
                    intent_baseline: self.walk_intent_baseline,
                    manual_takeover_seq: self.manual_takeover_seq,
                };
                let outcome = observation.outcome();
                self.walk.observe_outcome(outcome);
                if self.walk.poll(id.get(), &observation) {
                    let key = self.walk.request().expect("pending Walk owns its key").1;
                    let matched = outcome.seq != 0 && outcome.request_id == id.get();
                    self.stored = Some(StoredCompletion {
                        value: SeatCompletion::Walk(WalkCompletion {
                            id,
                            generation: self.walk_generation,
                            sequence: if matched { outcome.seq } else { 0 },
                            failed: !self.walk.value(id.get()),
                            x: key.tile.x,
                            z: key.tile.z,
                            level: key.tile.level,
                            radius: key.radius,
                            blocked: self.walk.blocked(id.get()),
                            no_path: self.walk.no_path(id.get()),
                            allow_teleports: key.allow_teleports.unwrap_or(false),
                            cancel_reason: if observation.cancelled() {
                                WalkCancelReason::UserInput
                            } else {
                                WalkCancelReason::None
                            },
                        }),
                        terminal_seq: 0,
                    });
                }
            }
            Some(Awaited::Seat { id, kind }) => {
                let Some(page) = native.seat_page else { return };
                let Some(result) = page
                    .results()
                    .iter()
                    .find(|result| result.id == id && result.seq > release_floor)
                else {
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
        if self.pending_rows() >= MAX_SEAT_GAMEPLAY_ROWS {
            return None;
        }
        let stored = self.stored.take()?;
        self.awaiting.take()?;
        self.awaited_start = None;
        self.rearm_walk_on_tick = false;
        if matches!(
            stored.value,
            SeatCompletion::Walk(_) | SeatCompletion::Aborted(_)
        ) {
            self.walk.reset();
            self.walk_request = None;
        }
        self.consumed_terminal_seq = self.consumed_terminal_seq.max(stored.terminal_seq);
        self.lifecycle.push(InteractReq::WaitSettled);
        Some(stored.value)
    }

    pub fn loop_settled(&mut self) -> Result<(), ClientError> {
        if self.pending_rows() >= MAX_SEAT_GAMEPLAY_ROWS {
            return Err(ClientError::RowsFull);
        }
        self.lifecycle.push(InteractReq::LoopSettled);
        Ok(())
    }

    /// Lifecycle rows travel with seat batches, so guardian holds fold them
    /// under the same bound as JS. No second delivery path is permitted.
    pub fn drain_lifecycle(&mut self) -> Vec<InteractReq> {
        Vec::new()
    }

    pub fn pending_starts(&self) -> usize {
        self.starts.len()
    }
    fn pending_rows(&self) -> usize {
        self.lifecycle
            .len()
            .saturating_add(self.reqs.len())
            .saturating_add(
                self.restored
                    .iter()
                    .map(QueuedSeatBatch::rows)
                    .fold(0, usize::saturating_add),
            )
    }

    pub fn drain_seat_batches(&mut self) -> Vec<QueuedSeatBatch> {
        let mut batches: Vec<_> = self.restored.drain(..).collect();
        if !self.starts.is_empty()
            || !self.cancels.is_empty()
            || !self.lifecycle.is_empty()
            || !self.reqs.is_empty()
            || self.consumed_terminal_seq > self.acknowledged_terminal_seq
        {
            let mut reqs = std::mem::take(&mut self.reqs);
            reqs.extend(
                std::mem::take(&mut self.lifecycle)
                    .into_iter()
                    .map(|request| SeatInteractDecoded {
                        lineage: None,
                        mirror_owner: None,
                        request,
                    }),
            );
            batches.push(QueuedSeatBatch {
                generation: self.work_generation,
                observed_walk_outcome_seq: self.observed_walk_outcome_seq,
                batch: DecodedSeatBatch {
                    consumed_terminal_seq: self.consumed_terminal_seq,
                    starts: self.starts.clone(),
                    decisions: Vec::new(),
                    cancels: self.cancels.clone(),
                    mirror_revokes: Vec::new(),
                    reqs,
                },
                machine_events: Vec::new(),
                held_waits: HeldWaitFacts::default(),
                held_fold: false,
            });
            self.sent_terminal_seq = self.consumed_terminal_seq;
        }
        batches
    }

    /// Keep only one-shot gameplay rows when the slot restores a batch:
    /// pending controls and the terminal watermark remain with this client
    /// and are sent again until a seat page acknowledges them.
    pub fn restore_seat_batches(&mut self, batches: Vec<QueuedSeatBatch>) {
        for mut batch in batches.into_iter().rev() {
            batch.batch.starts.clear();
            batch.batch.cancels.clear();
            batch.batch.consumed_terminal_seq = 0;
            if !batch.batch.reqs.is_empty() || batch.held_waits.rows() != 0 {
                self.restored.push_front(batch);
            }
        }
    }
}

/// A borrowed view of the same flood ranks the JS adapter reads. No snapshot
/// arrays are copied into the synchronous Luau run.
struct BorrowedReach<'a>(ReachViewInput<'a>);

impl BorrowedReach<'_> {
    fn index(&self, tile: WorldTile) -> Option<usize> {
        let view = self.0;
        if !view.available || view.width <= 0 || view.height <= 0 || tile.level != view.level {
            return None;
        }
        let x = tile.x.checked_sub(view.base_x)?;
        let z = tile.z.checked_sub(view.base_z)?;
        (x >= 0 && z >= 0 && x < view.width && z < view.height)
            .then(|| {
                (x as usize)
                    .checked_mul(view.height as usize)?
                    .checked_add(z as usize)
            })
            .flatten()
    }

    fn bit(&self, words: &[u32], tile: WorldTile) -> bool {
        self.index(tile)
            .and_then(|index| {
                words
                    .get(index / 32)
                    .map(|word| word & (1 << (index % 32)) != 0)
            })
            .unwrap_or(false)
    }
}

impl ArrivalProbe for BorrowedReach<'_> {
    fn can_reach_from(
        &self,
        origin: WorldTile,
        destination: WorldTile,
        options: &SceneReachOptions,
    ) -> bool {
        if self
            .index(origin)
            .and_then(|index| self.0.exact_rank.get(index))
            != Some(&0)
        {
            return false;
        }
        let Some(index) = self.index(destination) else {
            return false;
        };
        let (bits, ranks) = if options.adjacent_ok {
            (self.0.reachable_adj, self.0.adjacent_rank)
        } else {
            (self.0.reachable, self.0.exact_rank)
        };
        bits.get(index / 32)
            .is_some_and(|word| word & (1 << (index % 32)) != 0)
            && ranks.get(index).is_some_and(|rank| {
                *rank != u16::MAX && u32::from(*rank) <= options.max_steps.unwrap_or(400)
            })
    }

    fn walkable(&self, tile: WorldTile) -> bool {
        self.bit(self.0.walkable, tile)
    }

    fn probeable(&self, tile: WorldTile) -> bool {
        self.index(tile).is_some()
    }
}

struct WalkObservation<'a, 's, 'n> {
    input: &'a SnapshotInput<'s>,
    native: NativeFactsInput<'n>,
    token: RequestId,
    generation: u64,
    intent_baseline: u64,
    manual_takeover_seq: u64,
}

impl Observation for WalkObservation<'_, '_, '_> {
    fn outcome(&self) -> HostOutcome {
        let native = self.native;
        if native.walk_outcome_generation != self.generation {
            return HostOutcome::empty();
        }
        HostOutcome {
            seq: native.walk_outcome_seq,
            generation: native.walk_outcome_generation,
            request_id: native.walk_outcome_request_id,
            failed: native.walk_outcome_failed,
            blocked: native.walk_outcome_blocked,
            no_path: native.walk_outcome_no_path,
            key: WalkKey {
                tile: WorldTile {
                    x: native.walk_outcome_x,
                    z: native.walk_outcome_z,
                    level: native.walk_outcome_level,
                },
                radius: native.walk_outcome_radius,
                allow_teleports: Some(native.walk_outcome_allow_teleports),
            },
        }
    }

    fn cancelled(&self) -> bool {
        self.input.user_move_intent_seq > self.intent_baseline
            || self.manual_takeover_seq > self.intent_baseline
            || (self.native.walk_outcome_request_id == self.token.get()
                && self.native.walk_outcome_generation == self.generation
                && self.input.walk_outcome_cancel_reason == WalkCancelReason::UserInput)
    }

    fn arrived(&self, key: WalkKey) -> bool {
        self.input.here.is_some_and(|here| {
            let me = WorldTile {
                x: here.x,
                z: here.z,
                level: here.level,
            };
            let reach = BorrowedReach(self.input.reach);
            api::query::is_arrived(me, key.tile, key.radius, || &reach)
        })
    }

    fn moving(&self) -> Option<bool> {
        self.native.local_player_moving
    }
}

//! The Underground Pass route, saved per account.
//!
//! The route is stored in the player's `ibanmulti` varp, which the server never
//! sends (`quest_upass.varp:4-5`), and the grid is crossed again later (the
//! Regicide quest uses the same floor). So what the step learns is also kept
//! as one fact of the account's `knowledge` domain in the process local-state
//! service's SQLite store ([`LocalStateDomain::Knowledge`], key [`KEY`],
//! payload [`GridMemory::to_payload`]). The service owns the database, its
//! writer thread and its audit line. The fact is the cross-run copy; inside one
//! run the route (and the trap count, which is never saved) lives in the run's
//! retained cell (`QuesterRetained::upass_grid`), and a run that has read its
//! route does not take the fact's content again.
//!
//! The step reads the fact when its run first needs it and writes it again
//! whenever it learns something. It never waits on the disk on a tick: the read
//! and each write are queued without blocking, and their results are observed
//! on later ticks. A nonempty route is written `Known` with `Observed`
//! provenance; a cleared route is written as an `Unknown` tombstone
//! (`ObservationRefused`). A `Partial` fact is read as the route it holds. Each
//! write names the durable version it replaces (compare-and-swap): the version
//! read, then the version of each completed write. A failed write is sent again
//! with the same observed stamp and the same expected version; a write the store
//! refuses as a version conflict is never sent again, so a stale positive route
//! cannot replace a later invalidation.
//!
//! One accepted write is in flight at a time and is followed until its
//! terminal outcome: it is never forgotten and submitted again. A change that
//! arrives meanwhile is sent after it. The step ends only after its last write
//! settled; if the writer does not settle within [`CONFIRM_TICKS`] observed
//! ticks, the step ends anyway with the route kept in the run's memory and one
//! warning line (the next step of the run sends it again).
//!
//! A scope with no SQL fact starts with an empty route. A fact that does not
//! decode starts an empty route with one log line; a store the service refuses
//! does the same and the run learns without saving.

use super::upass::{GridMemory, GridRun};
use crate::local_state::{KnowledgeTicket, SlotState};
use crate::native::ActionError;
use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{DurableTimestamp, RequestId, Settlement};
use api::local_state::{
    validate_knowledge_payload, DomainEdit, ErrorCode, KnowledgeKey, KnowledgeProvenance,
    KnowledgeRecord, KnowledgeRevocation, KnowledgeStatus, LocalStateDomain, StoreVersion,
    SubmitReceipt, KNOWLEDGE_SCHEMA_VERSION,
};
use api::process_condition::Recoverability;
use std::sync::Arc;
use std::task::Poll;

/// The fact's key in the account's knowledge scope.
pub(super) const KEY: &str = "upass.grid_route";

/// Observed ticks to wait for the account's knowledge fact to be read (one
/// SQL read on the service's writer thread; this is far longer than that takes).
pub(super) const LOAD_TICKS: u64 = 20;
/// Observed ticks the step's end waits for its last accepted write to reach a
/// terminal outcome. A write is one small SQLite transaction on the service's
/// writer thread; the bound is chosen, not measured: 40 ticks are 12 seconds on
/// the 300 ms campaign engine and 24 seconds on a 600 ms engine, far past any
/// healthy write. It bounds the step's end only: the write itself is followed
/// until it settles.
pub(super) const CONFIRM_TICKS: u64 = 40;
/// Failed or refused writes in a row before the run stops saving.
pub(super) const MAX_WRITE_FAILURES: u8 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The fact has not been read yet.
    Loading,
    /// Writes go to the account's knowledge fact.
    Saving,
    /// Nothing is saved: no capability for this slot, a store the service
    /// refused, a store that did not take the writes, or a fact that changed
    /// under the run (a version conflict).
    MemoryOnly,
}

/// What reading the saved route found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Loaded {
    /// A saved route, with this many confirmed columns.
    Saved(usize),
    /// The account has no saved route yet.
    Absent,
    /// The saved route was invalidated (an `Unknown` tombstone): learning
    /// starts empty.
    Invalidated,
    /// The saved fact does not decode; the route starts empty and the next
    /// write replaces the fact.
    Corrupt,
    /// The service refused the account's store: the run learns without saving.
    Refused(ErrorCode),
    /// The slot has no state capability: the run learns without saving.
    NoCapability,
    /// An earlier step of this run already read the route: the run's own copy
    /// is used, and the fact's content is not used for it.
    Kept {
        /// Confirmed columns of the run's route.
        columns: usize,
        /// Traps this run has taken.
        traps: u8,
        /// The store is not known to hold the route yet.
        unsaved: bool,
    },
}

impl Loaded {
    /// The one line the step logs for this result.
    pub(super) fn log(self, account: &str) {
        let (level, line) = match self {
            Loaded::Saved(columns) => (
                Level::Info,
                format!("saved route for {account}: {columns} column(s) confirmed"),
            ),
            Loaded::Absent => (
                Level::Info,
                format!("no saved route for {account}; learning it"),
            ),
            Loaded::Invalidated => (
                Level::Info,
                format!("the saved route for {account} was invalidated; learning it"),
            ),
            Loaded::Corrupt => (
                Level::Warn,
                format!("the saved route for {account} does not decode; learning it again"),
            ),
            Loaded::Refused(code) => (
                Level::Warn,
                format!("the saved route of {account} is not readable ({code}); learned for this run only"),
            ),
            Loaded::NoCapability => (
                Level::Info,
                "no state capability for this slot; the route is learned for this run only"
                    .to_string(),
            ),
            Loaded::Kept {
                columns,
                traps,
                unsaved,
            } => (
                Level::Info,
                format!(
                    "this run's route for {account}: {columns} column(s) confirmed, {traps} trap(s) taken so far{}",
                    if unsaved { ", not yet saved" } else { "" }
                ),
            ),
        };
        host_log!(Category::ScriptTrace, level, "quester upass grid: {line}");
    }
}

/// The step's hand on the saved route: what is written and what is waiting to be.
pub(super) struct RouteStore {
    mode: Mode,
    /// The fact's read, queued on the service.
    ticket: Option<KnowledgeTicket>,
    /// The accepted write in flight. Never dropped before its terminal outcome.
    pending: Option<RequestId>,
    /// The durable version the next write replaces: the fact's version as read
    /// ([`StoreVersion::INITIAL`] for no fact), then each completed write's.
    expected: StoreVersion,
    /// When the memory last changed: the observed stamp of the write that
    /// saves it. A retry of the same memory sends the same stamp.
    observed: DurableTimestamp,
    /// The memory changed since the last submission.
    dirty: bool,
    /// The run stopped saving after [`MAX_WRITE_FAILURES`] failures: the store
    /// lacks the route, and a later step of the run tries again.
    gave_up: bool,
    failures: u8,
    /// The tick the read began waiting at.
    load_since: Option<u64>,
    /// No submission before this observed tick (a failed or refused write is
    /// tried again once per tick).
    retry_at: u64,
}

impl RouteStore {
    /// A step that reads the saved route before it decides anything.
    pub(super) const fn loading() -> Self {
        Self {
            mode: Mode::Loading,
            ticket: None,
            pending: None,
            expected: StoreVersion::INITIAL,
            observed: DurableTimestamp::from_unix_seconds(0),
            dirty: false,
            gave_up: false,
            failures: 0,
            load_since: None,
            retry_at: 0,
        }
    }

    /// A step that learns for this run only.
    #[cfg(test)]
    pub(super) const fn memory_only() -> Self {
        Self {
            mode: Mode::MemoryOnly,
            ..Self::loading()
        }
    }

    /// The version the next write replaces.
    #[cfg(test)]
    pub(super) fn expected_for_test(&self) -> StoreVersion {
        self.expected
    }

    pub(super) fn is_loading(&self) -> bool {
        self.mode == Mode::Loading
    }

    /// The memory changed: write it, stamped now.
    pub(super) fn changed(&mut self) {
        self.dirty = true;
        self.observed = DurableTimestamp::now();
    }

    /// Whether nothing is waiting to be written or settled. An accepted write
    /// that has not reached its terminal outcome is never settled, whatever the
    /// mode.
    pub(super) fn settled(&self) -> bool {
        self.pending.is_none() && !(self.mode == Mode::Saving && self.dirty)
    }

    /// Whether the account's store may lack the route: a change not sent or not
    /// settled, or a store that did not take the writes. The run keeps this
    /// next to the route, so a later step of the run sends it again. A write
    /// the store refused as a version conflict is not owed again: the fact
    /// changed elsewhere and the run's copy must not replace it.
    pub(super) fn unsaved(&self) -> bool {
        self.gave_up || (self.mode == Mode::Saving && (self.dirty || self.pending.is_some()))
    }

    /// Read the saved route, or take the run's own copy `kept` when an earlier
    /// step of the run read it (the fact's content is then not used, but its
    /// version is the one the run's writes replace). `Pending` while the fact is
    /// being read, for at most [`LOAD_TICKS`] observed ticks from the first
    /// wait. The result says what was found; the caller logs its one line
    /// ([`Loaded::log`]).
    pub(super) fn load(
        &mut self,
        state: Option<&SlotState>,
        tick: u64,
        kept: GridRun,
    ) -> Poll<Result<(GridMemory, Loaded), ActionError>> {
        let run_memory = kept.kept();
        let Some(state) = state else {
            self.mode = Mode::MemoryOnly;
            return Poll::Ready(Ok((run_memory.unwrap_or_default(), Loaded::NoCapability)));
        };
        let identity = state.identity(LocalStateDomain::Knowledge);
        let ticket = match self.ticket {
            Some(ticket) => ticket,
            None => match state.port().start_knowledge_read(&identity) {
                Ok(ticket) => *self.ticket.insert(ticket),
                Err(code) if waits(code) => return self.wait(tick),
                Err(code) => return self.refused(code, run_memory),
            },
        };
        let stored = match state.port().poll_knowledge_read(
            &identity,
            ticket,
            &KnowledgeKey::upass_grid_route(),
        ) {
            None => return self.wait(tick),
            Some(Err(code)) if waits(code) => {
                // The read ended without a result (an evicted cache entry, a
                // stopping service): queue another within the same wait.
                self.ticket = None;
                return self.wait(tick);
            }
            Some(Err(code)) => return self.refused(code, run_memory),
            Some(Ok(stored)) => stored,
        };
        self.mode = Mode::Saving;
        self.ticket = None;
        self.expected = stored
            .as_ref()
            .map_or(StoreVersion::INITIAL, |stored| stored.version());
        if let Some(memory) = run_memory {
            if kept.unsaved() {
                self.changed();
            }
            return Poll::Ready(Ok((
                memory,
                Loaded::Kept {
                    columns: memory.confirmed(),
                    traps: memory.traps(),
                    unsaved: kept.unsaved(),
                },
            )));
        }
        Poll::Ready(Ok(match stored {
            None => (GridMemory::default(), Loaded::Absent),
            Some(stored) => match decode(stored.record()) {
                Decoded::Route(memory) => (memory, Loaded::Saved(memory.confirmed())),
                Decoded::Invalidated => (GridMemory::default(), Loaded::Invalidated),
                Decoded::Corrupt => (GridMemory::default(), Loaded::Corrupt),
            },
        }))
    }

    fn wait(&mut self, tick: u64) -> Poll<Result<(GridMemory, Loaded), ActionError>> {
        let since = *self.load_since.get_or_insert(tick);
        if tick.saturating_sub(since) > LOAD_TICKS {
            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                "Underground Pass grid: the account's saved route was not readable \
                 within the wait; it cannot be told from a route not yet learned",
            ))));
        }
        Poll::Pending
    }

    fn refused(
        &mut self,
        code: ErrorCode,
        run_memory: Option<GridMemory>,
    ) -> Poll<Result<(GridMemory, Loaded), ActionError>> {
        self.mode = Mode::MemoryOnly;
        self.ticket = None;
        Poll::Ready(Ok((run_memory.unwrap_or_default(), Loaded::Refused(code))))
    }

    /// One tick of writing: observe the write in flight, then send the memory
    /// again if it changed. Never blocks, never sends while a write is in
    /// flight, and sends a failed or refused write again once per observed
    /// tick, with its original stamp and expected version.
    pub(super) fn pump(&mut self, state: Option<&SlotState>, tick: u64, memory: &GridMemory) {
        let (Mode::Saving, Some(state)) = (self.mode, state) else {
            return;
        };
        if let Some(request) = self.pending {
            match state.port().observe(request) {
                Some(Settlement::Completed(receipt)) => {
                    self.pending = None;
                    self.failures = 0;
                    self.gave_up = false;
                    self.expected = receipt.version();
                }
                Some(Settlement::Failed(ErrorCode::Conflict { expected, found })) => {
                    self.pending = None;
                    self.conflict(state, expected, found);
                    return;
                }
                Some(Settlement::Failed(code)) => {
                    self.pending = None;
                    self.dirty = true;
                    self.fail(state, tick, format_args!("{code}"));
                }
                Some(Settlement::Cancelled(reason)) => {
                    self.pending = None;
                    self.dirty = true;
                    self.fail(state, tick, format_args!("cancelled ({reason:?})"));
                }
                // Not settled yet: the same request is followed on.
                None => return,
            }
        }
        if self.mode != Mode::Saving || !self.dirty || tick < self.retry_at {
            return;
        }
        let record = match record_of(state, memory, self.expected, self.observed) {
            Ok(record) => record,
            Err(code) => {
                self.mode = Mode::MemoryOnly;
                host_log!(
                    Category::ScriptTrace,
                    Level::Warn,
                    "quester upass grid: the route cannot be written ({code}); it is learned for this run only"
                );
                return;
            }
        };
        match state.port().submit(DomainEdit::Knowledge(record)) {
            SubmitReceipt::Accepted { request } => {
                self.pending = Some(request);
                self.dirty = false;
            }
            SubmitReceipt::Refused(ErrorCode::Conflict { expected, found }) => {
                self.conflict(state, expected, found);
            }
            SubmitReceipt::Refused(code) => {
                self.fail(state, tick, format_args!("refused: {code}"));
            }
        }
    }

    /// The line the step logs when it ends with its last write unsettled after
    /// `waited` observed ticks: the route stays in the run's memory (flagged
    /// unsaved there), and the line says so. The accepted request is still the
    /// service's; it reaches its own terminal outcome there. The step sends it
    /// to the script's log, so the operator sees it without debug output.
    pub(super) fn unconfirmed_line(state: Option<&SlotState>, waited: u64) -> String {
        format!(
            "quester upass grid: the route of {} is not confirmed saved after {waited} ticks; it is kept in this run's memory and sent again by the run's next step",
            state.map_or("", |state| state.account().account())
        )
    }

    /// The fact changed under the run (another process wrote it, or it was
    /// invalidated) after the version this write expected: the run's copy is
    /// not written over it, now or by a later step of the run.
    fn conflict(&mut self, state: &SlotState, expected: StoreVersion, found: StoreVersion) {
        self.mode = Mode::MemoryOnly;
        self.dirty = false;
        self.gave_up = false;
        host_log!(
            Category::ScriptTrace,
            Level::Warn,
            "quester upass grid: the saved route of {} changed elsewhere (expected version {expected}, found {found}); it is not replaced, and the route is kept for this run only",
            state.account()
        );
    }

    fn fail(&mut self, state: &SlotState, tick: u64, why: std::fmt::Arguments<'_>) {
        self.failures += 1;
        self.retry_at = tick + 1;
        if self.failures >= MAX_WRITE_FAILURES {
            self.mode = Mode::MemoryOnly;
            self.gave_up = true;
            host_log!(
                Category::ScriptTrace,
                Level::Warn,
                "quester upass grid: the route of {} was not saved {} times in a row ({why}); it is kept for this run only",
                state.account(),
                self.failures
            );
        } else {
            host_log!(
                Category::ScriptTrace,
                Level::Warn,
                "quester upass grid: the route of {} was not saved ({why}); trying again",
                state.account()
            );
        }
    }
}

/// Whether a read refusal is one to wait out: a cold or stopping store, or a
/// transient capacity refusal.
fn waits(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::NotReady { .. })
        || code.recoverability() == Some(Recoverability::TransientCapacity)
}

/// The fact that saves `memory` for the slot's account, replacing `expected`
/// and stamped `observed`. A cleared route is an `Unknown` tombstone.
fn record_of(
    state: &SlotState,
    memory: &GridMemory,
    expected: StoreVersion,
    observed: DurableTimestamp,
) -> Result<KnowledgeRecord, ErrorCode> {
    let empty = memory.is_empty();
    let payload = memory.to_payload();
    let payload =
        validate_knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, if empty { &[] } else { &payload })?;
    let status = if empty {
        KnowledgeStatus::Unknown
    } else {
        KnowledgeStatus::Known
    };
    let record = KnowledgeRecord::try_new(
        state.account().clone(),
        state.scope().clone(),
        expected,
        KEY,
        KnowledgeProvenance::Observed,
        status,
        payload,
        observed,
    )?;
    Ok(if empty {
        record.invalidated_with(expected, observed, KnowledgeRevocation::ObservationRefused)
    } else {
        record
    })
}

/// What a stored fact holds for the step.
enum Decoded {
    Route(GridMemory),
    Invalidated,
    Corrupt,
}

/// The memory a stored fact holds. `Known` and `Partial` carry a route; an
/// `Unknown` fact (a tombstone or an empty route) holds nothing.
fn decode(record: &KnowledgeRecord) -> Decoded {
    match record.status() {
        KnowledgeStatus::Unknown => Decoded::Invalidated,
        KnowledgeStatus::Known | KnowledgeStatus::Partial => {
            GridMemory::from_payload(record.payload()).map_or(Decoded::Corrupt, Decoded::Route)
        }
    }
}

/// The step's saved-route port, exposed for the host's SQLite tests
/// (`host-play` `local_state::upass_route_tests`): the same reads and writes
/// the step makes, against the real service.
#[cfg(feature = "test-hooks")]
pub mod probe {
    use super::*;

    /// The run's route as the store reads and writes it.
    pub struct RouteProbe {
        store: RouteStore,
        memory: GridMemory,
    }

    /// What a read found, without the step's types.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ProbeLoaded {
        Saved(usize),
        Absent,
        Invalidated,
        Corrupt,
        Refused(ErrorCode),
    }

    impl Default for RouteProbe {
        fn default() -> Self {
            Self {
                store: RouteStore::loading(),
                memory: GridMemory::default(),
            }
        }
    }

    impl RouteProbe {
        /// Read the saved route at `tick`; `Pending` while it reads.
        pub fn load(
            &mut self,
            state: &SlotState,
            tick: u64,
        ) -> Poll<Result<ProbeLoaded, ActionError>> {
            self.store
                .load(Some(state), tick, GridRun::default())
                .map_ok(|(memory, loaded)| {
                    self.memory = memory;
                    match loaded {
                        Loaded::Saved(columns) => ProbeLoaded::Saved(columns),
                        Loaded::Absent => ProbeLoaded::Absent,
                        Loaded::Invalidated => ProbeLoaded::Invalidated,
                        Loaded::Corrupt => ProbeLoaded::Corrupt,
                        Loaded::Refused(code) => ProbeLoaded::Refused(code),
                        Loaded::NoCapability | Loaded::Kept { .. } => {
                            unreachable!("a probe always has a capability and no run copy")
                        }
                    }
                })
        }

        /// The six saved bytes of the route read or learned.
        pub fn payload(&self) -> [u8; super::super::upass::PAYLOAD_LEN] {
            self.memory.to_payload()
        }

        /// Learn the route a step writes from `payload` (a history it can write).
        pub fn learn(&mut self, payload: [u8; super::super::upass::PAYLOAD_LEN]) {
            self.memory = GridMemory::from_payload(&payload).expect("a route the step writes");
            self.store.changed();
        }

        /// One tick of writing.
        pub fn pump(&mut self, state: &SlotState, tick: u64) {
            let memory = self.memory;
            self.store.pump(Some(state), tick, &memory);
        }

        /// Nothing waits to be written or settled.
        pub fn settled(&self) -> bool {
            self.store.settled()
        }

        /// The store may lack the route and a later step owes it a write.
        pub fn unsaved(&self) -> bool {
            self.store.unsaved()
        }

        /// The version the next write replaces.
        pub fn expected(&self) -> StoreVersion {
            self.store.expected
        }
    }
}

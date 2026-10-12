//! Cooperative work admission (F0 item 4: contract §26 §2).
//!
//! Frozen here: the [`WorkFamily`] table, identity/revision envelope,
//! [`WorkCharge`], [`try_submit`](AdmissionLedger::try_submit) refusals,
//! accepted-job states, [`Checkpoint`] / [`StepContext`] obligations,
//! cancellation, [`WorkProgress`], and the once-published
//! [`identity::Settlement`] terminal. Numerical quanta, fairness and the
//! live worker are T1.2 / G2b. Operational cutover is T5.W / I-work-prep.
//!
//! This module is vocabulary plus a protocol ledger that tests (and later
//! T1.2) drive. It does not spawn a worker, sleep, or stand in for a
//! production executor (§26 §2.6).
//!
//! Current producers this vocabulary will migrate (characterization, not
//! cooperation): T5 map bake `run_job` (`crates/host-play/src/map_cache.rs:1719-1732,1960-1988`,
//! `MapJobStatus`/`MapProgress` at `:844-868,1939-1946,2045-2052`); T4 walk-map
//! decode (`crates/panel/src/walk_map.rs:58,1026-1038,1165-1186`); card
//! preparation (`crates/api/src/selected.rs:310-330`,
//! [`crate::selected::FamilyPreparation`] is thread-bound and must not
//! survive a yield).

use std::collections::HashMap;
use std::fmt;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Instant;

use crate::identity::{
    Cancellation, Deadline, ProcessIncarnation, RequestId, RequestNamespace, Scope, Settlement,
    Terminal, TerminalError, WorkIdentity,
};
use crate::process_condition::{
    ProcessCapability, ProcessConditions, UnavailableRefusal, UnavailableTerminal,
};

/// Registered work family (§26 §2.2 family table). Missing classifications
/// prevent F0 completion; each row names the current producer/consumer
/// owners. Domain adapters preserve current observable outcomes at T5.W.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkFamily {
    /// Route search. Producer T5; consumers T1/T2. Activation T5.W(route)
    /// after T1.2 and T5.1.
    Route,
    /// Inspection search. Producer T5. Activation T5.W(inspection).
    Inspection,
    /// Bank-pick search, including T1-owned `script_bank`. Activation
    /// T5.W(bank-pick).
    BankPick,
    /// Map bake/publish. Producer T5; files stay outside SQL. Activation
    /// T5.W(map-bake). Interruptible unit: one planned bake unit (today
    /// `MapProgress.stage`).
    MapBake,
    /// Walk-map decode. Producer T4 panel; T3 core requests. Activation
    /// T5.W(walk-map-decode). Today's PNG length check is not a time bound
    /// (`crates/panel/src/walk_map.rs:58,1165-1178`).
    WalkMapDecode,
    /// Native card preparation. Producer T1 (I-work-prep); T3 callers.
    /// [`crate::selected::FamilyPreparation`] cannot be retained across a
    /// yield (`selected.rs:310-328`).
    CardPreparation,
    /// Catalogue preparation. Same I-work-prep unit as card/profile.
    CataloguePreparation,
    /// Profile preparation. Same I-work-prep unit as card/catalogue.
    ProfilePreparation,
}

impl WorkFamily {
    /// Every registered family.
    pub const ALL: [WorkFamily; 8] = [
        WorkFamily::Route,
        WorkFamily::Inspection,
        WorkFamily::BankPick,
        WorkFamily::MapBake,
        WorkFamily::WalkMapDecode,
        WorkFamily::CardPreparation,
        WorkFamily::CataloguePreparation,
        WorkFamily::ProfilePreparation,
    ];

    /// Track that produces this family's job (physical owner of the
    /// operational unit).
    pub const fn producer(self) -> &'static str {
        match self {
            Self::Route | Self::Inspection | Self::BankPick | Self::MapBake => "T5",
            Self::WalkMapDecode => "T4",
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => "T1",
        }
    }

    /// Track that consumes the family's result.
    pub const fn consumer(self) -> &'static str {
        match self {
            Self::Route | Self::Inspection | Self::BankPick => "T2",
            Self::MapBake => "T3",
            Self::WalkMapDecode => "T3",
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => "T3",
        }
    }

    /// Smallest interruptible unit declared for this family. Numerical
    /// allowances are G2b, not this string.
    pub const fn interruptible_unit(self) -> &'static str {
        match self {
            Self::Route | Self::Inspection | Self::BankPick => {
                "one declared search slice (bounded expansions)"
            }
            Self::MapBake => "one planned bake unit",
            Self::WalkMapDecode => "one bounded decode unit (not PNG-length alone)",
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                "one preparation phase"
            }
        }
    }
}

/// Family input revision: a newer revision makes an older result stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputRevision(u64);

impl InputRevision {
    /// First revision.
    pub const FIRST: Self = Self(0);

    /// A revision from its raw value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next revision, wrapping at `u64::MAX` is refused by the caller
    /// using identity exhaustion if they mint through a checked add; this
    /// type itself is a stamp, not a minter.
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }
}

/// Identity-bound ticket returned by a successful submit: request, family
/// and input revision. Display names and table indices are not identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkTicket {
    identity: WorkIdentity,
    family: WorkFamily,
    revision: InputRevision,
}

impl WorkTicket {
    /// Compose a ticket. Nested identities must already agree on
    /// [`WorkIdentity`].
    pub const fn new(identity: WorkIdentity, family: WorkFamily, revision: InputRevision) -> Self {
        Self {
            identity,
            family,
            revision,
        }
    }

    /// Job correlation envelope.
    pub const fn identity(self) -> WorkIdentity {
        self.identity
    }

    /// Registered family.
    pub const fn family(self) -> WorkFamily {
        self.family
    }

    /// Input revision.
    pub const fn revision(self) -> InputRevision {
        self.revision
    }

    /// Request id.
    pub const fn request(self) -> RequestId {
        self.identity.request()
    }

    /// Originating scope.
    pub const fn scope(self) -> Scope {
        self.identity.scope()
    }
}

/// Checked upper bound reserved at submit: retained input/state bytes, peak
/// scratch bytes, terminal/result bytes and operation handles. Immutable
/// backing leases stay charged in their owner ledger; they are not copied
/// per job. Input size alone is not the charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkCharge {
    input_bytes: usize,
    scratch_bytes: usize,
    result_bytes: usize,
    operation_handles: u32,
}

impl WorkCharge {
    /// A charge of the four bounded quantities.
    pub const fn new(
        input_bytes: usize,
        scratch_bytes: usize,
        result_bytes: usize,
        operation_handles: u32,
    ) -> Self {
        Self {
            input_bytes,
            scratch_bytes,
            result_bytes,
            operation_handles,
        }
    }

    /// Input/state bytes.
    pub const fn input_bytes(self) -> usize {
        self.input_bytes
    }

    /// Peak scratch bytes.
    pub const fn scratch_bytes(self) -> usize {
        self.scratch_bytes
    }

    /// Terminal/result bytes.
    pub const fn result_bytes(self) -> usize {
        self.result_bytes
    }

    /// Operation handles (close before yield; §21 active-operation
    /// accounting).
    pub const fn operation_handles(self) -> u32 {
        self.operation_handles
    }

    /// Bytes counted against the ledger (input + scratch + result).
    pub const fn total_bytes(self) -> usize {
        self.input_bytes
            .saturating_add(self.scratch_bytes)
            .saturating_add(self.result_bytes)
    }
}

/// Finite admission bounds. Every limit is nonzero; a caller cannot
/// request an unlimited slice or an unbounded job table. Numerical values
/// are G2b policy; F0 only freezes that they are finite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionBounds {
    max_jobs: NonZeroUsize,
    max_bytes: NonZeroUsize,
    max_handles: NonZeroU32,
}

impl AdmissionBounds {
    /// Finite, nonzero job/byte/handle caps.
    pub const fn new(
        max_jobs: NonZeroUsize,
        max_bytes: NonZeroUsize,
        max_handles: NonZeroU32,
    ) -> Self {
        Self {
            max_jobs,
            max_bytes,
            max_handles,
        }
    }

    /// Job-slot cap (queued + executing + yielded + unconsumed terminal).
    pub const fn max_jobs(self) -> NonZeroUsize {
        self.max_jobs
    }

    /// Byte cap, including retained yielded/terminal state.
    pub const fn max_bytes(self) -> NonZeroUsize {
        self.max_bytes
    }

    /// Operation-handle cap.
    pub const fn max_handles(self) -> NonZeroU32 {
        self.max_handles
    }
}

/// Why [`AdmissionLedger::try_submit`] refused. Full is temporary capacity.
/// Unavailable means process restart. Neither is accepted, queued elsewhere
/// or completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubmissionRefusal {
    /// Temporary capacity pressure: retry after a capacity notice or a
    /// later ordinary demand turn. Not Unavailable.
    #[error("work admission full")]
    Full,
    /// The service is stopping; no new job is accepted.
    #[error("work admission stopping")]
    Stopping,
    /// The identity does not belong to this incarnation or is not
    /// authorised for the family.
    #[error("work admission invalid authority")]
    InvalidAuthority,
    /// The request cannot be represented (duplicate id, unusable charge).
    #[error("work admission invalid argument")]
    InvalidArgument,
    /// Process-level [`ProcessServiceCode::WorkAdmissionUnavailable`].
    #[error(transparent)]
    Unavailable(UnavailableRefusal),
}

/// Accepted-job state: `Queued → Executing → Yielded → Executing`, then
/// exactly one terminal. Yielded is a resumable cursor, not cancellation,
/// not operator-run pause, not restart from the first phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobState {
    /// Accepted, not yet executing.
    Queued,
    /// The worker is inside a slice.
    Executing,
    /// The worker owns a resumable cursor; the job is eligible for later
    /// dispatch.
    Yielded,
}

/// Finite slice allowance. Nonzero units and an absolute monotonic
/// deadline; never unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceBudget {
    units: NonZeroU32,
    deadline: Deadline,
}

impl SliceBudget {
    /// A finite slice.
    pub const fn new(units: NonZeroU32, deadline: Deadline) -> Self {
        Self { units, deadline }
    }

    /// Work-unit allowance for this invocation.
    pub const fn units(self) -> NonZeroU32 {
        self.units
    }

    /// Absolute monotonic slice deadline.
    pub const fn deadline(self) -> Deadline {
        self.deadline
    }
}

/// Checkpoint result. A job must honour it; calling checkpoint and
/// ignoring Yield is not cooperation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    /// Perform at most one more declared unit, then checkpoint again.
    Continue,
    /// Save the next cursor and return [`StepOutcome::Yielded`] before
    /// another unit.
    Yield,
    /// Stop new work and enter the cancellation terminal path. Cleanup
    /// itself stays bounded.
    Cancel(Cancellation),
}

/// Outcome of one [`CooperativeJob::step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome<R, E> {
    /// Cursor saved; eligible for later dispatch.
    Yielded,
    /// Family result. Domain outcomes such as NoPath stay in `R`.
    Completed(R),
    /// Family error.
    Failed(E),
}

/// Borrowed only for this `step` invocation. The job must not retain it
/// across a yield.
#[derive(Debug)]
pub struct StepContext<'a> {
    now: Instant,
    deadline: Deadline,
    remaining_units: u32,
    cancel: Option<Cancellation>,
    units_this_slice: u32,
    yield_seen: bool,
    continued_after_yield: bool,
    _borrow: std::marker::PhantomData<&'a ()>,
}

impl<'a> StepContext<'a> {
    /// A context for one slice. `cancel` is the revocation recorded on the
    /// ticket, if any.
    pub fn new(now: Instant, budget: SliceBudget, cancel: Option<Cancellation>) -> Self {
        Self {
            now,
            deadline: budget.deadline,
            remaining_units: budget.units.get(),
            cancel,
            units_this_slice: 0,
            yield_seen: false,
            continued_after_yield: false,
            _borrow: std::marker::PhantomData,
        }
    }

    /// Observe cancellation and the current slice budget.
    pub fn checkpoint(&mut self) -> Checkpoint {
        let decision = if let Some(reason) = self.cancel {
            Checkpoint::Cancel(reason)
        } else if self.deadline.expired(self.now) || self.remaining_units == 0 {
            Checkpoint::Yield
        } else {
            Checkpoint::Continue
        };
        if !matches!(decision, Checkpoint::Continue) {
            self.yield_seen = true;
        }
        decision
    }

    /// Consume one work unit from the allowance, then checkpoint.
    pub fn consume_unit(&mut self) -> Checkpoint {
        if self.yield_seen {
            self.continued_after_yield = true;
        }
        self.units_this_slice = self.units_this_slice.saturating_add(1);
        self.remaining_units = self.remaining_units.saturating_sub(1);
        self.checkpoint()
    }

    /// Units consumed during this invocation.
    pub const fn units_this_slice(&self) -> u32 {
        self.units_this_slice
    }

    /// Whether this invocation already saw Yield/Cancel and then did more
    /// work.
    pub const fn continued_after_yield(&self) -> bool {
        self.continued_after_yield
    }

    /// Whether more than one unit was consumed in this slice.
    pub const fn exceeded_one_unit(&self) -> bool {
        self.units_this_slice > 1
    }

    /// The `now` this slice was given. Tests inject a fake clock by
    /// passing a chosen instant.
    pub const fn now(&self) -> Instant {
        self.now
    }

    /// The slice deadline.
    pub const fn deadline(&self) -> Deadline {
        self.deadline
    }
}

/// What may survive a yield (§26 §2.4). Forbidden items fail the job with
/// a typed cooperation violation; they are not silently kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForbiddenRetain {
    /// A queue/UI/slot mutex guard.
    QueueMutexGuard,
    /// A mutable Client/world/script/UI borrow.
    ClientBorrow,
    /// A frame/render-workspace lease.
    FrameLease,
    /// A V8 handle.
    V8Handle,
    /// A live store transaction.
    StoreTransaction,
    /// A borrowed step context or stack reference.
    StepContextBorrow,
    /// A live operation file handle.
    LiveFileHandle,
    /// Responsibility for another owner's resource, including
    /// thread-bound [`crate::selected::FamilyPreparation`].
    OtherOwnerResource,
}

/// Allowed retained state across a yield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllowedRetain {
    /// Owned cursor/counter.
    Cursor,
    /// Bounded job-private scratch.
    Scratch,
    /// Immutable input/result lease.
    ImmutableLease,
    /// Typed identity/revision.
    Identity,
    /// Cancellation state.
    CancellationState,
    /// Path/checkpoint receipt for a partial on-disk artifact.
    CheckpointReceipt,
}

/// What the job still holds when it returns Yielded. All retained bytes
/// stay charged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetainedState {
    forbidden: Option<ForbiddenRetain>,
    bytes: usize,
}

impl RetainedState {
    /// No forbidden items; `bytes` remain charged.
    pub const fn clean(bytes: usize) -> Self {
        Self {
            forbidden: None,
            bytes,
        }
    }

    /// A forbidden retain; the protocol fails the job.
    pub const fn forbidden(kind: ForbiddenRetain) -> Self {
        Self {
            forbidden: Some(kind),
            bytes: 0,
        }
    }

    /// Forbidden item, if any.
    pub const fn forbidden_item(self) -> Option<ForbiddenRetain> {
        self.forbidden
    }

    /// Retained bytes still charged.
    pub const fn bytes(self) -> usize {
        self.bytes
    }
}

/// A typed cooperation violation. It fails that job; it does not silently
/// monopolise another slice. A worker that never returns uses
/// [`crate::process_condition::ProcessServiceCode::WorkAdmissionUnavailable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CooperativeViolation {
    /// Yielded state carried a prohibited guard/borrow/handle.
    ForbiddenRetain(ForbiddenRetain),
    /// More than one declared unit in one slice, or work after Yield.
    UnboundedSlice,
    /// Checkpoint returned Yield and the job started another unit.
    IgnoredYield,
    /// Retained bytes exceeded the reserved charge.
    ChargeExceeded,
}

impl fmt::Display for CooperativeViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForbiddenRetain(kind) => {
                write!(f, "cooperative job retained forbidden state {kind:?}")
            }
            Self::UnboundedSlice => f.write_str("cooperative job exceeded its slice unit"),
            Self::IgnoredYield => f.write_str("cooperative job ignored checkpoint Yield"),
            Self::ChargeExceeded => f.write_str("cooperative job retained more than its charge"),
        }
    }
}

impl std::error::Error for CooperativeViolation {}

/// Error side of a job terminal: family error, cooperation, or the
/// process-unavailable failure of an already-accepted job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError<E> {
    /// Family-typed error.
    Family(E),
    /// The step violated the cooperative bound.
    Cooperation(CooperativeViolation),
    /// Accepted job settled because WorkAdmission became unavailable.
    Unavailable(UnavailableTerminal),
}

/// Immediate result of [`AdmissionLedger::cancel`]. Requested is not proof
/// the worker returned or released resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// Revocation recorded; the service will not publish a late success.
    Requested,
    /// A terminal was already published; cancellation cannot rewrite it.
    AlreadyTerminal,
    /// Unknown or disagreeing identity/revision.
    StaleIdentity,
}

/// Bounded, replaceable progress snapshot. Sequence increases within the
/// same job. Totals may be absent. Progress is not an acknowledgement or
/// terminal. Non-replaceable domain facts use their existing
/// receipt/event protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkProgress {
    identity: WorkIdentity,
    sequence: u64,
    phase: u16,
    completed: u32,
    total: Option<u32>,
}

impl WorkProgress {
    /// A progress snapshot. `sequence` must increase for later snapshots
    /// of the same job.
    pub const fn new(
        identity: WorkIdentity,
        sequence: u64,
        phase: u16,
        completed: u32,
        total: Option<u32>,
    ) -> Self {
        Self {
            identity,
            sequence,
            phase,
            completed,
            total,
        }
    }

    /// Job identity.
    pub const fn identity(&self) -> WorkIdentity {
        self.identity
    }

    /// Sequence within this job.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Family-defined phase.
    pub const fn phase(&self) -> u16 {
        self.phase
    }

    /// Completed units of `total`, if known.
    pub const fn completed(&self) -> u32 {
        self.completed
    }

    /// Total units, or `None` when unknown.
    pub const fn total(&self) -> Option<u32> {
        self.total
    }
}

/// One cooperative job. The context is borrowed for this invocation only.
pub trait CooperativeJob {
    /// Family result.
    type Output;
    /// Family error.
    type Error;

    /// Perform at most one declared interruptible unit. Check cancellation
    /// and the budget at entry, at each phase boundary, and before every
    /// externally observable publish.
    fn step(&mut self, ctx: &mut StepContext<'_>) -> StepOutcome<Self::Output, Self::Error>;

    /// What still lives on the job after Yielded. Forbidden items fail
    /// the job.
    fn retained(&self) -> RetainedState;
}

struct JobSlot<R, E> {
    ticket: WorkTicket,
    state: JobState,
    charge: WorkCharge,
    cancel: Option<Cancellation>,
    terminal: Terminal<Settlement<R, JobError<E>>>,
    progress: Option<WorkProgress>,
    worker_returned: bool,
    abandoned: bool,
}

/// Protocol ledger for one process incarnation: bounded admission,
/// cancellation-versus-publication order, yield retention checks, and
/// charge lifetime. T1.2 wraps this with a worker and numerical policy.
/// Tests drive it with controlled jobs and injected clocks.
pub struct AdmissionLedger<R, E> {
    incarnation: ProcessIncarnation,
    bounds: AdmissionBounds,
    stopping: bool,
    jobs: HashMap<RequestId, JobSlot<R, E>>,
}

impl<R, E> AdmissionLedger<R, E> {
    /// An empty ledger for `incarnation` with finite bounds.
    pub fn new(incarnation: ProcessIncarnation, bounds: AdmissionBounds) -> Self {
        Self {
            incarnation,
            bounds,
            stopping: false,
            jobs: HashMap::new(),
        }
    }

    /// The incarnation this ledger admits for.
    pub const fn incarnation(&self) -> ProcessIncarnation {
        self.incarnation
    }

    /// Refuse further submits.
    pub fn begin_stop(&mut self) {
        self.stopping = true;
    }

    /// Bytes currently charged (queued, executing, yielded, unconsumed
    /// terminal, and abandoned-but-not-returned).
    pub fn charged_bytes(&self) -> usize {
        self.jobs
            .values()
            .map(|slot| slot.charge.total_bytes())
            .fold(0usize, usize::saturating_add)
    }

    /// Job records currently charged.
    pub fn charged_jobs(&self) -> usize {
        self.jobs.len()
    }

    /// Handles currently charged.
    pub fn charged_handles(&self) -> u32 {
        self.jobs
            .values()
            .map(|slot| slot.charge.operation_handles())
            .fold(0u32, u32::saturating_add)
    }

    /// Whether `ticket` still occupies charge.
    pub fn is_charged(&self, ticket: &WorkTicket) -> bool {
        self.jobs.contains_key(&ticket.request())
    }

    /// Current non-terminal state, if the ticket is live and not yet
    /// published.
    pub fn job_state(&self, ticket: &WorkTicket) -> Option<JobState> {
        let slot = self.jobs.get(&ticket.request())?;
        if slot.ticket != *ticket {
            return None;
        }
        if slot.terminal.borrow().is_some() {
            return None;
        }
        Some(slot.state)
    }

    /// Recorded cancellation, if any.
    pub fn cancellation(&self, ticket: &WorkTicket) -> Option<Cancellation> {
        let slot = self.jobs.get(&ticket.request())?;
        if slot.ticket != *ticket {
            return None;
        }
        slot.cancel
    }

    /// Latest progress snapshot, if any.
    pub fn progress(&self, ticket: &WorkTicket) -> Option<&WorkProgress> {
        let slot = self.jobs.get(&ticket.request())?;
        if slot.ticket != *ticket {
            return None;
        }
        slot.progress.as_ref()
    }

    /// Borrow the retained terminal without consuming it.
    pub fn borrow_terminal(&self, ticket: &WorkTicket) -> Option<&Settlement<R, JobError<E>>> {
        let slot = self.jobs.get(&ticket.request())?;
        if slot.ticket != *ticket {
            return None;
        }
        slot.terminal.borrow()
    }

    fn admit_capacity(&self, charge: WorkCharge) -> Result<(), SubmissionRefusal> {
        if self.jobs.len() >= self.bounds.max_jobs.get() {
            return Err(SubmissionRefusal::Full);
        }
        let bytes = self.charged_bytes().saturating_add(charge.total_bytes());
        if bytes > self.bounds.max_bytes.get() {
            return Err(SubmissionRefusal::Full);
        }
        let handles = self
            .charged_handles()
            .saturating_add(charge.operation_handles());
        if handles > self.bounds.max_handles.get() {
            return Err(SubmissionRefusal::Full);
        }
        Ok(())
    }

    /// Never waits for capacity or executes a job on the caller. Acceptance
    /// reserves a job slot, memory and terminal capacity together and
    /// returns an identity-bound ticket. A refusal retains the job with the
    /// caller and creates no accepted-job terminal.
    pub fn try_submit(
        &mut self,
        identity: WorkIdentity,
        family: WorkFamily,
        revision: InputRevision,
        charge: WorkCharge,
        conditions: Option<&ProcessConditions>,
    ) -> Result<WorkTicket, SubmissionRefusal> {
        if identity.incarnation() != self.incarnation {
            return Err(SubmissionRefusal::InvalidAuthority);
        }
        // §26 §2.2: work ids are unique within the process incarnation.
        // Isolate RequestIds are unique per run; F0-1/2 H4 refuses them
        // here so a job cannot share an isolate handle.
        if identity.request().namespace() != RequestNamespace::Host {
            return Err(SubmissionRefusal::InvalidArgument);
        }
        if let Some(conditions) = conditions {
            if let Err(refusal) = conditions.admit(
                ProcessCapability::BackgroundWork,
                Some(identity.request()),
                identity.scope(),
            ) {
                return Err(SubmissionRefusal::Unavailable(refusal));
            }
        }
        if self.stopping {
            return Err(SubmissionRefusal::Stopping);
        }
        if self.jobs.contains_key(&identity.request()) {
            return Err(SubmissionRefusal::InvalidArgument);
        }
        self.admit_capacity(charge)?;
        let ticket = WorkTicket::new(identity, family, revision);
        self.jobs.insert(
            identity.request(),
            JobSlot {
                ticket,
                state: JobState::Queued,
                charge,
                cancel: None,
                terminal: Terminal::new(),
                progress: None,
                worker_returned: false,
                abandoned: false,
            },
        );
        Ok(ticket)
    }

    /// Idempotent and identity-scoped. Records revocation; does not prove
    /// the worker returned.
    pub fn cancel(&mut self, ticket: WorkTicket, reason: Cancellation) -> CancelOutcome {
        let Some(slot) = self.jobs.get_mut(&ticket.request()) else {
            return CancelOutcome::StaleIdentity;
        };
        if slot.ticket != ticket {
            return CancelOutcome::StaleIdentity;
        }
        if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
            return CancelOutcome::AlreadyTerminal;
        }
        slot.cancel = Some(reason);
        CancelOutcome::Requested
    }

    /// Replaceable progress. A lower-or-equal sequence is ignored so
    /// coalescing cannot walk backwards. Progress never takes terminal
    /// custody.
    pub fn publish_progress(
        &mut self,
        ticket: &WorkTicket,
        progress: WorkProgress,
    ) -> Result<(), CancelOutcome> {
        let Some(slot) = self.jobs.get_mut(&ticket.request()) else {
            return Err(CancelOutcome::StaleIdentity);
        };
        if slot.ticket != *ticket {
            return Err(CancelOutcome::StaleIdentity);
        }
        if progress.identity() != ticket.identity() {
            return Err(CancelOutcome::StaleIdentity);
        }
        if slot
            .progress
            .as_ref()
            .is_some_and(|current| progress.sequence() <= current.sequence())
        {
            return Ok(());
        }
        slot.progress = Some(progress);
        Ok(())
    }

    fn publish_settlement(
        slot: &mut JobSlot<R, E>,
        settlement: Settlement<R, JobError<E>>,
    ) -> Result<(), TerminalError> {
        if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
            return Err(TerminalError::AlreadyPublished);
        }
        if let (Settlement::Completed(_), Some(reason)) = (&settlement, slot.cancel) {
            return slot.terminal.publish(Settlement::Cancelled(reason));
        }
        slot.terminal.publish(settlement)
    }

    /// Drive one slice of `job` for `ticket`. Injected `now` is the fake
    /// clock. After the step the service re-checks cancellation before a
    /// success publication.
    pub fn drive<J>(
        &mut self,
        ticket: &WorkTicket,
        job: &mut J,
        now: Instant,
        budget: SliceBudget,
    ) -> Result<Option<JobState>, CancelOutcome>
    where
        J: CooperativeJob<Output = R, Error = E>,
    {
        let Some(slot) = self.jobs.get_mut(&ticket.request()) else {
            return Err(CancelOutcome::StaleIdentity);
        };
        if slot.ticket != *ticket {
            return Err(CancelOutcome::StaleIdentity);
        }
        if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
            return Ok(None);
        }
        slot.state = JobState::Executing;
        let mut ctx = StepContext::new(now, budget, slot.cancel);
        let outcome = job.step(&mut ctx);
        if let Some(reason) = slot.cancel {
            if matches!(outcome, StepOutcome::Completed(_) | StepOutcome::Yielded) {
                let _ = Self::publish_settlement(slot, Settlement::Cancelled(reason));
                return Ok(None);
            }
        }
        if ctx.continued_after_yield() {
            let _ = Self::publish_settlement(
                slot,
                Settlement::Failed(JobError::Cooperation(CooperativeViolation::IgnoredYield)),
            );
            return Ok(None);
        }
        if ctx.exceeded_one_unit() {
            let _ = Self::publish_settlement(
                slot,
                Settlement::Failed(JobError::Cooperation(CooperativeViolation::UnboundedSlice)),
            );
            return Ok(None);
        }
        match outcome {
            StepOutcome::Yielded => {
                if let Some(reason) = slot.cancel {
                    let _ = Self::publish_settlement(slot, Settlement::Cancelled(reason));
                    return Ok(None);
                }
                let retained = job.retained();
                if let Some(kind) = retained.forbidden_item() {
                    let _ = Self::publish_settlement(
                        slot,
                        Settlement::Failed(JobError::Cooperation(
                            CooperativeViolation::ForbiddenRetain(kind),
                        )),
                    );
                    return Ok(None);
                }
                if retained.bytes() > slot.charge.total_bytes() {
                    let _ = Self::publish_settlement(
                        slot,
                        Settlement::Failed(JobError::Cooperation(
                            CooperativeViolation::ChargeExceeded,
                        )),
                    );
                    return Ok(None);
                }
                slot.state = JobState::Yielded;
                Ok(Some(JobState::Yielded))
            }
            StepOutcome::Completed(value) => {
                let _ = Self::publish_settlement(slot, Settlement::Completed(value));
                Ok(None)
            }
            StepOutcome::Failed(error) => {
                let _ = Self::publish_settlement(slot, Settlement::Failed(JobError::Family(error)));
                Ok(None)
            }
        }
    }

    /// Settle an accepted job that the outage made impossible. Failure
    /// settles result custody, not physical retirement.
    pub fn fail_unavailable(
        &mut self,
        ticket: &WorkTicket,
        condition: crate::process_condition::ConditionId,
    ) -> Result<(), CancelOutcome> {
        let Some(slot) = self.jobs.get_mut(&ticket.request()) else {
            return Err(CancelOutcome::StaleIdentity);
        };
        if slot.ticket != *ticket {
            return Err(CancelOutcome::StaleIdentity);
        }
        let terminal = UnavailableTerminal::new(condition, ticket.request(), ticket.scope());
        let _ = Self::publish_settlement(slot, Settlement::Failed(JobError::Unavailable(terminal)));
        Ok(())
    }

    /// Consume the terminal, transferring the bounded result once. Charge
    /// is released only once the worker has also returned (or the job never
    /// left the ledger without a worker, i.e. `worker_returned`).
    pub fn consume(
        &mut self,
        ticket: &WorkTicket,
    ) -> Result<Settlement<R, JobError<E>>, TerminalError> {
        let Some(slot) = self.jobs.get_mut(&ticket.request()) else {
            return Err(TerminalError::NotPublished);
        };
        if slot.ticket != *ticket {
            return Err(TerminalError::NotPublished);
        }
        let settlement = slot.terminal.consume()?;
        if slot.worker_returned {
            self.jobs.remove(&ticket.request());
        }
        Ok(settlement)
    }

    /// Request cancellation and mark the ticket abandoned. Credit stays
    /// until [`Self::worker_returned`].
    pub fn abandon(&mut self, ticket: WorkTicket, reason: Cancellation) -> CancelOutcome {
        let outcome = self.cancel(ticket, reason);
        if let Some(slot) = self.jobs.get_mut(&ticket.request()) {
            if slot.ticket == ticket {
                slot.abandoned = true;
            }
        }
        self.maybe_release(ticket.request());
        outcome
    }

    /// The worker actually returned. Combined with consumption or
    /// abandonment this may free charge. A timeout cannot stand in for
    /// this call.
    pub fn worker_returned(&mut self, ticket: &WorkTicket) -> Result<(), CancelOutcome> {
        let Some(slot) = self.jobs.get_mut(&ticket.request()) else {
            return Err(CancelOutcome::StaleIdentity);
        };
        if slot.ticket != *ticket {
            return Err(CancelOutcome::StaleIdentity);
        }
        slot.worker_returned = true;
        if slot.abandoned && slot.terminal.is_pending() {
            let reason = slot.cancel.unwrap_or(Cancellation::Shutdown);
            let _ = slot.terminal.publish(Settlement::Cancelled(reason));
        }
        self.maybe_release(ticket.request());
        Ok(())
    }

    /// Drop an abandoned, already-terminal result after the worker
    /// returned, without transferring it to a consumer.
    pub fn discard_abandoned(&mut self, ticket: &WorkTicket) -> Result<(), CancelOutcome> {
        let Some(slot) = self.jobs.get(&ticket.request()) else {
            return Err(CancelOutcome::StaleIdentity);
        };
        if slot.ticket != *ticket || !slot.abandoned {
            return Err(CancelOutcome::StaleIdentity);
        }
        if !slot.worker_returned {
            return Ok(());
        }
        self.jobs.remove(&ticket.request());
        Ok(())
    }

    fn maybe_release(&mut self, request: RequestId) {
        let Some(slot) = self.jobs.get(&request) else {
            return;
        };
        let free = slot.worker_returned
            && (slot.terminal.is_consumed() || (slot.abandoned && !slot.terminal.is_pending()));
        if free {
            self.jobs.remove(&request);
        }
    }
}

#[cfg(test)]
#[path = "work_tests.rs"]
mod tests;

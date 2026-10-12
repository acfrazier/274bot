//! Cooperative work admission (F0 item 4: contract §26 §2).
//!
//! Frozen here: the [`WorkFamily`] table, identity/revision envelope,
//! [`WorkCharge`], [`WorkAdmission`] refusals, accepted-job states,
//! [`Checkpoint`] / [`StepContext`] obligations, cancellation,
//! [`WorkProgress`], capacity/progress/terminal [`identity::Publication`]s,
//! and the once-published [`identity::Settlement`] terminal. Numerical
//! quanta, fairness and the live worker are T1.2 / G2b. Operational
//! cutover is T5.W / I-work-prep.
//!
//! [`WorkAdmission`] is the frozen service vocabulary. F0 ships no driving
//! implementation (§23 §1, §26 §2.6). Protocol fixtures live under
//! `cfg(test)`.
//!
//! [`crate::selected::FamilyPreparation`] is `!Send` (`selected.rs:324-329`)
//! and cannot be a [`CooperativeJob`].
//!
//! ```compile_fail
//! fn require_send<T: Send>() {}
//! require_send::<api::selected::FamilyPreparation>();
//! ```
//!
//! Current producers this vocabulary will migrate (characterization, not
//! cooperation): T5 map bake `run_job` (`crates/host-play/src/map_cache.rs:1719-1732,1960-1988`,
//! `MapJobStatus`/`MapProgress` at `:844-868,1939-1946,2045-2052`); T4 walk-map
//! decode (`crates/panel/src/walk_map.rs:58,1026-1038,1165-1186`); T1
//! `script_bank.rs` bank-pick; T5 `route_inspect.rs` inspection; card
//! preparation (`crates/api/src/selected.rs:310-330`).

use std::cell::Cell;
use std::fmt;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Instant;

use crate::identity::{Cancellation, Deadline, RequestId, Scope, Settlement, WorkIdentity};
use crate::process_condition::{ProcessConditions, UnavailableRefusal, UnavailableTerminal};

/// Physical track owner of a work family (§21 / §23 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Track {
    /// Bot runtime / WorkAdmission / native context.
    T1,
    /// Script / Gatherer / combat / walk.
    T2,
    /// Frontend-core.
    T3,
    /// Panel.
    T4,
    /// Nav / host-play producers.
    T5,
}

impl Track {
    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::T1 => "T1",
            Self::T2 => "T2",
            Self::T3 => "T3",
            Self::T4 => "T4",
            Self::T5 => "T5",
        }
    }
}

/// Registered work family (§26 §2.2 family table). Missing classifications
/// prevent F0 completion. Domain adapters preserve current observable
/// outcomes at T5.W.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkFamily {
    /// Route search. Producer T5; consumers T1 and T2. Activation
    /// T5.W(route) after T1.2 and T5.1. Bound by `nav::contract::RouteService`.
    Route,
    /// Inspection search. Producer T5 (`host-play/src/route_inspect.rs`).
    /// Activation T5.W(inspection).
    Inspection,
    /// Bank-pick search, including T1-owned `script_bank.rs`. Activation
    /// T5.W(bank-pick).
    BankPick,
    /// Map bake/publish. Producer T5; files stay outside SQL. Activation
    /// T5.W(map-bake). Characterizes `MapStage` / `MapJobStatus`.
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

    /// Track that produces this family's job.
    pub const fn producer(self) -> Track {
        match self {
            Self::Route | Self::Inspection | Self::BankPick | Self::MapBake => Track::T5,
            Self::WalkMapDecode => Track::T4,
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                Track::T1
            }
        }
    }

    /// Tracks that consume the family's result.
    pub const fn consumers(self) -> &'static [Track] {
        match self {
            Self::Route | Self::Inspection | Self::BankPick => &[Track::T1, Track::T2],
            Self::MapBake | Self::WalkMapDecode => &[Track::T3],
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                &[Track::T3]
            }
        }
    }

    /// Smallest interruptible unit declared for this family. Numerical
    /// allowances are G2b, not this string.
    pub const fn interruptible_unit(self) -> &'static str {
        match self {
            Self::Route | Self::Inspection | Self::BankPick => {
                "one declared search slice (bounded expansions)"
            }
            Self::MapBake => "one planned bake unit (MapProgress.stage)",
            Self::WalkMapDecode => "one decode_checked job (not PNG-length alone)",
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                "one preparation phase"
            }
        }
    }

    /// Progress phases, in order.
    pub const fn phases(self) -> &'static [&'static str] {
        match self {
            Self::Route => &["Search", "Commit"],
            Self::Inspection => &["Queue", "Search", "Publish"],
            Self::BankPick => &["Queue", "Search", "Post"],
            Self::MapBake => &[
                "ReadingCache",
                "DerivingPois",
                "BakingPlane",
                "BuildingZoomLevels",
                "Publishing",
            ],
            Self::WalkMapDecode => &["Receive", "Decode", "Publish"],
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                &["Load", "Prepare", "Seal"]
            }
        }
    }

    /// Typed result name. NoPath stays a route result, not an error.
    pub const fn result_name(self) -> &'static str {
        match self {
            Self::Route => "RouteFamilyResult (Path | NoPath)",
            Self::Inspection => "InspectPublished (ok/reason/hops)",
            Self::BankPick => "BankSelectionInput (index/kind)",
            Self::MapBake => "MapBakeResult (Ready | Paused)",
            Self::WalkMapDecode => "WalkMapDecodeResult (Decoded)",
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                "prepared card/catalogue/profile handle"
            }
        }
    }

    /// Typed error name.
    pub const fn error_name(self) -> &'static str {
        match self {
            Self::Route => "RouteFamilyError (BudgetExhausted | NotSettled)",
            Self::Inspection => "inspect failure reason",
            Self::BankPick => "pick failure / unreachable fallback",
            Self::MapBake => "MapBakeError (Failed)",
            Self::WalkMapDecode => "WalkMapDecodeError (Limit | Decode)",
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                "preparation failure"
            }
        }
    }

    /// Cancellation mapping: WorkAdmission reasons pass through; domain
    /// pause is not cancellation.
    pub const fn cancellation_mapping(self) -> &'static str {
        match self {
            Self::MapBake => {
                "Operator/Superseded/ScopeInvalidated/Shutdown → Cancelled; MapJobStatus::Paused is a family result, not Cancelled"
            }
            _ => "Operator/Superseded/ScopeInvalidated/Shutdown → Settlement::Cancelled",
        }
    }

    /// How a charge is calculated from the family's input.
    pub const fn charge_rule(self) -> &'static str {
        match self {
            Self::Route => {
                "request envelope bytes + reserved plan-lease result bytes; scratch/handles 0 until G2b"
            }
            Self::MapBake => {
                "input identity + retained checkpoint receipt bytes + result map handle"
            }
            Self::WalkMapDecode => "PNG input bytes (capped) + decoded RGBA result bytes; 0 handles after yield",
            Self::BankPick | Self::Inspection => {
                "request + world-lease (not copied) + result snapshot bytes"
            }
            Self::CardPreparation | Self::CataloguePreparation | Self::ProfilePreparation => {
                "source bytes + scratch; FamilyPreparation is not charged because it cannot be retained"
            }
        }
    }

    /// How the input revision is taken from the family's input.
    pub const fn revision_rule(self) -> &'static str {
        "producer-assigned InputRevision on the request; a newer revision stale-ages an older result"
    }
}

/// Per-family typed contract. Implemented for route as
/// `nav::contract::RouteService`. Other families record phases/units on
/// [`WorkFamily`] until T5.W supplies their input types.
pub trait WorkFamilySpec {
    /// Registered family.
    const FAMILY: WorkFamily;
    /// Family input.
    type Input;
    /// Family result (domain outcomes such as NoPath live here).
    type Output;
    /// Family error.
    type Error: fmt::Debug + std::error::Error;
    /// Progress phase.
    type Phase: Copy + Eq + fmt::Debug;
    /// Input revision stamped on this request.
    fn revision(input: &Self::Input) -> InputRevision;
    /// Charge calculated from the input. Input size alone is not the charge.
    fn charge(input: &Self::Input) -> WorkCharge;
    /// Map a WorkAdmission cancellation onto the family's cancellation.
    fn map_cancellation(reason: Cancellation) -> Cancellation;
}

/// Characterizes `MapStage` (`map_cache.rs:844-868`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapBakePhase {
    /// Reading cache archives.
    ReadingCache,
    /// Deriving packed POIs.
    DerivingPois,
    /// Baking one plane.
    BakingPlane,
    /// Building zoom levels.
    BuildingZoomLevels,
    /// Publishing the ready map.
    Publishing,
}

impl MapBakePhase {
    /// Every bake phase, in order.
    pub const ALL: [Self; 5] = [
        Self::ReadingCache,
        Self::DerivingPois,
        Self::BakingPlane,
        Self::BuildingZoomLevels,
        Self::Publishing,
    ];

    /// Stable progress index.
    pub const fn index(self) -> u16 {
        match self {
            Self::ReadingCache => 0,
            Self::DerivingPois => 1,
            Self::BakingPlane => 2,
            Self::BuildingZoomLevels => 3,
            Self::Publishing => 4,
        }
    }
}

/// Characterizes `MapJobStatus` (`map_cache.rs:862-868,1939-1946`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapBakeStatus {
    /// Accepted, not yet running.
    Queued,
    /// Running with replaceable [`WorkProgress`].
    Running,
    /// Domain success.
    Ready,
    /// Domain pause/checkpoint — a family result, not WorkAdmission cancel.
    Paused,
    /// WorkAdmission cancellation.
    Cancelled,
    /// Domain failure.
    Failed,
}

impl MapBakeStatus {
    /// `is_terminal` in `map_cache.rs:1939-1946`.
    pub const fn is_domain_ending(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Paused | Self::Cancelled | Self::Failed
        )
    }

    /// Settles as [`Settlement::Completed`].
    pub const fn is_family_result(self) -> bool {
        matches!(self, Self::Ready | Self::Paused)
    }
}

/// Characterizes `decode_loop` (`walk_map.rs:1165-1186`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WalkMapDecodeCommand {
    /// One `decode_checked` job: the interruptible unit.
    Job,
    /// Worker shutdown. Not a unit.
    Stop,
}

impl WalkMapDecodeCommand {
    /// Whether this command is one declared decode unit.
    pub const fn is_interruptible_unit(self) -> bool {
        matches!(self, Self::Job)
    }
}

/// Walk-map decode phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WalkMapDecodePhase {
    /// `cmd.recv()`.
    Receive,
    /// `decode_checked`.
    Decode,
    /// `done.send`.
    Publish,
}

/// Inspection phases (`route_inspect.rs` queue / search / publish).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectionPhase {
    /// Queued on the inspect worker.
    Queue,
    /// Shared search.
    Search,
    /// Posted hops/reason.
    Publish,
}

/// Bank-pick phases (`script_bank.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BankPickPhase {
    /// Queued on the pick worker.
    Queue,
    /// Search / radius short-circuit.
    Search,
    /// Posted `BankSelectionInput`.
    Post,
}

/// Card/catalogue/profile preparation phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreparationPhase {
    /// Load source.
    Load,
    /// Run [`crate::selected::FamilyPreparation`] on the worker thread.
    Prepare,
    /// Seal the prepared handle; the capability is dropped.
    Seal,
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

    /// The next revision, or `None` at `u64::MAX`. This type is a stamp, not
    /// a minter; callers that allocate revisions must refuse exhaustion
    /// without wrapping.
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }
}

/// Identity-bound ticket returned by a successful submit: job, family
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

    /// Host-minted job id.
    pub const fn job(self) -> RequestId {
        self.identity.job()
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

/// Why [`WorkAdmission::try_submit`] refused. Full is temporary capacity.
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
    /// The request cannot be represented (duplicate or reused job id,
    /// unusable charge).
    #[error("work admission invalid argument")]
    InvalidArgument,
    /// Process-level [`crate::process_condition::ProcessServiceCode::WorkAdmissionUnavailable`].
    #[error("work admission unavailable: {}", .0.operator_text())]
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
/// deadline; never unlimited. The deadline is a clock that advances; it is
/// not a snapshot of `now` copied at slice start.
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
#[derive(Debug, PartialEq, Eq)]
pub enum StepOutcome<R, E> {
    /// Cursor saved; eligible for later dispatch.
    Yielded,
    /// Family result. Domain outcomes such as NoPath stay in `R`.
    Completed(R),
    /// Family error.
    Failed(E),
}

/// Monotonic clock a [`StepContext`] reads at every checkpoint. Tests
/// inject a fake that can advance mid-slice.
pub trait MonotonicClock {
    /// Current monotonic instant.
    fn now(&self) -> Instant;
}

impl MonotonicClock for Instant {
    fn now(&self) -> Instant {
        *self
    }
}

/// Borrowed only for this `step` invocation. The job must not retain it
/// across a yield. Cancellation is observed live; time is read from the
/// clock at each checkpoint.
pub struct StepContext<'a> {
    clock: &'a dyn MonotonicClock,
    deadline: Deadline,
    remaining_units: u32,
    allowance: u32,
    cancel: &'a Cell<Option<Cancellation>>,
    units_this_slice: u32,
    yield_seen: bool,
    continued_after_yield: bool,
}

impl<'a> StepContext<'a> {
    /// A context for one slice. `cancel` is the per-job revocation cell;
    /// a revocation registered during the step is visible at the next
    /// checkpoint.
    pub fn new(
        clock: &'a dyn MonotonicClock,
        budget: SliceBudget,
        cancel: &'a Cell<Option<Cancellation>>,
    ) -> Self {
        Self {
            clock,
            deadline: budget.deadline,
            remaining_units: budget.units.get(),
            allowance: budget.units.get(),
            cancel,
            units_this_slice: 0,
            yield_seen: false,
            continued_after_yield: false,
        }
    }

    /// Observe cancellation, the clock, and the current slice budget.
    pub fn checkpoint(&mut self) -> Checkpoint {
        let decision = if let Some(reason) = self.cancel.get() {
            Checkpoint::Cancel(reason)
        } else if self.deadline.expired(self.clock.now()) || self.remaining_units == 0 {
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

    /// Whether this invocation consumed more units than the allowance.
    /// One unit per checkpoint holds by construction of [`Self::consume_unit`];
    /// this flags units beyond the allowance.
    pub const fn exceeded_allowance(&self) -> bool {
        self.units_this_slice > self.allowance
    }

    /// The clock's current instant.
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// The slice deadline.
    pub const fn deadline(&self) -> Deadline {
        self.deadline
    }
}

impl fmt::Debug for StepContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StepContext")
            .field("now", &self.clock.now())
            .field("deadline", &self.deadline)
            .field("remaining_units", &self.remaining_units)
            .field("units_this_slice", &self.units_this_slice)
            .finish_non_exhaustive()
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum CooperativeViolation {
    /// Yielded state carried a prohibited guard/borrow/handle.
    #[error("cooperative job retained forbidden state {0:?}")]
    ForbiddenRetain(ForbiddenRetain),
    /// Units beyond the slice allowance.
    #[error("cooperative job exceeded its slice allowance")]
    UnboundedSlice,
    /// Checkpoint returned Yield and the job started another unit.
    #[error("cooperative job ignored checkpoint Yield")]
    IgnoredYield,
    /// Retained bytes exceeded the reserved charge.
    #[error("cooperative job retained more than its charge")]
    ChargeExceeded,
}

/// Error side of a job terminal: family error, cooperation, or the
/// process-unavailable failure of an already-accepted job.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum JobError<E: fmt::Debug + std::error::Error> {
    /// Family-typed error.
    #[error(transparent)]
    Family(E),
    /// The step violated the cooperative bound.
    #[error(transparent)]
    Cooperation(CooperativeViolation),
    /// Accepted job settled because WorkAdmission became unavailable.
    #[error("{}", .0.operator_text())]
    Unavailable(UnavailableTerminal),
}

/// Immediate result of cancel. Requested is not proof the worker returned
/// or released resources. Not an [`std::error::Error`]: Requested is a
/// success path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// Revocation recorded; the service will not publish a late success.
    Requested,
    /// A terminal was already published; cancellation cannot rewrite it.
    AlreadyTerminal,
    /// Unknown or disagreeing identity/revision.
    StaleIdentity,
}

/// Ledger/protocol error. Used instead of [`CancelOutcome`] as an `Err`
/// type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    /// Unknown or disagreeing identity/revision.
    #[error("work identity is stale or unknown")]
    StaleIdentity,
    /// A terminal was already published.
    #[error("job already has a terminal")]
    AlreadyTerminal,
    /// The worker has not returned, so credit stays charged.
    #[error("worker has not returned")]
    WorkerNotReturned,
    /// Condition is not `WorkAdmissionUnavailable` for this incarnation.
    #[error("condition is not WorkAdmissionUnavailable for this ledger incarnation")]
    WrongCondition,
    /// Nothing has been published yet.
    #[error("terminal not published")]
    NotPublished,
    /// The terminal was already consumed.
    #[error("terminal already consumed")]
    AlreadyConsumed,
    /// A publication sequence could not advance.
    #[error("publication sequence exhausted")]
    SequenceExhausted,
}

/// Bounded, replaceable progress snapshot. Sequence increases within the
/// same job. Totals may be absent. Progress is not an acknowledgement or
/// terminal. Non-replaceable domain facts use their existing
/// receipt/event protocol.
///
/// [`Clone`] copies the snapshot into a [`crate::identity::Publication`]
/// cell. That is the consumer justification.
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

/// Capacity snapshot published before a capacity notice (§26 §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkCapacity {
    jobs: usize,
    bytes: usize,
    handles: u32,
}

impl WorkCapacity {
    /// Empty ledger.
    pub const EMPTY: Self = Self {
        jobs: 0,
        bytes: 0,
        handles: 0,
    };

    /// Occupied job slots.
    pub const fn jobs(self) -> usize {
        self.jobs
    }

    /// Occupied bytes.
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Occupied handles.
    pub const fn handles(self) -> u32 {
        self.handles
    }
}

/// One cooperative job. The context is borrowed for this invocation only.
/// `Send + 'static` excludes borrows, mutex guards, V8 handles and
/// `!Send` [`crate::selected::FamilyPreparation`]. [`Self::retained`]
/// remains for byte and receipt accounting.
pub trait CooperativeJob: Send + 'static {
    /// Family result.
    type Output: Send + 'static;
    /// Family error.
    type Error: Send + 'static + fmt::Debug + std::error::Error;

    /// Perform at most one declared interruptible unit before the next
    /// checkpoint. Check cancellation and the budget at entry, at each
    /// phase boundary, and before every externally observable publish.
    fn step(&mut self, ctx: &mut StepContext<'_>) -> StepOutcome<Self::Output, Self::Error>;

    /// What still lives on the job after Yielded. Forbidden items fail
    /// the job.
    fn retained(&self) -> RetainedState;
}

/// Frozen WorkAdmission operations. T1.2 supplies the worker and numerical
/// policy. F0 ships no driving implementation.
pub trait WorkAdmission {
    /// Family result.
    type Output;
    /// Family error.
    type Error: fmt::Debug + std::error::Error;

    /// Never waits for capacity or executes a job on the caller.
    fn try_submit(
        &mut self,
        identity: WorkIdentity,
        family: WorkFamily,
        revision: InputRevision,
        charge: WorkCharge,
        conditions: &ProcessConditions,
    ) -> Result<WorkTicket, SubmissionRefusal>;

    /// Idempotent identity-scoped cancellation.
    fn cancel(&mut self, ticket: WorkTicket, reason: Cancellation) -> CancelOutcome;

    /// Replaceable progress. Never takes terminal custody.
    fn publish_progress(
        &mut self,
        ticket: &WorkTicket,
        progress: WorkProgress,
    ) -> Result<(), LedgerError>;

    /// Settle an accepted job the outage made impossible.
    fn fail_unavailable(
        &mut self,
        ticket: &WorkTicket,
        condition: crate::process_condition::ConditionId,
    ) -> Result<(), LedgerError>;

    /// Consume the terminal once.
    fn consume(
        &mut self,
        ticket: &WorkTicket,
    ) -> Result<Settlement<Self::Output, JobError<Self::Error>>, LedgerError>;

    /// Request cancellation and mark the ticket abandoned.
    fn abandon(&mut self, ticket: WorkTicket, reason: Cancellation) -> CancelOutcome;

    /// The worker actually returned.
    fn worker_returned(&mut self, ticket: &WorkTicket) -> Result<(), LedgerError>;

    /// Drop an abandoned, already-terminal result after the worker returned.
    fn discard_abandoned(&mut self, ticket: &WorkTicket) -> Result<(), LedgerError>;
}

#[cfg(test)]
mod ledger {
    use super::*;
    use std::collections::HashMap;

    use crate::identity::{
        Notice, Notifier, ProcessIncarnation, Publication, PublishSequence, RequestNamespace,
        Terminal, TerminalError,
    };
    use crate::process_condition::{
        AdmissionRefusal, CustodyError, ProcessCapability, ProcessServiceCode, StrandedCustody,
    };

    struct JobSlot<R, E>
    where
        E: fmt::Debug + std::error::Error,
    {
        ticket: WorkTicket,
        state: JobState,
        charge: WorkCharge,
        cancel: Cell<Option<Cancellation>>,
        terminal: Terminal<Settlement<R, JobError<E>>>,
        progress: Option<WorkProgress>,
        stranded: StrandedCustody,
        abandoned: bool,
    }

    struct Silent;

    impl Notifier for Silent {
        fn notify(&self, _notice: Notice) {}
    }

    /// Protocol fixture: bounded admission, cancellation-versus-publication
    /// order, yield retention checks, charge lifetime, and publish-before-notify.
    /// Not a shipped executor.
    pub struct AdmissionLedger<R, E>
    where
        E: fmt::Debug + std::error::Error,
    {
        incarnation: ProcessIncarnation,
        bounds: AdmissionBounds,
        stopping: bool,
        jobs: HashMap<RequestId, JobSlot<R, E>>,
        last_job: Option<RequestId>,
        capacity: Publication<WorkCapacity>,
        progress_cell: Publication<Option<WorkProgress>>,
        terminal_cell: Publication<Option<WorkTicket>>,
    }

    impl<R, E> AdmissionLedger<R, E>
    where
        R: Send + 'static,
        E: Send + 'static + fmt::Debug + std::error::Error,
    {
        pub fn new(incarnation: ProcessIncarnation, bounds: AdmissionBounds) -> Self {
            Self {
                incarnation,
                bounds,
                stopping: false,
                jobs: HashMap::new(),
                last_job: None,
                capacity: Publication::new(Scope::Process, WorkCapacity::EMPTY),
                progress_cell: Publication::new(Scope::Process, None),
                terminal_cell: Publication::new(Scope::Process, None),
            }
        }

        pub const fn incarnation(&self) -> ProcessIncarnation {
            self.incarnation
        }

        pub fn begin_stop(&mut self) {
            self.stopping = true;
        }

        pub fn charged_bytes(&self) -> usize {
            self.jobs
                .values()
                .map(|slot| slot.charge.total_bytes())
                .fold(0usize, usize::saturating_add)
        }

        pub fn charged_jobs(&self) -> usize {
            self.jobs.len()
        }

        pub fn charged_handles(&self) -> u32 {
            self.jobs
                .values()
                .map(|slot| slot.charge.operation_handles())
                .fold(0u32, u32::saturating_add)
        }

        pub fn is_charged(&self, ticket: &WorkTicket) -> bool {
            self.jobs.contains_key(&ticket.job())
        }

        pub fn job_state(&self, ticket: &WorkTicket) -> Option<JobState> {
            let slot = self.jobs.get(&ticket.job())?;
            if slot.ticket != *ticket {
                return None;
            }
            if slot.terminal.borrow().is_some() {
                return None;
            }
            Some(slot.state)
        }

        pub fn cancellation(&self, ticket: &WorkTicket) -> Option<Cancellation> {
            let slot = self.jobs.get(&ticket.job())?;
            if slot.ticket != *ticket {
                return None;
            }
            slot.cancel.get()
        }

        pub fn progress(&self, ticket: &WorkTicket) -> Option<&WorkProgress> {
            let slot = self.jobs.get(&ticket.job())?;
            if slot.ticket != *ticket {
                return None;
            }
            slot.progress.as_ref()
        }

        pub fn borrow_terminal(&self, ticket: &WorkTicket) -> Option<&Settlement<R, JobError<E>>> {
            let slot = self.jobs.get(&ticket.job())?;
            if slot.ticket != *ticket {
                return None;
            }
            slot.terminal.borrow()
        }

        pub fn capacity_publication(&self) -> &Publication<WorkCapacity> {
            &self.capacity
        }

        pub fn progress_publication(&self) -> &Publication<Option<WorkProgress>> {
            &self.progress_cell
        }

        pub fn terminal_publication(&self) -> &Publication<Option<WorkTicket>> {
            &self.terminal_cell
        }

        fn snapshot_capacity(&self) -> WorkCapacity {
            WorkCapacity {
                jobs: self.charged_jobs(),
                bytes: self.charged_bytes(),
                handles: self.charged_handles(),
            }
        }

        fn publish_capacity(
            &self,
            notifier: &dyn Notifier,
        ) -> Result<PublishSequence, LedgerError> {
            self.capacity
                .publish(self.snapshot_capacity(), notifier)
                .map_err(|_| LedgerError::SequenceExhausted)
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

        fn publish_settlement(
            slot: &mut JobSlot<R, E>,
            settlement: Settlement<R, JobError<E>>,
        ) -> Result<(), TerminalError> {
            if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
                return Err(TerminalError::AlreadyPublished);
            }
            if let (Settlement::Completed(_), Some(reason)) = (&settlement, slot.cancel.get()) {
                return slot.terminal.publish(Settlement::Cancelled(reason));
            }
            slot.terminal.publish(settlement)
        }

        fn maybe_release(&mut self, job: RequestId) {
            let free = {
                let Some(slot) = self.jobs.get(&job) else {
                    return;
                };
                let returned = !slot.stranded.charged();
                returned
                    && (slot.terminal.is_consumed()
                        || (slot.abandoned && !slot.terminal.is_pending()))
            };
            if free {
                let slot = self.jobs.remove(&job).expect("present");
                let _ = slot.stranded.release();
                let _ = self.publish_capacity(&Silent);
            }
        }

        /// Drive one slice of `job` for `ticket`. The clock and the job's
        /// revocation cell are borrowed for the step.
        pub fn drive<J, C>(
            &mut self,
            ticket: &WorkTicket,
            job: &mut J,
            clock: &C,
            budget: SliceBudget,
        ) -> Result<Option<JobState>, LedgerError>
        where
            J: CooperativeJob<Output = R, Error = E>,
            C: MonotonicClock,
        {
            enum DriveEnd {
                Yielded,
                Terminal,
                Idle,
            }
            let end = {
                let Some(slot) = self.jobs.get_mut(&ticket.job()) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.ticket != *ticket {
                    return Err(LedgerError::StaleIdentity);
                }
                if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
                    DriveEnd::Idle
                } else {
                    slot.state = JobState::Executing;
                    let (outcome, continued, exceeded) = {
                        let mut ctx = StepContext::new(clock, budget, &slot.cancel);
                        let outcome = job.step(&mut ctx);
                        (
                            outcome,
                            ctx.continued_after_yield(),
                            ctx.exceeded_allowance(),
                        )
                    };
                    if let Some(reason) = slot.cancel.get() {
                        if matches!(outcome, StepOutcome::Completed(_) | StepOutcome::Yielded) {
                            let _ = Self::publish_settlement(slot, Settlement::Cancelled(reason));
                            DriveEnd::Terminal
                        } else if continued {
                            let _ = Self::publish_settlement(
                                slot,
                                Settlement::Failed(JobError::Cooperation(
                                    CooperativeViolation::IgnoredYield,
                                )),
                            );
                            DriveEnd::Terminal
                        } else if exceeded {
                            let _ = Self::publish_settlement(
                                slot,
                                Settlement::Failed(JobError::Cooperation(
                                    CooperativeViolation::UnboundedSlice,
                                )),
                            );
                            DriveEnd::Terminal
                        } else {
                            match outcome {
                                StepOutcome::Failed(error) => {
                                    let _ = Self::publish_settlement(
                                        slot,
                                        Settlement::Failed(JobError::Family(error)),
                                    );
                                    DriveEnd::Terminal
                                }
                                StepOutcome::Completed(_) | StepOutcome::Yielded => DriveEnd::Idle,
                            }
                        }
                    } else if continued {
                        let _ = Self::publish_settlement(
                            slot,
                            Settlement::Failed(JobError::Cooperation(
                                CooperativeViolation::IgnoredYield,
                            )),
                        );
                        DriveEnd::Terminal
                    } else if exceeded {
                        let _ = Self::publish_settlement(
                            slot,
                            Settlement::Failed(JobError::Cooperation(
                                CooperativeViolation::UnboundedSlice,
                            )),
                        );
                        DriveEnd::Terminal
                    } else {
                        match outcome {
                            StepOutcome::Yielded => {
                                let retained = job.retained();
                                if let Some(kind) = retained.forbidden_item() {
                                    let _ = Self::publish_settlement(
                                        slot,
                                        Settlement::Failed(JobError::Cooperation(
                                            CooperativeViolation::ForbiddenRetain(kind),
                                        )),
                                    );
                                    DriveEnd::Terminal
                                } else if retained.bytes() > slot.charge.total_bytes() {
                                    let _ = Self::publish_settlement(
                                        slot,
                                        Settlement::Failed(JobError::Cooperation(
                                            CooperativeViolation::ChargeExceeded,
                                        )),
                                    );
                                    DriveEnd::Terminal
                                } else {
                                    slot.state = JobState::Yielded;
                                    DriveEnd::Yielded
                                }
                            }
                            StepOutcome::Completed(value) => {
                                let _ =
                                    Self::publish_settlement(slot, Settlement::Completed(value));
                                DriveEnd::Terminal
                            }
                            StepOutcome::Failed(error) => {
                                let _ = Self::publish_settlement(
                                    slot,
                                    Settlement::Failed(JobError::Family(error)),
                                );
                                DriveEnd::Terminal
                            }
                        }
                    }
                }
            };
            match end {
                DriveEnd::Yielded => Ok(Some(JobState::Yielded)),
                DriveEnd::Idle => Ok(None),
                DriveEnd::Terminal => {
                    let _ = self.terminal_cell.publish(Some(*ticket), &Silent);
                    Ok(None)
                }
            }
        }
    }

    impl<R, E> WorkAdmission for AdmissionLedger<R, E>
    where
        R: Send + 'static,
        E: Send + 'static + fmt::Debug + std::error::Error,
    {
        type Output = R;
        type Error = E;

        fn try_submit(
            &mut self,
            identity: WorkIdentity,
            family: WorkFamily,
            revision: InputRevision,
            charge: WorkCharge,
            conditions: &ProcessConditions,
        ) -> Result<WorkTicket, SubmissionRefusal> {
            if identity.incarnation() != self.incarnation
                || conditions.incarnation() != self.incarnation
            {
                return Err(SubmissionRefusal::InvalidAuthority);
            }
            // Defence in depth: WorkIdentity already refuses isolate job ids
            // (`JobIdNotHostMinted`).
            if identity.job().namespace() != RequestNamespace::Host {
                return Err(SubmissionRefusal::InvalidArgument);
            }
            match conditions.admit(
                ProcessCapability::BackgroundWork,
                Some(identity.job()),
                identity.scope(),
            ) {
                Ok(()) => {}
                Err(AdmissionRefusal::Unavailable(refusal)) => {
                    return Err(SubmissionRefusal::Unavailable(refusal));
                }
            }
            if self.stopping {
                return Err(SubmissionRefusal::Stopping);
            }
            if self.jobs.contains_key(&identity.job()) {
                return Err(SubmissionRefusal::InvalidArgument);
            }
            if let Some(last) = self.last_job {
                if identity.job().get() <= last.get() {
                    return Err(SubmissionRefusal::InvalidArgument);
                }
            }
            self.admit_capacity(charge)?;
            let ticket = WorkTicket::new(identity, family, revision);
            self.jobs.insert(
                identity.job(),
                JobSlot {
                    ticket,
                    state: JobState::Queued,
                    charge,
                    cancel: Cell::new(None),
                    terminal: Terminal::new(),
                    progress: None,
                    stranded: StrandedCustody::new(identity.job(), identity.scope()),
                    abandoned: false,
                },
            );
            self.last_job = Some(identity.job());
            let _ = self.publish_capacity(&Silent);
            Ok(ticket)
        }

        fn cancel(&mut self, ticket: WorkTicket, reason: Cancellation) -> CancelOutcome {
            let Some(slot) = self.jobs.get_mut(&ticket.job()) else {
                return CancelOutcome::StaleIdentity;
            };
            if slot.ticket != ticket {
                return CancelOutcome::StaleIdentity;
            }
            if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
                return CancelOutcome::AlreadyTerminal;
            }
            slot.cancel.set(Some(reason));
            CancelOutcome::Requested
        }

        fn publish_progress(
            &mut self,
            ticket: &WorkTicket,
            progress: WorkProgress,
        ) -> Result<(), LedgerError> {
            {
                let Some(slot) = self.jobs.get_mut(&ticket.job()) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.ticket != *ticket {
                    return Err(LedgerError::StaleIdentity);
                }
                if progress.identity() != ticket.identity() {
                    return Err(LedgerError::StaleIdentity);
                }
                if slot
                    .progress
                    .as_ref()
                    .is_some_and(|current| progress.sequence() <= current.sequence())
                {
                    return Ok(());
                }
                slot.progress = Some(progress.clone());
            }
            self.progress_cell
                .publish(Some(progress), &Silent)
                .map_err(|_| LedgerError::SequenceExhausted)?;
            Ok(())
        }

        fn fail_unavailable(
            &mut self,
            ticket: &WorkTicket,
            condition: crate::process_condition::ConditionId,
        ) -> Result<(), LedgerError> {
            if condition.code() != ProcessServiceCode::WorkAdmissionUnavailable
                || condition.incarnation() != self.incarnation
            {
                return Err(LedgerError::WrongCondition);
            }
            {
                let Some(slot) = self.jobs.get_mut(&ticket.job()) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.ticket != *ticket {
                    return Err(LedgerError::StaleIdentity);
                }
                if slot.terminal.borrow().is_some() || slot.terminal.is_consumed() {
                    return Err(LedgerError::AlreadyTerminal);
                }
                let terminal = slot.stranded.fail_unavailable(condition).map_err(|err| {
                    #[allow(unreachable_patterns)]
                    match err {
                        CustodyError::AlreadyFailed => LedgerError::AlreadyTerminal,
                        CustodyError::WorkerStillCharged(_) => LedgerError::WorkerNotReturned,
                        // Identity r3 adds AlreadyCancelled; a second
                        // terminal reason cannot rewrite the published
                        // failure.
                        _ => LedgerError::AlreadyTerminal,
                    }
                })?;
                let _ = Self::publish_settlement(
                    slot,
                    Settlement::Failed(JobError::Unavailable(terminal)),
                );
            }
            let _ = self.terminal_cell.publish(Some(*ticket), &Silent);
            Ok(())
        }

        fn consume(
            &mut self,
            ticket: &WorkTicket,
        ) -> Result<Settlement<R, JobError<E>>, LedgerError> {
            let settlement = {
                let Some(slot) = self.jobs.get_mut(&ticket.job()) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.ticket != *ticket {
                    return Err(LedgerError::StaleIdentity);
                }
                slot.terminal.consume().map_err(|err| match err {
                    TerminalError::NotPublished => LedgerError::NotPublished,
                    TerminalError::AlreadyConsumed => LedgerError::AlreadyConsumed,
                    TerminalError::AlreadyPublished => LedgerError::AlreadyTerminal,
                })?
            };
            self.maybe_release(ticket.job());
            Ok(settlement)
        }

        fn abandon(&mut self, ticket: WorkTicket, reason: Cancellation) -> CancelOutcome {
            let outcome = self.cancel(ticket, reason);
            if let Some(slot) = self.jobs.get_mut(&ticket.job()) {
                if slot.ticket == ticket {
                    slot.abandoned = true;
                }
            }
            self.maybe_release(ticket.job());
            outcome
        }

        fn worker_returned(&mut self, ticket: &WorkTicket) -> Result<(), LedgerError> {
            let published = {
                let Some(slot) = self.jobs.get_mut(&ticket.job()) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.ticket != *ticket {
                    return Err(LedgerError::StaleIdentity);
                }
                slot.stranded.worker_returned();
                if slot.abandoned && slot.terminal.is_pending() {
                    let reason = slot.cancel.get().unwrap_or(Cancellation::Shutdown);
                    let _ = slot.terminal.publish(Settlement::Cancelled(reason));
                    true
                } else {
                    false
                }
            };
            if published {
                let _ = self.terminal_cell.publish(Some(*ticket), &Silent);
            }
            self.maybe_release(ticket.job());
            Ok(())
        }

        fn discard_abandoned(&mut self, ticket: &WorkTicket) -> Result<(), LedgerError> {
            let charged = {
                let Some(slot) = self.jobs.get(&ticket.job()) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.ticket != *ticket || !slot.abandoned {
                    return Err(LedgerError::StaleIdentity);
                }
                slot.stranded.charged()
            };
            if charged {
                return Err(LedgerError::WorkerNotReturned);
            }
            let slot = self.jobs.remove(&ticket.job()).expect("present");
            let _ = slot.stranded.release();
            let _ = self.publish_capacity(&Silent);
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) use ledger::AdmissionLedger;

#[cfg(test)]
#[path = "work_tests.rs"]
mod tests;

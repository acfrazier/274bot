//! Cooperative work admission (F0 item 4: contract §26 §2).
//!
//! Frozen here: the [`WorkFamily`] table, identity/revision envelope,
//! [`WorkCharge`], [`WorkAdmission`] refusals, accepted-job states,
//! [`Checkpoint`] / [`StepContext`] obligations, cancellation,
//! [`WorkProgress`], the capacity/progress/terminal
//! [`WorkPublications`], and the once-published [`identity::Settlement`]
//! terminal. Numerical quanta, fairness and the live worker are T1.2 / G2b.
//! Operational cutover is T5.W / I-work-prep.
//!
//! [`WorkAdmission`] is the frozen caller vocabulary and [`WorkerCustody`]
//! the frozen worker-side vocabulary. F0 ships no driving implementation
//! (§23 §1, §26 §2.6). Protocol fixtures live under `cfg(test)`.
//!
//! [`crate::selected::FamilyPreparation`] is `!Send` (`selected.rs:324-329`)
//! and cannot be a [`CooperativeJob`].
//!
//! ```compile_fail,E0277
//! fn require_send<T: Send>() {}
//! require_send::<api::selected::FamilyPreparation>();
//! ```
//!
//! A ticket is typed by its family: a route ticket cannot be consumed as a
//! map-bake result.
//!
//! ```compile_fail,E0308
//! use api::identity::{Cancellation, Settlement};
//! use api::work::{
//!     InputRevision, MapBakePhase, WorkAdmission, WorkCharge, WorkFamily, WorkFamilySpec,
//!     WorkTicket,
//! };
//!
//! #[derive(Debug, thiserror::Error)]
//! #[error("never")]
//! struct Never;
//!
//! struct RouteSpec;
//! impl WorkFamilySpec for RouteSpec {
//!     const FAMILY: WorkFamily = WorkFamily::Route;
//!     type Input = ();
//!     type Output = u32;
//!     type Error = Never;
//!     type Phase = MapBakePhase;
//!     fn revision(_: &()) -> InputRevision { InputRevision::FIRST }
//!     fn charge(_: &()) -> WorkCharge { WorkCharge::new(0, 0, 0, 0) }
//!     fn map_cancellation(reason: Cancellation) -> Cancellation { reason }
//! }
//!
//! struct BakeSpec;
//! impl WorkFamilySpec for BakeSpec {
//!     const FAMILY: WorkFamily = WorkFamily::MapBake;
//!     type Input = ();
//!     type Output = String;
//!     type Error = Never;
//!     type Phase = MapBakePhase;
//!     fn revision(_: &()) -> InputRevision { InputRevision::FIRST }
//!     fn charge(_: &()) -> WorkCharge { WorkCharge::new(0, 0, 0, 0) }
//!     fn map_cancellation(reason: Cancellation) -> Cancellation { reason }
//! }
//!
//! fn consume_as_bake<S: WorkAdmission>(service: &mut S, ticket: &WorkTicket<RouteSpec>) {
//!     let _: Settlement<String, _> = service.consume::<BakeSpec>(ticket).unwrap();
//! }
//! ```
//!
//! A job's progress phases are its family's: a job reporting map-bake
//! phases cannot be submitted under a family whose phases are walk-map
//! decode phases.
//!
//! ```compile_fail,E0271
//! use api::identity::{Cancellation, WorkIdentity};
//! use api::process_condition::ProcessConditions;
//! use api::work::{
//!     CooperativeJob, InputRevision, MapBakePhase, RetainedState, StepContext, StepOutcome,
//!     WalkMapDecodePhase, WorkAdmission, WorkCharge, WorkFamily, WorkFamilySpec,
//! };
//!
//! #[derive(Debug, thiserror::Error)]
//! #[error("never")]
//! struct Never;
//!
//! struct DecodeSpec;
//! impl WorkFamilySpec for DecodeSpec {
//!     const FAMILY: WorkFamily = WorkFamily::WalkMapDecode;
//!     type Input = ();
//!     type Output = u32;
//!     type Error = Never;
//!     type Phase = WalkMapDecodePhase;
//!     fn revision(_: &()) -> InputRevision { InputRevision::FIRST }
//!     fn charge(_: &()) -> WorkCharge { WorkCharge::new(0, 0, 0, 0) }
//!     fn map_cancellation(reason: Cancellation) -> Cancellation { reason }
//! }
//!
//! struct BakeJob;
//! impl CooperativeJob for BakeJob {
//!     type Output = u32;
//!     type Error = Never;
//!     type Phase = MapBakePhase;
//!     fn step(&mut self, ctx: &mut StepContext<'_, MapBakePhase>) -> StepOutcome<u32, Never> {
//!         ctx.publish_progress(MapBakePhase::Publishing, 1, Some(1));
//!         StepOutcome::Completed(1)
//!     }
//!     fn retained(&self) -> RetainedState { RetainedState::clean(0) }
//! }
//!
//! fn submit<S: WorkAdmission>(service: &mut S, identity: WorkIdentity, conditions: &ProcessConditions) {
//!     let _ = service.try_submit::<DecodeSpec, BakeJob>(identity, &(), BakeJob, conditions);
//! }
//! ```
//!
//! Current producers this vocabulary will migrate (characterization in the
//! producers' own crates, not cooperation): T5 map bake `run_job`
//! (`crates/host-play/src/map_cache.rs:1960-2043`, `MapJobStatus`/`MapProgress`
//! at `:844-869,1939-1946`, `finish_job` at `:2045-2053`); T4 walk-map decode
//! (`crates/panel/src/walk_map.rs:109-126,1165-1187`); T1 `script_bank.rs`
//! bank-pick; T5 `route_inspect.rs` inspection; card preparation
//! (`crates/api/src/selected.rs:310-330`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use crate::identity::{
    Cancellation, Deadline, Publication, RequestId, Scope, Settlement, WorkIdentity,
};
use crate::process_condition::{
    ConditionId, ProcessConditions, UnavailableRefusal, UnavailableTerminal,
};

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
    /// (`crates/panel/src/walk_map.rs:58,1165-1170`).
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

    /// Cancellation mapping. Every WorkAdmission reason settles as
    /// [`Settlement::Cancelled`]; the family adapter decides what today's
    /// consumers observe. For map bake the real producer has exactly one
    /// stopped-before-done ending, the resumable `MapJobStatus::Paused`:
    /// `finish_job` maps `MapCacheError::Cancelled` or a set cancel flag to
    /// `Paused` (`host-play/src/map_cache.rs:2045-2053`), `map_bind.rs:82`
    /// retries `Paused`, and `MapJobStatus::Cancelled` is matched but never
    /// constructed. See [`MapBakeStatus::cancellation_observed_as`].
    pub const fn cancellation_mapping(self) -> &'static str {
        match self {
            Self::MapBake => {
                "Operator/Superseded/ScopeInvalidated/Shutdown → Settlement::Cancelled(reason); \
                 observed by today's consumers as the resumable MapJobStatus::Paused with the \
                 checkpoint receipt retained (finish_job, map_cache.rs:2045-2053; retried at \
                 map_bind.rs:82). MapJobStatus::Cancelled is never produced today. The \
                 T5.W(map-bake) adapter translates."
            }
            _ => "Operator/Superseded/ScopeInvalidated/Shutdown → Settlement::Cancelled",
        }
    }

    /// How a charge is calculated from the family's input.
    pub const fn charge_rule(self) -> &'static str {
        match self {
            Self::Route => {
                "request envelope bytes; the plan lease is charged in its T5 owner ledger \
                 from publication (§26 §2.2: caller-held result leases obey their own bounded \
                 owner ledger); scratch/handles 0 until G2b"
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

/// A family's progress phase: a stable index for [`WorkProgress::phase`].
pub trait WorkPhase: Copy + Eq + fmt::Debug {
    /// Stable progress index.
    fn index(self) -> u16;
}

/// Per-family typed contract. Implemented for route as
/// `nav::contract::RouteService`. Other families record phases/units on
/// [`WorkFamily`] until T5.W supplies their input types.
///
/// Submission derives the family, charge and revision from this spec
/// ([`WorkAdmission::try_submit`]); the caller cannot substitute its own.
pub trait WorkFamilySpec: 'static {
    /// Registered family.
    const FAMILY: WorkFamily;
    /// Family input.
    type Input;
    /// Family result (domain outcomes such as NoPath live here).
    type Output: Send + 'static;
    /// Family error.
    type Error: Send + 'static + fmt::Debug + std::error::Error;
    /// Progress phase.
    type Phase: WorkPhase;
    /// Input revision stamped on this request.
    fn revision(input: &Self::Input) -> InputRevision;
    /// Charge calculated from the input. Input size alone is not the charge.
    fn charge(input: &Self::Input) -> WorkCharge;
    /// Map a WorkAdmission cancellation onto the family's cancellation.
    fn map_cancellation(reason: Cancellation) -> Cancellation;
}

/// Characterizes `MapStage` (`map_cache.rs:844-851`). The host-play
/// producer's tests map every real stage onto this enum.
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

impl WorkPhase for MapBakePhase {
    fn index(self) -> u16 {
        Self::index(self)
    }
}

/// Characterizes `MapJobStatus` (`map_cache.rs:861-869`) one variant per
/// variant. The host-play producer's tests map every real variant onto this
/// enum and drive `finish_job` to pin the cancellation observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapBakeStatus {
    /// Accepted, not yet running.
    Queued,
    /// Running with replaceable [`WorkProgress`].
    Running,
    /// Domain success.
    Ready,
    /// Today's one stopped-before-done ending, resumable from the checkpoint
    /// receipt: `finish_job` maps `MapCacheError::Cancelled` or a set cancel
    /// flag here (`map_cache.rs:2045-2053`); the bake's own pause
    /// (`ImageBakeOutcome::Paused` → `MapCacheError::Cancelled`,
    /// `map_producer.rs:148`) lands here too. `map_bind.rs:82` retries it.
    Paused,
    /// Matched by consumers (`map_bind.rs:154`, `panel/src/session.rs:4612`,
    /// `tui/src/bin.rs:1415`) but never constructed by the producer.
    Cancelled,
    /// Domain failure.
    Failed,
}

impl MapBakeStatus {
    /// Every status, in `MapJobStatus` declaration order.
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Running,
        Self::Ready,
        Self::Paused,
        Self::Cancelled,
        Self::Failed,
    ];

    /// `is_terminal` in `map_cache.rs:1939-1946`.
    pub const fn is_domain_ending(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Paused | Self::Cancelled | Self::Failed
        )
    }

    /// Whether today's producer ever constructs this status.
    pub const fn is_produced_today(self) -> bool {
        !matches!(self, Self::Cancelled)
    }

    /// Settles as [`Settlement::Completed`] when the bake finished or paused
    /// itself.
    pub const fn is_family_result(self) -> bool {
        matches!(self, Self::Ready | Self::Paused)
    }

    /// Today's status a WorkAdmission [`Settlement::Cancelled`] is observed
    /// as: `finish_job` lands a set cancel flag on `Paused`
    /// (`map_cache.rs:2045-2053`). The T5.W(map-bake) adapter translates;
    /// it does not produce `Cancelled`.
    pub const fn cancellation_observed_as() -> Self {
        Self::Paused
    }
}

/// Characterizes `WorkerCmd` / `decode_loop` (`walk_map.rs:123-126,1172-1187`).
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

impl WorkPhase for WalkMapDecodePhase {
    fn index(self) -> u16 {
        match self {
            Self::Receive => 0,
            Self::Decode => 1,
            Self::Publish => 2,
        }
    }
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

impl WorkPhase for InspectionPhase {
    fn index(self) -> u16 {
        match self {
            Self::Queue => 0,
            Self::Search => 1,
            Self::Publish => 2,
        }
    }
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

impl WorkPhase for BankPickPhase {
    fn index(self) -> u16 {
        match self {
            Self::Queue => 0,
            Self::Search => 1,
            Self::Post => 2,
        }
    }
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

impl WorkPhase for PreparationPhase {
    fn index(self) -> u16 {
        match self {
            Self::Load => 0,
            Self::Prepare => 1,
            Self::Seal => 2,
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

/// Identity-bound ticket returned by a successful submit: job identity and
/// input revision, typed by the family spec `F` it was submitted under.
/// Display names and table indices are not identities. The service mints
/// tickets; composing one by hand grants nothing the identity check does
/// not.
///
/// The binding is to the exact spec type `F`, not only to `F::FAMILY`:
/// [`WorkFamilySpec`] is an open trait and registration does not enforce
/// one spec per family, so the service records the submitted spec's
/// `TypeId` and every ticket operation — cancel, abandon, borrow, consume,
/// progress — is [`CancelOutcome::StaleIdentity`] /
/// [`LedgerError::StaleIdentity`] under another spec, including one that
/// declares the same family with different output, error or phase types.
pub struct WorkTicket<F: WorkFamilySpec> {
    identity: WorkIdentity,
    revision: InputRevision,
    family: PhantomData<fn() -> F>,
}

impl<F: WorkFamilySpec> WorkTicket<F> {
    /// Compose a ticket. Nested identities must already agree on
    /// [`WorkIdentity`].
    pub const fn new(identity: WorkIdentity, revision: InputRevision) -> Self {
        Self {
            identity,
            revision,
            family: PhantomData,
        }
    }

    /// Job correlation envelope.
    pub const fn identity(self) -> WorkIdentity {
        self.identity
    }

    /// Registered family.
    pub const fn family(self) -> WorkFamily {
        F::FAMILY
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

impl<F: WorkFamilySpec> Clone for WorkTicket<F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F: WorkFamilySpec> Copy for WorkTicket<F> {}

impl<F: WorkFamilySpec> PartialEq for WorkTicket<F> {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity && self.revision == other.revision
    }
}

impl<F: WorkFamilySpec> Eq for WorkTicket<F> {}

impl<F: WorkFamilySpec> Hash for WorkTicket<F> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
        self.revision.hash(state);
    }
}

impl<F: WorkFamilySpec> fmt::Debug for WorkTicket<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkTicket")
            .field("identity", &self.identity)
            .field("family", &F::FAMILY)
            .field("revision", &self.revision)
            .finish()
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

    /// Bytes counted against the ledger (input + scratch + result), or
    /// `None` when the sum cannot be represented: such a charge is refused
    /// with [`SubmissionRefusal::InvalidArgument`], never saturated.
    pub const fn total_bytes(self) -> Option<usize> {
        match self.input_bytes.checked_add(self.scratch_bytes) {
            Some(partial) => partial.checked_add(self.result_bytes),
            None => None,
        }
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
    /// The identity, or the conditions ledger it was checked against,
    /// belongs to another process incarnation.
    #[error("work admission invalid authority")]
    InvalidAuthority,
    /// The request cannot be represented: a duplicate or reused job id, a
    /// job id outside the host namespace, or a charge whose sum cannot be
    /// represented.
    #[error("work admission invalid argument")]
    InvalidArgument,
    /// Process-level [`crate::process_condition::ProcessServiceCode::WorkAdmissionUnavailable`].
    #[error("work admission unavailable: {}", .0.operator_text())]
    Unavailable(UnavailableRefusal),
}

/// A refused submission: the job comes back to the caller with the refusal
/// (§26 §2.2 "a refused submission retains the job/input with the caller").
#[derive(Debug)]
pub struct Refused<J> {
    refusal: SubmissionRefusal,
    job: J,
}

impl<J> Refused<J> {
    /// Hand `job` back with `refusal`.
    pub const fn new(refusal: SubmissionRefusal, job: J) -> Self {
        Self { refusal, job }
    }

    /// Why.
    pub const fn refusal(&self) -> SubmissionRefusal {
        self.refusal
    }

    /// The job, still the caller's.
    pub const fn job(&self) -> &J {
        &self.job
    }

    /// Take the job back.
    pub fn into_job(self) -> J {
        self.job
    }

    /// The refusal and the job.
    pub fn into_parts(self) -> (SubmissionRefusal, J) {
        (self.refusal, self.job)
    }
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
/// deadline; never unlimited. The deadline is compared against a clock that
/// advances ([`MonotonicClock`]); it is not a snapshot of `now` copied at
/// slice start.
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

/// Monotonic clock a [`StepContext`] reads at every checkpoint. Production
/// uses [`SystemClock`]; tests inject a fake that can advance mid-slice. An
/// `Instant` is a snapshot, not a clock, and deliberately does not implement
/// this trait: a deadline compared against a snapshot can never expire
/// mid-slice.
pub trait MonotonicClock {
    /// Current monotonic instant.
    fn now(&self) -> Instant;
}

/// The process monotonic clock: every read is a fresh [`Instant::now`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl MonotonicClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A progress snapshot a job reported through
/// [`StepContext::publish_progress`]. The service stamps the identity and
/// sequence; the job never assigns either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressReport {
    phase: u16,
    completed: u32,
    total: Option<u32>,
}

impl ProgressReport {
    /// Family phase index.
    pub const fn phase(self) -> u16 {
        self.phase
    }

    /// Completed units of `total`, if known.
    pub const fn completed(self) -> u32 {
        self.completed
    }

    /// Total units, or `None` when unknown.
    pub const fn total(self) -> Option<u32> {
        self.total
    }
}

/// Live cancellation read a [`StepContext`] performs at every checkpoint
/// (§26 §2.2: a checkpoint observes exact-job cancellation/revocation).
/// `Sync`, so the registering owner (the service, on a caller's
/// [`WorkAdmission::cancel`]) and the stepping worker share one source: a
/// revocation registered during a step is visible at that step's next
/// checkpoint without waiting for the step to return. The authoritative
/// selection and its ordering against terminal publication stay in the
/// service's custody state; a source only publishes that selection to the
/// worker. It is not a second, independent cancellation selection.
pub trait CancellationSource: Sync {
    /// The registered cancellation, if any.
    fn cancellation(&self) -> Option<Cancellation>;
}

/// A lock-free [`CancellationSource`] a job's service and worker share: an
/// atomic holding the first registered [`Cancellation`]. The service
/// registers after its custody selected that cancellation, so the source
/// mirrors the selection; registration is first-wins and idempotent. This
/// is the shape T1.2's worker reads; the protocol fixture uses it directly.
#[derive(Debug, Default)]
pub struct SharedCancellation(AtomicU8);

impl SharedCancellation {
    const NONE: u8 = 0;

    /// No cancellation registered.
    pub const fn new() -> Self {
        Self(AtomicU8::new(Self::NONE))
    }

    /// Register `reason` unless a cancellation already stands; the standing
    /// one is returned as the error and is never replaced.
    pub fn register(&self, reason: Cancellation) -> Result<(), Cancellation> {
        match self.0.compare_exchange(
            Self::NONE,
            Self::encode(reason),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(standing) => Err(Self::decode(standing)
                .expect("a SharedCancellation holds only encoded cancellations")),
        }
    }

    const fn encode(reason: Cancellation) -> u8 {
        match reason {
            Cancellation::Operator => 1,
            Cancellation::Superseded => 2,
            Cancellation::ScopeInvalidated => 3,
            Cancellation::Shutdown => 4,
        }
    }

    const fn decode(code: u8) -> Option<Cancellation> {
        match code {
            1 => Some(Cancellation::Operator),
            2 => Some(Cancellation::Superseded),
            3 => Some(Cancellation::ScopeInvalidated),
            4 => Some(Cancellation::Shutdown),
            _ => None,
        }
    }
}

impl CancellationSource for SharedCancellation {
    fn cancellation(&self) -> Option<Cancellation> {
        Self::decode(self.0.load(Ordering::Acquire))
    }
}

/// Borrowed only for this `step` invocation. The job must not retain it
/// across a yield. Cancellation is read live from the job's
/// [`CancellationSource`] at each checkpoint; time is read from the clock
/// at each checkpoint; progress reported here is published by the service
/// with a service-assigned sequence. `P` is the submitted family's
/// [`WorkFamilySpec::Phase`], so a job can only report its own family's
/// phases.
pub struct StepContext<'a, P: WorkPhase> {
    clock: &'a dyn MonotonicClock,
    deadline: Deadline,
    remaining_units: u32,
    allowance: u32,
    cancel: &'a dyn CancellationSource,
    units_this_slice: u32,
    yield_seen: bool,
    continued_after_yield: bool,
    progress: Option<ProgressReport>,
    phase: PhantomData<fn(P)>,
}

impl<'a, P: WorkPhase> StepContext<'a, P> {
    /// A context for one slice. `cancel` is the job's live revocation
    /// source, shared with the registering service; a revocation registered
    /// during the step is visible at the next checkpoint.
    pub fn new(
        clock: &'a dyn MonotonicClock,
        budget: SliceBudget,
        cancel: &'a dyn CancellationSource,
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
            progress: None,
            phase: PhantomData,
        }
    }

    /// Observe cancellation (read live from the shared source), the clock,
    /// and the current slice budget.
    pub fn checkpoint(&mut self) -> Checkpoint {
        let decision = if let Some(reason) = self.cancel.cancellation() {
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

    /// Report replaceable progress (§26 §2.2) in this family's phases. The
    /// service stamps the job identity and an increasing sequence and
    /// publishes the latest report of this slice when the slice returns; a
    /// later report within the same slice replaces an earlier one. Progress
    /// never takes terminal custody and is not an acknowledgement.
    pub fn publish_progress(&mut self, phase: P, completed: u32, total: Option<u32>) {
        self.progress = Some(ProgressReport {
            phase: phase.index(),
            completed,
            total,
        });
    }

    /// The latest report of this slice, for the service to stamp and
    /// publish.
    pub const fn reported_progress(&self) -> Option<ProgressReport> {
        self.progress
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

impl<P: WorkPhase> fmt::Debug for StepContext<'_, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StepContext")
            .field("now", &self.clock.now())
            .field("deadline", &self.deadline)
            .field("remaining_units", &self.remaining_units)
            .field("units_this_slice", &self.units_this_slice)
            .field("progress", &self.progress)
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

/// Error side of a job terminal: family error, a contained internal failure,
/// cooperation, or the process-unavailable failure of an accepted job.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobError<E: fmt::Debug + std::error::Error> {
    /// Family-typed error.
    #[error(transparent)]
    Family(E),
    /// A returned job panic was caught and contained by its worker.
    #[error("background work job failed internally")]
    FailedInternally,
    /// The step violated the cooperative bound.
    #[error(transparent)]
    Cooperation(CooperativeViolation),
    /// Accepted job settled because WorkAdmission became unavailable.
    #[error("{}", .0.operator_text())]
    Unavailable(UnavailableTerminal),
}

/// The terminal of a job submitted under family `F`: its typed result or
/// its [`JobError`].
pub type JobSettlement<F> =
    Settlement<<F as WorkFamilySpec>::Output, JobError<<F as WorkFamilySpec>::Error>>;

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
    /// Unknown or disagreeing identity/family/revision.
    #[error("work identity is stale or unknown")]
    StaleIdentity,
    /// A terminal was already published (an intent's cancellation selected
    /// first counts: see [`WorkerCustody::fail_unavailable`]).
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

    /// A snapshot of occupied slots, bytes and handles.
    pub const fn new(jobs: usize, bytes: usize, handles: u32) -> Self {
        Self {
            jobs,
            bytes,
            handles,
        }
    }

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

/// The three service publications (§26 §2.5): capacity (every admission
/// and release), the latest progress snapshot, and the identity of the job
/// whose terminal was most recently published. Each publishes its state
/// before notifying; a reader woken by a notice reads that sequence or a
/// later one.
#[derive(Debug)]
pub struct WorkPublications {
    capacity: Publication<WorkCapacity>,
    progress: Publication<Option<WorkProgress>>,
    terminal: Publication<Option<WorkIdentity>>,
}

impl WorkPublications {
    /// Empty publications for `scope`.
    pub fn new(scope: Scope) -> Self {
        Self {
            capacity: Publication::new(scope, WorkCapacity::EMPTY),
            progress: Publication::new(scope, None),
            terminal: Publication::new(scope, None),
        }
    }

    /// Occupied capacity after the latest admission or release.
    pub const fn capacity(&self) -> &Publication<WorkCapacity> {
        &self.capacity
    }

    /// The latest progress snapshot of any job.
    pub const fn progress(&self) -> &Publication<Option<WorkProgress>> {
        &self.progress
    }

    /// The job whose terminal was most recently published.
    pub const fn terminal(&self) -> &Publication<Option<WorkIdentity>> {
        &self.terminal
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
    /// Progress phases this job reports; submission requires the family's
    /// [`WorkFamilySpec::Phase`].
    type Phase: WorkPhase;

    /// Perform at most one declared interruptible unit before the next
    /// checkpoint. Check cancellation and the budget at entry, at each
    /// phase boundary, before every externally observable publish, and
    /// again before returning a success result.
    fn step(
        &mut self,
        ctx: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error>;

    /// What still lives on the job after Yielded. Forbidden items fail
    /// the job.
    fn retained(&self) -> RetainedState;
}

/// Frozen WorkAdmission caller operations (§26 §2.2). T1.2 supplies the
/// worker and numerical policy; F0 ships no driving implementation.
///
/// Submission transfers job custody: an accepted job lives in the service
/// until its terminal is consumed, and a refused submission hands the job
/// back. Tickets, terminals and progress are typed by the family spec the
/// job was submitted under; one service serves every registered family
/// (§21). Worker-side operations are [`WorkerCustody`], never here: a
/// ticket holder cannot free credit, settle an outage or forge progress.
///
/// Methods are generic over the family, so the trait is not
/// dyn-compatible: callers are generic over `S: WorkAdmission` or hold the
/// composition's concrete service.
pub trait WorkAdmission {
    /// Never waits for capacity or executes a job on the caller. The family,
    /// charge and revision come from `F` and `input`, not from the caller;
    /// the job's result, error and progress-phase types must be `F`'s.
    /// Acceptance takes custody of `job`; a refusal returns it.
    fn try_submit<F, J>(
        &mut self,
        identity: WorkIdentity,
        input: &F::Input,
        job: J,
        conditions: &ProcessConditions,
    ) -> Result<WorkTicket<F>, Refused<J>>
    where
        F: WorkFamilySpec,
        J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>;

    /// Idempotent identity-scoped cancellation (§26 §2.2). `Requested`
    /// means the revocation was recorded in the job's custody, published to
    /// the job's live [`CancellationSource`], and the service's worker was
    /// woken ([`WorkerWake`]), in that order: an executing step observes it
    /// at its next checkpoint and a parked worker does not wait for its next
    /// slice. A repeated request records nothing, wakes nothing, and is
    /// still `Requested` (the first selection stands); `AlreadyTerminal` and
    /// `StaleIdentity` record and wake nothing. Requested is not proof that
    /// the worker returned or released resources. A ticket of another spec
    /// type is stale even when it names the same family (see
    /// [`WorkTicket`]).
    fn cancel<F: WorkFamilySpec>(
        &mut self,
        ticket: WorkTicket<F>,
        reason: Cancellation,
    ) -> CancelOutcome;

    /// Request cancellation (with [`Self::cancel`]'s recording and wake
    /// obligations) and mark the ticket abandoned: its result is discarded
    /// when the worker actually returns, and credit is retained until
    /// then.
    fn abandon<F: WorkFamilySpec>(
        &mut self,
        ticket: WorkTicket<F>,
        reason: Cancellation,
    ) -> CancelOutcome;

    /// Poll: borrow the retained terminal without consuming it. `Ok(None)`
    /// while pending.
    fn borrow_terminal<F: WorkFamilySpec>(
        &self,
        ticket: &WorkTicket<F>,
    ) -> Result<Option<&JobSettlement<F>>, LedgerError>;

    /// Consume the terminal once, transferring the family result and
    /// acknowledging custody.
    fn consume<F: WorkFamilySpec>(
        &mut self,
        ticket: &WorkTicket<F>,
    ) -> Result<JobSettlement<F>, LedgerError>;

    /// The latest progress snapshot of the job, if any.
    fn progress<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> Option<&WorkProgress>;

    /// Read access to the capacity, progress and terminal publications.
    fn publications(&self) -> &WorkPublications;
}

/// Frozen worker-side custody obligations (§26 §2.5, §3.2). Only T1.2's
/// worker holds this capability; callers never do. Credit is freed only by
/// an actual worker return, and an outage settles an accepted job through
/// the §3 condition.
pub trait WorkerCustody {
    /// The worker actually returned for `job`. An abandoned job with no
    /// terminal settles `Cancelled` here; its credit is released only now.
    fn worker_returned(&mut self, job: WorkIdentity) -> Result<(), LedgerError>;

    /// Settle an accepted job the outage made impossible. If an intent's
    /// cancellation already selected the terminal, that cancellation is
    /// published instead and this returns [`LedgerError::AlreadyTerminal`].
    fn fail_unavailable(
        &mut self,
        job: WorkIdentity,
        condition: ConditionId,
    ) -> Result<(), LedgerError>;
}

/// The neutral wake a service gives its own worker after recording a
/// revocation (§26 §2.2: `cancel` "records revocation and wakes the
/// service"), so a parked worker observes the revocation — and runs the
/// job's bounded cancellation path — without waiting for its next
/// scheduled slice. T1.2 maps it onto its worker park; the fixture counts
/// it. It is not the consumer [`crate::identity::Notifier`]: no publication
/// precedes it, it carries no sequence, and its recipient is the worker,
/// not a consumer. Wakes may coalesce; the woken worker re-reads its jobs'
/// [`CancellationSource`]s.
pub trait WorkerWake: Send + Sync {
    /// Wake the worker. Called after the revocation is registered in custody
    /// and published to the job's source, never before.
    fn wake(&self);
}

#[cfg(test)]
mod ledger {
    use super::*;
    use std::any::{Any, TypeId};
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::identity::{
        Notice, Notifier, ProcessIncarnation, PublishSequence, RequestNamespace, Terminal,
        TerminalError,
    };
    use crate::process_condition::{
        AdmissionRefusal, CustodyError, ProcessCapability, ProcessServiceCode, StrandedCustody,
    };

    /// A [`JobSettlement`] behind the family.
    type Erased = Box<dyn Any + Send>;

    enum ErasedStep {
        Yielded,
        Completed(Erased),
        Failed(Erased),
    }

    /// What the context observed during one slice, read after the typed
    /// step returned and before erasure.
    struct SliceReport {
        continued: bool,
        exceeded: bool,
        progress: Option<ProgressReport>,
    }

    /// The job in custody, with its family's types erased so one ledger
    /// serves every family. The step runs under a context typed by the
    /// family's phase; erasure happens after that typed call.
    trait ErasedJob: Send {
        fn step(
            &mut self,
            clock: &dyn MonotonicClock,
            budget: SliceBudget,
            cancel: &dyn CancellationSource,
        ) -> (ErasedStep, SliceReport);
        fn retained(&self) -> RetainedState;
    }

    struct Typed<F, J> {
        job: J,
        family: PhantomData<fn() -> F>,
    }

    impl<F, J> ErasedJob for Typed<F, J>
    where
        F: WorkFamilySpec,
        J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
    {
        fn step(
            &mut self,
            clock: &dyn MonotonicClock,
            budget: SliceBudget,
            cancel: &dyn CancellationSource,
        ) -> (ErasedStep, SliceReport) {
            let mut ctx = StepContext::<F::Phase>::new(clock, budget, cancel);
            let outcome = match self.job.step(&mut ctx) {
                StepOutcome::Yielded => ErasedStep::Yielded,
                StepOutcome::Completed(value) => {
                    ErasedStep::Completed(Box::new(
                        Settlement::<F::Output, JobError<F::Error>>::Completed(value),
                    ))
                }
                StepOutcome::Failed(error) => ErasedStep::Failed(Box::new(Settlement::<
                    F::Output,
                    JobError<F::Error>,
                >::Failed(
                    JobError::Family(error),
                ))),
            };
            let report = SliceReport {
                continued: ctx.continued_after_yield(),
                exceeded: ctx.exceeded_allowance(),
                progress: ctx.reported_progress(),
            };
            (outcome, report)
        }

        fn retained(&self) -> RetainedState {
            self.job.retained()
        }
    }

    /// Terminals the service itself selects.
    enum ServiceTerminal {
        Cancelled(Cancellation),
        Cooperation(CooperativeViolation),
        Unavailable(UnavailableTerminal),
    }

    fn erase_service_terminal<F: WorkFamilySpec>(terminal: ServiceTerminal) -> Erased {
        let settlement: JobSettlement<F> = match terminal {
            ServiceTerminal::Cancelled(reason) => Settlement::Cancelled(reason),
            ServiceTerminal::Cooperation(violation) => {
                Settlement::Failed(JobError::Cooperation(violation))
            }
            ServiceTerminal::Unavailable(terminal) => {
                Settlement::Failed(JobError::Unavailable(terminal))
            }
        };
        Box::new(settlement)
    }

    struct JobSlot {
        identity: WorkIdentity,
        /// The submitted spec type: a ticket under any other spec is stale,
        /// even one declaring the same `WorkFamily`.
        spec: TypeId,
        revision: InputRevision,
        state: JobState,
        charge: WorkCharge,
        /// `None` while a dispatched [`Slice`] holds the job.
        job: Option<Box<dyn ErasedJob>>,
        /// How many times the job was dispatched; the current dispatch's
        /// [`Slice`] carries this number in its stamp.
        dispatches: u64,
        settle: fn(ServiceTerminal) -> Erased,
        terminal: Terminal<Erased>,
        progress: Option<WorkProgress>,
        /// Authoritative terminal selection (cancellation versus outage).
        stranded: StrandedCustody,
        /// The live source the worker reads; mirrors `stranded`'s selected
        /// cancellation, registered only after custody selected it.
        cancel: Arc<SharedCancellation>,
        abandoned: bool,
    }

    impl JobSlot {
        fn matches<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> bool {
            self.identity == ticket.identity()
                && self.spec == TypeId::of::<F>()
                && self.revision == ticket.revision()
        }

        fn has_terminal(&self) -> bool {
            self.terminal.borrow().is_some() || self.terminal.is_consumed()
        }

        fn publish(&mut self, terminal: ServiceTerminal) {
            let erased = (self.settle)(terminal);
            self.terminal
                .publish(erased)
                .expect("terminal checked pending before publication");
        }
    }

    /// Which step of which dispatch of which job produced an outcome. A
    /// [`Slice`] carries the stamp of its latest step; the [`SliceOutcome`]
    /// that step returned carries the same one, and [`AdmissionLedger::settle`]
    /// accepts no other: another job's outcome (same or another spec) and a
    /// replayed earlier step of this job are refused before custody,
    /// progress or the terminal change.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct SliceStamp {
        identity: WorkIdentity,
        spec: TypeId,
        dispatch: u64,
        step: u32,
    }

    /// The worker's custody of one dispatched slice: the job and its live
    /// cancellation source, out of the ledger while the step runs, so the
    /// service can register a revocation mid-step without waiting for the
    /// step to return.
    pub struct Slice {
        stamp: SliceStamp,
        job: Box<dyn ErasedJob>,
        cancel: Arc<SharedCancellation>,
    }

    /// What a stepped [`Slice`] hands back to [`AdmissionLedger::settle`],
    /// stamped with the slice and step that produced it.
    pub struct SliceOutcome {
        stamp: SliceStamp,
        outcome: ErasedStep,
        report: SliceReport,
    }

    impl Slice {
        /// Run one step under `clock` and `budget`, reading cancellation
        /// live from the shared source. The outcome is bound to this step
        /// of this slice; an earlier step's outcome is stale afterwards.
        pub fn step(&mut self, clock: &dyn MonotonicClock, budget: SliceBudget) -> SliceOutcome {
            self.stamp.step += 1;
            let (outcome, report) = self.job.step(clock, budget, &*self.cancel);
            SliceOutcome {
                stamp: self.stamp,
                outcome,
                report,
            }
        }

        /// The live source this slice's checkpoints read, as the worker
        /// holds it.
        pub fn cancellation_source(&self) -> Arc<SharedCancellation> {
            Arc::clone(&self.cancel)
        }
    }

    impl fmt::Debug for Slice {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Slice")
                .field("identity", &self.stamp.identity)
                .field("dispatch", &self.stamp.dispatch)
                .field("step", &self.stamp.step)
                .field("cancellation", &self.cancel.cancellation())
                .finish_non_exhaustive()
        }
    }

    struct Silent;

    impl Notifier for Silent {
        fn notify(&self, _notice: Notice) {}
    }

    impl WorkerWake for Silent {
        fn wake(&self) {}
    }

    /// Protocol fixture: bounded admission, custody transfer,
    /// cancellation-versus-publication order, yield retention checks,
    /// charge lifetime, publish-before-notify through a real notifier, and
    /// the cancellation wake through a real [`WorkerWake`]. The service and
    /// the worker are split at [`Self::dispatch`] / [`Self::settle`] so a
    /// revocation can be registered while a slice is out; a settlement is
    /// bound to the dispatched slice by [`SliceStamp`], so one job's outcome
    /// never settles another's. Not a shipped executor.
    pub struct AdmissionLedger {
        incarnation: ProcessIncarnation,
        bounds: AdmissionBounds,
        stopping: bool,
        jobs: HashMap<RequestId, JobSlot>,
        last_job: Option<RequestId>,
        publications: Arc<WorkPublications>,
        notifier: Arc<dyn Notifier>,
        wake: Arc<dyn WorkerWake>,
    }

    impl AdmissionLedger {
        pub fn new(incarnation: ProcessIncarnation, bounds: AdmissionBounds) -> Self {
            Self::with_notifier(
                incarnation,
                bounds,
                Arc::new(WorkPublications::new(Scope::Process)),
                Arc::new(Silent),
                Arc::new(Silent),
            )
        }

        pub fn with_notifier(
            incarnation: ProcessIncarnation,
            bounds: AdmissionBounds,
            publications: Arc<WorkPublications>,
            notifier: Arc<dyn Notifier>,
            wake: Arc<dyn WorkerWake>,
        ) -> Self {
            Self {
                incarnation,
                bounds,
                stopping: false,
                jobs: HashMap::new(),
                last_job: None,
                publications,
                notifier,
                wake,
            }
        }

        pub fn begin_stop(&mut self) {
            self.stopping = true;
        }

        pub fn charged_bytes(&self) -> usize {
            self.jobs
                .values()
                .map(|slot| {
                    slot.charge
                        .total_bytes()
                        .expect("admitted charges are representable")
                })
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

        fn slot<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> Option<&JobSlot> {
            self.jobs
                .get(&ticket.job())
                .filter(|slot| slot.matches(ticket))
        }

        fn slot_mut<F: WorkFamilySpec>(&mut self, ticket: &WorkTicket<F>) -> Option<&mut JobSlot> {
            self.jobs
                .get_mut(&ticket.job())
                .filter(|slot| slot.matches(ticket))
        }

        fn slot_by_identity(&mut self, job: WorkIdentity) -> Option<&mut JobSlot> {
            self.jobs
                .get_mut(&job.job())
                .filter(|slot| slot.identity == job)
        }

        pub fn is_charged<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> bool {
            self.slot(ticket).is_some()
        }

        pub fn job_state<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> Option<JobState> {
            let slot = self.slot(ticket)?;
            if slot.terminal.borrow().is_some() {
                return None;
            }
            Some(slot.state)
        }

        pub fn cancellation<F: WorkFamilySpec>(
            &self,
            ticket: &WorkTicket<F>,
        ) -> Option<Cancellation> {
            self.slot(ticket)?.stranded.cancellation()
        }

        fn snapshot_capacity(&self) -> WorkCapacity {
            WorkCapacity::new(
                self.charged_jobs(),
                self.charged_bytes(),
                self.charged_handles(),
            )
        }

        fn publish_capacity(&self) -> PublishSequence {
            self.publications
                .capacity()
                .publish(self.snapshot_capacity(), &*self.notifier)
                .expect("capacity sequence not exhausted")
        }

        fn publish_terminal(&self, job: WorkIdentity) {
            self.publications
                .terminal()
                .publish(Some(job), &*self.notifier)
                .expect("terminal sequence not exhausted");
        }

        fn admit<F: WorkFamilySpec>(
            &self,
            identity: WorkIdentity,
            input: &F::Input,
            conditions: &ProcessConditions,
        ) -> Result<(WorkCharge, InputRevision), SubmissionRefusal> {
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
            let charge = F::charge(input);
            self.admit_capacity(charge)?;
            Ok((charge, F::revision(input)))
        }

        fn admit_capacity(&self, charge: WorkCharge) -> Result<(), SubmissionRefusal> {
            let total = charge
                .total_bytes()
                .ok_or(SubmissionRefusal::InvalidArgument)?;
            if self.jobs.len() >= self.bounds.max_jobs.get() {
                return Err(SubmissionRefusal::Full);
            }
            let bytes = self
                .charged_bytes()
                .checked_add(total)
                .ok_or(SubmissionRefusal::InvalidArgument)?;
            if bytes > self.bounds.max_bytes.get() {
                return Err(SubmissionRefusal::Full);
            }
            let handles = self
                .charged_handles()
                .checked_add(charge.operation_handles())
                .ok_or(SubmissionRefusal::InvalidArgument)?;
            if handles > self.bounds.max_handles.get() {
                return Err(SubmissionRefusal::Full);
            }
            Ok(())
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
                slot.stranded
                    .release()
                    .expect("released only after worker return");
                self.publish_capacity();
            }
        }

        /// Hand the job in custody for `job` to the worker for one slice.
        /// `Ok(None)` when the job already has a terminal. While the slice
        /// is out the slot keeps its custody, charge and live cancellation
        /// source; a second dispatch is refused until [`Self::settle`].
        pub fn dispatch(&mut self, job: WorkIdentity) -> Result<Option<Slice>, LedgerError> {
            let Some(slot) = self.slot_by_identity(job) else {
                return Err(LedgerError::StaleIdentity);
            };
            if slot.has_terminal() {
                return Ok(None);
            }
            let Some(erased) = slot.job.take() else {
                return Err(LedgerError::WorkerNotReturned);
            };
            slot.state = JobState::Executing;
            slot.dispatches += 1;
            Ok(Some(Slice {
                stamp: SliceStamp {
                    identity: slot.identity,
                    spec: slot.spec,
                    dispatch: slot.dispatches,
                    step: 0,
                },
                job: erased,
                cancel: Arc::clone(&slot.cancel),
            }))
        }

        /// Take the stepped slice back and arbitrate its outcome against
        /// the custody's selected cancellation: a cancellation selected
        /// before publication is the job's one terminal, whatever the step
        /// returned; a terminal published while the slice was out (an
        /// outage) stands.
        ///
        /// `result` must be the outcome of `slice`'s latest step: the same
        /// job, the same submitted spec, the same dispatch and the same
        /// step. Anything else — another job's outcome, under the same or
        /// another spec, or a replayed earlier step — is refused with
        /// [`LedgerError::StaleIdentity`] before custody, progress or the
        /// terminal change. A refused settlement consumes the slice; the
        /// fixture has no protocol for a worker that mispairs an outcome
        /// beyond refusing it.
        pub fn settle(
            &mut self,
            slice: Slice,
            result: SliceOutcome,
        ) -> Result<Option<JobState>, LedgerError> {
            let job = slice.stamp.identity;
            let Some(slot) = self.slot_by_identity(job) else {
                return Err(LedgerError::StaleIdentity);
            };
            if result.stamp != slice.stamp
                || slice.stamp.dispatch != slot.dispatches
                || slot.job.is_some()
            {
                return Err(LedgerError::StaleIdentity);
            }
            slot.job = Some(slice.job);
            if slot.has_terminal() {
                return Ok(None);
            }
            let SliceOutcome {
                stamp: _,
                outcome,
                report,
            } = result;
            let progress = report.progress.map(|report| {
                let sequence = slot.progress.as_ref().map_or(0, WorkProgress::sequence) + 1;
                WorkProgress::new(
                    slot.identity,
                    sequence,
                    report.phase(),
                    report.completed(),
                    report.total(),
                )
            });
            if let Some(progress) = &progress {
                slot.progress = Some(progress.clone());
            }
            let cooperation = if report.continued {
                Some(CooperativeViolation::IgnoredYield)
            } else if report.exceeded {
                Some(CooperativeViolation::UnboundedSlice)
            } else {
                None
            };
            let terminal: Option<Erased> = match (slot.stranded.cancellation(), outcome) {
                // The cancellation selected before publication is the one
                // terminal: a late success, a yield or a late family failure
                // cannot replace it (§26 §2.5).
                (Some(reason), _) => Some((slot.settle)(ServiceTerminal::Cancelled(reason))),
                (None, ErasedStep::Failed(error)) => Some(match cooperation {
                    Some(violation) => (slot.settle)(ServiceTerminal::Cooperation(violation)),
                    None => error,
                }),
                (None, ErasedStep::Completed(value)) => Some(match cooperation {
                    Some(violation) => (slot.settle)(ServiceTerminal::Cooperation(violation)),
                    None => value,
                }),
                (None, ErasedStep::Yielded) => match cooperation {
                    Some(violation) => Some((slot.settle)(ServiceTerminal::Cooperation(violation))),
                    None => {
                        let retained = slot
                            .job
                            .as_ref()
                            .expect("the slice's job was just returned")
                            .retained();
                        if let Some(kind) = retained.forbidden_item() {
                            Some((slot.settle)(ServiceTerminal::Cooperation(
                                CooperativeViolation::ForbiddenRetain(kind),
                            )))
                        } else if slot
                            .charge
                            .total_bytes()
                            .is_none_or(|total| retained.bytes() > total)
                        {
                            Some((slot.settle)(ServiceTerminal::Cooperation(
                                CooperativeViolation::ChargeExceeded,
                            )))
                        } else {
                            slot.state = JobState::Yielded;
                            None
                        }
                    }
                },
            };
            let state = slot.state;
            let settled = match terminal {
                Some(erased) => {
                    slot.terminal
                        .publish(erased)
                        .expect("terminal checked pending before publication");
                    true
                }
                None => false,
            };
            if let Some(progress) = progress {
                self.publications
                    .progress()
                    .publish(Some(progress), &*self.notifier)
                    .map_err(|_| LedgerError::SequenceExhausted)?;
            }
            if settled {
                self.publish_terminal(job);
                return Ok(None);
            }
            Ok(match state {
                JobState::Yielded => Some(JobState::Yielded),
                JobState::Queued | JobState::Executing => None,
            })
        }

        /// Dispatch, step and settle one slice on this thread.
        pub fn drive(
            &mut self,
            job: WorkIdentity,
            clock: &dyn MonotonicClock,
            budget: SliceBudget,
        ) -> Result<Option<JobState>, LedgerError> {
            let Some(mut slice) = self.dispatch(job)? else {
                return Ok(None);
            };
            let result = slice.step(clock, budget);
            self.settle(slice, result)
        }
    }

    impl WorkAdmission for AdmissionLedger {
        fn try_submit<F, J>(
            &mut self,
            identity: WorkIdentity,
            input: &F::Input,
            job: J,
            conditions: &ProcessConditions,
        ) -> Result<WorkTicket<F>, Refused<J>>
        where
            F: WorkFamilySpec,
            J: CooperativeJob<Output = F::Output, Error = F::Error, Phase = F::Phase>,
        {
            let (charge, revision) = match self.admit::<F>(identity, input, conditions) {
                Ok(admitted) => admitted,
                Err(refusal) => return Err(Refused::new(refusal, job)),
            };
            self.jobs.insert(
                identity.job(),
                JobSlot {
                    identity,
                    spec: TypeId::of::<F>(),
                    revision,
                    state: JobState::Queued,
                    charge,
                    job: Some(Box::new(Typed::<F, J> {
                        job,
                        family: PhantomData,
                    })),
                    dispatches: 0,
                    settle: erase_service_terminal::<F>,
                    terminal: Terminal::new(),
                    progress: None,
                    stranded: StrandedCustody::new(identity.job(), identity.scope()),
                    cancel: Arc::new(SharedCancellation::new()),
                    abandoned: false,
                },
            );
            self.last_job = Some(identity.job());
            self.publish_capacity();
            Ok(WorkTicket::new(identity, revision))
        }

        fn cancel<F: WorkFamilySpec>(
            &mut self,
            ticket: WorkTicket<F>,
            reason: Cancellation,
        ) -> CancelOutcome {
            let Some(slot) = self.slot_mut(&ticket) else {
                return CancelOutcome::StaleIdentity;
            };
            if slot.has_terminal() {
                return CancelOutcome::AlreadyTerminal;
            }
            match slot.stranded.cancel(reason) {
                Ok(()) => {
                    // Recorded in custody first; then published to the live
                    // source the stepping worker reads; then the wake.
                    slot.cancel
                        .register(reason)
                        .expect("the source is free until custody selects a cancellation");
                    self.wake.wake();
                    CancelOutcome::Requested
                }
                // Idempotent: the first selection stands; nothing is
                // recorded and nothing is woken.
                Err(CustodyError::AlreadyCancelled(_)) => CancelOutcome::Requested,
                Err(CustodyError::AlreadyFailed | CustodyError::WorkerStillCharged(_)) => {
                    CancelOutcome::AlreadyTerminal
                }
            }
        }

        fn abandon<F: WorkFamilySpec>(
            &mut self,
            ticket: WorkTicket<F>,
            reason: Cancellation,
        ) -> CancelOutcome {
            let outcome = self.cancel(ticket, reason);
            if let Some(slot) = self.slot_mut(&ticket) {
                slot.abandoned = true;
            }
            self.maybe_release(ticket.job());
            outcome
        }

        fn borrow_terminal<F: WorkFamilySpec>(
            &self,
            ticket: &WorkTicket<F>,
        ) -> Result<Option<&JobSettlement<F>>, LedgerError> {
            let slot = self.slot(ticket).ok_or(LedgerError::StaleIdentity)?;
            if slot.terminal.is_consumed() {
                return Err(LedgerError::AlreadyConsumed);
            }
            match slot.terminal.borrow() {
                None => Ok(None),
                Some(erased) => erased
                    .downcast_ref()
                    .map(Some)
                    .ok_or(LedgerError::StaleIdentity),
            }
        }

        fn consume<F: WorkFamilySpec>(
            &mut self,
            ticket: &WorkTicket<F>,
        ) -> Result<JobSettlement<F>, LedgerError> {
            let settlement = {
                let slot = self.slot_mut(ticket).ok_or(LedgerError::StaleIdentity)?;
                if slot
                    .terminal
                    .borrow()
                    .is_some_and(|erased| !erased.is::<JobSettlement<F>>())
                {
                    return Err(LedgerError::StaleIdentity);
                }
                let erased = slot.terminal.consume().map_err(|err| match err {
                    TerminalError::NotPublished => LedgerError::NotPublished,
                    TerminalError::AlreadyConsumed => LedgerError::AlreadyConsumed,
                    TerminalError::AlreadyPublished => LedgerError::AlreadyTerminal,
                })?;
                *erased
                    .downcast::<JobSettlement<F>>()
                    .expect("terminal type checked before consumption")
            };
            self.maybe_release(ticket.job());
            Ok(settlement)
        }

        fn progress<F: WorkFamilySpec>(&self, ticket: &WorkTicket<F>) -> Option<&WorkProgress> {
            self.slot(ticket)?.progress.as_ref()
        }

        fn publications(&self) -> &WorkPublications {
            &self.publications
        }
    }

    impl WorkerCustody for AdmissionLedger {
        fn worker_returned(&mut self, job: WorkIdentity) -> Result<(), LedgerError> {
            let published = {
                let Some(slot) = self.slot_by_identity(job) else {
                    return Err(LedgerError::StaleIdentity);
                };
                slot.stranded.worker_returned();
                if slot.abandoned && slot.terminal.is_pending() {
                    let reason = slot
                        .stranded
                        .cancellation()
                        .unwrap_or(Cancellation::Shutdown);
                    slot.publish(ServiceTerminal::Cancelled(reason));
                    true
                } else {
                    false
                }
            };
            if published {
                self.publish_terminal(job);
            }
            self.maybe_release(job.job());
            Ok(())
        }

        fn fail_unavailable(
            &mut self,
            job: WorkIdentity,
            condition: ConditionId,
        ) -> Result<(), LedgerError> {
            if condition.code() != ProcessServiceCode::WorkAdmissionUnavailable
                || condition.incarnation() != self.incarnation
            {
                return Err(LedgerError::WrongCondition);
            }
            let result = {
                let Some(slot) = self.slot_by_identity(job) else {
                    return Err(LedgerError::StaleIdentity);
                };
                if slot.has_terminal() {
                    return Err(LedgerError::AlreadyTerminal);
                }
                match slot.stranded.fail_unavailable(condition) {
                    Ok(terminal) => {
                        slot.publish(ServiceTerminal::Unavailable(terminal));
                        Ok(())
                    }
                    // The intent's cancellation was selected first (§26
                    // §3.2 row 2); the worker is gone, so the service
                    // publishes that selection and the outage cannot
                    // rewrite it.
                    Err(CustodyError::AlreadyCancelled(reason)) => {
                        slot.publish(ServiceTerminal::Cancelled(reason));
                        Err(LedgerError::AlreadyTerminal)
                    }
                    Err(CustodyError::AlreadyFailed) => return Err(LedgerError::AlreadyTerminal),
                    Err(CustodyError::WorkerStillCharged(_)) => {
                        return Err(LedgerError::WorkerNotReturned)
                    }
                }
            };
            self.publish_terminal(job);
            result
        }
    }
}

#[cfg(test)]
pub(crate) use ledger::AdmissionLedger;

#[cfg(test)]
#[path = "work_tests.rs"]
mod tests;

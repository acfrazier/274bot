//! Frozen native script values (F0 item 3, §25 as superseded by §28).
//!
//! These definitions do not activate a seat, encode a wire, or own a runtime.
//! `script::native` supplies behavior and typed family arguments. The seat model
//! is a dev-only conformance oracle, not a product dependency.
#![warn(missing_docs)]

use crate::identity::{Cancellation, ExecutionId, RequestId, Scope, SessionKey, Settlement};
use crate::selected::RunKey;
use std::num::{NonZeroU32, NonZeroU64};

/// Maximum admitted entries (§28.3).
pub const MAX_LIVE: usize = 16;
/// Maximum roots and reserved future root positions (§28.3).
pub const MAX_ROOTS: usize = 4;
/// Every live leaf reserves one of the four future root positions (§28.3).
pub const MAX_LIVE_LEAVES: usize = MAX_ROOTS;
/// Maximum seat depth, including the root (§28.3).
pub const MAX_LINEAGE_DEPTH: usize = 4;
/// Shared live plus retained-terminal credits (§25 R5-8.1).
pub const MAX_RESULTS: usize = 32;
/// Maximum unacknowledged starts (§28.4).
pub const MAX_PENDING_STARTS: usize = 32;
/// Maximum carried decisions, one per live prompt (§28.5).
pub const MAX_DECIDE_CARRY: usize = 16;
/// Maximum carried cancellations, including pending hybrid children (§28.5).
pub const MAX_CANCEL_CARRY: usize = MAX_LIVE + MAX_PENDING_STARTS;
/// Exact unpinned revocation-history capacity (§28.2).
pub const REVOKED_RING: usize = 64;
/// Pre-cutover held walk requests (§25 R5-1.2).
pub const HELD_WALKS: usize = 8;
/// Calls per row per tick, across passes (§25 R5-6.3).
pub const CALLS_PER_TICK: usize = 256;
/// Synchronous asks per machine step (§25 R5-6.3).
pub const ASKS_PER_STEP: usize = 512;
/// Maximum interaction rows and early gameplay-gate capacity (§28.5).
pub const MAX_INTERACT_REQS: usize = 256;
/// Reserved legacy Walk releases (§28.5).
pub const MAX_WALK_RELEASES: usize = 16;
/// Walk releases plus coalesced GatherStop and CombatStop (§28.5).
pub const MAX_RELEASES: usize = MAX_WALK_RELEASES + 2;
/// One reserved run-policy row (§28.5).
pub const MAX_RUN_POLICY: usize = 1;
/// Reserved control/release rows, independent of gameplay occupancy (§28.5).
pub const CONTROL_RESERVATION: usize =
    MAX_PENDING_STARTS + MAX_DECIDE_CARRY + MAX_CANCEL_CARRY + MAX_RUN_POLICY + MAX_RELEASES;
/// Encoder gameplay/lifecycle capacity. Unused reservation does not enlarge it.
pub const MAX_ENCODED_GAMEPLAY: usize = MAX_INTERACT_REQS - CONTROL_RESERVATION;
/// Maximum charge of one pending start, excluding script-owned closure heap.
pub const MAX_START_CHARGE: usize = 64 * 1024;
/// Fixed metadata charged for one canonical pending payload (§28.4).
pub const START_METADATA: usize = 128;
/// Charge for each rooted callback handle (§28.4).
pub const HOOK_CHARGE: usize = 16;
/// Maximum rooted callback handles per pending start (§28.4).
pub const MAX_HOOKS: usize = 32;
/// Existing Gather encoded-settings limit, not a general argument-size limit.
pub const GATHER_SETTINGS_MAX_BYTES: usize = 4 * 1024;

/// Session authorities with a characterized foreground predicate (§27 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFamily {
    /// Hunt does not exclude other requests' gameplay.
    Hunt,
    /// Gather owns the gameplay foreground.
    Gather,
    /// Combat owns the gameplay foreground.
    Combat,
    /// A quest-progress read owns the gameplay foreground.
    Progress,
    /// Quester's session classification; its family activation characterizes foreground.
    Quester,
    /// DeathRecovery's session classification; its family activation characterizes foreground.
    DeathRecovery,
}
impl SessionFamily {
    /// Current foreground predicate. `None` is an uncharacterized family,
    /// never permission to infer a new filter (§25 R5-8.5, §27 §6).
    pub const fn foreground(self) -> Option<bool> {
        match self {
            Self::Hunt => Some(false),
            Self::Gather | Self::Combat | Self::Progress => Some(true),
            Self::Quester | Self::DeathRecovery => None,
        }
    }
}

/// Requests have a class; one-shot actions are never requests (§25 R5-1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestClass {
    /// A foreground-capable session authority container.
    Session(SessionFamily),
    /// One awaited operation that can compose under a hook.
    Capability,
}
/// How a child acquired its current parent (§28.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildOrigin {
    /// Created in the parent's hook or its continuation.
    Started,
    /// Adopted after a non-root ancestor completed normally.
    Adopted,
}
/// Lineage carrier interpretation, independent of its V8 representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lineage {
    /// A structurally guarded non-hook entry.
    Root,
    /// The innermost hook or one of its asynchronous continuations.
    Token(RequestId),
    /// An unrelated replacement of the continuation mapping.
    Foreign,
}
impl Lineage {
    /// Resolve Foreign against the synchronous guard, without erasing its
    /// diagnostic classification. Log that classification once per run.
    pub const fn parent(self, innermost_hook: Option<RequestId>) -> Option<RequestId> {
        match self {
            Self::Root => None,
            Self::Token(id) => Some(id),
            Self::Foreign => innermost_hook,
        }
    }
}
/// Lookup classification for an unpinned handle (§28.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityStatus {
    /// Successfully admitted, or an active in-isolate row.
    Live,
    /// Unacknowledged or credit-deferred start, not live authority.
    Pending,
    /// Exact revocation proof.
    Revoked,
    /// Ordinary completion proof. A new push becomes root.
    Completed,
    /// Below history/generation floor. Never infer root or revocation.
    Expired,
}
/// Separate authority boundary bit. ManualWalk retains Done(false).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityBoundary {
    /// Ordinary completion does not revoke an accepted lease.
    Preserved,
    /// Owner or ancestor revoked, including ManualWalk's authority-only bit.
    Revoked,
}
/// Whether a pending-owned emission acquired successful admission (§28.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionWitness {
    /// Drop if dispatched before successful admission. Never replay.
    Pending,
    /// Owner admitted before dispatch.
    Admitted,
}
/// Immutable push-time authority. Custody retains this value through dispatch;
/// neither terminal retirement nor history-floor movement reconstructs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedAuthority {
    owner: RequestId,
    ancestors: [Option<RequestId>; MAX_LINEAGE_DEPTH],
    session: Option<RequestId>,
}
impl CapturedAuthority {
    /// Capture an owner and nearest-first seat ancestry. A mirror/pending owner
    /// is not a seat depth node, so four seat ancestors fit (§28.2).
    pub fn new(
        owner: RequestId,
        ancestors: &[RequestId],
        session: Option<RequestId>,
    ) -> Result<Self, ContractError> {
        if ancestors.len() > MAX_LINEAGE_DEPTH {
            return Err(ContractError::LineageDepth);
        }
        if ancestors.contains(&owner)
            || ancestors
                .iter()
                .enumerate()
                .any(|(i, id)| ancestors[..i].contains(id))
        {
            return Err(ContractError::CyclicLineage);
        }
        if session.is_some_and(|id| id != owner && !ancestors.contains(&id)) {
            return Err(ContractError::SessionOutsideLineage);
        }
        let mut captured = [None; MAX_LINEAGE_DEPTH];
        for (destination, id) in captured.iter_mut().zip(ancestors) {
            *destination = Some(*id);
        }
        Ok(Self {
            owner,
            ancestors: captured,
            session,
        })
    }
    /// The owner at push, even after ordinary completion.
    pub const fn owner(&self) -> RequestId {
        self.owner
    }
    /// Captured nearest-first ancestors, without allocation.
    pub fn ancestors(&self) -> impl Iterator<Item = RequestId> + '_ {
        self.ancestors.iter().flatten().copied()
    }
    /// Push-time Session identity, not dispatch-time reconstruction.
    pub const fn session(&self) -> Option<RequestId> {
        self.session
    }
}
/// Lookup proof at a gameplay gate, separate from a pinned lease's boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmissionProof {
    /// Valid root lookup or an exact accepted/pending row lease.
    Proven,
    /// An unpinned lookup below the history/generation floor.
    ExpiredLookup,
    /// Nonzero external authority with no live, retained, or pinned proof.
    InvalidExternal,
}
/// Host actions are outside seat lineage and encoder custody (§27 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmissionClass {
    /// Ordinary game action.
    Gameplay,
    /// AbortWalk, GatherStop or CombatStop; never foreground-filtered.
    Release,
    /// Non-game lifecycle row; not foreground-filtered but encoder-capped.
    Lifecycle,
    /// Guardian or another host actor, dispatched immediately in host order.
    Host,
}
/// Frozen dispatch-table result, not a slot dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDisposition {
    /// Deliver once in the owning order.
    Dispatch,
    /// Drop and increment the typed dropped-row counter.
    Drop,
    /// Preserve reserved release/control custody while the guardian holds.
    Retain,
    /// Invalid external authority: drop and report the protocol defect.
    ProtocolDefect,
}
/// Evaluate the §28.2 eligibility table on already captured, proven authority.
/// The caller separately checks its F0 `Fence<ExecutionId>` and session gate.
/// Stop/reset/held-tick whole-queue discard precedes this per-row table.
///
/// Gameplay `EmissionProof::InvalidExternal` is a protocol defect in both
/// hold and live columns of the §28.2 matrix. Classify it before the guardian
/// hold branch so a hold cannot hide the defect. The defective row is still
/// dropped and never replayed.
///
/// `Lifecycle` → `Retain` during a hold is a F0 choice. SG keeps releases and
/// controls and does not specify lifecycle rows (`28-seat-sg.md` §28.2).
pub const fn dispatch_disposition(
    class: EmissionClass,
    captured_session: Option<RequestId>,
    foreground: Option<RequestId>,
    witness: AdmissionWitness,
    proof: EmissionProof,
    boundary: AuthorityBoundary,
    guardian_held: bool,
) -> DispatchDisposition {
    if matches!(class, EmissionClass::Host) {
        return DispatchDisposition::Dispatch;
    }
    if matches!(class, EmissionClass::Gameplay) && matches!(proof, EmissionProof::InvalidExternal) {
        return DispatchDisposition::ProtocolDefect;
    }
    if guardian_held {
        return if matches!(class, EmissionClass::Gameplay) {
            DispatchDisposition::Drop
        } else {
            DispatchDisposition::Retain
        };
    }
    if !matches!(class, EmissionClass::Gameplay) {
        return DispatchDisposition::Dispatch;
    }
    match proof {
        EmissionProof::ExpiredLookup => return DispatchDisposition::Drop,
        EmissionProof::InvalidExternal => return DispatchDisposition::ProtocolDefect,
        EmissionProof::Proven => {}
    }
    if matches!(boundary, AuthorityBoundary::Revoked)
        || matches!(witness, AdmissionWitness::Pending)
    {
        return DispatchDisposition::Drop;
    }
    match foreground {
        None => DispatchDisposition::Dispatch,
        Some(current) => match captured_session {
            Some(captured) if captured.get() == current.get() => DispatchDisposition::Dispatch,
            _ => DispatchDisposition::Drop,
        },
    }
}

/// Closed admission/effect refusal vocabulary (§25 R5-9.1 plus §28.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Live, root, leaf-reservation or pending capacity is unavailable.
    #[error("busy")]
    Busy,
    /// A live Started child already occupies the parent's child slot.
    #[error("parent busy")]
    ParentBusy,
    /// Exact parent revocation proof.
    #[error("parent not live")]
    ParentNotLive,
    /// The child would exceed the inclusive depth bound.
    #[error("lineage depth")]
    LineageDepth,
    /// Local pending payload/handle custody exceeds its bound.
    #[error("arguments too large")]
    ArgsTooLarge,
    /// Unpinned parent lookup is below the history/generation floor.
    #[error("history expired")]
    HistoryExpired,
    /// Unrecognized family or external authority, not forged completion.
    #[error("unknown request")]
    Unknown,
    /// A machine attempted an effect while waiting for its decision.
    #[error("awaiting decision")]
    AwaitingDecision,
}
/// Seat-specific reason detail, carried beside the shared Cancellation value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// Non-kept reset. Local settlement, no published seat terminal.
    Reset,
    /// Replacement of an exclusive Capability subtree.
    Superseded,
    /// Stop. No JS settlement or published seat terminal.
    Terminated,
    /// Manual Combat takeover; walking takeover remains Done(false).
    UserInput,
    /// Descendant of a revoked parent, settled leaf-first.
    ParentAborted,
}
impl AbortReason {
    /// Project the detailed reason into F0's shared cancellation vocabulary.
    pub const fn cancellation(self) -> Cancellation {
        match self {
            Self::Superseded => Cancellation::Superseded,
            Self::Terminated => Cancellation::Operator,
            Self::Reset | Self::UserInput | Self::ParentAborted => Cancellation::ScopeInvalidated,
        }
    }
}
/// Completion payload. A refusal is a terminal completion of a never-live start.
#[derive(Debug, PartialEq, Eq)]
pub enum Completion<T> {
    /// The entry's machine completed, including timeout/manual-walk false.
    Done(T),
    /// Admission refused with a result credit, or locally before transfer.
    Refused(Refusal),
}
/// Script outcome composed over the one F0 Settlement envelope. Detailed
/// abort reasons do not establish a second cancellation/settlement protocol.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome<T, E> {
    settlement: Settlement<Completion<T>, E>,
    abort: Option<AbortReason>,
}
impl<T, E> Outcome<T, E> {
    /// Ordinary machine completion; does not revoke captured authority.
    pub fn done(value: T) -> Self {
        Self {
            settlement: Settlement::Completed(Completion::Done(value)),
            abort: None,
        }
    }
    /// Native/hook failure. Thrown JS values stay local to the isolate.
    pub fn failed(error: E) -> Self {
        Self {
            settlement: Settlement::Failed(error),
            abort: None,
        }
    }
    /// Refusal, distinct from stale-stamp drop or credit deferral.
    pub fn refused(reason: Refusal) -> Self {
        Self {
            settlement: Settlement::Completed(Completion::Refused(reason)),
            abort: None,
        }
    }
    /// External revocation with its shared and script-specific reasons.
    pub fn aborted(reason: AbortReason) -> Self {
        Self {
            settlement: Settlement::Cancelled(reason.cancellation()),
            abort: Some(reason),
        }
    }
    /// Borrow the shared terminal envelope.
    pub const fn settlement(&self) -> &Settlement<Completion<T>, E> {
        &self.settlement
    }
    /// Detailed revocation reason, absent for ordinary Done/Failed/Refused.
    pub const fn abort_reason(&self) -> Option<AbortReason> {
        self.abort
    }
    /// Ordinary completion and HookThrew do not revoke accepted authority.
    pub const fn revokes_authority(&self) -> bool {
        self.abort.is_some()
    }
    /// Consume the outcome into the shared settlement and reason detail.
    pub fn into_settlement(self) -> (Settlement<Completion<T>, E>, Option<AbortReason>) {
        (self.settlement, self.abort)
    }
}
/// Hook cancellation is a control, not always a revocation (§25 R5-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackCancelReason {
    /// End only the named row as Failed. Detached children survive.
    HookThrew,
    /// Cancel the named subtree leaf-first, including pending hybrid starts.
    ParentAborted,
}
/// One carried callback cancellation (§28.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackCancellation {
    of: RequestId,
    reason: CallbackCancelReason,
}
impl CallbackCancellation {
    /// Name the control's target and semantic reason.
    pub const fn new(of: RequestId, reason: CallbackCancelReason) -> Self {
        Self { of, reason }
    }
    /// Target request.
    pub const fn of(self) -> RequestId {
        self.of
    }
    /// Hook failure or hybrid parent revocation.
    pub const fn reason(self) -> CallbackCancelReason {
        self.reason
    }
}
/// Nonzero per-request decision sequence; exhausted sequences never wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecisionSequence(NonZeroU32);
impl DecisionSequence {
    /// First decision of a live request.
    pub const FIRST: Self = Self(NonZeroU32::MIN);
    /// Decode a prompt/answer sequence. Zero names no decision.
    pub const fn from_raw(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
    /// Its wire-compatible value.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
    /// Next prompt. Exhaustion is a protocol error, not sequence reuse.
    pub fn next(self) -> Result<Self, ContractError> {
        self.get()
            .checked_add(1)
            .and_then(Self::from_raw)
            .ok_or(ContractError::SequenceExhausted)
    }
}
/// Typed callback answer retained until decision acknowledgement or terminal receipt.
#[derive(Debug)]
pub struct Decide<A> {
    of: RequestId,
    sequence: DecisionSequence,
    answer: A,
}
impl<A> Decide<A> {
    /// Capture the actual observation-based answer once.
    pub const fn new(of: RequestId, sequence: DecisionSequence, answer: A) -> Self {
        Self {
            of,
            sequence,
            answer,
        }
    }
    /// Request whose prompt was answered.
    pub const fn of(&self) -> RequestId {
        self.of
    }
    /// Prompt sequence, not a progress-ring event sequence.
    pub const fn sequence(&self) -> DecisionSequence {
        self.sequence
    }
    /// Borrow the answer without cloning it.
    pub const fn answer(&self) -> &A {
        &self.answer
    }
    /// Transfer the owned answer.
    pub fn into_answer(self) -> A {
        self.answer
    }
}
/// A retained prompt lives on the status page, never in an overflowing progress ring.
#[derive(Debug)]
pub struct Prompt<A> {
    sequence: DecisionSequence,
    point: &'static str,
    args: A,
}
impl<A> Prompt<A> {
    /// Record the decision point and the observation the machine decided on.
    pub const fn new(sequence: DecisionSequence, point: &'static str, args: A) -> Self {
        Self {
            sequence,
            point,
            args,
        }
    }
    /// Per-request prompt sequence.
    pub const fn sequence(&self) -> DecisionSequence {
        self.sequence
    }
    /// Family-specific callback point.
    pub const fn point(&self) -> &'static str {
        self.point
    }
    /// Borrowed observation-based arguments.
    pub const fn args(&self) -> &A {
        &self.args
    }
}
/// Family-typed progress payload (§25 R5-9.1). Decisions are deliberately
/// absent: a prompt is durable page state and cannot be lost to ring overflow.
#[derive(Debug)]
pub enum NativeEvent<P, K, L, R> {
    /// Family phase observation.
    Phase(P),
    /// One ordered kill event, not a per-tick aggregate.
    Kill(K),
    /// Typed log identifier and arguments, owned by the family.
    Log(L),
    /// Family decision point and the observation it decided on.
    Reevaluate(R),
    /// Number of progress events lost, never a lost prompt.
    Lost(NonZeroU32),
}
/// Per-request sequenced progress payload. A publisher wraps this value in
/// F0's `identity::Event` with its existing Scope; it is not another notifier.
#[derive(Debug)]
pub struct NativeProgress<E> {
    of: RequestId,
    sequence: NonZeroU32,
    event: E,
}
impl<E> NativeProgress<E> {
    /// Construct one event with its nonzero, per-request sequence.
    pub const fn new(of: RequestId, sequence: NonZeroU32, event: E) -> Self {
        Self {
            of,
            sequence,
            event,
        }
    }
    /// Event owner.
    pub const fn of(&self) -> RequestId {
        self.of
    }
    /// Deduplication/order sequence. Never zero: the first event is 1.
    pub const fn sequence(&self) -> NonZeroU32 {
        self.sequence
    }
    /// Borrow the family payload without cloning.
    pub const fn event(&self) -> &E {
        &self.event
    }
    /// Move the payload to its one callback invocation.
    pub fn into_event(self) -> E {
        self.event
    }
}
/// Monotone terminal sequence. Zero is only the initial consumption watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalSequence(NonZeroU64);
impl TerminalSequence {
    /// First terminal in a run/generation.
    pub const FIRST: Self = Self(NonZeroU64::MIN);
    /// Decode a published sequence. Zero is not a terminal.
    pub const fn from_raw(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
    /// Wire-compatible sequence.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
    /// Advance without wrapping.
    pub fn next(self) -> Result<Self, ContractError> {
        self.get()
            .checked_add(1)
            .and_then(Self::from_raw)
            .ok_or(ContractError::SequenceExhausted)
    }
}
/// Generation-stamped native request header. Typed arguments stay with the
/// family in script; no JSON/JS value is required by this neutral envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestStamp {
    id: RequestId,
    parent: Option<RequestId>,
    local_parent: Option<RequestId>,
    execution: ExecutionId,
}
impl RequestStamp {
    /// Stamp one request. The execution's RunKey carries the current session.
    pub fn new(
        id: RequestId,
        parent: Option<RequestId>,
        local_parent: Option<RequestId>,
        execution: ExecutionId,
    ) -> Result<Self, ContractError> {
        if execution.run().session_key().is_none() {
            return Err(ContractError::UnboundRun);
        }
        if parent == Some(id) || local_parent == Some(id) {
            return Err(ContractError::CyclicLineage);
        }
        Ok(Self {
            id,
            parent,
            local_parent,
            execution,
        })
    }
    /// Request identity, using F0's isolate/host namespace.
    pub const fn id(self) -> RequestId {
        self.id
    }
    /// Nearest seat ancestor, if any.
    pub const fn parent(self) -> Option<RequestId> {
        self.parent
    }
    /// Local mirror parent, retained even after payload acknowledgement.
    pub const fn local_parent(self) -> Option<RequestId> {
        self.local_parent
    }
    /// Execution fence checked by the mutable dispatch owner.
    pub const fn execution(self) -> ExecutionId {
        self.execution
    }
    /// Current run/session composite stamp.
    pub const fn run(self) -> RunKey {
        self.execution.run()
    }
    /// Session epoch associated with this run stamp.
    pub fn session(self) -> SessionKey {
        self.run().session_key().expect("validated bound run")
    }
    /// Neutral event/submission scope.
    pub const fn scope(self) -> Scope {
        Scope::Execution(self.execution)
    }
}
/// How eligible request state behaves during Pause (§25 R5-8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PausePolicy {
    /// Retain cursor, promise and active time.
    Freeze,
    /// Interrupt the active sub-operation while keeping its Session.
    CancelOperation,
}
/// Manual movement semantics, per family (§25 R5-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverPolicy {
    /// Complete walking work Done(false), separately revoking authority.
    InterruptWalk,
    /// Revoke Combat as UserInput.
    CancelCombat,
    /// Signal Gather without ending the Session.
    SignalGather,
    /// No manual-input takeover behavior.
    None,
}
/// Closed reconnect verbs (§25 R5-8.5), composable for Session sub-operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicy {
    /// Level-triggered custody until sticky acknowledgement/terminal receipt.
    Carry,
    /// Continue at the saved cursor, never restart the request.
    Resume,
    /// Settle the sub-operation; its Session remains live.
    Interrupt,
    /// Discard a one-shot action, never replay it.
    Drop,
}
/// Boundary publication/settlement policy (§25 R5-7, §28.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryDelivery {
    /// Published with an existing live credit; local promise settles on receipt.
    RetainedTerminal,
    /// Non-kept reset settles locally, clears ledger/carry and resets sequence.
    LocalReset,
    /// Stop clears rows, outcomes, ledger and carry without JS settlement.
    SilentStop,
    /// Hold/pause/kept reconnect produce no new terminal.
    Hold,
}
/// Normative batch phases (§28.4). The seat model owns their transition rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStage {
    /// Free retained credits through the consumed-terminal watermark.
    Watermark,
    /// Cancel before Decide; controls convert existing credits.
    Controls,
    /// Retry carried starts in carry order, including credit-deferred starts.
    PendingStarts,
    /// Apply immutable captured authority after admission/control processing.
    Dispatch,
}
/// Frozen order, not an alternate batch executor.
pub const BATCH_ORDER: [BatchStage; 4] = [
    BatchStage::Watermark,
    BatchStage::Controls,
    BatchStage::PendingStarts,
    BatchStage::Dispatch,
];
/// Encoder-reserved release kinds. Unlike the model's compact sentinels, the
/// native surface names singleton releases explicitly (§28.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// Coalesces for the same armed Walk request id.
    AbortWalk(RequestId),
    /// One singleton beyond the Walk release slots.
    GatherStop,
    /// Another independent singleton beyond the Walk release slots.
    CombatStop,
}
/// Complete R5-7 boundary rows plus the distinct §28 held-tick queue event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// Ordinary machine Done, including a timeout Done(false).
    Done,
    /// Native failure, synchronous throw or ask-Promise not-impl.
    Failed,
    /// Awaited-hook throw is Failed, never an ancestor revocation.
    HookThrew,
    /// Hybrid parent revokes the named child subtree.
    ParentAborted,
    /// Capability group replacement revokes the old subtree.
    Superseded,
    /// Done(false) for the walking owner, with its separate authority bit.
    ManualWalk,
    /// UserInput for the Combat owner.
    ManualCombat,
    /// Gather receives a signal and continues.
    GatherSignal,
    /// Tear down without publishing terminals or settling JS waits.
    Stop,
    /// Non-kept reset locally settles Reset and clears ledger/carry/sequence.
    Reset,
    /// Kept reconnect retains request/prompt/control/page state.
    KeptReconnect,
    /// Pause freezes request and active-clock state.
    Pause,
    /// Guardian takes priority over every Session foreground.
    GuardianHold,
    /// A dropped held tick discards gameplay and releases, not requests.
    HeldTick,
}
impl Boundary {
    /// Whether the boundary publishes, locally settles, silently clears, or holds.
    pub const fn delivery(self) -> BoundaryDelivery {
        match self {
            Self::Done
            | Self::Failed
            | Self::HookThrew
            | Self::ParentAborted
            | Self::Superseded
            | Self::ManualWalk
            | Self::ManualCombat => BoundaryDelivery::RetainedTerminal,
            Self::Reset => BoundaryDelivery::LocalReset,
            Self::Stop => BoundaryDelivery::SilentStop,
            Self::GatherSignal
            | Self::KeptReconnect
            | Self::Pause
            | Self::GuardianHold
            | Self::HeldTick => BoundaryDelivery::Hold,
        }
    }
    /// Leaf-first descendant revocation precedes the owner terminal.
    /// Stop/Reset keep their own reason on every level, with no seat publication.
    pub const fn revokes_descendants(self) -> bool {
        matches!(
            self,
            Self::ParentAborted
                | Self::Superseded
                | Self::ManualWalk
                | Self::ManualCombat
                | Self::Stop
                | Self::Reset
        )
    }
    /// Guardian hold and kept reconnect preserve reserved releases. A held
    /// output tick and non-kept teardown instead discard the whole queue.
    pub const fn keeps_queued_releases(self) -> bool {
        !matches!(self, Self::Stop | Self::Reset | Self::HeldTick)
    }
}
/// One live status-page entry. A prompt is durable page state, not progress.
#[derive(Debug)]
pub struct LiveStatus<P, A> {
    stamp: RequestStamp,
    origin: ChildOrigin,
    phase: P,
    decisions_consumed: u32,
    prompt: Option<Prompt<A>>,
}
impl<P, A> LiveStatus<P, A> {
    /// Capture one live entry. Its next prompt must follow consumed decisions.
    pub fn new(
        stamp: RequestStamp,
        origin: ChildOrigin,
        phase: P,
        decisions_consumed: u32,
        prompt: Option<Prompt<A>>,
    ) -> Result<Self, ContractError> {
        if prompt.as_ref().is_some_and(|prompt| {
            decisions_consumed.checked_add(1) != Some(prompt.sequence().get())
        }) {
            return Err(ContractError::DecisionOrder);
        }
        Ok(Self {
            stamp,
            origin,
            phase,
            decisions_consumed,
            prompt,
        })
    }
    /// Identity and parent links.
    pub const fn stamp(&self) -> RequestStamp {
        self.stamp
    }
    /// Started versus adopted child-slot semantics.
    pub const fn origin(&self) -> ChildOrigin {
        self.origin
    }
    /// Current family phase, retained in the keyframe without a JS value.
    pub const fn phase(&self) -> &P {
        &self.phase
    }
    /// Sticky acknowledgement of consumed decisions.
    pub const fn decisions_consumed(&self) -> u32 {
        self.decisions_consumed
    }
    /// Retained current prompt.
    pub const fn prompt(&self) -> Option<&Prompt<A>> {
        self.prompt.as_ref()
    }
}
/// Retained immutable terminal record. The mutable publisher uses F0's
/// `identity::Terminal` for exactly-once publication/consumption.
#[derive(Debug)]
pub struct ResultRecord<T, E> {
    of: RequestId,
    sequence: TerminalSequence,
    outcome: Outcome<T, E>,
}
impl<T, E> ResultRecord<T, E> {
    /// Assign the terminal's sequence when its existing credit converts.
    pub const fn new(of: RequestId, sequence: TerminalSequence, outcome: Outcome<T, E>) -> Self {
        Self {
            of,
            sequence,
            outcome,
        }
    }
    /// Request whose live credit became retained, or the refused start.
    pub const fn of(&self) -> RequestId {
        self.of
    }
    /// Ascending settlement sequence.
    pub const fn sequence(&self) -> TerminalSequence {
        self.sequence
    }
    /// Borrow the result until the adapter reports its consumption watermark.
    pub const fn outcome(&self) -> &Outcome<T, E> {
        &self.outcome
    }
    /// Transfer the result once at the owning terminal cell.
    pub fn into_outcome(self) -> Outcome<T, E> {
        self.outcome
    }
}
/// Borrowed level-triggered page. No world clone or allocation is needed.
#[derive(Debug)]
pub struct StatusPage<'a, P, A, T, E> {
    live: &'a [LiveStatus<P, A>],
    results: &'a [ResultRecord<T, E>],
}
impl<'a, P, A, T, E> StatusPage<'a, P, A, T, E> {
    /// Validate the bounded credit accounting and ascending terminal order.
    pub fn new(
        live: &'a [LiveStatus<P, A>],
        results: &'a [ResultRecord<T, E>],
    ) -> Result<Self, ContractError> {
        if live.len() > MAX_LIVE || live.len().saturating_add(results.len()) > MAX_RESULTS {
            return Err(ContractError::ResultCredits);
        }
        if results
            .windows(2)
            .any(|pair| pair[0].sequence() >= pair[1].sequence())
        {
            return Err(ContractError::TerminalOrder);
        }
        Ok(Self { live, results })
    }
    /// Every admitted live entry, including outstanding prompts.
    pub const fn live(&self) -> &'a [LiveStatus<P, A>] {
        self.live
    }
    /// Every unconsumed result, in terminal sequence order.
    pub const fn results(&self) -> &'a [ResultRecord<T, E>] {
        self.results
    }
}

/// Failures of constructing frozen values, not seat admission outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// More ancestors than the bounded captured lease supports.
    #[error("lineage depth exceeded")]
    LineageDepth,
    /// Duplicate/self lineage cannot represent a forest.
    #[error("cyclic lineage")]
    CyclicLineage,
    /// Captured Session must be the owner or one of its seat ancestors.
    #[error("session outside captured lineage")]
    SessionOutsideLineage,
    /// A request must name a real slot/session, not the legacy placeholder.
    #[error("unbound run")]
    UnboundRun,
    /// No reuse after sequence exhaustion.
    #[error("script sequence exhausted")]
    SequenceExhausted,
    /// A status page violates the live plus retained credit bound.
    #[error("result credit bound exceeded")]
    ResultCredits,
    /// Terminals must appear in strictly increasing sequence order.
    #[error("terminal sequence order")]
    TerminalOrder,
    /// A current prompt must be exactly one past decisions_consumed.
    #[error("decision sequence order")]
    DecisionOrder,
}

#[cfg(test)]
#[path = "script_contract_tests.rs"]
mod tests;

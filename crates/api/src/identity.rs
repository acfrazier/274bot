//! Identity and time envelope (F0 item 1: contract §26 §1 item 1, B03
//! "Identity envelope", §25 R5-1.2 `RequestId` namespace).
//!
//! Frozen here:
//!
//! - the identities and their nesting: a [`SlotKey`] (one slot worker
//!   lifetime) contains [`SessionKey`]s (one game session each) and
//!   [`RunKey`]s (one operator run each, stamped with the session it currently
//!   executes in); an [`ExecutionId`] is the machine or isolate that happens
//!   to implement a run; a [`RequestId`] names one request inside its
//!   namespace; a [`ProcessIncarnation`] names one process start;
//! - exhaustion: every generation and sequence advances by a checked add and
//!   refuses with [`IdentityError::Exhausted`] instead of wrapping or reusing
//!   a value, leaving its state as it was;
//! - generation checks: [`Fence`] admits only the completion stamped with the
//!   current identity, [`Scope::contains`] checks nesting agreement,
//!   [`Terminal`] publishes once and is consumed once;
//! - publish-before-notify: [`Publication::publish`] makes the new state
//!   readable before it calls the [`Notifier`], and a [`Notice`] carries only
//!   identity and a sequence, never a payload or a wake primitive;
//! - the time domains, kept as distinct types: an absolute monotonic
//!   [`Deadline`], pausable [`ActiveTime`] from an [`ActiveClock`],
//!   [`ProcessTime`] elapsed since the [`ProcessOrigin`] of this incarnation,
//!   and a durable wall-clock [`DurableTimestamp`].
//!
//! Nothing here is a transport token, park or retirement type; those are
//! T1-internal (§24 G1).
//!
//! Today's representations (recorded for the T1/T2 activation that migrates
//! them): the slot incarnation is `SlotScript.incarnation: u64` with `0` for
//! "unbound" (`crates/script/src/slot.rs:164`), minted by
//! `crates/host-play/src/script_runtime.rs:84-92`; the session epoch is
//! `SlotScript.work_epoch` (`slot.rs:242`, `wrapping_add`), published into
//! `RunKey.session` by `rekey_session`; the execution generation is
//! `SlotScript.runtime_generation` (`slot.rs:257`, `wrapping_add`); request
//! ids are the native ledger's `NonZeroU64` cursor
//! (`crates/script/src/native/ledger.rs:132`) and the adapter `Handle`
//! (`crates/script/src/machine.rs:148`); the active clock lived in
//! `crates/script/src/native/ledger.rs` and moved here unchanged.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::selected::RunKey;

/// Which identity or sequence space refused to advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentityKind {
    /// Slot worker incarnations ([`SlotKeyMinter`]).
    Slot,
    /// Session epochs within one slot ([`SessionKey::next`]).
    SessionEpoch,
    /// Operator runs within one slot ([`RunKey::next_run`]).
    Run,
    /// Execution generations within one run ([`ExecutionId::next_generation`]).
    ExecutionGeneration,
    /// Host-minted request ids ([`RequestIdMinter`]).
    HostRequest,
    /// Publication sequences ([`Publication::publish`]).
    PublishSequence,
}

impl IdentityKind {
    /// Stable lower-case label for logs and errors.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Slot => "slot",
            Self::SessionEpoch => "session epoch",
            Self::Run => "run",
            Self::ExecutionGeneration => "execution generation",
            Self::HostRequest => "host request",
            Self::PublishSequence => "publication sequence",
        }
    }
}

impl fmt::Display for IdentityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The namespace a [`RequestId`] was minted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestNamespace {
    /// Isolate-minted: the adapter `Handle`, unique per run (§25 R5-1.2).
    Isolate,
    /// Host-minted: native cards, host-originated sessions and background
    /// work; unique within the process incarnation (§26 §2.2).
    Host,
}

impl fmt::Display for RequestNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Isolate => "isolate",
            Self::Host => "host",
        })
    }
}

/// Why an identity could not be minted, advanced or composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The space has no unused value left; it never wraps or reuses.
    #[error("{0} identity space exhausted: refusing to reuse a generation")]
    Exhausted(IdentityKind),
    /// A raw request id does not lie in the namespace it was claimed for.
    #[error("request id {raw} is outside the {namespace} namespace")]
    OutOfNamespace {
        /// The offending raw value.
        raw: u64,
        /// The namespace the caller claimed.
        namespace: RequestNamespace,
    },
    /// An isolate-minted request id is unique only within its run, so a work
    /// identity correlating one must name that run or execution.
    #[error("an isolate-minted request id needs a run or execution scope")]
    IsolateRequestWithoutRun,
    /// A job id must be host-minted: only the host namespace is unique within
    /// the process incarnation (§26 §2.2), so an isolate-minted id, unique
    /// only per run, names no job.
    #[error("request id {raw} is isolate-minted and cannot name a job")]
    JobIdNotHostMinted {
        /// The offending raw value.
        raw: u64,
    },
}

fn checked_next(kind: IdentityKind, value: u64) -> Result<u64, IdentityError> {
    value.checked_add(1).ok_or(IdentityError::Exhausted(kind))
}

/// Mint the value after `last`, refusing on overflow. `last` holds the last
/// value minted, so the whole space up to `u64::MAX` is usable.
fn mint_atomic(kind: IdentityKind, last: &AtomicU64) -> Result<NonZeroU64, IdentityError> {
    let previous = last
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        })
        .map_err(|_| IdentityError::Exhausted(kind))?;
    NonZeroU64::new(previous.wrapping_add(1)).ok_or(IdentityError::Exhausted(kind))
}

/// Opaque identity of one process start (§26 §3.1 `condition_id`).
///
/// Not a pid, user name, wall-clock reading or reusable service index: it is
/// exactly the start nonce composition supplies at process start, under the
/// non-reuse policy composition owns (see
/// [`from_start_nonce`](Self::from_start_nonce)). This type adds no
/// heuristic of its own. Compare only for equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessIncarnation(NonZeroU64);

impl ProcessIncarnation {
    /// The incarnation named by `nonce`, the value composition minted once
    /// for this process start.
    ///
    /// Non-reuse across process starts is the nonce's assurance, not this
    /// type's: the T1 composition owner supplies a value no earlier start of
    /// this installation produced. The accepted source is an open freeze
    /// decision recorded with F0-1/2 and owned by T1: a durable counter in
    /// local state (F0-7 `LocalStatePort`) guarantees it; a 64-bit value from
    /// OS entropy gives only a collision probability the orchestrator must
    /// accept explicitly. Neither a clock reading nor a pid qualifies: both
    /// repeat. Two calls with the same nonce name the same incarnation, so
    /// a nonce is minted once per process. Tests pass distinct nonces for
    /// the distinct incarnations they need.
    pub const fn from_start_nonce(nonce: NonZeroU64) -> Self {
        Self(nonce)
    }

    /// The start nonce this incarnation was named by.
    pub const fn start_nonce(self) -> NonZeroU64 {
        self.0
    }
}

/// One slot worker lifetime: the process-unique incarnation Play assigns when
/// it spawns the worker. A respawned profile gets a new key even when its
/// visible name is reused. Never persisted and never a display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotKey(NonZeroU64);

impl SlotKey {
    /// The key for a raw incarnation; `0` is today's "unbound" sentinel and
    /// names no slot.
    pub const fn from_incarnation(incarnation: u64) -> Option<Self> {
        match NonZeroU64::new(incarnation) {
            Some(incarnation) => Some(Self(incarnation)),
            None => None,
        }
    }

    /// The raw incarnation.
    pub const fn incarnation(self) -> NonZeroU64 {
        self.0
    }
}

/// Process-wide minter of [`SlotKey`]s: starts at 1, never reuses, refuses
/// at exhaustion.
#[derive(Debug)]
pub struct SlotKeyMinter {
    last: AtomicU64,
}

impl SlotKeyMinter {
    /// A minter whose first key is incarnation 1.
    pub const fn new() -> Self {
        Self::starting_at(NonZeroU64::MIN)
    }

    /// A minter whose first key is `first`.
    pub const fn starting_at(first: NonZeroU64) -> Self {
        Self {
            last: AtomicU64::new(first.get() - 1),
        }
    }

    /// Mint the next key, or refuse once the space is exhausted.
    pub fn mint(&self) -> Result<SlotKey, IdentityError> {
        mint_atomic(IdentityKind::Slot, &self.last).map(SlotKey)
    }
}

impl Default for SlotKeyMinter {
    fn default() -> Self {
        Self::new()
    }
}

/// One game session of one slot: the slot plus the session epoch, which
/// advances at every session boundary (clean logout, drop, relog, Stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionKey {
    slot: SlotKey,
    epoch: u64,
}

impl SessionKey {
    /// The first session epoch of a slot (epoch 0, before any boundary).
    pub const fn first(slot: SlotKey) -> Self {
        Self { slot, epoch: 0 }
    }

    /// A session key from its parts.
    pub const fn new(slot: SlotKey, epoch: u64) -> Self {
        Self { slot, epoch }
    }

    /// The key of the session after the next boundary, or a refusal once the
    /// epoch space is exhausted.
    pub fn next(self) -> Result<Self, IdentityError> {
        Ok(Self {
            slot: self.slot,
            epoch: checked_next(IdentityKind::SessionEpoch, self.epoch)?,
        })
    }

    /// The owning slot.
    pub const fn slot(self) -> SlotKey {
        self.slot
    }

    /// The session epoch within the slot.
    pub const fn epoch(self) -> u64 {
        self.epoch
    }
}

impl RunKey {
    /// The slot this run belongs to, or `None` for an unbound placeholder
    /// (`slot == 0`).
    pub const fn slot_key(self) -> Option<SlotKey> {
        SlotKey::from_incarnation(self.slot)
    }

    /// The session this run is currently stamped with, or `None` for an
    /// unbound placeholder.
    pub const fn session_key(self) -> Option<SessionKey> {
        match self.slot_key() {
            Some(slot) => Some(SessionKey::new(slot, self.session)),
            None => None,
        }
    }

    /// Whether both keys name the same operator run, whatever session each is
    /// stamped with. A relog re-stamps the session and keeps the run.
    pub const fn same_run(self, other: Self) -> bool {
        self.slot == other.slot && self.run == other.run
    }
}

/// The machine or isolate that implements a run right now. A watchdog
/// restart keeps the run and advances the generation; a completion stamped
/// with an older generation is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecutionId {
    run: RunKey,
    generation: NonZeroU64,
}

impl ExecutionId {
    /// The first execution of a run (generation 1).
    pub const fn first(run: RunKey) -> Self {
        Self {
            run,
            generation: NonZeroU64::MIN,
        }
    }

    /// An execution id from its parts.
    pub const fn new(run: RunKey, generation: NonZeroU64) -> Self {
        Self { run, generation }
    }

    /// The execution that replaces this one, or a refusal once the generation
    /// space is exhausted.
    pub fn next_generation(self) -> Result<Self, IdentityError> {
        Ok(Self {
            run: self.run,
            generation: self
                .generation
                .checked_add(1)
                .ok_or(IdentityError::Exhausted(IdentityKind::ExecutionGeneration))?,
        })
    }

    /// The run this execution implements.
    pub const fn run(self) -> RunKey {
        self.run
    }

    /// The generation within the run.
    pub const fn generation(self) -> NonZeroU64 {
        self.generation
    }
}

/// One request: a seat entry, a native request or a background job (§25
/// R5-1.2; §26 §2.2).
///
/// Namespace (frozen): raw `0` is "none/root" and is represented by
/// `Option::None`; isolate-minted ids are the adapter `Handle` and occupy
/// `1 ..= 2^62 - 1`, unique per run; host-minted ids occupy `2^62 ..` and are
/// unique within the process incarnation. One id names at most one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(NonZeroU64);

impl RequestId {
    /// Smallest isolate-minted id.
    pub const ISOLATE_FIRST: u64 = 1;
    /// Largest isolate-minted id.
    pub const ISOLATE_LAST: u64 = (1 << 62) - 1;
    /// Smallest host-minted id.
    pub const HOST_FIRST: u64 = 1 << 62;

    /// An isolate-minted id from the adapter handle, refused outside
    /// `1 ..= 2^62 - 1`.
    pub const fn isolate(handle: u64) -> Result<Self, IdentityError> {
        if handle < Self::ISOLATE_FIRST || handle > Self::ISOLATE_LAST {
            return Err(IdentityError::OutOfNamespace {
                raw: handle,
                namespace: RequestNamespace::Isolate,
            });
        }
        match NonZeroU64::new(handle) {
            Some(handle) => Ok(Self(handle)),
            None => Err(IdentityError::OutOfNamespace {
                raw: handle,
                namespace: RequestNamespace::Isolate,
            }),
        }
    }

    /// Decode a raw wire value: `0` is "none".
    pub const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// The raw value, never `0`.
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Which namespace minted this id.
    pub const fn namespace(self) -> RequestNamespace {
        if self.0.get() >= Self::HOST_FIRST {
            RequestNamespace::Host
        } else {
            RequestNamespace::Isolate
        }
    }
}

/// Minter of host-namespace [`RequestId`]s for one process incarnation:
/// starts at `2^62`, never reuses, never crosses into the isolate range and
/// refuses at exhaustion.
#[derive(Debug)]
pub struct RequestIdMinter {
    last: AtomicU64,
}

impl RequestIdMinter {
    /// A minter whose first id is `2^62`.
    pub const fn new() -> Self {
        Self {
            last: AtomicU64::new(RequestId::HOST_FIRST - 1),
        }
    }

    /// A minter whose first id is `first`; refused below `2^62`.
    pub const fn starting_at(first: u64) -> Result<Self, IdentityError> {
        if first < RequestId::HOST_FIRST {
            return Err(IdentityError::OutOfNamespace {
                raw: first,
                namespace: RequestNamespace::Host,
            });
        }
        Ok(Self {
            last: AtomicU64::new(first - 1),
        })
    }

    /// Mint the next host id, or refuse once the space is exhausted.
    pub fn mint(&self) -> Result<RequestId, IdentityError> {
        mint_atomic(IdentityKind::HostRequest, &self.last).map(RequestId)
    }
}

impl Default for RequestIdMinter {
    fn default() -> Self {
        Self::new()
    }
}

/// The originating scope of an event, request or job: process-wide, or
/// nested under a slot, session, run or execution (B03; §26 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Process-scoped: preparation and map jobs need no fictitious slot.
    Process,
    /// One slot worker lifetime.
    Slot(SlotKey),
    /// One game session of a slot.
    Session(SessionKey),
    /// One operator run (stamped with its current session).
    Run(RunKey),
    /// One execution of a run.
    Execution(ExecutionId),
}

impl Scope {
    /// The slot this scope lies under, if any.
    pub const fn slot(&self) -> Option<SlotKey> {
        match self {
            Self::Process => None,
            Self::Slot(slot) => Some(*slot),
            Self::Session(session) => Some(session.slot()),
            Self::Run(run) => run.slot_key(),
            Self::Execution(execution) => execution.run().slot_key(),
        }
    }

    /// The session this scope lies under, if any.
    pub const fn session(&self) -> Option<SessionKey> {
        match self {
            Self::Process | Self::Slot(_) => None,
            Self::Session(session) => Some(*session),
            Self::Run(run) => run.session_key(),
            Self::Execution(execution) => execution.run().session_key(),
        }
    }

    /// The run this scope lies under, if any.
    pub const fn run(&self) -> Option<RunKey> {
        match self {
            Self::Process | Self::Slot(_) | Self::Session(_) => None,
            Self::Run(run) => Some(*run),
            Self::Execution(execution) => Some(execution.run()),
        }
    }

    /// The execution this scope names, if any.
    pub const fn execution(&self) -> Option<ExecutionId> {
        match self {
            Self::Execution(execution) => Some(*execution),
            _ => None,
        }
    }

    /// Whether `inner` lies within this scope (nesting agreement): a scope
    /// contains itself, the process contains everything, and a narrower scope
    /// is contained only when every identity it names agrees with this one.
    pub fn contains(&self, inner: &Scope) -> bool {
        match self {
            Self::Process => true,
            Self::Slot(slot) => inner.slot() == Some(*slot),
            Self::Session(session) => inner.session() == Some(*session),
            Self::Run(run) => inner.run() == Some(*run),
            Self::Execution(execution) => inner.execution() == Some(*execution),
        }
    }
}

/// B03's event envelope: a payload with the scope it belongs to and the
/// request it answers, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event<T> {
    scope: Scope,
    request: Option<RequestId>,
    payload: T,
}

impl<T> Event<T> {
    /// An event for `scope`, optionally answering `request`.
    pub const fn new(scope: Scope, request: Option<RequestId>, payload: T) -> Self {
        Self {
            scope,
            request,
            payload,
        }
    }

    /// The scope the event belongs to.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// The request this event answers, if any.
    pub const fn request(&self) -> Option<RequestId> {
        self.request
    }

    /// The payload.
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    /// Take the payload.
    pub fn into_payload(self) -> T {
        self.payload
    }
}

/// The job correlation envelope of §26 §2.2: the job id, the process
/// incarnation it was accepted in, its originating scope and the request
/// that submitted it. The registered work family and input revision are
/// per-family values (F0 item 4).
///
/// The job id is host-minted from the composition's one incarnation-wide
/// [`RequestIdMinter`], so it is unique within the process incarnation
/// whatever run submitted it; the submitting request (an isolate-minted id
/// unique per run, or a host-minted one) is kept only as correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkIdentity {
    job: RequestId,
    incarnation: ProcessIncarnation,
    scope: Scope,
    origin: Option<RequestId>,
}

impl WorkIdentity {
    /// Compose a work identity. `job` must be host-minted
    /// ([`IdentityError::JobIdNotHostMinted`] otherwise); an isolate-minted
    /// `origin` is refused unless the scope names its run or execution
    /// ([`IdentityError::IsolateRequestWithoutRun`]).
    pub fn new(
        job: RequestId,
        incarnation: ProcessIncarnation,
        scope: Scope,
        origin: Option<RequestId>,
    ) -> Result<Self, IdentityError> {
        if job.namespace() != RequestNamespace::Host {
            return Err(IdentityError::JobIdNotHostMinted { raw: job.get() });
        }
        if origin.is_some_and(|origin| origin.namespace() == RequestNamespace::Isolate)
            && scope.run().is_none()
        {
            return Err(IdentityError::IsolateRequestWithoutRun);
        }
        Ok(Self {
            job,
            incarnation,
            scope,
            origin,
        })
    }

    /// The job id: host-minted, unique within the incarnation.
    pub const fn job(&self) -> RequestId {
        self.job
    }

    /// The process incarnation the job was accepted in.
    pub const fn incarnation(&self) -> ProcessIncarnation {
        self.incarnation
    }

    /// The originating scope.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// The request that submitted the job, if one did: correlation only,
    /// never the job's identity.
    pub const fn origin(&self) -> Option<RequestId> {
        self.origin
    }
}

/// Why a fence rejected a completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FenceError<K: fmt::Debug> {
    /// The fence holds nothing: it was never armed or was revoked.
    #[error("completion stamped {stamped:?} rejected: the fence holds nothing")]
    Unarmed {
        /// The identity the rejected completion carried.
        stamped: K,
    },
    /// The completion carries an identity the owner no longer holds.
    #[error("completion stamped {stamped:?} rejected: the owner holds {current:?}")]
    Stale {
        /// The identity the owner holds now.
        current: K,
        /// The identity the rejected completion carried.
        stamped: K,
    },
}

impl<K: Copy + fmt::Debug> FenceError<K> {
    /// The identity the rejected completion carried.
    pub const fn stamped(&self) -> K {
        match self {
            Self::Unarmed { stamped } | Self::Stale { stamped, .. } => *stamped,
        }
    }

    /// The identity the owner holds now, if it holds one.
    pub const fn current(&self) -> Option<K> {
        match self {
            Self::Unarmed { .. } => None,
            Self::Stale { current, .. } => Some(*current),
        }
    }
}

/// The owner-side generation fence (B03 "Authority, cancellation and
/// ordering" 2–5; §26 §2.5): a completion is admitted only while it carries
/// exactly the identity the owner currently holds. Worker-side checks are an
/// optimisation; this check is the safety proof.
#[derive(Debug)]
pub struct Fence<K> {
    current: Option<K>,
}

impl<K: Copy + PartialEq + fmt::Debug> Fence<K> {
    /// A fence that admits nothing until armed.
    pub const fn new() -> Self {
        Self { current: None }
    }

    /// Hold `key`: completions stamped with it are admitted; every older
    /// stamp becomes stale.
    pub fn arm(&mut self, key: K) {
        self.current = Some(key);
    }

    /// Hold nothing: every completion is stale until the next arm.
    pub fn revoke(&mut self) {
        self.current = None;
    }

    /// The identity currently held.
    pub const fn current(&self) -> Option<K> {
        self.current
    }

    /// Admit a completion stamped `stamped`, or reject it.
    pub fn admit(&self, stamped: K) -> Result<K, FenceError<K>> {
        match self.current {
            Some(current) if current == stamped => Ok(stamped),
            Some(current) => Err(FenceError::Stale { current, stamped }),
            None => Err(FenceError::Unarmed { stamped }),
        }
    }
}

impl<K: Copy + PartialEq + fmt::Debug> Default for Fence<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a terminal could not be published or consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TerminalError {
    /// The terminal was already published; it is immutable.
    #[error("terminal already published")]
    AlreadyPublished,
    /// Nothing has been published yet.
    #[error("terminal not published")]
    NotPublished,
    /// The terminal was already consumed; custody transferred once.
    #[error("terminal already consumed")]
    AlreadyConsumed,
}

enum TerminalState<T> {
    Pending,
    Published(T),
    Consumed,
}

/// The one terminal of an accepted request (B03 §5; §26 §2.2): published
/// exactly once and immutable afterwards; borrowed by polling; consumed
/// exactly once, which transfers the result and acknowledges custody.
pub struct Terminal<T> {
    state: TerminalState<T>,
}

impl<T> Terminal<T> {
    /// A pending terminal.
    pub const fn new() -> Self {
        Self {
            state: TerminalState::Pending,
        }
    }

    /// Publish the terminal; a second publication is refused and changes
    /// nothing.
    pub fn publish(&mut self, value: T) -> Result<(), TerminalError> {
        match self.state {
            TerminalState::Pending => {
                self.state = TerminalState::Published(value);
                Ok(())
            }
            TerminalState::Published(_) | TerminalState::Consumed => {
                Err(TerminalError::AlreadyPublished)
            }
        }
    }

    /// Borrow the retained terminal without consuming it.
    pub const fn borrow(&self) -> Option<&T> {
        match &self.state {
            TerminalState::Published(value) => Some(value),
            TerminalState::Pending | TerminalState::Consumed => None,
        }
    }

    /// Consume the terminal, transferring its result once.
    pub fn consume(&mut self) -> Result<T, TerminalError> {
        match std::mem::replace(&mut self.state, TerminalState::Consumed) {
            TerminalState::Published(value) => Ok(value),
            TerminalState::Pending => {
                self.state = TerminalState::Pending;
                Err(TerminalError::NotPublished)
            }
            TerminalState::Consumed => Err(TerminalError::AlreadyConsumed),
        }
    }

    /// Whether nothing has been published yet.
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, TerminalState::Pending)
    }

    /// Whether the terminal has been consumed.
    pub const fn is_consumed(&self) -> bool {
        matches!(self.state, TerminalState::Consumed)
    }
}

impl<T> Default for Terminal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for Terminal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.state {
            TerminalState::Pending => f.write_str("Terminal::Pending"),
            TerminalState::Published(value) => {
                f.debug_tuple("Terminal::Published").field(value).finish()
            }
            TerminalState::Consumed => f.write_str("Terminal::Consumed"),
        }
    }
}

/// Why an accepted request was cancelled rather than completed or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cancellation {
    /// The operator (or its owner on the operator's behalf) withdrew it.
    Operator,
    /// A newer replaceable request of the same identity took its place.
    Superseded,
    /// Its scope ended: the session, run or execution it was issued under is
    /// no longer current.
    ScopeInvalidated,
    /// The slot or process is shutting down.
    Shutdown,
}

/// The neutral terminal of an accepted request (B03; §26 §2.2): exactly one
/// of completed with the family result, failed with the family error, or
/// cancelled with a neutral reason. Domain outcomes such as "no path" are
/// family results, not failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement<T, E> {
    /// The request completed; `T` is the family result.
    Completed(T),
    /// The request failed; `E` is the family error.
    Failed(E),
    /// The request was cancelled before it completed.
    Cancelled(Cancellation),
}

impl<T, E> Settlement<T, E> {
    /// Whether this is a completion.
    pub const fn is_completed(&self) -> bool {
        matches!(self, Self::Completed(_))
    }

    /// The cancellation reason, if cancelled.
    pub const fn cancellation(&self) -> Option<Cancellation> {
        match self {
            Self::Cancelled(reason) => Some(*reason),
            Self::Completed(_) | Self::Failed(_) => None,
        }
    }
}

/// The sequence number of a publication: `0` before the first publish, then
/// one more per publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct PublishSequence(u64);

impl PublishSequence {
    /// The raw sequence.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// What a [`Notifier`] receives: the scope that published and the sequence
/// it published. No payload and no wake primitive: the notified party
/// re-reads the publication, so notifications may coalesce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    scope: Scope,
    sequence: PublishSequence,
}

impl Notice {
    /// The scope that published.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// The sequence that is now readable.
    pub const fn sequence(&self) -> PublishSequence {
        self.sequence
    }
}

/// The neutral wake-up a publication issues after its state is readable.
/// T1 maps it onto its park; other owners map it onto their own wait.
pub trait Notifier: Send + Sync {
    /// Called once per publish, after the published state is readable.
    fn notify(&self, notice: Notice);
}

struct Published<T> {
    sequence: PublishSequence,
    value: T,
}

/// A replaceable state cell with publish-before-notify semantics (§26 §2.5):
/// [`publish`](Self::publish) stores the new value and sequence, releases the
/// lock, and only then calls the notifier, so a reader woken by the notice
/// always observes that sequence or a later one.
pub struct Publication<T> {
    scope: Scope,
    inner: Mutex<Published<T>>,
}

impl<T> Publication<T> {
    /// A publication for `scope` holding `initial` at sequence 0.
    pub fn new(scope: Scope, initial: T) -> Self {
        Self {
            scope,
            inner: Mutex::new(Published {
                sequence: PublishSequence(0),
                value: initial,
            }),
        }
    }

    /// The scope this publication belongs to.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Publish `value`, then notify. Returns the new sequence. Once the
    /// sequence space is exhausted the publication is refused: the retained
    /// value and sequence stay as they were, `value` is dropped and no
    /// notice is issued, so observers never see a sequence go backwards.
    pub fn publish(
        &self,
        value: T,
        notifier: &dyn Notifier,
    ) -> Result<PublishSequence, IdentityError> {
        let sequence = {
            let mut published = self.inner.lock();
            let next = PublishSequence(checked_next(
                IdentityKind::PublishSequence,
                published.sequence.0,
            )?);
            published.sequence = next;
            published.value = value;
            next
        };
        notifier.notify(Notice {
            scope: self.scope,
            sequence,
        });
        Ok(sequence)
    }

    /// Read the current value and its sequence.
    pub fn read<R>(&self, f: impl FnOnce(PublishSequence, &T) -> R) -> R {
        let published = self.inner.lock();
        f(published.sequence, &published.value)
    }

    /// The current sequence.
    pub fn sequence(&self) -> PublishSequence {
        self.inner.lock().sequence
    }
}

impl<T: fmt::Debug> fmt::Debug for Publication<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let published = self.inner.lock();
        f.debug_struct("Publication")
            .field("scope", &self.scope)
            .field("sequence", &published.sequence)
            .field("value", &published.value)
            .finish()
    }
}

/// An absolute monotonic deadline. Never a relative timeout: every wait
/// re-checks the same instant after every wake (§20 L5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Deadline(Instant);

impl Deadline {
    /// A deadline at `instant`.
    pub const fn at(instant: Instant) -> Self {
        Self(instant)
    }

    /// A deadline `after` from `now`.
    pub fn after(now: Instant, after: Duration) -> Self {
        Self(now + after)
    }

    /// The instant.
    pub const fn instant(self) -> Instant {
        self.0
    }

    /// Whether `now` has reached the deadline.
    pub fn expired(self, now: Instant) -> bool {
        now >= self.0
    }

    /// Time left at `now`, zero once expired.
    pub fn remaining(self, now: Instant) -> Duration {
        self.0.saturating_duration_since(now)
    }

    /// The earlier of two deadlines.
    pub fn earlier(self, other: Self) -> Self {
        if other.0 < self.0 {
            other
        } else {
            self
        }
    }
}

/// Active-work time: time a run spent eligible to act. It does not advance
/// while the run is paused, held, not ready or in recovery, so family
/// deadlines measured in it are never spent by a hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ActiveTime(Duration);

impl ActiveTime {
    /// No active time yet.
    pub const ZERO: Self = Self(Duration::ZERO);

    /// The accumulated duration.
    pub const fn as_duration(self) -> Duration {
        self.0
    }

    /// This time plus `after`, saturating.
    pub fn saturating_add(self, after: Duration) -> Self {
        Self(self.0.saturating_add(after))
    }
}

/// The clock that produces [`ActiveTime`]: it accumulates monotonic time only
/// across spans its owner marked eligible.
#[derive(Debug, Default)]
pub struct ActiveClock {
    elapsed: Duration,
    eligible_since: Option<Instant>,
}

impl ActiveClock {
    /// A clock with no active time and no open span.
    pub const fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
            eligible_since: None,
        }
    }

    /// Record whether the run is eligible to act at `now`. An open eligible
    /// span closes and is banked when eligibility ends; a new span opens when
    /// it begins. Repeating the same state changes nothing.
    pub fn observe(&mut self, now: Instant, eligible: bool) {
        match (self.eligible_since, eligible) {
            (Some(since), false) => {
                self.elapsed += now.saturating_duration_since(since);
                self.eligible_since = None;
            }
            (None, true) => self.eligible_since = Some(now),
            _ => {}
        }
    }

    /// Active time at `now`: the banked spans plus the open one, if any.
    pub fn now(&self, now: Instant) -> ActiveTime {
        ActiveTime(
            self.elapsed
                + self
                    .eligible_since
                    .map_or(Duration::ZERO, |since| now.saturating_duration_since(since)),
        )
    }

    /// Whether an eligible span is open.
    pub const fn is_eligible(&self) -> bool {
        self.eligible_since.is_some()
    }
}

/// Monotonic time elapsed since this process incarnation's origin (§26 §3.1
/// `detected_at`). Not active-work time and not a wall-clock timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ProcessTime(Duration);

impl ProcessTime {
    /// The origin itself.
    pub const ZERO: Self = Self(Duration::ZERO);

    /// The elapsed duration.
    pub const fn as_duration(self) -> Duration {
        self.0
    }
}

/// The monotonic origin of a process incarnation, from which
/// [`ProcessTime`] is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessOrigin(Instant);

impl ProcessOrigin {
    /// An origin at `instant`.
    pub const fn at(instant: Instant) -> Self {
        Self(instant)
    }

    /// An origin at the current instant.
    pub fn now() -> Self {
        Self(Instant::now())
    }

    /// Process time at `now`, zero for instants before the origin.
    pub fn elapsed(self, now: Instant) -> ProcessTime {
        ProcessTime(now.saturating_duration_since(self.0))
    }
}

/// Durable chronology: whole seconds since the Unix epoch, the only clock a
/// durable record may carry (today `slot_bank_memory::unix_now`). Never used
/// for deadlines or active time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurableTimestamp(u64);

impl DurableTimestamp {
    /// A timestamp from whole seconds since the Unix epoch.
    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    /// A timestamp for `time`, or `None` before the epoch.
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        time.duration_since(UNIX_EPOCH)
            .ok()
            .map(|since| Self(since.as_secs()))
    }

    /// The current wall-clock timestamp; the epoch itself if the clock is
    /// set before it.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now()).unwrap_or(Self(0))
    }

    /// Whole seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;

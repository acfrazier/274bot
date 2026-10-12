//! T1-owned neutral render reservation protocol (D12 / §23 §2).
//!
//! This service owns bounded reservation metadata and per-head retained-scene
//! handles only. It creates no render thread and never borrows a `Client` or
//! world. Composition and the fork-local backend map the typed identities to
//! opaque scalar values at the client edge. The independent monotonic pump
//! must remain live while heads are attached; it calls [`RenderService::pump`]
//! at the returned deadline even if a slot is not requesting frames.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use api::identity::{
    Deadline, Fence, IdentityError, IdentityKind, Notice, Notifier, Publication, PublishSequence,
    Scope, SlotKey,
};
use thiserror::Error;

/// Maximum number of simultaneously attached render heads.
///
/// This is a finite admission ceiling, not a claim that every value up to the
/// cap has been M3-qualified for GPU resources. T4.4 publishes the supported
/// headed count and its measured limits before production activation.
pub const RENDER_HEAD_CAP: usize = 8;

const NOTICE_BATCH_CAP: usize = RENDER_HEAD_CAP + 4;

/// A render-head generation, distinct from the slot worker's [`SlotKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HeadGeneration(NonZeroU64);

impl HeadGeneration {
    /// The opaque scalar passed to a fork-local render backend.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// A head identity: one slot's current attachment to the render service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeadIdentity {
    slot: SlotKey,
    generation: HeadGeneration,
}

impl HeadIdentity {
    /// The slot worker lifetime that owns this head.
    pub const fn slot(self) -> SlotKey {
        self.slot
    }

    /// The generation that changes after detach/reattach or retirement.
    pub const fn generation(self) -> HeadGeneration {
        self.generation
    }
}

/// A process-unique render reservation number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReservationId(NonZeroU64);

impl ReservationId {
    /// The opaque scalar passed to a fork-local render backend.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// The complete identity checked when a grant is consumed or released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationIdentity {
    head: HeadIdentity,
    reservation_id: ReservationId,
}

impl ReservationIdentity {
    /// The slot and head generation reserved by this request.
    pub const fn head(self) -> HeadIdentity {
        self.head
    }

    /// The monotonically unique request number.
    pub const fn reservation_id(self) -> ReservationId {
        self.reservation_id
    }
}

/// A backend-owned, immutable retained 3D scene handle.
///
/// The neutral service stores this opaque scalar per head; the backend owns
/// the resource it names. Zero is a valid backend handle and is not a sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneHandle(u64);

impl SceneHandle {
    /// Wrap an opaque backend handle without interpreting it.
    pub const fn from_opaque(raw: u64) -> Self {
        Self(raw)
    }

    /// The unchanged scalar supplied by the backend.
    pub const fn opaque(self) -> u64 {
        self.0
    }
}

/// The protocol state visible in a head's published cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationState {
    /// Attached, with no due request.
    Idle,
    /// One metadata-only request waits for the shared workspace.
    Pending,
    /// The workspace is reserved until the published expiry deadline.
    Granted,
    /// The head owns the non-Clone frame lease.
    Consumed,
    /// A grant expired without being consumed; new demand is required.
    Expired,
    /// Cancellation, detach or retirement invalidated the request.
    Cancelled,
}

/// A bounded snapshot of one head's authoritative reservation and retained scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadSnapshot {
    identity: HeadIdentity,
    state: ReservationState,
    reservation: Option<ReservationIdentity>,
    due_at: Option<Deadline>,
    granted_at: Option<Instant>,
    expires_at: Option<Deadline>,
    retained_scene: Option<SceneHandle>,
}

impl HeadSnapshot {
    fn idle(identity: HeadIdentity) -> Self {
        Self {
            identity,
            state: ReservationState::Idle,
            reservation: None,
            due_at: None,
            granted_at: None,
            expires_at: None,
            retained_scene: None,
        }
    }

    /// The slot and head generation represented by this cell.
    pub const fn identity(self) -> HeadIdentity {
        self.identity
    }

    /// The current reservation state.
    pub const fn state(self) -> ReservationState {
        self.state
    }

    /// The current or most recently expired/cancelled request, if any.
    pub const fn reservation(self) -> Option<ReservationIdentity> {
        self.reservation
    }

    /// The original due time for the current or last request, if any.
    pub const fn due_at(self) -> Option<Deadline> {
        self.due_at
    }

    /// The grant instant, if a grant was issued.
    pub const fn granted_at(self) -> Option<Instant> {
        self.granted_at
    }

    /// The absolute grant expiry, if a grant was issued.
    pub const fn expires_at(self) -> Option<Deadline> {
        self.expires_at
    }

    /// The per-head retained scene. `None` is the explicit PendingScene/empty scene.
    pub const fn retained_scene(self) -> Option<SceneHandle> {
        self.retained_scene
    }
}

/// A typed reason a head or pump was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RenderRefusal {
    /// The bounded head table is full.
    #[error("render head cap of {cap} is full")]
    HeadCap {
        /// The finite configured head limit.
        cap: usize,
    },
    /// A slot already has an attached head.
    #[error("slot already has an attached render head")]
    SlotAlreadyAttached,
    /// The independent monotonic pump is not registered.
    #[error("no live render pump is registered")]
    NoLivePump,
    /// This service already has its one required independent pump.
    #[error("a render pump is already registered")]
    PumpAlreadyRegistered,
}

/// A typed render protocol error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RenderError {
    /// Admission refused for a named, closed reason.
    #[error("render admission refused: {0}")]
    Refused(#[from] RenderRefusal),
    /// The lease belongs to a different `RenderService` instance.
    #[error("render lease belongs to a different service")]
    WrongService,
    /// The head generation is no longer admitted by this service.
    #[error("render head lease is stale or retired")]
    StaleHead,
    /// The reservation identity does not match the head's current request.
    #[error("render reservation identity is stale")]
    StaleReservation,
    /// A grant reached its exact expiry before consumption.
    #[error("render grant expired before consumption")]
    Expired,
    /// The request was explicitly cancelled or its head was retired.
    #[error("render reservation was cancelled")]
    Cancelled,
    /// Another caller already consumed this grant.
    #[error("render grant was already consumed")]
    AlreadyConsumed,
    /// The request is still pending and has no workspace grant.
    #[error("render reservation has not been granted")]
    NotGranted,
    /// The same head already owns an active `FrameLease`.
    #[error("render head already owns a frame lease")]
    FrameInProgress,
    /// A zero-length grant lifetime cannot provide an observable grant.
    #[error("render grant lifetime must be positive")]
    ZeroGrantTtl,
    /// The grant expiry cannot be represented by the monotonic clock.
    #[error("render grant expiry exceeds the monotonic clock range")]
    DeadlineOverflow,
    /// The service refused to reuse an exhausted head-generation value.
    #[error("render head-generation space exhausted")]
    HeadGenerationExhausted,
    /// The service refused to reuse an exhausted reservation identity.
    #[error("render reservation-id space exhausted")]
    ReservationIdExhausted,
    /// The service refused to reuse an exhausted queue insertion sequence.
    #[error("render insertion-sequence space exhausted")]
    InsertionSequenceExhausted,
    /// The publication's identity sequence could not advance without reuse.
    #[error("render publication failed: {0}")]
    Publication(#[from] IdentityError),
}

/// The outcome of posting demand for a scene workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// The returned identity is now granted and may be passed to `try_consume`.
    Granted(ReservationIdentity),
    /// The request is published as pending; no scene lease was issued.
    Busy(ReservationIdentity),
}

/// The process-wide bounded render reservation service.
///
/// `grant_ttl` must be positive. F0 tests use a 20-ms fixture; this constructor
/// has no implicit production default because M3 owns that qualification.
pub struct RenderService {
    inner: Arc<ServiceInner>,
}

impl RenderService {
    /// Create a service with the caller-qualified positive grant lifetime.
    pub fn new(grant_ttl: Duration) -> Result<Self, RenderError> {
        if grant_ttl.is_zero() {
            return Err(RenderError::ZeroGrantTtl);
        }
        Ok(Self {
            inner: Arc::new(ServiceInner {
                grant_ttl,
                state: Mutex::new(ServiceState::new()),
            }),
        })
    }

    /// Register the one independent process/window pump required before attach.
    pub fn register_pump(&self) -> Result<RenderPumpLease, RenderError> {
        let mut state = lock_unpoisoned(&self.inner.state);
        if state.pump_registered {
            return Err(RenderRefusal::PumpAlreadyRegistered.into());
        }
        state.pump_registered = true;
        Ok(RenderPumpLease {
            service: Arc::clone(&self.inner),
            active: true,
        })
    }

    /// Attach one headed slot, refusing when the finite table is full.
    ///
    /// Attach is available only while the independent monotonic pump is live.
    pub fn attach(
        &self,
        pump: &RenderPumpLease,
        slot: SlotKey,
        notifier: Arc<dyn Notifier>,
    ) -> Result<HeadLease, RenderError> {
        if !Arc::ptr_eq(&self.inner, &pump.service) {
            return Err(RenderError::WrongService);
        }
        if !pump.active {
            return Err(RenderRefusal::NoLivePump.into());
        }
        let mut state = lock_unpoisoned(&self.inner.state);
        if !state.pump_registered {
            return Err(RenderRefusal::NoLivePump.into());
        }
        if state
            .heads
            .iter()
            .flatten()
            .any(|entry| entry.identity.slot == slot)
        {
            return Err(RenderRefusal::SlotAlreadyAttached.into());
        }
        let Some(index) = state.heads.iter().position(Option::is_none) else {
            return Err(RenderRefusal::HeadCap {
                cap: RENDER_HEAD_CAP,
            }
            .into());
        };
        let generation = HeadGeneration(take_next(
            &mut state.next_head_generation,
            RenderError::HeadGenerationExhausted,
        )?);
        let identity = HeadIdentity { slot, generation };
        let cell = Arc::new(HeadCell {
            publication: Publication::new(Scope::Slot(slot), HeadSnapshot::idle(identity)),
        });
        let mut fence = Fence::new();
        fence.arm(identity);
        state.heads[index] = Some(HeadEntry {
            identity,
            fence,
            cell: Arc::clone(&cell),
            notifier,
            request: RequestState::Idle,
            focused: false,
            retained_scene: None,
        });
        Ok(HeadLease {
            service: Arc::clone(&self.inner),
            cell,
            identity,
            active: true,
        })
    }

    /// Publish/coalesce due demand, arbitrate it and return Busy rather than
    /// waiting for the workspace.
    pub fn request_frame(
        &self,
        head: &HeadLease,
        due_at: Deadline,
        now: Instant,
    ) -> Result<RequestOutcome, RenderError> {
        if !Arc::ptr_eq(&self.inner, &head.service) {
            return Err(RenderError::WrongService);
        }
        if !head.active {
            return Err(RenderError::StaleHead);
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.inner.state);
            request_locked(&self.inner, &mut state, head, due_at, now, &mut notices)
        };
        notices.dispatch();
        result
    }

    /// Poll the authoritative per-head cell and process any due grant expiry.
    pub fn poll(
        &self,
        head: &HeadLease,
        now: Instant,
    ) -> Result<(PublishSequence, HeadSnapshot), RenderError> {
        if !Arc::ptr_eq(&self.inner, &head.service) {
            return Err(RenderError::WrongService);
        }
        if !head.active {
            return Err(RenderError::StaleHead);
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.inner.state);
            let check = require_head(&state, head)
                .and_then(|_| process_due(&self.inner, &mut state, now, &mut notices));
            check.map(|()| head.snapshot())
        };
        notices.dispatch();
        result
    }

    /// Atomically consume a live, identity-matching grant exactly once.
    pub fn try_consume(
        &self,
        head: &HeadLease,
        identity: ReservationIdentity,
        now: Instant,
    ) -> Result<FrameLease, RenderError> {
        if !Arc::ptr_eq(&self.inner, &head.service) {
            return Err(RenderError::WrongService);
        }
        if !head.active {
            return Err(RenderError::StaleHead);
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.inner.state);
            consume_locked(&self.inner, &mut state, head, identity, now, &mut notices)
        };
        notices.dispatch();
        result.map(|()| FrameLease {
            service: Arc::clone(&self.inner),
            cell: Arc::clone(&head.cell),
            identity,
            active: true,
        })
    }

    /// Cancel this head's current request. A consumed frame remains charged
    /// until its `FrameLease` is dropped or finished.
    pub fn cancel_request(&self, head: &HeadLease, now: Instant) -> Result<(), RenderError> {
        if !Arc::ptr_eq(&self.inner, &head.service) {
            return Err(RenderError::WrongService);
        }
        if !head.active {
            return Err(RenderError::StaleHead);
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.inner.state);
            cancel_locked(&self.inner, &mut state, head, now, &mut notices)
        };
        notices.dispatch();
        result
    }

    /// Change focus priority without changing an existing request's due time
    /// or insertion sequence.
    pub fn set_focused(
        &self,
        head: &HeadLease,
        focused: bool,
        now: Instant,
    ) -> Result<(), RenderError> {
        if !Arc::ptr_eq(&self.inner, &head.service) {
            return Err(RenderError::WrongService);
        }
        if !head.active {
            return Err(RenderError::StaleHead);
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.inner.state);
            let result = require_head_mut(&mut state, head).map(|entry| entry.focused = focused);
            result.and_then(|()| process_due(&self.inner, &mut state, now, &mut notices))
        };
        notices.dispatch();
        result
    }

    /// Retire an attached head. Pending/granted demand is cancelled and
    /// released; a consumed workspace remains charged until its frame drains.
    pub fn retire(&self, head: &mut HeadLease, now: Instant) -> Result<(), RenderError> {
        if !Arc::ptr_eq(&self.inner, &head.service) {
            return Err(RenderError::WrongService);
        }
        if !head.active {
            return Err(RenderError::StaleHead);
        }
        let result = retire_inner(&self.inner, &head.identity, &head.cell, now);
        let remains_attached = lock_unpoisoned(&self.inner.state)
            .heads
            .iter()
            .flatten()
            .any(|entry| entry.identity == head.identity && Arc::ptr_eq(&entry.cell, &head.cell));
        if !remains_attached {
            head.active = false;
        }
        result
    }

    /// Run the independent monotonic pump and return its next deadline.
    ///
    /// A consumed frame has no expiry deadline: its workspace is never
    /// reclaimed while borrowed. Releasing it immediately arbitrates pending
    /// demand, and a device stall must be surfaced by the backend.
    pub fn pump(
        &self,
        pump: &RenderPumpLease,
        now: Instant,
    ) -> Result<Option<Deadline>, RenderError> {
        if !Arc::ptr_eq(&self.inner, &pump.service) {
            return Err(RenderError::WrongService);
        }
        if !pump.active {
            return Err(RenderRefusal::NoLivePump.into());
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.inner.state);
            if state.pump_registered {
                process_due(&self.inner, &mut state, now, &mut notices)
                    .map(|()| next_deadline(&state))
            } else {
                Err(RenderRefusal::NoLivePump.into())
            }
        };
        notices.dispatch();
        result
    }

    /// Unregister the independent pump and retire every attached head.
    pub fn unregister_pump(&self, pump: &mut RenderPumpLease) -> Result<(), RenderError> {
        if !Arc::ptr_eq(&self.inner, &pump.service) {
            return Err(RenderError::WrongService);
        }
        let result = unregister_pump_inner(&self.inner);
        if result.is_ok() {
            pump.active = false;
        }
        result
    }
}

/// The independent process/window deadline pump's registration lease.
///
/// Dropping or unregistering it invalidates every attached head. It is
/// non-Clone so a stale composition cannot keep a pump registered.
pub struct RenderPumpLease {
    service: Arc<ServiceInner>,
    active: bool,
}
impl fmt::Debug for RenderPumpLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderPumpLease")
            .field("active", &self.active)
            .finish()
    }
}

impl Drop for RenderPumpLease {
    fn drop(&mut self) {
        if self.active {
            let _ = unregister_pump_inner(&self.service);
            self.active = false;
        }
    }
}

/// A non-Clone attached head lease carrying `(SlotKey, head_generation)`.
///
/// It owns no `Client`, world borrow, frame, or render closure. Dropping an
/// active lease retires its demand as a fail-safe detach.
pub struct HeadLease {
    service: Arc<ServiceInner>,
    cell: Arc<HeadCell>,
    identity: HeadIdentity,
    active: bool,
}
impl fmt::Debug for HeadLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeadLease")
            .field("identity", &self.identity)
            .field("active", &self.active)
            .finish()
    }
}

impl HeadLease {
    /// The complete slot/head generation identity.
    pub const fn identity(&self) -> HeadIdentity {
        self.identity
    }

    /// Read the latest published snapshot without advancing the deadline pump.
    pub fn snapshot(&self) -> (PublishSequence, HeadSnapshot) {
        self.cell
            .publication
            .read(|sequence, snapshot| (sequence, *snapshot))
    }
}

impl Drop for HeadLease {
    fn drop(&mut self) {
        if self.active {
            let _ = retire_inner(&self.service, &self.identity, &self.cell, Instant::now());
            self.active = false;
        }
    }
}

/// The non-Clone exclusive ownership token for the one shared scene workspace.
///
/// The caller borrows its live `Client` only for the synchronous work guarded by
/// this token. Drop and [`finish`](Self::finish) release only this reservation.
pub struct FrameLease {
    service: Arc<ServiceInner>,
    cell: Arc<HeadCell>,
    identity: ReservationIdentity,
    active: bool,
}
impl fmt::Debug for FrameLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameLease")
            .field("identity", &self.identity)
            .field("active", &self.active)
            .finish()
    }
}

impl FrameLease {
    /// The full identity this workspace token owns.
    pub const fn identity(&self) -> ReservationIdentity {
        self.identity
    }
    /// Publish a successfully completed scene only while this head generation
    /// and consumed reservation are still current. Publication never wraps.
    pub fn publish_retained_scene(&self, scene: SceneHandle) -> Result<(), RenderError> {
        if !self.active {
            return Err(RenderError::Cancelled);
        }
        let mut notices = NoticeBatch::new();
        let result = {
            let mut state = lock_unpoisoned(&self.service.state);
            publish_scene_locked(&mut state, self.identity, &self.cell, scene, &mut notices)
        };
        notices.dispatch();
        result
    }

    /// Explicitly release at `now`; useful to make fake-clock protocol tests
    /// deterministic. Normal unwinding/drop follows the identical path with
    /// the process monotonic clock.
    ///
    /// # Errors
    ///
    /// Returns a typed error if publication cannot advance without wrapping or
    /// a pending grant's expiry cannot be represented by the monotonic clock.
    pub fn finish(mut self, now: Instant) -> Result<(), RenderError> {
        let result = release_frame(&self.service, self.identity, &self.cell, now);
        self.active = false;
        result
    }
}

impl Drop for FrameLease {
    fn drop(&mut self) {
        if self.active {
            let _ = release_frame(&self.service, self.identity, &self.cell, Instant::now());
            self.active = false;
        }
    }
}

struct ServiceInner {
    grant_ttl: Duration,
    state: Mutex<ServiceState>,
}

struct ServiceState {
    heads: [Option<HeadEntry>; RENDER_HEAD_CAP],
    workspace: Option<WorkspaceOwner>,
    pump_registered: bool,
    next_head_generation: Option<NonZeroU64>,
    next_reservation_id: Option<NonZeroU64>,
    next_insertion_sequence: Option<NonZeroU64>,
}

impl ServiceState {
    fn new() -> Self {
        Self {
            heads: std::array::from_fn(|_| None),
            workspace: None,
            pump_registered: false,
            next_head_generation: NonZeroU64::new(1),
            next_reservation_id: NonZeroU64::new(1),
            next_insertion_sequence: NonZeroU64::new(1),
        }
    }
}

struct HeadCell {
    publication: Publication<HeadSnapshot>,
}

struct HeadEntry {
    identity: HeadIdentity,
    fence: Fence<HeadIdentity>,
    cell: Arc<HeadCell>,
    notifier: Arc<dyn Notifier>,
    request: RequestState,
    focused: bool,
    retained_scene: Option<SceneHandle>,
}

#[derive(Debug, Clone, Copy)]
enum RequestState {
    Idle,
    Pending {
        identity: ReservationIdentity,
        due_at: Deadline,
        insertion_sequence: NonZeroU64,
    },
    Granted {
        identity: ReservationIdentity,
        due_at: Deadline,
        granted_at: Instant,
        expires_at: Deadline,
    },
    Consumed {
        identity: ReservationIdentity,
        due_at: Deadline,
        granted_at: Instant,
        expires_at: Deadline,
    },
    Expired {
        identity: ReservationIdentity,
        due_at: Deadline,
        granted_at: Instant,
        expires_at: Deadline,
    },
    Cancelled(Option<ReservationIdentity>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceOwner {
    Granted(ReservationIdentity),
    Consumed(ReservationIdentity),
}

impl HeadEntry {
    fn snapshot(&self) -> HeadSnapshot {
        match self.request {
            RequestState::Idle => HeadSnapshot {
                identity: self.identity,
                state: ReservationState::Idle,
                reservation: None,
                due_at: None,
                granted_at: None,
                expires_at: None,
                retained_scene: self.retained_scene,
            },
            RequestState::Pending {
                identity, due_at, ..
            } => HeadSnapshot {
                identity: self.identity,
                state: ReservationState::Pending,
                reservation: Some(identity),
                due_at: Some(due_at),
                granted_at: None,
                expires_at: None,
                retained_scene: self.retained_scene,
            },
            RequestState::Granted {
                identity,
                due_at,
                granted_at,
                expires_at,
                ..
            } => HeadSnapshot {
                identity: self.identity,
                state: ReservationState::Granted,
                reservation: Some(identity),
                due_at: Some(due_at),
                granted_at: Some(granted_at),
                expires_at: Some(expires_at),
                retained_scene: self.retained_scene,
            },
            RequestState::Consumed {
                identity,
                due_at,
                granted_at,
                expires_at,
            } => HeadSnapshot {
                identity: self.identity,
                state: ReservationState::Consumed,
                reservation: Some(identity),
                due_at: Some(due_at),
                granted_at: Some(granted_at),
                expires_at: Some(expires_at),
                retained_scene: self.retained_scene,
            },
            RequestState::Expired {
                identity,
                due_at,
                granted_at,
                expires_at,
            } => HeadSnapshot {
                identity: self.identity,
                state: ReservationState::Expired,
                reservation: Some(identity),
                due_at: Some(due_at),
                granted_at: Some(granted_at),
                expires_at: Some(expires_at),
                retained_scene: self.retained_scene,
            },
            RequestState::Cancelled(identity) => HeadSnapshot {
                identity: self.identity,
                state: ReservationState::Cancelled,
                reservation: identity,
                due_at: None,
                granted_at: None,
                expires_at: None,
                retained_scene: self.retained_scene,
            },
        }
    }
}

struct NoticeBatch {
    entries: [Option<PendingNotice>; NOTICE_BATCH_CAP],
    len: usize,
}

impl NoticeBatch {
    fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            len: 0,
        }
    }

    fn push(&mut self, notifier: Arc<dyn Notifier>, notice: Notice) {
        assert!(
            self.len < NOTICE_BATCH_CAP,
            "render transition notice bound"
        );
        self.entries[self.len] = Some(PendingNotice { notifier, notice });
        self.len += 1;
    }

    fn dispatch(&mut self) {
        for entry in self.entries[..self.len].iter_mut() {
            if let Some(pending) = entry.take() {
                pending.notifier.notify(pending.notice);
            }
        }
        self.len = 0;
    }
}

struct PendingNotice {
    notifier: Arc<dyn Notifier>,
    notice: Notice,
}

#[derive(Default)]
struct CaptureNotice(Mutex<Option<Notice>>);

impl Notifier for CaptureNotice {
    fn notify(&self, notice: Notice) {
        *lock_unpoisoned(&self.0) = Some(notice);
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn take_next(
    next: &mut Option<NonZeroU64>,
    exhausted: RenderError,
) -> Result<NonZeroU64, RenderError> {
    let current = next.take().ok_or(exhausted)?;
    *next = current.get().checked_add(1).and_then(NonZeroU64::new);
    Ok(current)
}

fn require_head(state: &ServiceState, head: &HeadLease) -> Result<usize, RenderError> {
    if !head.active {
        return Err(RenderError::StaleHead);
    }
    state
        .heads
        .iter()
        .position(|entry| {
            entry.as_ref().is_some_and(|entry| {
                entry.identity == head.identity && Arc::ptr_eq(&entry.cell, &head.cell)
            })
        })
        .ok_or(RenderError::StaleHead)
        .and_then(|index| {
            state.heads[index]
                .as_ref()
                .expect("found render head")
                .fence
                .admit(head.identity)
                .map(|_| index)
                .map_err(|_| RenderError::StaleHead)
        })
}

fn require_head_mut<'a>(
    state: &'a mut ServiceState,
    head: &HeadLease,
) -> Result<&'a mut HeadEntry, RenderError> {
    let index = require_head(state, head)?;
    state.heads[index].as_mut().ok_or(RenderError::StaleHead)
}

fn ensure_publication_capacity(entry: &HeadEntry, publications: u64) -> Result<(), RenderError> {
    let available = u64::MAX - entry.cell.publication.sequence().get();
    if available < publications {
        Err(IdentityError::Exhausted(IdentityKind::PublishSequence).into())
    } else {
        Ok(())
    }
}

fn publish_entry(entry: &HeadEntry, notices: &mut NoticeBatch) -> Result<(), RenderError> {
    ensure_publication_capacity(entry, 1)?;
    let capture = CaptureNotice::default();
    entry.cell.publication.publish(entry.snapshot(), &capture)?;
    let notice = lock_unpoisoned(&capture.0)
        .take()
        .expect("Publication calls its notifier exactly once");
    notices.push(Arc::clone(&entry.notifier), notice);
    Ok(())
}

fn request_locked(
    service: &ServiceInner,
    state: &mut ServiceState,
    head: &HeadLease,
    due_at: Deadline,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<RequestOutcome, RenderError> {
    expire_grant(state, now, notices)?;
    let index = require_head(state, head)?;
    let new_request = matches!(
        state.heads[index].as_ref().expect("head checked").request,
        RequestState::Idle | RequestState::Expired { .. } | RequestState::Cancelled(_)
    );
    if new_request {
        ensure_publication_capacity(state.heads[index].as_ref().expect("head checked"), 1)?;
        let reservation_id = ReservationId(take_next(
            &mut state.next_reservation_id,
            RenderError::ReservationIdExhausted,
        )?);
        let insertion_sequence = take_next(
            &mut state.next_insertion_sequence,
            RenderError::InsertionSequenceExhausted,
        )?;
        let identity = ReservationIdentity {
            head: head.identity,
            reservation_id,
        };
        let entry = state.heads[index].as_mut().expect("head checked");
        entry.request = RequestState::Pending {
            identity,
            due_at,
            insertion_sequence,
        };
        publish_entry(entry, notices)?;
    }
    grant_pending(service, state, now, notices)?;
    let entry = state.heads[index].as_ref().ok_or(RenderError::StaleHead)?;
    match entry.request {
        RequestState::Pending { identity, .. } => Ok(RequestOutcome::Busy(identity)),
        RequestState::Granted { identity, .. } => Ok(RequestOutcome::Granted(identity)),
        RequestState::Consumed { .. } => Err(RenderError::FrameInProgress),
        RequestState::Idle => Err(RenderError::NotGranted),
        RequestState::Expired { .. } | RequestState::Cancelled(_) => Err(RenderError::NotGranted),
    }
}

fn process_due(
    service: &ServiceInner,
    state: &mut ServiceState,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    expire_grant(state, now, notices)?;
    grant_pending(service, state, now, notices)
}

fn expire_grant(
    state: &mut ServiceState,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    let Some(WorkspaceOwner::Granted(identity)) = state.workspace else {
        return Ok(());
    };
    let Some(index) = find_entry(state, identity.head) else {
        state.workspace = None;
        return Ok(());
    };
    let expired = state.heads[index]
        .as_ref()
        .and_then(|entry| match entry.request {
            RequestState::Granted {
                identity: current,
                granted_at,
                expires_at,
                ..
            } if current == identity && expires_at.expired(now) => Some((granted_at, expires_at)),
            _ => None,
        });
    if let Some((granted_at, expires_at)) = expired {
        let entry = state.heads[index].as_mut().expect("head found");
        ensure_publication_capacity(entry, 1)?;
        let due_at = match entry.request {
            RequestState::Granted { due_at, .. } => due_at,
            _ => unreachable!("grant checked while holding ordering lock"),
        };
        state.workspace = None;
        entry.request = RequestState::Expired {
            identity,
            due_at,
            granted_at,
            expires_at,
        };
        publish_entry(entry, notices)?;
    }
    Ok(())
}

fn grant_pending(
    service: &ServiceInner,
    state: &mut ServiceState,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    if state.workspace.is_some() {
        return Ok(());
    }
    let Some(index) = select_pending(state, now) else {
        return Ok(());
    };
    let (identity, due_at) = match state.heads[index]
        .as_ref()
        .expect("selected pending head")
        .request
    {
        RequestState::Pending {
            identity, due_at, ..
        } => (identity, due_at),
        _ => unreachable!("selected pending request while holding ordering lock"),
    };
    let expires_at = Deadline::at(
        now.checked_add(service.grant_ttl)
            .ok_or(RenderError::DeadlineOverflow)?,
    );
    let entry = state.heads[index].as_mut().expect("selected head remains");
    ensure_publication_capacity(entry, 1)?;
    state.workspace = Some(WorkspaceOwner::Granted(identity));
    entry.request = RequestState::Granted {
        identity,
        due_at,
        granted_at: now,
        expires_at,
    };
    publish_entry(entry, notices)?;
    Ok(())
}

fn select_pending(state: &ServiceState, now: Instant) -> Option<usize> {
    let mut best: Option<(usize, Deadline, bool, NonZeroU64)> = None;
    for (index, entry) in state.heads.iter().enumerate() {
        let Some(entry) = entry else { continue };
        let RequestState::Pending {
            due_at,
            insertion_sequence,
            ..
        } = entry.request
        else {
            continue;
        };
        if !due_at.expired(now) {
            continue;
        }
        let candidate = (index, due_at, entry.focused, insertion_sequence);
        let replace = match best {
            None => true,
            Some((_, best_due, best_focused, best_sequence)) => {
                due_at < best_due
                    || (due_at == best_due
                        && (entry.focused && !best_focused
                            || (entry.focused == best_focused
                                && insertion_sequence < best_sequence)))
            }
        };
        if replace {
            best = Some(candidate);
        }
    }
    best.map(|(index, _, _, _)| index)
}

fn consume_locked(
    service: &ServiceInner,
    state: &mut ServiceState,
    head: &HeadLease,
    identity: ReservationIdentity,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    process_due(service, state, now, notices)?;
    let index = require_head(state, head)?;
    if identity.head != head.identity {
        return Err(RenderError::StaleReservation);
    }
    let entry = state.heads[index].as_mut().ok_or(RenderError::StaleHead)?;
    match entry.request {
        RequestState::Granted {
            identity: current,
            due_at,
            granted_at,
            expires_at,
        } if current == identity
            && state.workspace == Some(WorkspaceOwner::Granted(identity))
            && !expires_at.expired(now) =>
        {
            ensure_publication_capacity(entry, 2)?;
            entry.request = RequestState::Consumed {
                identity,
                due_at,
                granted_at,
                expires_at,
            };
            state.workspace = Some(WorkspaceOwner::Consumed(identity));
            publish_entry(entry, notices)?;
            Ok(())
        }
        RequestState::Expired {
            identity: current, ..
        } if current == identity => Err(RenderError::Expired),
        RequestState::Cancelled(Some(current)) if current == identity => {
            Err(RenderError::Cancelled)
        }
        RequestState::Consumed {
            identity: current, ..
        } if current == identity => Err(RenderError::AlreadyConsumed),
        RequestState::Pending {
            identity: current, ..
        } if current == identity => Err(RenderError::NotGranted),
        RequestState::Granted { .. } => Err(RenderError::StaleReservation),
        RequestState::Expired { .. }
        | RequestState::Cancelled(_)
        | RequestState::Idle
        | RequestState::Pending { .. }
        | RequestState::Consumed { .. } => Err(RenderError::StaleReservation),
    }
}

fn cancel_locked(
    service: &ServiceInner,
    state: &mut ServiceState,
    head: &HeadLease,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    expire_grant(state, now, notices)?;
    let index = require_head(state, head)?;
    let current = match state.heads[index].as_ref().expect("head checked").request {
        RequestState::Idle => None,
        RequestState::Pending { identity, .. }
        | RequestState::Granted { identity, .. }
        | RequestState::Consumed { identity, .. }
        | RequestState::Expired { identity, .. } => Some(identity),
        RequestState::Cancelled(identity) => identity,
    };
    if matches!(
        state.heads[index].as_ref().expect("head checked").request,
        RequestState::Cancelled(_)
    ) {
        return grant_pending(service, state, now, notices);
    }
    ensure_publication_capacity(state.heads[index].as_ref().expect("head checked"), 1)?;
    if state.workspace == current.map(WorkspaceOwner::Granted) {
        state.workspace = None;
    }
    let entry = state.heads[index].as_mut().expect("head checked");
    entry.request = RequestState::Cancelled(current);
    publish_entry(entry, notices)?;
    grant_pending(service, state, now, notices)
}

fn retire_inner(
    service: &Arc<ServiceInner>,
    identity: &HeadIdentity,
    cell: &Arc<HeadCell>,
    now: Instant,
) -> Result<(), RenderError> {
    let mut notices = NoticeBatch::new();
    let result = {
        let mut state = lock_unpoisoned(&service.state);
        retire_locked(service, &mut state, identity, cell, now, &mut notices)
    };
    notices.dispatch();
    result
}

fn retire_locked(
    service: &ServiceInner,
    state: &mut ServiceState,
    identity: &HeadIdentity,
    cell: &Arc<HeadCell>,
    now: Instant,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    expire_grant(state, now, notices)?;
    let index = state
        .heads
        .iter()
        .position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.identity == *identity && Arc::ptr_eq(&entry.cell, cell))
        })
        .ok_or(RenderError::StaleHead)?;
    let entry = state.heads[index].as_mut().ok_or(RenderError::StaleHead)?;
    let current = match entry.request {
        RequestState::Idle => None,
        RequestState::Pending { identity, .. }
        | RequestState::Granted { identity, .. }
        | RequestState::Consumed { identity, .. }
        | RequestState::Expired { identity, .. } => Some(identity),
        RequestState::Cancelled(identity) => identity,
    };
    ensure_publication_capacity(entry, 1)?;
    if state.workspace == current.map(WorkspaceOwner::Granted) {
        state.workspace = None;
    }
    entry.fence.revoke();
    entry.request = RequestState::Cancelled(current);
    entry.retained_scene = None;
    publish_entry(entry, notices)?;
    state.heads[index] = None;
    grant_pending(service, state, now, notices)
}

fn publish_scene_locked(
    state: &mut ServiceState,
    identity: ReservationIdentity,
    cell: &Arc<HeadCell>,
    scene: SceneHandle,
    notices: &mut NoticeBatch,
) -> Result<(), RenderError> {
    let index = state
        .heads
        .iter()
        .position(|entry| {
            entry.as_ref().is_some_and(|entry| {
                entry.identity == identity.head && Arc::ptr_eq(&entry.cell, cell)
            })
        })
        .ok_or(RenderError::Cancelled)?;
    let owns_workspace = state.workspace == Some(WorkspaceOwner::Consumed(identity));
    let entry = state.heads[index].as_mut().ok_or(RenderError::Cancelled)?;
    entry
        .fence
        .admit(identity.head)
        .map_err(|_| RenderError::StaleHead)?;
    match entry.request {
        RequestState::Consumed {
            identity: current, ..
        } if current == identity && owns_workspace => {
            ensure_publication_capacity(entry, 2)?;
            entry.retained_scene = Some(scene);
            publish_entry(entry, notices)?;
            Ok(())
        }
        RequestState::Cancelled(Some(current)) if current == identity => {
            Err(RenderError::Cancelled)
        }
        RequestState::Expired {
            identity: current, ..
        } if current == identity => Err(RenderError::Expired),
        _ => Err(RenderError::StaleReservation),
    }
}

fn release_frame(
    service: &Arc<ServiceInner>,
    identity: ReservationIdentity,
    cell: &Arc<HeadCell>,
    now: Instant,
) -> Result<(), RenderError> {
    let mut notices = NoticeBatch::new();
    let result = {
        let mut state = lock_unpoisoned(&service.state);
        if state.workspace != Some(WorkspaceOwner::Consumed(identity)) {
            Ok(())
        } else {
            state.workspace = None;
            let result = (|| {
                if let Some(index) = state.heads.iter().position(|entry| {
                    entry.as_ref().is_some_and(|entry| {
                        entry.identity == identity.head && Arc::ptr_eq(&entry.cell, cell)
                    })
                }) {
                    let entry = state.heads[index].as_mut().expect("head found");
                    if matches!(entry.request, RequestState::Consumed { identity: current, .. } if current == identity)
                    {
                        ensure_publication_capacity(entry, 1)?;
                        entry.request = RequestState::Idle;
                        publish_entry(entry, &mut notices)?;
                    }
                }
                grant_pending(service, &mut state, now, &mut notices)
            })();
            result
        }
    };
    notices.dispatch();
    result
}

fn unregister_pump_inner(service: &Arc<ServiceInner>) -> Result<(), RenderError> {
    let mut notices = NoticeBatch::new();
    let result = {
        let mut state = lock_unpoisoned(&service.state);
        (|| {
            if !state.pump_registered {
                return Err(RenderRefusal::NoLivePump.into());
            }
            for entry in state.heads.iter().flatten() {
                ensure_publication_capacity(entry, 1)?;
            }
            state.pump_registered = false;
            if matches!(state.workspace, Some(WorkspaceOwner::Granted(_))) {
                state.workspace = None;
            }
            for index in 0..RENDER_HEAD_CAP {
                if let Some(entry) = state.heads[index].as_mut() {
                    let identity = match entry.request {
                        RequestState::Idle => None,
                        RequestState::Pending { identity, .. }
                        | RequestState::Granted { identity, .. }
                        | RequestState::Consumed { identity, .. }
                        | RequestState::Expired { identity, .. } => Some(identity),
                        RequestState::Cancelled(identity) => identity,
                    };
                    entry.fence.revoke();
                    entry.request = RequestState::Cancelled(identity);
                    entry.retained_scene = None;
                    publish_entry(entry, &mut notices)?;
                    state.heads[index] = None;
                }
            }
            Ok(())
        })()
    };
    notices.dispatch();
    result
}

fn find_entry(state: &ServiceState, identity: HeadIdentity) -> Option<usize> {
    state.heads.iter().position(|entry| {
        entry
            .as_ref()
            .is_some_and(|entry| entry.identity == identity)
    })
}

fn next_deadline(state: &ServiceState) -> Option<Deadline> {
    match state.workspace {
        Some(WorkspaceOwner::Granted(identity)) => {
            let index = find_entry(state, identity.head)?;
            match state.heads[index].as_ref()?.request {
                RequestState::Granted { expires_at, .. } => Some(expires_at),
                _ => None,
            }
        }
        Some(WorkspaceOwner::Consumed(_)) => None,
        None => state
            .heads
            .iter()
            .flatten()
            .filter_map(|entry| match entry.request {
                RequestState::Pending { due_at, .. } => Some(due_at),
                _ => None,
            })
            .min(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    const TTL: Duration = Duration::from_millis(20);

    #[derive(Default)]
    struct NoopNotifier;

    impl Notifier for NoopNotifier {
        fn notify(&self, _notice: Notice) {}
    }

    #[derive(Default)]
    struct ObservingNotifier {
        cell: Mutex<Option<std::sync::Weak<HeadCell>>>,
        observed: Mutex<Vec<(Notice, PublishSequence, ReservationState)>>,
    }

    impl ObservingNotifier {
        fn bind(&self, head: &HeadLease) {
            *lock_unpoisoned(&self.cell) = Some(Arc::downgrade(&head.cell));
        }
    }

    impl Notifier for ObservingNotifier {
        fn notify(&self, notice: Notice) {
            let cell = lock_unpoisoned(&self.cell)
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .expect("head cell bound before first publication");
            let (sequence, snapshot) = cell
                .publication
                .read(|sequence, snapshot| (sequence, *snapshot));
            assert!(sequence >= notice.sequence());
            lock_unpoisoned(&self.observed).push((notice, sequence, snapshot.state()));
        }
    }

    fn clock() -> Instant {
        Instant::now()
    }

    fn slot(raw: u64) -> SlotKey {
        SlotKey::from_incarnation(raw).expect("nonzero test slot")
    }

    fn service(now: Instant) -> (RenderService, RenderPumpLease) {
        let service = RenderService::new(TTL).expect("positive fixture TTL");
        let pump = service.register_pump().expect("first pump");
        assert_eq!(service.pump(&pump, now).unwrap(), None);
        (service, pump)
    }

    fn attach(
        service: &RenderService,
        pump: &RenderPumpLease,
        raw_slot: u64,
        notifier: Arc<dyn Notifier>,
    ) -> HeadLease {
        service.attach(pump, slot(raw_slot), notifier).unwrap()
    }

    fn request(
        service: &RenderService,
        head: &HeadLease,
        due_at: Instant,
        now: Instant,
    ) -> RequestOutcome {
        service
            .request_frame(head, Deadline::at(due_at), now)
            .unwrap()
    }

    fn granted(outcome: RequestOutcome) -> ReservationIdentity {
        match outcome {
            RequestOutcome::Granted(identity) => identity,
            RequestOutcome::Busy(_) => panic!("request should be granted"),
        }
    }

    fn pending(outcome: RequestOutcome) -> ReservationIdentity {
        match outcome {
            RequestOutcome::Busy(identity) => identity,
            RequestOutcome::Granted(_) => panic!("request should be pending"),
        }
    }

    #[test]
    fn grant_is_pending_then_granted_then_consumed_and_finished_once() {
        let now = clock();
        let (service, pump) = service(now);
        let observer = Arc::new(ObservingNotifier::default());
        let head = attach(&service, &pump, 1, observer.clone());
        observer.bind(&head);

        let grant = granted(request(&service, &head, now, now));
        {
            let observed = lock_unpoisoned(&observer.observed);
            assert_eq!(observed.len(), 2);
            assert_eq!(observed[0].0.sequence().get(), 1);
            assert_eq!(observed[1].0.sequence().get(), 2);
        }
        let (sequence, snapshot) = service.poll(&head, now).unwrap();
        assert!(
            sequence.get() >= 2,
            "Pending and Granted are both published"
        );
        assert_eq!(snapshot.state(), ReservationState::Granted);
        assert_eq!(snapshot.reservation(), Some(grant));
        assert_eq!(snapshot.granted_at(), Some(now));
        assert_eq!(snapshot.expires_at(), Some(Deadline::at(now + TTL)));

        let frame = service.try_consume(&head, grant, now + TTL / 2).unwrap();
        assert_eq!(head.snapshot().1.state(), ReservationState::Consumed);
        frame.finish(now + TTL / 2).unwrap();
        assert_eq!(head.snapshot().1.state(), ReservationState::Idle);
        assert_eq!(
            service
                .try_consume(&head, grant, now + TTL / 2)
                .unwrap_err(),
            RenderError::StaleReservation
        );
    }

    #[test]
    fn busy_publishes_pending_before_return_and_repeated_demand_coalesces() {
        let now = clock();
        let (service, pump) = service(now);
        let owner = attach(&service, &pump, 2, Arc::new(NoopNotifier));
        let owner_grant = granted(request(&service, &owner, now, now));
        let active = service.try_consume(&owner, owner_grant, now).unwrap();

        let observer = Arc::new(ObservingNotifier::default());
        let waiting = attach(&service, &pump, 3, observer.clone());
        observer.bind(&waiting);
        let first = pending(request(&service, &waiting, now, now));
        assert_eq!(waiting.snapshot().1.state(), ReservationState::Pending);
        assert_eq!(
            lock_unpoisoned(&observer.observed).last().unwrap().2,
            ReservationState::Pending
        );

        let sequence_before = waiting.snapshot().0;
        let again = pending(request(&service, &waiting, now + TTL, now));
        assert_eq!(again, first, "a repeated due request reuses its identity");
        assert_eq!(
            waiting.snapshot().0,
            sequence_before,
            "polling/coalescing adds no queue row or notice"
        );
        assert_eq!(waiting.snapshot().1.due_at(), Some(Deadline::at(now)));
        active.finish(now).unwrap();
        assert_eq!(waiting.snapshot().1.state(), ReservationState::Granted);
    }

    #[test]
    fn exactly_one_concurrent_consumer_gets_a_frame_lease() {
        let now = clock();
        let (service, pump) = service(now);
        let head = attach(&service, &pump, 4, Arc::new(NoopNotifier));
        let identity = granted(request(&service, &head, now, now));
        let barrier = Arc::new(Barrier::new(3));

        let (first, second) = thread::scope(|scope| {
            let first_barrier = Arc::clone(&barrier);
            let first_service = &service;
            let first_head = &head;
            let first = scope.spawn(move || {
                first_barrier.wait();
                first_service.try_consume(first_head, identity, now)
            });
            let second_barrier = Arc::clone(&barrier);
            let second_service = &service;
            let second_head = &head;
            let second = scope.spawn(move || {
                second_barrier.wait();
                second_service.try_consume(second_head, identity, now)
            });
            barrier.wait();
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let error = first.err().or_else(|| second.err()).unwrap();
        assert_eq!(error, RenderError::AlreadyConsumed);
    }

    #[test]
    fn expiry_at_boundary_releases_and_reselects_without_regranting_dormant_head() {
        let now = clock();
        let (service, pump) = service(now);
        let stalled = attach(&service, &pump, 5, Arc::new(NoopNotifier));
        let peer = attach(&service, &pump, 6, Arc::new(NoopNotifier));
        let old_grant = granted(request(&service, &stalled, now, now));
        let peer_request = pending(request(&service, &peer, now, now));
        let expiry = now + TTL;

        assert_eq!(
            service.pump(&pump, expiry).unwrap(),
            Some(Deadline::at(expiry + TTL))
        );
        assert_eq!(stalled.snapshot().1.state(), ReservationState::Expired);
        assert_eq!(stalled.snapshot().1.granted_at(), Some(now));
        assert_eq!(
            stalled.snapshot().1.expires_at(),
            Some(Deadline::at(expiry))
        );
        assert_eq!(peer.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(
            service
                .try_consume(&stalled, old_grant, expiry)
                .unwrap_err(),
            RenderError::Expired
        );
        service.pump(&pump, expiry + TTL).unwrap();
        assert_eq!(stalled.snapshot().1.state(), ReservationState::Expired);
        assert_eq!(peer.snapshot().1.state(), ReservationState::Expired);

        let new_due = expiry + TTL + Duration::from_millis(1);
        let new = pending(request(&service, &stalled, new_due, expiry + TTL));
        assert_ne!(new.reservation_id(), old_grant.reservation_id());
        assert_eq!(new.head(), stalled.identity());
        assert_eq!(stalled.snapshot().1.due_at(), Some(Deadline::at(new_due)));
        assert_ne!(new, peer_request);
        assert_eq!(
            service
                .pump(&pump, new_due - Duration::from_millis(1))
                .unwrap(),
            Some(Deadline::at(new_due))
        );
        assert_eq!(
            service.pump(&pump, new_due).unwrap(),
            Some(Deadline::at(new_due + TTL))
        );
        assert_eq!(stalled.snapshot().1.state(), ReservationState::Granted);
    }
    #[test]
    fn missing_notice_does_not_lose_a_grant_because_poll_reads_authoritative_state() {
        let now = clock();
        let (service, pump) = service(now);
        let head = attach(&service, &pump, 7, Arc::new(NoopNotifier));
        let identity = granted(request(&service, &head, now, now));
        let (sequence, snapshot) = service.poll(&head, now).unwrap();
        assert!(sequence.get() >= 2);
        assert_eq!(snapshot.state(), ReservationState::Granted);
        assert_eq!(snapshot.reservation(), Some(identity));
        assert!(service.try_consume(&head, identity, now).is_ok());
    }

    #[test]
    fn dropping_frame_lease_releases_workspace_and_grants_pending_head() {
        let now = clock();
        let (service, pump) = service(now);
        let owner = attach(&service, &pump, 204, Arc::new(NoopNotifier));
        let waiting = attach(&service, &pump, 205, Arc::new(NoopNotifier));
        let owner_token = granted(request(&service, &owner, now, now));
        let frame = service.try_consume(&owner, owner_token, now).unwrap();
        let waiting_token = pending(request(&service, &waiting, now, now));

        drop(frame);
        assert_eq!(waiting.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(waiting.snapshot().1.reservation(), Some(waiting_token));
    }

    #[test]
    fn oldest_due_wins_and_focus_only_breaks_an_equal_due_tie() {
        let now = clock();
        let (service, pump) = service(now);
        let owner = attach(&service, &pump, 8, Arc::new(NoopNotifier));
        let owner_grant = granted(request(&service, &owner, now, now));
        let owner_frame = service.try_consume(&owner, owner_grant, now).unwrap();

        let older = attach(&service, &pump, 9, Arc::new(NoopNotifier));
        let focused_younger = attach(&service, &pump, 10, Arc::new(NoopNotifier));
        let older_request = pending(request(
            &service,
            &older,
            now - Duration::from_millis(2),
            now,
        ));
        let younger_request = pending(request(
            &service,
            &focused_younger,
            now - Duration::from_millis(1),
            now,
        ));
        service.set_focused(&focused_younger, true, now).unwrap();
        owner_frame.finish(now).unwrap();
        assert_eq!(older.snapshot().1.reservation(), Some(older_request));
        assert_eq!(older.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(
            focused_younger.snapshot().1.reservation(),
            Some(younger_request)
        );
        assert_eq!(
            focused_younger.snapshot().1.state(),
            ReservationState::Pending
        );

        let next_owner_frame = service.try_consume(&older, older_request, now).unwrap();
        next_owner_frame.finish(now).unwrap();
        assert_eq!(
            focused_younger.snapshot().1.state(),
            ReservationState::Granted
        );
        let younger_frame = service
            .try_consume(&focused_younger, younger_request, now)
            .unwrap();
        younger_frame.finish(now).unwrap();

        let tie_owner = attach(&service, &pump, 11, Arc::new(NoopNotifier));
        let tie_grant = granted(request(&service, &tie_owner, now, now));
        let tie_frame = service.try_consume(&tie_owner, tie_grant, now).unwrap();
        let first = attach(&service, &pump, 12, Arc::new(NoopNotifier));
        let focused = attach(&service, &pump, 13, Arc::new(NoopNotifier));
        let same_due = now + Duration::from_millis(1);
        let first_request = pending(request(&service, &first, same_due, now));
        let focused_request = pending(request(&service, &focused, same_due, now));
        service.set_focused(&focused, true, now).unwrap();
        tie_frame.finish(now).unwrap();
        assert_eq!(
            service.pump(&pump, same_due).unwrap(),
            Some(Deadline::at(same_due + TTL))
        );
        assert_eq!(focused.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(focused.snapshot().1.reservation(), Some(focused_request));
        assert_eq!(first.snapshot().1.state(), ReservationState::Pending);
        assert_eq!(first.snapshot().1.reservation(), Some(first_request));
    }

    #[test]
    fn equal_due_unfocused_requests_keep_insertion_order() {
        let now = clock();
        let (service, pump) = service(now);
        let owner = attach(&service, &pump, 22, Arc::new(NoopNotifier));
        let owner_token = granted(request(&service, &owner, now, now));
        let owner_frame = service.try_consume(&owner, owner_token, now).unwrap();

        let first = attach(&service, &pump, 23, Arc::new(NoopNotifier));
        let second = attach(&service, &pump, 24, Arc::new(NoopNotifier));
        let due = now + Duration::from_millis(10);
        let first_token = pending(request(&service, &first, due, now));
        let second_token = pending(request(&service, &second, due, now));
        owner_frame.finish(due).unwrap();
        assert_eq!(first.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(first.snapshot().1.reservation(), Some(first_token));
        assert_eq!(second.snapshot().1.state(), ReservationState::Pending);
        assert_eq!(second.snapshot().1.reservation(), Some(second_token));

        let first_frame = service.try_consume(&first, first_token, due).unwrap();
        first_frame.finish(due).unwrap();
        assert_eq!(second.snapshot().1.state(), ReservationState::Granted);
    }

    #[test]
    fn cancellation_releases_a_grant_and_invalidates_its_token() {
        let now = clock();
        let (service, pump) = service(now);
        let cancelled = attach(&service, &pump, 14, Arc::new(NoopNotifier));
        let peer = attach(&service, &pump, 15, Arc::new(NoopNotifier));
        let token = granted(request(&service, &cancelled, now, now));
        let peer_token = pending(request(&service, &peer, now, now));

        service.cancel_request(&cancelled, now).unwrap();
        assert_eq!(cancelled.snapshot().1.state(), ReservationState::Cancelled);
        assert_eq!(peer.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(peer.snapshot().1.reservation(), Some(peer_token));
        assert_eq!(
            service.try_consume(&cancelled, token, now).unwrap_err(),
            RenderError::Cancelled
        );
    }

    #[test]
    fn wrong_service_cannot_retire_another_services_head() {
        let now = clock();
        let (owner, mut owner_pump) = service(now);
        let (foreign, _foreign_pump) = service(now);
        let mut head = attach(&owner, &owner_pump, 16, Arc::new(NoopNotifier));
        assert_eq!(
            foreign.unregister_pump(&mut owner_pump).unwrap_err(),
            RenderError::WrongService
        );
        assert_eq!(owner.pump(&owner_pump, now).unwrap(), None);

        assert_eq!(
            foreign.retire(&mut head, now).unwrap_err(),
            RenderError::WrongService
        );
        let token = granted(request(&owner, &head, now, now));
        let frame = owner.try_consume(&head, token, now).unwrap();
        frame.finish(now).unwrap();
    }

    #[test]
    fn retirement_during_consumed_lease_revokes_output_but_waits_for_real_release() {
        let now = clock();
        let (service, pump) = service(now);
        let mut retiring = attach(&service, &pump, 16, Arc::new(NoopNotifier));
        let retiring_cell = Arc::clone(&retiring.cell);
        let peer = attach(&service, &pump, 17, Arc::new(NoopNotifier));
        let token = granted(request(&service, &retiring, now, now));
        let frame = service.try_consume(&retiring, token, now).unwrap();
        let peer_token = pending(request(&service, &peer, now, now));
        service.retire(&mut retiring, now).unwrap();

        assert_eq!(
            retiring_cell
                .publication
                .read(|_, snapshot| snapshot.state()),
            ReservationState::Cancelled
        );
        assert_eq!(
            frame.publish_retained_scene(SceneHandle::from_opaque(41)),
            Err(RenderError::Cancelled)
        );
        assert_eq!(peer.snapshot().1.state(), ReservationState::Pending);
        frame.finish(now).unwrap();
        assert_eq!(peer.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(peer.snapshot().1.reservation(), Some(peer_token));
    }

    #[test]
    fn retirement_races_publication_and_frame_release_without_leasing_twice() {
        let now = clock();
        let (service, pump) = service(now);
        let mut retiring = attach(&service, &pump, 20, Arc::new(NoopNotifier));
        let peer = attach(&service, &pump, 21, Arc::new(NoopNotifier));
        let token = granted(request(&service, &retiring, now, now));
        let frame = service.try_consume(&retiring, token, now).unwrap();
        let peer_token = pending(request(&service, &peer, now, now));
        let barrier = Arc::new(Barrier::new(3));

        let (retired, published) = thread::scope(|scope| {
            let retire_barrier = Arc::clone(&barrier);
            let retire_service = &service;
            let retiring_head = &mut retiring;
            let retire = scope.spawn(move || {
                retire_barrier.wait();
                retire_service.retire(retiring_head, now)
            });
            let publish_barrier = Arc::clone(&barrier);
            let publish = scope.spawn(move || {
                publish_barrier.wait();
                let result = frame.publish_retained_scene(SceneHandle::from_opaque(303));
                drop(frame);
                result
            });
            barrier.wait();
            (retire.join().unwrap(), publish.join().unwrap())
        });

        assert_eq!(retired, Ok(()));
        assert!(matches!(published, Ok(()) | Err(RenderError::Cancelled)));
        assert_eq!(retiring.snapshot().1.state(), ReservationState::Cancelled);
        assert_eq!(retiring.snapshot().1.retained_scene(), None);
        assert_eq!(peer.snapshot().1.state(), ReservationState::Granted);
        assert_eq!(peer.snapshot().1.reservation(), Some(peer_token));
    }

    #[test]
    fn retained_scene_is_per_head_and_retirement_clears_only_its_own_scene() {
        let now = clock();
        let (service, pump) = service(now);
        let mut first = attach(&service, &pump, 18, Arc::new(NoopNotifier));
        let second = attach(&service, &pump, 19, Arc::new(NoopNotifier));
        let first_token = granted(request(&service, &first, now, now));
        let first_frame = service.try_consume(&first, first_token, now).unwrap();
        first_frame
            .publish_retained_scene(SceneHandle::from_opaque(101))
            .unwrap();
        first_frame.finish(now).unwrap();

        let second_token = granted(request(&service, &second, now, now));
        let second_frame = service.try_consume(&second, second_token, now).unwrap();
        second_frame
            .publish_retained_scene(SceneHandle::from_opaque(202))
            .unwrap();
        second_frame.finish(now).unwrap();
        assert_eq!(
            first.snapshot().1.retained_scene(),
            Some(SceneHandle::from_opaque(101))
        );
        assert_eq!(
            second.snapshot().1.retained_scene(),
            Some(SceneHandle::from_opaque(202))
        );

        let old_identity = first.identity();
        let old_token = granted(request(&service, &first, now, now));
        service.retire(&mut first, now).unwrap();
        let reattached = attach(&service, &pump, 18, Arc::new(NoopNotifier));
        assert_ne!(
            reattached.identity().generation(),
            old_identity.generation()
        );
        assert_eq!(reattached.snapshot().1.retained_scene(), None);
        assert_eq!(
            service
                .try_consume(&reattached, old_token, now)
                .unwrap_err(),
            RenderError::StaleReservation
        );
        assert_eq!(
            second.snapshot().1.retained_scene(),
            Some(SceneHandle::from_opaque(202))
        );
    }

    #[test]
    fn head_cap_refuses_with_a_typed_reason() {
        let now = clock();
        let (service, pump) = service(now);
        let notifier: Arc<dyn Notifier> = Arc::new(NoopNotifier);
        let mut heads = Vec::new();
        for raw in 1..=RENDER_HEAD_CAP as u64 {
            heads.push(attach(&service, &pump, 100 + raw, Arc::clone(&notifier)));
        }
        let error = service.attach(&pump, slot(999), notifier).unwrap_err();
        assert_eq!(
            error,
            RenderError::Refused(RenderRefusal::HeadCap {
                cap: RENDER_HEAD_CAP
            })
        );
        drop(heads);
        assert_eq!(service.pump(&pump, now).unwrap(), None);
    }

    #[test]
    fn pump_registration_is_unique_and_drop_retires_heads() {
        let service = RenderService::new(TTL).unwrap();
        let pump = service.register_pump().unwrap();
        assert_eq!(
            service.register_pump().unwrap_err(),
            RenderError::Refused(RenderRefusal::PumpAlreadyRegistered)
        );
        let head = attach(&service, &pump, 200, Arc::new(NoopNotifier));
        let cell = Arc::clone(&head.cell);

        drop(pump);
        assert_eq!(
            cell.publication.read(|_, value| value.state()),
            ReservationState::Cancelled
        );
        let mut restarted = service.register_pump().unwrap();
        assert!(restarted.active);
        let attached = attach(&service, &restarted, 201, Arc::new(NoopNotifier));
        let attached_cell = Arc::clone(&attached.cell);
        service.unregister_pump(&mut restarted).unwrap();
        assert!(!restarted.active);
        assert_eq!(
            attached_cell.publication.read(|_, value| value.state()),
            ReservationState::Cancelled
        );
        assert_eq!(
            service.pump(&restarted, Instant::now()).unwrap_err(),
            RenderError::Refused(RenderRefusal::NoLivePump)
        );

        let foreign_service = RenderService::new(TTL).unwrap();
        let foreign_pump = foreign_service.register_pump().unwrap();
        assert_eq!(
            service
                .attach(&foreign_pump, slot(201), Arc::new(NoopNotifier))
                .unwrap_err(),
            RenderError::WrongService
        );
    }

    #[test]
    fn cancellation_and_consumption_share_one_ordering_point() {
        let now = clock();
        let (service, pump) = service(now);
        let head = attach(&service, &pump, 202, Arc::new(NoopNotifier));
        let token = granted(request(&service, &head, now, now));
        let barrier = Arc::new(Barrier::new(3));
        let (consume, cancel) = thread::scope(|scope| {
            let consume_barrier = Arc::clone(&barrier);
            let consume_service = &service;
            let consume_head = &head;
            let consume = scope.spawn(move || {
                consume_barrier.wait();
                consume_service.try_consume(consume_head, token, now)
            });
            let cancel_barrier = Arc::clone(&barrier);
            let cancel_service = &service;
            let cancel_head = &head;
            let cancel = scope.spawn(move || {
                cancel_barrier.wait();
                cancel_service.cancel_request(cancel_head, now)
            });
            barrier.wait();
            (consume.join().unwrap(), cancel.join().unwrap())
        });
        assert!(cancel.is_ok());
        assert_eq!(head.snapshot().1.state(), ReservationState::Cancelled);
        if let Ok(frame) = consume {
            assert_eq!(
                frame.publish_retained_scene(SceneHandle::from_opaque(7)),
                Err(RenderError::Cancelled)
            );
            frame.finish(now).unwrap();
        } else {
            assert_eq!(consume.unwrap_err(), RenderError::Cancelled);
        }
    }

    #[test]
    fn pending_due_deadline_is_the_next_pump_deadline() {
        let now = clock();
        let (service, pump) = service(now);
        let head = attach(&service, &pump, 203, Arc::new(NoopNotifier));
        let due = now + Duration::from_millis(5);
        assert_eq!(
            request(&service, &head, due, now),
            RequestOutcome::Busy(head.snapshot().1.reservation().unwrap())
        );
        assert_eq!(service.pump(&pump, now).unwrap(), Some(Deadline::at(due)));
        assert_eq!(
            service.pump(&pump, due).unwrap(),
            Some(Deadline::at(due + TTL))
        );
        assert_eq!(head.snapshot().1.state(), ReservationState::Granted);
    }

    #[test]
    fn identity_cursor_exhaustion_refuses_reuse() {
        let mut cursor = NonZeroU64::new(u64::MAX);
        assert_eq!(
            take_next(&mut cursor, RenderError::HeadGenerationExhausted),
            Ok(NonZeroU64::new(u64::MAX).unwrap())
        );
        assert_eq!(cursor, None);
        assert_eq!(
            take_next(&mut cursor, RenderError::HeadGenerationExhausted),
            Err(RenderError::HeadGenerationExhausted)
        );
    }

    #[test]
    fn ttl_overflow_keeps_the_request_published_as_pending() {
        let service = RenderService::new(Duration::MAX).unwrap();
        let pump = service.register_pump().unwrap();
        let now = clock();
        let head = attach(&service, &pump, 206, Arc::new(NoopNotifier));

        assert_eq!(
            service.request_frame(&head, Deadline::at(now), now),
            Err(RenderError::DeadlineOverflow)
        );
        assert_eq!(head.snapshot().1.state(), ReservationState::Pending);
        assert_eq!(service.pump(&pump, now), Err(RenderError::DeadlineOverflow));
    }
}

//! One bounded, process-owned DNS resolver for game transport.
//!
//! The caller supplies its slot identity and Park wake. Name resolution itself
//! runs only on the single `dns-resolver` worker. Pending state, cache entries,
//! and result cells all use fixed-capacity representations; a blocked native
//! lookup retains its active request until it actually returns.

use std::net::{SocketAddr, ToSocketAddrs};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_PENDING_KEYS: usize = 64;
const MAX_ADDRESSES: usize = 8;
const MAX_HOST_BYTES: usize = 253;
const MAX_CACHE_ENTRIES: usize = 128;
const MAX_PENDING_BYTES: usize = 64 * 1024;
const MAX_CACHE_BYTES: usize = 128 * 1024;
const TICKET_WAIT: Duration = Duration::from_secs(10);
const CACHE_TTL: Duration = Duration::from_secs(60);
const PROFILE_FLOOR: Duration = Duration::from_secs(120);
const UNPROFILED_STALL_LIMIT: Duration = Duration::from_secs(15 * 60);
const ARC_CONTROL_BYTES: usize = 2 * std::mem::size_of::<usize>();
const MAX_CONFIGURED_SLOTS: usize = 1024;
const PROFILE_LABEL_BYTES: usize = 64;
static PROCESS_RESOLVER: LazyLock<Result<Arc<Resolver>, ResolverConfigError>> =
    LazyLock::new(|| Resolver::system(MAX_CONFIGURED_SLOTS));

/// Stable slot identity. The resolver treats generations as attempt identity,
/// but enforces one live waiter for this slot across every generation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SlotKey(pub u64);

/// Identity supplied by the owner that is awaiting a DNS result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolveIdentity {
    pub slot: SlotKey,
    pub generation: u64,
}

/// A caller-owned wake target. Implementations must only publish/wake; the
/// resolver invokes this method after releasing its state lock.
pub trait ResolverWake: Send + Sync + 'static {
    fn wake(&self);
}

impl<F> ResolverWake for F
where
    F: Fn() + Send + Sync + 'static,
{
    fn wake(&self) {
        self()
    }
}

impl ResolverWake for super::park::Park {
    fn wake(&self) {
        self.publish();
    }
}

/// Up to eight resolved socket addresses, stored inline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedAddresses {
    len: u8,
    items: [Option<SocketAddr>; MAX_ADDRESSES],
}

impl ResolvedAddresses {
    pub const fn new() -> Self {
        Self {
            len: 0,
            items: [None; MAX_ADDRESSES],
        }
    }

    pub fn push(&mut self, address: SocketAddr) -> bool {
        let index = usize::from(self.len);
        if index == MAX_ADDRESSES {
            return false;
        }
        self.items[index] = Some(address);
        self.len += 1;
        true
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<SocketAddr> {
        if index >= self.len() {
            None
        } else {
            self.items[index]
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = SocketAddr> + '_ {
        self.items[..self.len()].iter().copied().flatten()
    }
}

impl Default for ResolvedAddresses {
    fn default() -> Self {
        Self::new()
    }
}

/// Typed service failure. Diagnostics are deliberately absent unless a future
/// caller can provide an already-redacted, bounded value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolverUnavailable {
    pub cause: ResolverUnavailableCause,
    pub detected_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverUnavailableCause {
    StartFailed,
    Panicked,
    ProgressLost,
}

/// Health policy currently selected for active-call hang detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverHealthPolicy {
    Unqualified,
    Qualified { threshold: Duration },
}

/// A bounded health snapshot for the process composition. It has no `api`
/// dependency and latches the first unavailable cause for this service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolverHealth {
    pub unavailable: Option<ResolverUnavailable>,
    pub policy: ResolverHealthPolicy,
    pub cache_epoch: u64,
}

/// Resolver-side failures returned to an individual ticket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverFailure {
    Cancelled,
    TimedOut,
    Lookup(ResolverLookupError),
    Unavailable(ResolverUnavailable),
}

/// Backend error categories. Native error text is not retained or used for
/// policy, so a hostname cannot leak through an error diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverLookupError {
    System(std::io::ErrorKind),
    NoAddresses,
}

/// Addresses and failures are Copy transport values; boxing the ready result
/// would add allocation and indirection to resolver polling.
#[expect(
    clippy::large_enum_variant,
    reason = "Keep up to eight resolved addresses inline without ticket polling allocation"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvePoll {
    Pending,
    Ready(Result<ResolvedAddresses, ResolverFailure>),
}

/// Admission refusal. All limits are checked before the request or waiter is
/// installed in resolver custody.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverAdmissionError {
    InvalidHost,
    Busy,
    SlotAlreadyWaiting,
    DeadlineOverflow,
    Unavailable(ResolverUnavailable),
}

/// Construction errors are limited to invalid configured client capacity or
/// failure to allocate the bounded waiter table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolverConfigError {
    NoSlots,
    TooManySlots,
    AllocationFailed,
}

/// Ticket for one slot generation. Clones refer to the same single waiter;
/// cancelling any clone cancels that waiter. Dropping a ticket leaves the
/// bounded waiter registered until explicit cancellation or its 10 s expiry.
#[derive(Clone)]
pub struct ResolveTicket {
    identity: ResolveIdentity,
    deadline: Instant,
    cell: Arc<TicketCell>,
    resolver: Weak<ResolverInner>,
}

impl ResolveTicket {
    pub const fn identity(&self) -> ResolveIdentity {
        self.identity
    }

    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Polls the result and runs due ticket/service maintenance first.
    pub fn poll(&self) -> ResolvePoll {
        if let Some(inner) = self.resolver.upgrade() {
            maintain_inner(&inner);
        }
        match mutex_lock(&self.cell.state).terminal {
            TicketTerminal::Pending => ResolvePoll::Pending,
            TicketTerminal::Complete(Ok(addresses)) => ResolvePoll::Ready(Ok(addresses)),
            TicketTerminal::Complete(Err(error)) => ResolvePoll::Ready(Err(error)),
            TicketTerminal::Cancelled => ResolvePoll::Ready(Err(ResolverFailure::Cancelled)),
            TicketTerminal::TimedOut => ResolvePoll::Ready(Err(ResolverFailure::TimedOut)),
            TicketTerminal::Unavailable(unavailable) => {
                ResolvePoll::Ready(Err(ResolverFailure::Unavailable(unavailable)))
            }
        }
    }

    /// Actual resolver backend start/end for this ticket's request. Cached
    /// positive and negative answers have no backend invocation to report.
    pub fn backend_timing(&self) -> Option<(Instant, Instant)> {
        mutex_lock(&self.cell.state).backend_timing
    }

    /// Atomically detaches this waiter. The slot quota is available to the
    /// next generation immediately; an active native lookup remains charged.
    pub fn cancel(&self) -> bool {
        self.resolver
            .upgrade()
            .is_some_and(|inner| cancel_ticket(&inner, &self.cell))
    }
}

impl std::fmt::Debug for ResolveTicket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolveTicket")
            .field("identity", &self.identity)
            .field("deadline", &self.deadline)
            .finish()
    }
}

/// Measured resolver profile metadata. Invalid or incomplete receipts remain
/// representable and select the conservative unqualified policy.
#[derive(Clone, Copy, Debug)]
pub struct ResolverProfileReceipt {
    configuration_digest: [u8; 32],
    provider: ProfileLabel,
    backend_version: ProfileLabel,
    observed_max_seconds: f64,
    measured_cold: bool,
    measured_warm: bool,
    measured_failure: bool,
    provenance_digest: [u8; 32],
}

impl ResolverProfileReceipt {
    #[expect(
        clippy::too_many_arguments,
        reason = "A profile receipt retains independent provider and configuration identities"
    )]
    pub fn new(
        configuration_digest: [u8; 32],
        provider: &str,
        backend_version: &str,
        observed_max_seconds: f64,
        measured_cold: bool,
        measured_warm: bool,
        measured_failure: bool,
        provenance_digest: [u8; 32],
    ) -> Self {
        Self {
            configuration_digest,
            provider: ProfileLabel::new(provider),
            backend_version: ProfileLabel::new(backend_version),
            observed_max_seconds,
            measured_cold,
            measured_warm,
            measured_failure,
            provenance_digest,
        }
    }

    pub fn provider(&self) -> &str {
        self.provider.as_str()
    }

    pub fn backend_version(&self) -> &str {
        self.backend_version.as_str()
    }
}

/// Off-pump resolver configuration identity and optional matching profile.
/// The owner performs any platform-specific identity collection and supplies
/// only a fixed digest here.
#[derive(Clone, Copy, Debug)]
pub struct ResolverConfiguration {
    digest: Option<[u8; 32]>,
    profile: Option<ResolverProfileReceipt>,
}

impl ResolverConfiguration {
    pub const fn unprofiled() -> Self {
        Self {
            digest: None,
            profile: None,
        }
    }

    pub const fn new(digest: Option<[u8; 32]>, profile: Option<ResolverProfileReceipt>) -> Self {
        Self { digest, profile }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProfileLabel {
    bytes: [u8; PROFILE_LABEL_BYTES],
    len: u8,
    valid: bool,
}

impl ProfileLabel {
    fn new(value: &str) -> Self {
        let mut bytes = [0; PROFILE_LABEL_BYTES];
        let valid = !value.is_empty() && value.is_ascii() && value.len() <= PROFILE_LABEL_BYTES;
        if valid {
            bytes[..value.len()].copy_from_slice(value.as_bytes());
        }
        Self {
            bytes,
            len: if valid { value.len() as u8 } else { 0 },
            valid,
        }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .expect("profile labels are validated ASCII")
    }
}

/// One Resolver owns one bounded FIFO and starts its one worker at
/// construction. Compositions should create one instance per process and call
/// `maintain` at their ≤100 ms service-health cadence.
pub struct Resolver {
    inner: Arc<ResolverInner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Resolver {
    /// One process resolver for all game clients. Constructing Play alone
    /// does not start WorkAdmission's unrelated background worker.
    pub fn process_shared() -> Result<Arc<Self>, ResolverConfigError> {
        match &*PROCESS_RESOLVER {
            Ok(resolver) => Ok(Arc::clone(resolver)),
            Err(error) => Err(*error),
        }
    }

    /// Construct the production resolver. The real `to_socket_addrs` backend
    /// and system monotonic clock are installed automatically. No profile is
    /// required; absent qualification selects the 15 min default.
    pub fn system(max_slots: usize) -> Result<Arc<Self>, ResolverConfigError> {
        Self::build(
            max_slots,
            Arc::new(SystemResolverBackend),
            Arc::new(SystemMonotonicClock),
            ResolverConfiguration::unprofiled(),
        )
    }

    fn build(
        max_slots: usize,
        backend: Arc<dyn ResolverBackend>,
        clock: Arc<dyn MonotonicClock>,
        configuration: ResolverConfiguration,
    ) -> Result<Arc<Self>, ResolverConfigError> {
        if max_slots == 0 {
            return Err(ResolverConfigError::NoSlots);
        }
        if max_slots > MAX_CONFIGURED_SLOTS {
            return Err(ResolverConfigError::TooManySlots);
        }
        let mut waiters = Vec::new();
        waiters
            .try_reserve_exact(max_slots)
            .map_err(|_| ResolverConfigError::AllocationFailed)?;
        waiters.resize_with(max_slots, || None);
        let policy = configuration
            .qualified_threshold()
            .map_or(ResolverHealthPolicy::Unqualified, |threshold| {
                ResolverHealthPolicy::Qualified { threshold }
            });
        let inner = Arc::new(ResolverInner {
            state: Mutex::new(ResolverState {
                requests: [None; MAX_PENDING_KEYS],
                queue: [0; MAX_PENDING_KEYS],
                queue_head: 0,
                queue_len: 0,
                waiters,
                active: None,
                active_index: None,
                shutdown: false,
                unavailable: None,
                policy,
                cache_epoch: 0,
                epoch_exhausted: false,
                cache: [None; MAX_CACHE_ENTRIES],
                cache_bytes: 0,
                cache_order: 0,
                buffer_reserved: false,
            }),
            changed: Condvar::new(),
            backend,
            clock,
            pending_bytes: Arc::new(AtomicUsize::new(0)),
        });
        let resolver = Arc::new(Self {
            inner,
            worker: Mutex::new(None),
        });
        resolver.start_worker();
        Ok(resolver)
    }

    /// Admit one resolve request. Host bytes are kept exactly as supplied for
    /// coalescing; non-ASCII, empty, and over-253-byte names are refused before
    /// any request or waiter is charged.
    pub fn resolve(
        &self,
        identity: ResolveIdentity,
        host: &str,
        port: u16,
        wake: Arc<dyn ResolverWake>,
    ) -> Result<ResolveTicket, ResolverAdmissionError> {
        maintain_inner(&self.inner);
        let key = ResolveKey::new(host, port).ok_or(ResolverAdmissionError::InvalidHost)?;
        let now = self.inner.clock.now();
        let deadline = now
            .checked_add(TICKET_WAIT)
            .ok_or(ResolverAdmissionError::DeadlineOverflow)?;
        let mut state = mutex_lock(&self.inner.state);
        if let Some(unavailable) = state.unavailable {
            return Err(ResolverAdmissionError::Unavailable(unavailable));
        }

        if let Some(existing) = state
            .waiters
            .iter()
            .flatten()
            .find(|waiter| waiter.identity.slot == identity.slot)
        {
            let same_attempt = existing.identity.generation == identity.generation
                && state
                    .requests
                    .get(existing.request_index)
                    .and_then(Option::as_ref)
                    .is_some_and(|request| request.id == existing.request_id && request.key == key);
            if same_attempt {
                return Ok(ResolveTicket {
                    identity,
                    deadline: existing.deadline,
                    cell: Arc::clone(&existing.cell),
                    resolver: Arc::downgrade(&self.inner),
                });
            }
            return Err(ResolverAdmissionError::SlotAlreadyWaiting);
        }

        prune_expired_cache(&mut state, now);
        if let Some(outcome) = cache_lookup(&mut state, key, now) {
            if !reserve_pending(&self.inner.pending_bytes, WAITER_CHARGE) {
                return Err(ResolverAdmissionError::Busy);
            }
            let cell = Arc::new(TicketCell::new(
                Arc::clone(&self.inner.pending_bytes),
                WAITER_CHARGE,
                TicketTerminal::Complete(outcome.into_result()),
            ));
            return Ok(ResolveTicket {
                identity,
                deadline,
                cell,
                resolver: Arc::downgrade(&self.inner),
            });
        }

        let existing_request = find_coalescible_request(&state, key);
        let new_request = existing_request.is_none();
        let request_index = match existing_request {
            Some(index) => index,
            None => state
                .requests
                .iter()
                .position(Option::is_none)
                .ok_or(ResolverAdmissionError::Busy)?,
        };
        let waiter_index = state
            .waiters
            .iter()
            .position(Option::is_none)
            .ok_or(ResolverAdmissionError::Busy)?;
        let request_charge = if new_request { REQUEST_CHARGE } else { 0 };
        let add_buffer_charge =
            new_request && state.active.is_none() && state.queue_len == 0 && !state.buffer_reserved;
        let buffer_charge = if add_buffer_charge {
            ACTIVE_BUFFER_CHARGE
        } else {
            0
        };
        let total_charge = request_charge
            .checked_add(WAITER_CHARGE)
            .and_then(|bytes| bytes.checked_add(buffer_charge))
            .ok_or(ResolverAdmissionError::Busy)?;
        if !reserve_pending(&self.inner.pending_bytes, total_charge) {
            return Err(ResolverAdmissionError::Busy);
        }

        let request_id = if new_request {
            next_request_id(&state)
        } else {
            state.requests[request_index]
                .as_ref()
                .expect("coalesced request exists")
                .id
        };
        let cell = Arc::new(TicketCell::new(
            Arc::clone(&self.inner.pending_bytes),
            WAITER_CHARGE,
            TicketTerminal::Pending,
        ));
        if new_request {
            state.requests[request_index] = Some(PendingRequest {
                key,
                id: request_id,
                epoch: None,
                waiter_count: 0,
            });
            push_queue(&mut state, request_index);
        }
        if add_buffer_charge {
            state.buffer_reserved = true;
        }
        state.requests[request_index]
            .as_mut()
            .expect("request was installed")
            .waiter_count += 1;
        state.waiters[waiter_index] = Some(Waiter {
            identity,
            request_index,
            request_id,
            deadline,
            cell: Arc::clone(&cell),
            wake,
        });
        let ticket = ResolveTicket {
            identity,
            deadline,
            cell,
            resolver: Arc::downgrade(&self.inner),
        };
        drop(state);
        self.inner.changed.notify_one();
        Ok(ticket)
    }

    /// Run ticket expiry and the active-call hang check. This method performs
    /// no DNS or blocking configuration collection and wakes only after the
    /// resolver state lock has been released.
    pub fn maintain(&self) -> ResolverHealth {
        maintain_inner(&self.inner)
    }

    pub fn health(&self) -> ResolverHealth {
        #[cfg(debug_assertions)]
        debug_assert!(
            !super::park::in_k2_predicate(),
            "resolver health must never take its state lock under K2"
        );
        let state = mutex_lock(&self.inner.state);
        health_snapshot(&state)
    }

    pub fn pending_bytes(&self) -> usize {
        self.inner.pending_bytes.load(Ordering::Acquire)
    }

    /// Publish a configuration refresh collected off-pump by the owner. Every
    /// refresh advances the cache epoch, drops cached entries, and requalifies
    /// against the supplied digest. An active call keeps its original start
    /// instant and receives no smaller than the 15 min default threshold.
    pub fn refresh_configuration(&self, configuration: ResolverConfiguration) -> ResolverHealth {
        let now = self.inner.clock.now();
        let mut wakes = WakeBatch::new();
        let health = {
            let mut state = mutex_lock(&self.inner.state);
            match state.cache_epoch.checked_add(1) {
                Some(epoch) => state.cache_epoch = epoch,
                None => state.epoch_exhausted = true,
            }
            clear_cache(&mut state);
            state.policy = configuration
                .qualified_threshold()
                .map_or(ResolverHealthPolicy::Unqualified, |threshold| {
                    ResolverHealthPolicy::Qualified { threshold }
                });
            if let Some(active) = state.active.as_mut() {
                active.threshold = active.threshold.max(UNPROFILED_STALL_LIMIT);
            }
            expire_waiters_locked(&self.inner, &mut state, now, &mut wakes);
            check_stall_locked(&self.inner, &mut state, now, &mut wakes);
            health_snapshot(&state)
        };
        wake_all(wakes);
        health
    }

    fn start_worker(self: &Arc<Self>) {
        let inner = Arc::clone(&self.inner);
        let panic_inner = Arc::clone(&inner);
        let spawned = thread::Builder::new()
            .name("dns-resolver".to_owned())
            .spawn(move || {
                let worker = catch_unwind(AssertUnwindSafe(|| resolver_worker(inner)));
                if worker.is_err() {
                    worker_panicked(&panic_inner, None);
                }
            });
        match spawned {
            Ok(handle) => *mutex_lock(&self.worker) = Some(handle),
            Err(_) => {
                let now = self.inner.clock.now();
                let mut wakes = WakeBatch::new();
                {
                    let mut state = mutex_lock(&self.inner.state);
                    latch_unavailable_locked(
                        &self.inner,
                        &mut state,
                        ResolverUnavailableCause::StartFailed,
                        now,
                        &mut wakes,
                    );
                }
                wake_all(wakes);
            }
        }
    }

    #[cfg(test)]
    fn injected(
        max_slots: usize,
        backend: Arc<dyn ResolverBackend>,
        clock: Arc<dyn MonotonicClock>,
        configuration: ResolverConfiguration,
    ) -> Result<Arc<Self>, ResolverConfigError> {
        Self::build(max_slots, backend, clock, configuration)
    }
}

impl Drop for Resolver {
    fn drop(&mut self) {
        {
            let mut state = mutex_lock(&self.inner.state);
            state.shutdown = true;
        }
        self.inner.changed.notify_all();
        if let Some(handle) = mutex_lock(&self.worker).take() {
            if handle.is_finished() {
                let _ = handle.join();
            }
            // A blocked native lookup is not joined or replaced. Dropping the
            // handle detaches it while its worker-owned request remains alive.
        }
    }
}

struct ResolverInner {
    state: Mutex<ResolverState>,
    changed: Condvar,
    backend: Arc<dyn ResolverBackend>,
    clock: Arc<dyn MonotonicClock>,
    pending_bytes: Arc<AtomicUsize>,
}

struct WakeBatch {
    values: [Option<Arc<dyn ResolverWake>>; MAX_CONFIGURED_SLOTS],
    len: usize,
}

impl WakeBatch {
    fn new() -> Self {
        Self {
            values: std::array::from_fn(|_| None),
            len: 0,
        }
    }

    fn push(&mut self, wake: Arc<dyn ResolverWake>) {
        assert!(
            self.len < MAX_CONFIGURED_SLOTS,
            "resolver wake batch overflow"
        );
        self.values[self.len] = Some(wake);
        self.len += 1;
    }
}

struct ResolverState {
    requests: [Option<PendingRequest>; MAX_PENDING_KEYS],
    queue: [usize; MAX_PENDING_KEYS],
    queue_head: usize,
    queue_len: usize,
    waiters: Vec<Option<Waiter>>,
    active: Option<ActiveCall>,
    active_index: Option<usize>,
    shutdown: bool,
    unavailable: Option<ResolverUnavailable>,
    policy: ResolverHealthPolicy,
    cache_epoch: u64,
    epoch_exhausted: bool,
    cache: [Option<CacheEntry>; MAX_CACHE_ENTRIES],
    cache_bytes: usize,
    cache_order: u128,
    buffer_reserved: bool,
}

#[derive(Clone, Copy)]
struct PendingRequest {
    key: ResolveKey,
    id: u128,
    epoch: Option<u64>,
    waiter_count: usize,
}

struct Waiter {
    identity: ResolveIdentity,
    request_index: usize,
    request_id: u128,
    deadline: Instant,
    cell: Arc<TicketCell>,
    wake: Arc<dyn ResolverWake>,
}

#[derive(Clone, Copy)]
struct ActiveCall {
    request_id: u128,
    started_at: Instant,
    threshold: Duration,
    epoch: u64,
}

#[derive(Clone, Copy)]
struct ResolveKey {
    host: [u8; MAX_HOST_BYTES],
    host_len: u8,
    port: u16,
}

impl ResolveKey {
    fn new(host: &str, port: u16) -> Option<Self> {
        if host.is_empty() || host.len() > MAX_HOST_BYTES || !host.is_ascii() {
            return None;
        }
        let mut bytes = [0; MAX_HOST_BYTES];
        bytes[..host.len()].copy_from_slice(host.as_bytes());
        Some(Self {
            host: bytes,
            host_len: host.len() as u8,
            port,
        })
    }

    fn host(&self) -> &str {
        std::str::from_utf8(&self.host[..usize::from(self.host_len)])
            .expect("validated ASCII resolver key")
    }
}

impl PartialEq for ResolveKey {
    fn eq(&self, other: &Self) -> bool {
        self.host_len == other.host_len
            && self.port == other.port
            && self.host[..usize::from(self.host_len)] == other.host[..usize::from(other.host_len)]
    }
}

impl Eq for ResolveKey {}

// Keep completed resolver results inline and Copy: boxing would allocate on
// the resolver completion path.
#[expect(
    clippy::large_enum_variant,
    reason = "Resolver completion carries bounded addresses inline without allocation"
)]
#[derive(Clone, Copy)]
enum ResolverOutcome {
    Resolved(ResolvedAddresses),
    Failed(ResolverLookupError),
}

impl ResolverOutcome {
    fn into_result(self) -> Result<ResolvedAddresses, ResolverFailure> {
        match self {
            Self::Resolved(addresses) => Ok(addresses),
            Self::Failed(error) => Err(ResolverFailure::Lookup(error)),
        }
    }
}

// Ticket state is Copy and polled on the transport path; boxing the result
// would impose allocation/indirection on that path.
#[expect(
    clippy::large_enum_variant,
    reason = "A terminal ticket carries bounded addresses inline without allocation"
)]
#[derive(Clone, Copy)]
enum TicketTerminal {
    Pending,
    Complete(Result<ResolvedAddresses, ResolverFailure>),
    Cancelled,
    TimedOut,
    Unavailable(ResolverUnavailable),
}

struct TicketCell {
    state: Mutex<TicketCellState>,
    _charge: ChargeLease,
}

struct TicketCellState {
    terminal: TicketTerminal,
    backend_timing: Option<(Instant, Instant)>,
}

impl TicketCell {
    fn new(ledger: Arc<AtomicUsize>, charge: usize, terminal: TicketTerminal) -> Self {
        Self {
            state: Mutex::new(TicketCellState {
                terminal,
                backend_timing: None,
            }),
            _charge: ChargeLease {
                ledger,
                bytes: charge,
            },
        }
    }
}

struct ChargeLease {
    ledger: Arc<AtomicUsize>,
    bytes: usize,
}

impl Drop for ChargeLease {
    fn drop(&mut self) {
        release_pending(&self.ledger, self.bytes);
    }
}

#[derive(Clone, Copy)]
struct CacheEntry {
    key: ResolveKey,
    epoch: u64,
    expires_at: Instant,
    last_used_at: Instant,
    insertion_order: u128,
    outcome: ResolverOutcome,
}

trait ResolverBackend: Send + Sync + 'static {
    fn lookup(&self, host: &str, port: u16) -> Result<ResolvedAddresses, ResolverLookupError>;
}

trait MonotonicClock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

struct SystemResolverBackend;

impl ResolverBackend for SystemResolverBackend {
    fn lookup(&self, host: &str, port: u16) -> Result<ResolvedAddresses, ResolverLookupError> {
        let addresses = (host, port)
            .to_socket_addrs()
            .map_err(|error| ResolverLookupError::System(error.kind()))?;
        let mut result = ResolvedAddresses::new();
        for address in addresses.take(MAX_ADDRESSES) {
            result.push(address);
        }
        if result.is_empty() {
            Err(ResolverLookupError::NoAddresses)
        } else {
            Ok(result)
        }
    }
}

struct SystemMonotonicClock;

impl MonotonicClock for SystemMonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl ResolverConfiguration {
    fn qualified_threshold(self) -> Option<Duration> {
        let digest = self.digest?;
        let profile = self.profile?;
        if profile.configuration_digest != digest
            || !profile.provider.valid
            || !profile.backend_version.valid
            || !profile.measured_cold
            || !profile.measured_warm
            || !profile.measured_failure
            || profile.provenance_digest.iter().all(|byte| *byte == 0)
            || !profile.observed_max_seconds.is_finite()
            || profile.observed_max_seconds <= 0.0
        {
            return None;
        }
        let observed = Duration::try_from_secs_f64(profile.observed_max_seconds).ok()?;
        if observed.is_zero() {
            return None;
        }
        observed
            .checked_mul(4)
            .map(|threshold| threshold.max(PROFILE_FLOOR))
    }
}

const REQUEST_CHARGE: usize = std::mem::size_of::<PendingRequest>()
    + std::mem::size_of::<ResolverOutcome>()
    + std::mem::size_of::<usize>();
const WAITER_CHARGE: usize =
    std::mem::size_of::<Waiter>() + std::mem::size_of::<TicketCell>() + ARC_CONTROL_BYTES;
const ACTIVE_BUFFER_CHARGE: usize = std::mem::size_of::<ResolvedAddresses>();
const CACHE_ENTRY_CHARGE: usize = std::mem::size_of::<CacheEntry>();

fn mutex_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn reserve_pending(ledger: &AtomicUsize, bytes: usize) -> bool {
    ledger
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(bytes)
                .filter(|next| *next <= MAX_PENDING_BYTES)
        })
        .is_ok()
}

fn release_pending(ledger: &AtomicUsize, bytes: usize) {
    let previous = ledger.fetch_sub(bytes, Ordering::AcqRel);
    debug_assert!(previous >= bytes);
}

fn next_request_id(state: &ResolverState) -> u128 {
    state
        .requests
        .iter()
        .flatten()
        .map(|request| request.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn find_coalescible_request(state: &ResolverState, key: ResolveKey) -> Option<usize> {
    if !state.epoch_exhausted {
        if let Some(index) = state.active_index {
            if state
                .requests
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|request| {
                    request.key == key && request.epoch == Some(state.cache_epoch)
                })
            {
                return Some(index);
            }
        }
    }
    for offset in 0..state.queue_len {
        let index = state.queue[(state.queue_head + offset) % MAX_PENDING_KEYS];
        if state
            .requests
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|request| request.key == key && request.epoch.is_none())
        {
            return Some(index);
        }
    }
    None
}

fn push_queue(state: &mut ResolverState, request_index: usize) {
    debug_assert!(state.queue_len < MAX_PENDING_KEYS);
    let tail = (state.queue_head + state.queue_len) % MAX_PENDING_KEYS;
    state.queue[tail] = request_index;
    state.queue_len += 1;
}

fn pop_queue(state: &mut ResolverState) -> Option<usize> {
    if state.queue_len == 0 {
        return None;
    }
    let index = state.queue[state.queue_head];
    state.queue_head = (state.queue_head + 1) % MAX_PENDING_KEYS;
    state.queue_len -= 1;
    if state.queue_len == 0 {
        state.queue_head = 0;
    }
    Some(index)
}

fn sweep_waiterless_queued(inner: &ResolverInner, state: &mut ResolverState) {
    let old_len = state.queue_len;
    let mut kept = [0; MAX_PENDING_KEYS];
    let mut kept_len = 0;
    for offset in 0..old_len {
        let index = state.queue[(state.queue_head + offset) % MAX_PENDING_KEYS];
        let waiter_count = state.requests[index]
            .as_ref()
            .map_or(0, |request| request.waiter_count);
        if waiter_count == 0 {
            if state.requests[index].take().is_some() {
                release_pending(&inner.pending_bytes, REQUEST_CHARGE);
            }
        } else {
            kept[kept_len] = index;
            kept_len += 1;
        }
    }
    state.queue = kept;
    state.queue_head = 0;
    state.queue_len = kept_len;
    if state.active.is_none() && state.queue_len == 0 && state.buffer_reserved {
        state.buffer_reserved = false;
        release_pending(&inner.pending_bytes, ACTIVE_BUFFER_CHARGE);
    }
}

fn detach_waiter_locked(
    state: &mut ResolverState,
    waiter_index: usize,
    terminal: TicketTerminal,
    wakes: &mut WakeBatch,
) -> bool {
    let Some(waiter) = state.waiters[waiter_index].take() else {
        return false;
    };
    {
        let mut ticket = mutex_lock(&waiter.cell.state);
        if matches!(ticket.terminal, TicketTerminal::Pending) {
            ticket.terminal = terminal;
        }
    }
    if let Some(request) = state.requests[waiter.request_index].as_mut() {
        if request.id == waiter.request_id {
            request.waiter_count = request.waiter_count.saturating_sub(1);
        }
    }
    wakes.push(waiter.wake);
    true
}

fn cancel_ticket(inner: &ResolverInner, cell: &Arc<TicketCell>) -> bool {
    let mut wakes = WakeBatch::new();
    let cancelled = {
        let mut state = mutex_lock(&inner.state);
        let Some(index) = state.waiters.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|waiter| Arc::ptr_eq(&waiter.cell, cell))
        }) else {
            return false;
        };
        let changed =
            detach_waiter_locked(&mut state, index, TicketTerminal::Cancelled, &mut wakes);
        sweep_waiterless_queued(inner, &mut state);
        changed
    };
    wake_all(wakes);
    cancelled
}

fn maintain_inner(inner: &ResolverInner) -> ResolverHealth {
    let now = inner.clock.now();
    let mut wakes = WakeBatch::new();
    let health = {
        let mut state = mutex_lock(&inner.state);
        expire_waiters_locked(inner, &mut state, now, &mut wakes);
        check_stall_locked(inner, &mut state, now, &mut wakes);
        health_snapshot(&state)
    };
    wake_all(wakes);
    health
}

fn expire_waiters_locked(
    inner: &ResolverInner,
    state: &mut ResolverState,
    now: Instant,
    wakes: &mut WakeBatch,
) {
    for index in 0..state.waiters.len() {
        if state.waiters[index]
            .as_ref()
            .is_some_and(|waiter| now >= waiter.deadline)
        {
            detach_waiter_locked(state, index, TicketTerminal::TimedOut, wakes);
        }
    }
    sweep_waiterless_queued(inner, state);
}

fn check_stall_locked(
    inner: &ResolverInner,
    state: &mut ResolverState,
    now: Instant,
    wakes: &mut WakeBatch,
) {
    if state.unavailable.is_some() {
        return;
    }
    let stalled = state
        .active
        .is_some_and(|active| now.saturating_duration_since(active.started_at) >= active.threshold);
    if stalled {
        latch_unavailable_locked(
            inner,
            state,
            ResolverUnavailableCause::ProgressLost,
            now,
            wakes,
        );
    }
}

fn latch_unavailable_locked(
    inner: &ResolverInner,
    state: &mut ResolverState,
    cause: ResolverUnavailableCause,
    now: Instant,
    wakes: &mut WakeBatch,
) {
    if state.unavailable.is_some() {
        return;
    }
    let unavailable = ResolverUnavailable {
        cause,
        detected_at: now,
    };
    state.unavailable = Some(unavailable);
    for index in 0..state.waiters.len() {
        if state.waiters[index].is_some() {
            detach_waiter_locked(
                state,
                index,
                TicketTerminal::Unavailable(unavailable),
                wakes,
            );
        }
    }
    for offset in 0..state.queue_len {
        let index = state.queue[(state.queue_head + offset) % MAX_PENDING_KEYS];
        if state.requests[index].take().is_some() {
            release_pending(&inner.pending_bytes, REQUEST_CHARGE);
        }
    }
    state.queue = [0; MAX_PENDING_KEYS];
    state.queue_head = 0;
    state.queue_len = 0;
    for request in state.requests.iter_mut().flatten() {
        request.waiter_count = 0;
    }
    if state.active.is_none() && state.buffer_reserved {
        state.buffer_reserved = false;
        release_pending(&inner.pending_bytes, ACTIVE_BUFFER_CHARGE);
    }
}

fn health_snapshot(state: &ResolverState) -> ResolverHealth {
    ResolverHealth {
        unavailable: state.unavailable,
        policy: state.policy,
        cache_epoch: state.cache_epoch,
    }
}

fn resolver_worker(inner: Arc<ResolverInner>) {
    loop {
        let Some(work) = take_next_work(&inner) else {
            return;
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            inner.backend.lookup(work.key.host(), work.key.port)
        }));
        match result {
            Ok(result) => finish_work(&inner, work, result),
            Err(_) => {
                worker_panicked(&inner, Some(work.request_id));
                return;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Work {
    request_index: usize,
    request_id: u128,
    key: ResolveKey,
}

fn take_next_work(inner: &ResolverInner) -> Option<Work> {
    let mut state = mutex_lock(&inner.state);
    loop {
        if state.unavailable.is_some() {
            return None;
        }
        if let Some(index) = pop_queue(&mut state) {
            let Some(request) = state.requests[index] else {
                continue;
            };
            if request.waiter_count == 0 {
                state.requests[index] = None;
                release_pending(&inner.pending_bytes, REQUEST_CHARGE);
                if state.queue_len == 0 && state.active.is_none() && state.buffer_reserved {
                    state.buffer_reserved = false;
                    release_pending(&inner.pending_bytes, ACTIVE_BUFFER_CHARGE);
                }
                continue;
            }
            let started_at = inner.clock.now();
            let threshold = policy_threshold(state.policy);
            let epoch = state.cache_epoch;
            state.requests[index]
                .as_mut()
                .expect("request exists")
                .epoch = Some(epoch);
            state.active = Some(ActiveCall {
                request_id: request.id,
                started_at,
                threshold,
                epoch,
            });
            state.active_index = Some(index);
            return Some(Work {
                request_index: index,
                request_id: request.id,
                key: request.key,
            });
        }
        if state.shutdown {
            return None;
        }
        state = inner
            .changed
            .wait(state)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

fn policy_threshold(policy: ResolverHealthPolicy) -> Duration {
    match policy {
        ResolverHealthPolicy::Unqualified => UNPROFILED_STALL_LIMIT,
        ResolverHealthPolicy::Qualified { threshold } => threshold,
    }
}

fn finish_work(
    inner: &ResolverInner,
    work: Work,
    result: Result<ResolvedAddresses, ResolverLookupError>,
) {
    let now = inner.clock.now();
    let outcome = match result {
        Ok(addresses) if !addresses.is_empty() => ResolverOutcome::Resolved(addresses),
        Ok(_) => ResolverOutcome::Failed(ResolverLookupError::NoAddresses),
        Err(error) => ResolverOutcome::Failed(error),
    };
    let mut wakes = WakeBatch::new();
    {
        let mut state = mutex_lock(&inner.state);
        if state
            .active
            .is_some_and(|active| active.request_id == work.request_id)
        {
            let active = state.active.take().expect("active call exists");
            state.active_index = None;
            if state.unavailable.is_none()
                && !state.epoch_exhausted
                && active.epoch == state.cache_epoch
            {
                cache_insert(&mut state, work.key, active.epoch, outcome, now);
            }
            for index in 0..state.waiters.len() {
                let matches_request = state.waiters[index].as_ref().is_some_and(|waiter| {
                    waiter.request_index == work.request_index
                        && waiter.request_id == work.request_id
                });
                if matches_request {
                    if let Some(waiter) = state.waiters[index].take() {
                        let mut cell = mutex_lock(&waiter.cell.state);
                        if matches!(cell.terminal, TicketTerminal::Pending) {
                            cell.terminal = TicketTerminal::Complete(outcome.into_result());
                            cell.backend_timing = Some((active.started_at, now));
                            wakes.push(waiter.wake);
                        }
                    }
                }
            }
            if state.requests[work.request_index]
                .is_some_and(|request| request.id == work.request_id)
            {
                state.requests[work.request_index] = None;
                release_pending(&inner.pending_bytes, REQUEST_CHARGE);
            }
            if state.queue_len == 0 && state.buffer_reserved {
                state.buffer_reserved = false;
                release_pending(&inner.pending_bytes, ACTIVE_BUFFER_CHARGE);
            }
        }
    }
    inner.changed.notify_all();
    wake_all(wakes);
}

fn worker_panicked(inner: &ResolverInner, active_request_id: Option<u128>) {
    let now = inner.clock.now();
    let mut wakes = WakeBatch::new();
    {
        let mut state = mutex_lock(&inner.state);
        latch_unavailable_locked(
            inner,
            &mut state,
            ResolverUnavailableCause::Panicked,
            now,
            &mut wakes,
        );
        if let Some(active) = state.active {
            if active_request_id.is_none_or(|id| id == active.request_id) {
                if let Some(index) = state.active_index.take() {
                    if state.requests[index].take().is_some() {
                        release_pending(&inner.pending_bytes, REQUEST_CHARGE);
                    }
                }
                state.active = None;
                if state.buffer_reserved {
                    state.buffer_reserved = false;
                    release_pending(&inner.pending_bytes, ACTIVE_BUFFER_CHARGE);
                }
            }
        }
    }
    inner.changed.notify_all();
    wake_all(wakes);
}

fn wake_all(mut wakes: WakeBatch) {
    for index in 0..wakes.len {
        if let Some(wake) = wakes.values[index].take() {
            wake.wake();
        }
    }
}

fn prune_expired_cache(state: &mut ResolverState, now: Instant) {
    for entry in &mut state.cache {
        if entry.is_some_and(|cached| now >= cached.expires_at) {
            *entry = None;
            state.cache_bytes = state.cache_bytes.saturating_sub(CACHE_ENTRY_CHARGE);
        }
    }
}

fn cache_lookup(
    state: &mut ResolverState,
    key: ResolveKey,
    now: Instant,
) -> Option<ResolverOutcome> {
    let epoch = state.cache_epoch;
    let index = state
        .cache
        .iter()
        .position(|entry| entry.is_some_and(|cached| cached.key == key && cached.epoch == epoch))?;
    let cached = state.cache[index].as_mut().expect("cache entry found");
    cached.last_used_at = now;
    Some(cached.outcome)
}

fn cache_insert(
    state: &mut ResolverState,
    key: ResolveKey,
    epoch: u64,
    outcome: ResolverOutcome,
    now: Instant,
) {
    prune_expired_cache(state, now);
    let Some(expires_at) = now.checked_add(CACHE_TTL) else {
        return;
    };
    for entry in &mut state.cache {
        if entry.is_some_and(|cached| cached.key == key && cached.epoch == epoch) {
            *entry = None;
            state.cache_bytes = state.cache_bytes.saturating_sub(CACHE_ENTRY_CHARGE);
        }
    }
    if CACHE_ENTRY_CHARGE > MAX_CACHE_BYTES {
        return;
    }
    while state.cache.iter().filter(|entry| entry.is_some()).count() >= MAX_CACHE_ENTRIES
        || state.cache_bytes.saturating_add(CACHE_ENTRY_CHARGE) > MAX_CACHE_BYTES
    {
        let Some(index) = state
            .cache
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.map(|cached| (index, cached)))
            .min_by_key(|(_, cached)| (cached.last_used_at, cached.insertion_order))
            .map(|(index, _)| index)
        else {
            return;
        };
        state.cache[index] = None;
        state.cache_bytes = state.cache_bytes.saturating_sub(CACHE_ENTRY_CHARGE);
    }
    let Some(index) = state.cache.iter().position(Option::is_none) else {
        return;
    };
    state.cache_order = state.cache_order.saturating_add(1);
    let insertion_order = state.cache_order;
    state.cache[index] = Some(CacheEntry {
        key,
        epoch,
        expires_at,
        last_used_at: now,
        insertion_order,
        outcome,
    });
    state.cache_bytes += CACHE_ENTRY_CHARGE;
}

fn clear_cache(state: &mut ResolverState) {
    state.cache = [None; MAX_CACHE_ENTRIES];
    state.cache_bytes = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::mpsc::{self, Receiver, Sender};

    #[derive(Clone)]
    struct FakeClock(Arc<Mutex<Instant>>);

    impl FakeClock {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Instant::now())))
        }

        fn advance(&self, amount: Duration) {
            let mut now = mutex_lock(&self.0);
            *now = now
                .checked_add(amount)
                .expect("fake time remains representable");
        }

        fn set_elapsed(&self, base: Instant, amount: Duration) {
            *mutex_lock(&self.0) = base
                .checked_add(amount)
                .expect("fake time remains representable");
        }
    }

    impl MonotonicClock for FakeClock {
        fn now(&self) -> Instant {
            *mutex_lock(&self.0)
        }
    }

    struct ImmediateBackend {
        calls: AtomicUsize,
        addresses: ResolvedAddresses,
    }

    impl ImmediateBackend {
        fn new() -> Self {
            let mut addresses = ResolvedAddresses::new();
            addresses.push(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                43594,
            )));
            Self {
                calls: AtomicUsize::new(0),
                addresses,
            }
        }
    }

    impl ResolverBackend for ImmediateBackend {
        fn lookup(
            &self,
            _host: &str,
            _port: u16,
        ) -> Result<ResolvedAddresses, ResolverLookupError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(self.addresses)
        }
    }

    struct GateBackend {
        entered: Sender<BackendCall>,
    }

    struct BackendCall {
        host: String,
        port: u16,
        result: Sender<Result<ResolvedAddresses, ResolverLookupError>>,
    }

    impl ResolverBackend for GateBackend {
        fn lookup(&self, host: &str, port: u16) -> Result<ResolvedAddresses, ResolverLookupError> {
            let (send, receive) = mpsc::channel();
            self.entered
                .send(BackendCall {
                    host: host.to_owned(),
                    port,
                    result: send,
                })
                .expect("test owns backend receiver");
            receive.recv().expect("test releases each blocking lookup")
        }
    }

    struct ChannelWake(Sender<()>);

    impl ResolverWake for ChannelWake {
        fn wake(&self) {
            let _ = self.0.send(());
        }
    }

    struct TestContext {
        resolver: Arc<Resolver>,
        clock: FakeClock,
        entered: Receiver<BackendCall>,
    }

    fn gated(max_slots: usize, configuration: ResolverConfiguration) -> TestContext {
        let (entered_tx, entered_rx) = mpsc::channel();
        let clock = FakeClock::new();
        let resolver = Resolver::injected(
            max_slots,
            Arc::new(GateBackend {
                entered: entered_tx,
            }),
            Arc::new(clock.clone()),
            configuration,
        )
        .expect("valid test slot cap");
        TestContext {
            resolver,
            clock,
            entered: entered_rx,
        }
    }

    fn immediate(max_slots: usize) -> (Arc<Resolver>, FakeClock, Arc<ImmediateBackend>) {
        let clock = FakeClock::new();
        let backend = Arc::new(ImmediateBackend::new());
        let resolver = Resolver::injected(
            max_slots,
            backend.clone(),
            Arc::new(clock.clone()),
            ResolverConfiguration::unprofiled(),
        )
        .expect("valid test slot cap");
        (resolver, clock, backend)
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "resolver health must never take its state lock under K2")]
    fn resolver_health_rejects_a_nested_park_predicate() {
        let (resolver, _, _) = immediate(1);
        let park = super::super::park::Park::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        park.wait(
            deadline,
            Instant::now,
            || resolver.health().unavailable.is_some(),
            || false,
            || false,
            || Some(()),
            false,
            None,
        );
    }

    fn ticket(
        resolver: &Resolver,
        slot: u64,
        generation: u64,
        host: &str,
        wake: &Sender<()>,
    ) -> Result<ResolveTicket, ResolverAdmissionError> {
        resolver.resolve(
            ResolveIdentity {
                slot: SlotKey(slot),
                generation,
            },
            host,
            43_594,
            Arc::new(ChannelWake(wake.clone())),
        )
    }

    fn wake_channel() -> (Sender<()>, Receiver<()>) {
        mpsc::channel()
    }

    fn unlocked_wake(resolver: &Resolver, send: Sender<bool>) -> Arc<dyn ResolverWake> {
        let inner = Arc::downgrade(&resolver.inner);
        Arc::new(move || {
            let unlocked = inner
                .upgrade()
                .is_some_and(|inner| inner.state.try_lock().is_ok());
            let _ = send.send(unlocked);
        })
    }

    fn wait_ready(
        ticket: &ResolveTicket,
        wakes: &Receiver<()>,
    ) -> Result<ResolvedAddresses, ResolverFailure> {
        loop {
            match ticket.poll() {
                ResolvePoll::Ready(result) => return result,
                ResolvePoll::Pending => {
                    wakes
                        .recv_timeout(Duration::from_secs(5))
                        .expect("resolver publishes a terminal wake");
                }
            }
        }
    }

    fn assert_unqualified_available(context: &TestContext, host: &str) {
        assert_eq!(context.resolver.health().unavailable, None);
        assert_eq!(
            context.resolver.health().policy,
            ResolverHealthPolicy::Unqualified
        );
        let (wake, wakes) = wake_channel();
        let ticket = ticket(&context.resolver, 1, 1, host, &wake).unwrap();
        let call = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        call.result.send(Ok(address_list())).unwrap();
        assert!(wait_ready(&ticket, &wakes).is_ok());
        wait_until_idle(&context.resolver);
    }

    fn address_list() -> ResolvedAddresses {
        let mut addresses = ResolvedAddresses::new();
        addresses.push(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::LOCALHOST,
            43_594,
        )));
        addresses
    }

    fn profile(digest: [u8; 32], seconds: f64) -> ResolverProfileReceipt {
        ResolverProfileReceipt::new(
            digest,
            "nss",
            "glibc-2.x",
            seconds,
            true,
            true,
            true,
            [7; 32],
        )
    }

    fn qualified(digest: [u8; 32], seconds: f64) -> ResolverConfiguration {
        ResolverConfiguration::new(Some(digest), Some(profile(digest, seconds)))
    }

    fn wait_until_idle(resolver: &Resolver) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = mutex_lock(&resolver.inner.state);
            if state.active.is_none() && state.queue_len == 0 {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "resolver worker did not become idle");
            let (state, _) = resolver
                .inner
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(state);
        }
    }

    #[test]
    fn g4_a_bounds_keys_waiters_bytes_and_host_and_cache_expiry_lru() {
        let context = gated(64, ResolverConfiguration::unprofiled());
        let mut tickets = Vec::new();
        let (first_tx, _first_rx) = wake_channel();
        tickets.push(ticket(&context.resolver, 0, 1, "host-0", &first_tx).unwrap());
        let active = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!((active.host.as_str(), active.port), ("host-0", 43_594));
        for slot in 1..MAX_PENDING_KEYS {
            let (tx, _rx) = wake_channel();
            tickets.push(
                ticket(
                    &context.resolver,
                    slot as u64,
                    1,
                    &format!("host-{slot}"),
                    &tx,
                )
                .unwrap(),
            );
        }
        assert_eq!(pending_request_count(&context.resolver), MAX_PENDING_KEYS);
        assert!(context.resolver.pending_bytes() <= MAX_PENDING_BYTES);
        let (extra_tx, _extra_rx) = wake_channel();
        assert_eq!(
            ticket(&context.resolver, 64, 1, "host-extra", &extra_tx).unwrap_err(),
            ResolverAdmissionError::Busy
        );
        let before_overlong = context.resolver.pending_bytes();
        assert_eq!(
            ticket(
                &context.resolver,
                64,
                1,
                &"x".repeat(MAX_HOST_BYTES + 1),
                &extra_tx
            )
            .unwrap_err(),
            ResolverAdmissionError::InvalidHost
        );
        assert_eq!(context.resolver.pending_bytes(), before_overlong);

        // Exact duplicate shares its one waiter; a generation change does not.
        let duplicate = ticket(&context.resolver, 0, 1, "host-0", &first_tx).unwrap();
        assert_eq!(tickets[0].identity(), duplicate.identity());
        assert_eq!(context.resolver.pending_bytes(), before_overlong);
        assert_eq!(
            ticket(&context.resolver, 0, 2, "host-next", &extra_tx).unwrap_err(),
            ResolverAdmissionError::SlotAlreadyWaiting
        );

        for ticket in tickets.iter().skip(1) {
            assert!(ticket.cancel());
        }
        assert!(tickets[0].cancel());
        active.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        drop(duplicate);
        drop(tickets);
        assert_eq!(context.resolver.pending_bytes(), 0);

        // The result cache expires at equality, and same-time LRU ties evict
        // the oldest insertion first.
        let (resolver, clock, backend) = immediate(MAX_CACHE_ENTRIES + 2);
        let (tx, rx) = wake_channel();
        let one = ticket(&resolver, 0, 1, "cache-one", &tx).unwrap();
        assert!(wait_ready(&one, &rx).is_ok());
        assert_eq!(one.backend_timing(), Some((clock.now(), clock.now())));
        drop(one);
        assert_eq!(backend.calls.load(AtomicOrdering::SeqCst), 1);
        clock.advance(CACHE_TTL - Duration::from_nanos(1));
        let hit = ticket(&resolver, 1, 1, "cache-one", &tx).unwrap();
        assert!(matches!(hit.poll(), ResolvePoll::Ready(Ok(_))));
        assert_eq!(hit.backend_timing(), None, "cache hit invoked no backend");
        drop(hit);
        assert_eq!(backend.calls.load(AtomicOrdering::SeqCst), 1);
        clock.advance(Duration::from_nanos(1));
        let expired = ticket(&resolver, 2, 1, "cache-one", &tx).unwrap();
        assert!(wait_ready(&expired, &rx).is_ok());
        assert_eq!(expired.backend_timing(), Some((clock.now(), clock.now())));
        drop(expired);
        assert_eq!(backend.calls.load(AtomicOrdering::SeqCst), 2);

        for index in 0..=MAX_CACHE_ENTRIES {
            let host = format!("lru-{index}");
            let current = ticket(&resolver, index as u64, 1, &host, &tx).unwrap();
            assert!(wait_ready(&current, &rx).is_ok());
            drop(current);
        }
        let state = mutex_lock(&resolver.inner.state);
        assert_eq!(state.cache_bytes, MAX_CACHE_ENTRIES * CACHE_ENTRY_CHARGE);
        assert!(state.cache_bytes <= MAX_CACHE_BYTES);
        assert!(!state
            .cache
            .iter()
            .flatten()
            .any(|entry| entry.key.host() == "lru-0"));
        assert!(state
            .cache
            .iter()
            .flatten()
            .any(|entry| entry.key.host() == "lru-128"));
    }

    #[test]
    fn g4_a_pending_byte_cap_refuses_before_admission() {
        let context = gated(MAX_CONFIGURED_SLOTS, ResolverConfiguration::unprofiled());
        let (tx, _rx) = wake_channel();
        let first = ticket(&context.resolver, 0, 1, "shared-name", &tx).unwrap();
        let active = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let mut tickets = vec![first];
        let mut next_slot = 1_u64;
        loop {
            match ticket(&context.resolver, next_slot, 1, "shared-name", &tx) {
                Ok(ticket) => tickets.push(ticket),
                Err(ResolverAdmissionError::Busy) => break,
                other => panic!("expected pending-byte refusal, got {other:?}"),
            }
            next_slot += 1;
        }
        assert!(context.resolver.pending_bytes() <= MAX_PENDING_BYTES);
        assert!(context.resolver.pending_bytes() + WAITER_CHARGE > MAX_PENDING_BYTES);
        for ticket in &tickets {
            assert!(ticket.cancel());
        }
        active.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        drop(tickets);
        assert_eq!(context.resolver.pending_bytes(), 0);
    }

    #[test]
    fn g4_b_coalesced_completion_obeys_cancellation_order() {
        let context = gated(8, ResolverConfiguration::unprofiled());
        let (wake_a, unlocked_a) = mpsc::channel();
        let a = context
            .resolver
            .resolve(
                ResolveIdentity {
                    slot: SlotKey(1),
                    generation: 10,
                },
                "coalesced",
                43_594,
                unlocked_wake(&context.resolver, wake_a),
            )
            .unwrap();
        let call = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let (wake_b, unlocked_b) = mpsc::channel();
        let b = context
            .resolver
            .resolve(
                ResolveIdentity {
                    slot: SlotKey(2),
                    generation: 20,
                },
                "coalesced",
                43_594,
                unlocked_wake(&context.resolver, wake_b),
            )
            .unwrap();
        assert_eq!(pending_request_count(&context.resolver), 1);
        assert!(a.cancel());
        assert!(unlocked_a.recv_timeout(Duration::from_secs(5)).unwrap());
        call.result.send(Ok(address_list())).unwrap();
        assert!(unlocked_b.recv_timeout(Duration::from_secs(5)).unwrap());
        assert_eq!(
            a.poll(),
            ResolvePoll::Ready(Err(ResolverFailure::Cancelled))
        );
        assert!(matches!(b.poll(), ResolvePoll::Ready(Ok(_))));
        assert!(!b.cancel());
        drop((a, b));
        assert_eq!(context.resolver.pending_bytes(), 0);

        // Completion first wins over a later cancellation attempt.
        let context = gated(4, ResolverConfiguration::unprofiled());
        let (wake, wake_rx) = wake_channel();
        let done = ticket(&context.resolver, 3, 30, "completion-first", &wake).unwrap();
        context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .result
            .send(Ok(address_list()))
            .unwrap();
        wake_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(done.poll(), ResolvePoll::Ready(Ok(_))));
        assert!(!done.cancel());
    }

    #[test]
    fn g4_c_timeout_releases_slot_and_queued_waiterless_work() {
        let context = gated(8, ResolverConfiguration::unprofiled());
        let (wake_a, rx_a) = wake_channel();
        let a = ticket(&context.resolver, 7, 1, "slow-a", &wake_a).unwrap();
        let active = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        context.clock.advance(TICKET_WAIT);
        context.resolver.maintain();
        rx_a.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(a.poll(), ResolvePoll::Ready(Err(ResolverFailure::TimedOut)));
        drop(a);
        context.clock.advance(Duration::from_secs(1));
        assert_eq!(
            context.resolver.pending_bytes(),
            REQUEST_CHARGE + ACTIVE_BUFFER_CHARGE
        );

        // Reusing the same slot with a new generation does not renew the old
        // active lookup. A second queued key disappears as soon as its waiter
        // is cancelled; cycling more generations cannot accumulate queue keys.
        let (wake_b, _rx_b) = wake_channel();
        let b = ticket(&context.resolver, 7, 2, "queued-b", &wake_b).unwrap();
        assert!(b.cancel());
        drop(b);
        assert_eq!(pending_request_count(&context.resolver), 1);
        for generation in 3..=80 {
            let (wake, _rx) = wake_channel();
            let queued = ticket(
                &context.resolver,
                7,
                generation,
                &format!("queued-{generation}"),
                &wake,
            )
            .unwrap();
            assert!(queued.cancel());
        }
        assert_eq!(pending_request_count(&context.resolver), 1);
        active.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        assert_eq!(context.resolver.pending_bytes(), 0);
    }

    #[test]
    fn g4_d_qualified_30_second_profile_uses_120_second_floor_and_wakes_waiters() {
        let digest = [3; 32];
        let context = gated(8, qualified(digest, 30.0));
        let (wake_a, rx_a) = wake_channel();
        let a = ticket(&context.resolver, 1, 1, "blocked-a", &wake_a).unwrap();
        let active = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let start = context.clock.now();
        context.clock.set_elapsed(start, Duration::from_secs(30));
        assert_eq!(context.resolver.maintain().unavailable, None);
        context.clock.set_elapsed(start, Duration::from_secs(115));
        let (wake_b, rx_b) = wake_channel();
        let b = ticket(&context.resolver, 2, 2, "blocked-b", &wake_b).unwrap();
        context.clock.set_elapsed(start, Duration::from_secs(120));
        let health = context.resolver.maintain();
        assert_eq!(
            health.unavailable.map(|value| value.cause),
            Some(ResolverUnavailableCause::ProgressLost)
        );
        rx_b.recv_timeout(Duration::from_secs(5)).unwrap();
        rx_a.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            b.poll(),
            ResolvePoll::Ready(Err(ResolverFailure::Unavailable(_)))
        ));
        assert_eq!(a.poll(), ResolvePoll::Ready(Err(ResolverFailure::TimedOut)));
        assert!(context.resolver.pending_bytes() >= REQUEST_CHARGE + ACTIVE_BUFFER_CHARGE);
        active.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        drop((a, b));
        assert_eq!(context.resolver.pending_bytes(), 0);
    }

    #[test]
    fn g4_e_profiles_are_checked_and_missing_or_invalid_profiles_stay_available() {
        let digest = [4; 32];
        let context = gated(4, qualified(digest, 60.0));
        let (wake, _rx) = wake_channel();
        let ticket = ticket(&context.resolver, 1, 1, "qualified", &wake).unwrap();
        let call = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let start = context.clock.now();
        context.clock.set_elapsed(start, Duration::from_secs(239));
        assert_eq!(
            context.resolver.maintain().policy,
            ResolverHealthPolicy::Qualified {
                threshold: Duration::from_secs(240)
            }
        );
        assert_eq!(context.resolver.health().unavailable, None);
        context.clock.set_elapsed(start, Duration::from_secs(240));
        assert_eq!(
            context
                .resolver
                .maintain()
                .unavailable
                .map(|value| value.cause),
            Some(ResolverUnavailableCause::ProgressLost)
        );
        call.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        drop(ticket);

        let invalid = [0.0, f64::NAN, f64::INFINITY, (u64::MAX as f64) * 2.0];
        for value in invalid {
            let receipt = profile(digest, value);
            let context = gated(2, ResolverConfiguration::new(Some(digest), Some(receipt)));
            assert_unqualified_available(&context, "unqualified");
        }
        let mismatched = gated(
            2,
            ResolverConfiguration::new(Some([1; 32]), Some(profile([2; 32], 60.0))),
        );
        assert_unqualified_available(&mismatched, "mismatched");
        let missing = gated(2, ResolverConfiguration::unprofiled());
        assert_unqualified_available(&missing, "missing");
        let incomplete =
            ResolverProfileReceipt::new(digest, "", "glibc", 60.0, true, false, true, [0; 32]);
        let incomplete = gated(
            2,
            ResolverConfiguration::new(Some(digest), Some(incomplete)),
        );
        assert_unqualified_available(&incomplete, "incomplete");
    }

    #[test]
    fn g4_f_outage_refuses_cache_and_new_work_without_releasing_active_custody() {
        let digest = [5; 32];
        let context = gated(4, qualified(digest, 30.0));
        let (wake, rx) = wake_channel();
        let cached = ticket(&context.resolver, 0, 1, "cached-key", &wake).unwrap();
        let cached_call = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        cached_call.result.send(Ok(address_list())).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(cached.poll(), ResolvePoll::Ready(Ok(_))));
        wait_until_idle(&context.resolver);
        drop(cached);

        let old = ticket(&context.resolver, 1, 1, "old-call", &wake).unwrap();
        let active = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let start = context.clock.now();
        context.clock.set_elapsed(start, Duration::from_secs(120));
        context.resolver.maintain();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            old.poll(),
            ResolvePoll::Ready(Err(ResolverFailure::TimedOut))
        );
        let before = context.resolver.pending_bytes();
        assert!(matches!(
            ticket(&context.resolver, 2, 1, "cached-key", &wake),
            Err(ResolverAdmissionError::Unavailable(_))
        ));
        assert!(matches!(
            ticket(&context.resolver, 3, 1, "new-call", &wake),
            Err(ResolverAdmissionError::Unavailable(_))
        ));
        assert_eq!(context.resolver.pending_bytes(), before);
        active.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        assert_eq!(context.resolver.pending_bytes(), WAITER_CHARGE);
        assert!(matches!(
            ticket(&context.resolver, 0, 1, "cached-key", &wake),
            Err(ResolverAdmissionError::Unavailable(_))
        ));
        drop(old);
        assert_eq!(context.resolver.pending_bytes(), 0);
    }

    #[test]
    fn g4_g_unprofiled_service_uses_exact_900_second_default() {
        let context = gated(4, ResolverConfiguration::unprofiled());
        let (wake_a, rx_a) = wake_channel();
        let a = ticket(&context.resolver, 1, 1, "default-a", &wake_a).unwrap();
        let active = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let start = context.clock.now();
        for elapsed in [
            Duration::from_secs(30),
            Duration::from_secs(120),
            Duration::from_millis(899_999),
        ] {
            context.clock.set_elapsed(start, elapsed);
            assert_eq!(context.resolver.maintain().unavailable, None);
            assert_eq!(
                context.resolver.health().policy,
                ResolverHealthPolicy::Unqualified
            );
        }
        context.clock.set_elapsed(start, Duration::from_secs(895));
        let (wake_b, rx_b) = wake_channel();
        let b = ticket(&context.resolver, 2, 1, "default-b", &wake_b).unwrap();
        assert_eq!(b.deadline(), start + Duration::from_secs(905));
        context.clock.set_elapsed(start, UNPROFILED_STALL_LIMIT);
        assert_eq!(
            context
                .resolver
                .maintain()
                .unavailable
                .map(|value| value.cause),
            Some(ResolverUnavailableCause::ProgressLost)
        );
        rx_b.recv_timeout(Duration::from_secs(5)).unwrap();
        rx_a.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            b.poll(),
            ResolvePoll::Ready(Err(ResolverFailure::Unavailable(_)))
        ));
        assert!(context.resolver.pending_bytes() >= REQUEST_CHARGE + ACTIVE_BUFFER_CHARGE);
        active.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&context.resolver);
        drop((a, b));
        assert_eq!(context.resolver.pending_bytes(), 0);
    }

    #[test]
    fn g4_h_configuration_epoch_separates_active_calls_and_never_renews_start() {
        let digest = [8; 32];
        let context = gated(8, qualified(digest, 60.0));
        let (wake_old, rx_old) = wake_channel();
        let old = ticket(&context.resolver, 1, 1, "same-name", &wake_old).unwrap();
        let first_call = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let start = context.clock.now();
        context.clock.set_elapsed(start, Duration::from_secs(95));
        let (wake_join, rx_join) = wake_channel();
        let joined = ticket(&context.resolver, 2, 1, "same-name", &wake_join).unwrap();
        assert_eq!(pending_request_count(&context.resolver), 1);

        context.clock.set_elapsed(start, Duration::from_secs(100));
        let refreshed = context
            .resolver
            .refresh_configuration(ResolverConfiguration::new(Some([9; 32]), None));
        assert_eq!(refreshed.policy, ResolverHealthPolicy::Unqualified);
        assert_eq!(refreshed.cache_epoch, 1);
        let (wake_new, rx_new) = wake_channel();
        let new_epoch = ticket(&context.resolver, 3, 2, "same-name", &wake_new).unwrap();
        assert_eq!(pending_request_count(&context.resolver), 2);
        context.clock.set_elapsed(start, Duration::from_secs(102));
        first_call.result.send(Ok(address_list())).unwrap();
        rx_old.recv_timeout(Duration::from_secs(5)).unwrap();
        rx_join.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            old.poll(),
            ResolvePoll::Ready(Err(ResolverFailure::TimedOut))
        );
        assert!(matches!(joined.poll(), ResolvePoll::Ready(Ok(_))));

        // The old epoch result is absent from the refreshed cache, but its
        // live joined ticket receives it; the queued new-epoch lookup is
        // the next physical backend call.
        let second_call = context
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(second_call.host, "same-name");
        assert_eq!(second_call.port, 43_594);
        assert!(new_epoch.poll() == ResolvePoll::Pending);
        second_call.result.send(Ok(address_list())).unwrap();
        rx_new.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(new_epoch.poll(), ResolvePoll::Ready(Ok(_))));
        wait_until_idle(&context.resolver);
        drop((old, joined, new_epoch));

        let repeated = gated(4, qualified(digest, 60.0));
        let (wake, rx) = wake_channel();
        let long = ticket(&repeated.resolver, 1, 1, "long-active", &wake).unwrap();
        let blocked = repeated
            .entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let base = repeated.clock.now();
        repeated.clock.set_elapsed(base, Duration::from_secs(200));
        repeated
            .resolver
            .refresh_configuration(ResolverConfiguration::new(Some([10; 32]), None));
        repeated.clock.set_elapsed(base, Duration::from_secs(300));
        repeated
            .resolver
            .refresh_configuration(ResolverConfiguration::new(Some([11; 32]), None));
        repeated.clock.set_elapsed(base, Duration::from_secs(895));
        let (wake_late, rx_late) = wake_channel();
        let late = ticket(&repeated.resolver, 2, 1, "late-demand", &wake_late).unwrap();
        assert_eq!(late.deadline(), base + Duration::from_secs(905));
        repeated.clock.set_elapsed(base, Duration::from_secs(899));
        assert_eq!(repeated.resolver.maintain().unavailable, None);
        repeated.clock.set_elapsed(base, Duration::from_secs(900));
        let unavailable = repeated.resolver.maintain().unavailable.unwrap();
        assert_eq!(unavailable.cause, ResolverUnavailableCause::ProgressLost);
        rx_late.recv_timeout(Duration::from_secs(5)).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            late.poll(),
            ResolvePoll::Ready(Err(ResolverFailure::Unavailable(unavailable)))
        );
        assert_eq!(pending_request_count(&repeated.resolver), 1);
        blocked.result.send(Ok(address_list())).unwrap();
        wait_until_idle(&repeated.resolver);
        drop((long, late));
        assert_eq!(repeated.resolver.pending_bytes(), 0);
    }

    fn pending_request_count(resolver: &Resolver) -> usize {
        mutex_lock(&resolver.inner.state)
            .requests
            .iter()
            .filter(|request| request.is_some())
            .count()
    }
}

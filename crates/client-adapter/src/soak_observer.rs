//! Process-scoped, read-only observation of complete game WSS binary messages.
//!
//! This module is available only with the `test-support` feature. Callbacks run
//! synchronously on the WSS reactor thread and receive borrowed payload bytes;
//! they must not retain the slice after returning.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, Weak};

use crate::control::BindingId;
use crate::ring::{IN_RING_BYTES, OUT_RING_BYTES, WS_MESSAGE_LIMIT};
use crate::shared::SharedConnection;
use crate::ws_endpoint::TlsIo;
use rustls::ClientConfig;

/// Logical reservation per active WSS connection. These are not allocator
/// measurements; metadata, other WS/TLS buffers, and allocator overhead are
/// excluded. Whole-process RSS and the separate G5-E allocator proof remain
/// necessary.
pub const RECEIVED_PLAINTEXT_RESERVED_BYTES: usize = WS_MESSAGE_LIMIT;
pub const PENDING_TLS_OUTPUT_RESERVED_BYTES: usize = 128 * 1024;
pub const HANDSHAKE_DEFRAMER_RESERVED_BYTES: usize = 64 * 1024;
pub const OUTBOUND_RING_RESERVED_BYTES: usize = OUT_RING_BYTES;
pub const INBOUND_RING_RESERVED_BYTES: usize = IN_RING_BYTES;
pub const WSS_RESERVED_TOTAL_BYTES: usize = RECEIVED_PLAINTEXT_RESERVED_BYTES
    + PENDING_TLS_OUTPUT_RESERVED_BYTES
    + HANDSHAKE_DEFRAMER_RESERVED_BYTES
    + OUTBOUND_RING_RESERVED_BYTES
    + INBOUND_RING_RESERVED_BYTES;

/// One sample of the five named logical WSS compartments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WssLedgerSample {
    pub binding: BindingId,
    pub local_port: Option<u16>,
    /// Unavailable when the provider does not expose an exact pending count.
    pub received_plaintext_bytes: Option<usize>,
    pub received_plaintext_reserved_bytes: usize,
    /// Exact rustls-accounted bytes, or None when control-output accounting is
    /// represented only by its conservative reserve.
    pub pending_tls_output_bytes: Option<usize>,
    pub pending_tls_output_reserved_bytes: usize,
    /// rustls/tungstenite internal handshake deframer occupancy is not
    /// published safely; this is its logical reservation, not a measurement.
    pub handshake_deframer_reserved_bytes: usize,
    pub outbound_ring_bytes: usize,
    pub outbound_ring_reserved_bytes: usize,
    /// Exact inline type layouts, included in the `SharedConnection` allocation.
    /// These are not separate heap requests.
    pub outbound_ring_inline_layout_bytes: usize,
    pub inbound_ring_inline_layout_bytes: usize,
    pub inbound_ring_bytes: usize,
    pub inbound_ring_reserved_bytes: usize,
    pub reserved_total_bytes: usize,
    pub overrun: bool,
}

#[derive(Clone)]
struct LedgerRegistration {
    binding: BindingId,
    local_port: Option<u16>,
    tls: Weak<Mutex<TlsIo>>,
    shared: Weak<SharedConnection>,
}

static LEDGER_REGISTRY: LazyLock<Mutex<Vec<LedgerRegistration>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

pub(super) fn register_wss_ledger(
    binding: BindingId,
    local_port: Option<u16>,
    tls: &Arc<Mutex<TlsIo>>,
    shared: &Arc<SharedConnection>,
) {
    let mut registry = LEDGER_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|entry| entry.binding != binding && entry.tls.strong_count() > 0);
    registry.push(LedgerRegistration {
        binding,
        local_port,
        tls: Arc::downgrade(tls),
        shared: Arc::downgrade(shared),
    });
}

pub(super) fn rebind_wss_ledger(expected: BindingId, next: BindingId) {
    let mut registry = LEDGER_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let registration = registry
        .iter_mut()
        .find(|entry| entry.binding == expected)
        .expect("connected test-support WSS must retain its ledger registration");
    registration.binding = next;
}

pub(super) fn retire_wss_ledger(binding: BindingId) {
    LEDGER_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|entry| entry.binding != binding && entry.tls.strong_count() > 0);
}

/// Sample each currently registered WSS connection without retaining it.
/// Actual occupied bytes are distinguished from logical reservation bytes.
pub fn wss_ledger_samples() -> Vec<WssLedgerSample> {
    let entries = {
        let mut registry = LEDGER_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.retain(|entry| entry.tls.strong_count() > 0 && entry.shared.strong_count() > 0);
        registry.clone()
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let tls = entry.tls.upgrade()?;
            let shared = entry.shared.upgrade()?;
            let (pending_tls_output_bytes, connected) = {
                let tls = tls.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                tls.soak_ledger_snapshot()
            };
            if !connected {
                return None;
            }
            let outbound_ring_bytes = shared.outbound.len();
            let inbound_ring_bytes = shared.inbound.len();
            let overrun = pending_tls_output_bytes
                .is_some_and(|bytes| bytes > PENDING_TLS_OUTPUT_RESERVED_BYTES)
                || outbound_ring_bytes > OUTBOUND_RING_RESERVED_BYTES
                || inbound_ring_bytes > INBOUND_RING_RESERVED_BYTES;
            Some(WssLedgerSample {
                binding: entry.binding,
                local_port: entry.local_port,
                received_plaintext_bytes: None,
                received_plaintext_reserved_bytes: RECEIVED_PLAINTEXT_RESERVED_BYTES,
                pending_tls_output_bytes,
                pending_tls_output_reserved_bytes: PENDING_TLS_OUTPUT_RESERVED_BYTES,
                handshake_deframer_reserved_bytes: HANDSHAKE_DEFRAMER_RESERVED_BYTES,
                outbound_ring_bytes,
                outbound_ring_reserved_bytes: OUTBOUND_RING_RESERVED_BYTES,
                inbound_ring_bytes,
                inbound_ring_reserved_bytes: INBOUND_RING_RESERVED_BYTES,
                outbound_ring_inline_layout_bytes: shared.outbound.inline_layout_bytes(),
                inbound_ring_inline_layout_bytes: shared.inbound.inline_layout_bytes(),
                reserved_total_bytes: WSS_RESERVED_TOTAL_BYTES,
                overrun,
            })
        })
        .collect()
}
/// Direction of a complete binary WebSocket message relative to the client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoakDirection {
    /// A game message submitted to tungstenite for transmission.
    Outbound,
    /// A complete game message returned by tungstenite after frame reassembly.
    Inbound,
}

/// Failure to install the process-scoped WSS observer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SoakObserverError {
    /// An observer has already been installed in this process.
    #[error("the selected test-support observer is already installed")]
    AlreadyInstalled,
}

type Observer = dyn Fn(BindingId, Option<u16>, SoakDirection, &[u8]) + Send + Sync + 'static;

static SOAK_OBSERVER: OnceLock<Box<Observer>> = OnceLock::new();

/// Install the process observer exactly once.
///
/// The callback is invoked synchronously without holding application locks.
/// Its payload is borrowed and valid only for the duration of the call. The
/// local port is the socket's TCP ephemeral port, or `None` when it could not
/// be obtained during endpoint construction.
pub fn install_soak_observer<F>(observer: F) -> Result<(), SoakObserverError>
where
    F: Fn(BindingId, Option<u16>, SoakDirection, &[u8]) + Send + Sync + 'static,
{
    SOAK_OBSERVER
        .set(Box::new(observer))
        .map_err(|_| SoakObserverError::AlreadyInstalled)
}

pub(super) fn observe(
    binding: BindingId,
    local_port: Option<u16>,
    direction: SoakDirection,
    payload: &[u8],
) {
    if let Some(observer) = SOAK_OBSERVER.get() {
        observer(binding, local_port, direction, payload);
    }
}
/// One operation whose Rust allocator requests are measured by a test owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationScopeKind {
    /// Allocation of the shared connection and its inline rings/control state.
    SharedConnection,
    /// Construction of a TLS client configuration.
    TlsClientConfigBuild,
    /// Construction of the platform-backed certificate verifier.
    PlatformVerifierBuild,
    /// Clone and policy application for a test-support TLS configuration.
    TlsClientConfigClone,
    /// Construction of one game TLS client connection.
    GameTlsConnection,
    /// Processing of one game TLS handshake packet through rustls verification.
    GameTlsVerification,
    /// Construction of one HTTPS TLS client connection.
    HttpsTlsConnection,
    /// Processing of one HTTPS handshake packet through rustls verification.
    HttpsTlsVerification,
    /// Construction of one WSS endpoint and its upgrade state.
    WssEndpoint,
    /// One tungstenite operation that may grow or materialize message staging.
    WssMessageStaging,
    /// Test-only known allocation used to prove byte reporting.
    TestKnownAllocation,
}

/// Marks the beginning or end of one allocation ownership scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationScopePhase {
    /// The operation begins on the current thread.
    Begin,
    /// The operation ends on the current thread.
    End,
}

/// Allocation-scope boundary delivered to the test-owned allocator observer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationScopeEvent {
    /// Unique scope identity within this process.
    pub scope_id: u64,
    /// Operation measured inside the scope.
    pub kind: AllocationScopeKind,
    /// Current boundary.
    pub phase: AllocationScopePhase,
    /// Adapter connection number when the operation belongs to a game connection.
    pub connection_id: Option<u64>,
    /// Full WSS binding when the operation has one.
    pub binding: Option<BindingId>,
    /// Observed `ClientConfig` generation when the operation uses one.
    pub config_generation_id: Option<u64>,
}

type AllocationScopeObserver = dyn Fn(AllocationScopeEvent) + Send + Sync + 'static;

static ALLOCATION_SCOPE_OBSERVER: OnceLock<Box<AllocationScopeObserver>> = OnceLock::new();
static NEXT_ALLOCATION_SCOPE_ID: AtomicU64 = AtomicU64::new(1);

/// Install one allocation-scope observer in this process.
pub fn install_allocation_scope_observer<F>(observer: F) -> Result<(), SoakObserverError>
where
    F: Fn(AllocationScopeEvent) + Send + Sync + 'static,
{
    ALLOCATION_SCOPE_OBSERVER
        .set(Box::new(observer))
        .map_err(|_| SoakObserverError::AlreadyInstalled)
}

/// Begin a scope around a real operation.
///
/// The returned guard emits the matching end event when it drops. If no
/// observer is installed, the guard is inactive and performs no allocation.
#[must_use]
pub fn begin_allocation_scope(
    kind: AllocationScopeKind,
    connection_id: Option<u64>,
    binding: Option<BindingId>,
    config_generation_id: Option<u64>,
) -> AllocationScopeGuard {
    let Some(observer) = ALLOCATION_SCOPE_OBSERVER.get() else {
        return AllocationScopeGuard { event: None };
    };
    let event = AllocationScopeEvent {
        scope_id: NEXT_ALLOCATION_SCOPE_ID.fetch_add(1, Ordering::Relaxed),
        kind,
        phase: AllocationScopePhase::Begin,
        connection_id,
        binding,
        config_generation_id,
    };
    observer(event);
    AllocationScopeGuard { event: Some(event) }
}

/// Completes one allocation ownership scope when dropped.
pub struct AllocationScopeGuard {
    event: Option<AllocationScopeEvent>,
}

impl AllocationScopeGuard {
    /// Scope identity assigned to an active observer, or `None` when inactive.
    pub fn scope_id(&self) -> Option<u64> {
        self.event.map(|event| event.scope_id)
    }
}

impl Drop for AllocationScopeGuard {
    fn drop(&mut self) {
        let Some(mut event) = self.event else {
            return;
        };
        event.phase = AllocationScopePhase::End;
        if let Some(observer) = ALLOCATION_SCOPE_OBSERVER.get() {
            observer(event);
        }
    }
}

/// Worker class for the process-scoped transport workers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkWorkerKind {
    /// The Mio transport reactor.
    Reactor,
    /// The game TLS handshake worker.
    GameTls,
    /// The HTTPS TLS handshake worker.
    HttpsTls,
}

/// Observation point on one real transport worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerObservationPoint {
    /// The worker thread has started.
    Started,
    /// The worker is about to process a handshake packet that can verify a peer.
    BeforeVerify,
    /// The worker has completed processing that handshake packet.
    AfterVerify,
}

type WorkerObserver =
    dyn Fn(NetworkWorkerKind, WorkerObservationPoint) + Send + Sync + 'static;

static WORKER_OBSERVER: OnceLock<Box<WorkerObserver>> = OnceLock::new();

/// Install one worker observer in this process.
pub fn install_worker_observer<F>(observer: F) -> Result<(), SoakObserverError>
where
    F: Fn(NetworkWorkerKind, WorkerObservationPoint) + Send + Sync + 'static,
{
    WORKER_OBSERVER
        .set(Box::new(observer))
        .map_err(|_| SoakObserverError::AlreadyInstalled)
}

pub(super) fn observe_worker(kind: NetworkWorkerKind, point: WorkerObservationPoint) {
    if let Some(observer) = WORKER_OBSERVER.get() {
        observer(kind, point);
    }
}

/// Whether a config generation was first created or reused by another worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientConfigGenerationEventKind {
    /// This `Arc<ClientConfig>` identity was first observed.
    Created,
    /// Another worker observed the same live `Arc<ClientConfig>`.
    Shared,
}

/// One event for an actual worker-owned `Arc<ClientConfig>`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientConfigGenerationEvent {
    /// Monotonic generation identity assigned by the test-support observer.
    pub generation_id: u64,
    /// Whether this call created or reused the generation.
    pub kind: ClientConfigGenerationEventKind,
    /// Strong references observed when this event was emitted.
    pub strong_count: usize,
}

/// Current liveness of one observed client configuration generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientConfigGenerationSample {
    /// Monotonic generation identity.
    pub generation_id: u64,
    /// Number of worker observations for this identity.
    pub observation_count: usize,
    /// Whether at least one strong `Arc` reference remains.
    pub alive: bool,
    /// Strong references now observed, or zero after reclamation.
    pub strong_count: usize,
}

type ClientConfigGenerationObserver =
    dyn Fn(ClientConfigGenerationEvent) + Send + Sync + 'static;

struct ClientConfigGenerationRecord {
    generation_id: u64,
    config: Weak<ClientConfig>,
    observation_count: usize,
}

static CLIENT_CONFIG_GENERATION_OBSERVER: OnceLock<Box<ClientConfigGenerationObserver>> =
    OnceLock::new();
static NEXT_CLIENT_CONFIG_GENERATION_ID: AtomicU64 = AtomicU64::new(1);
static CLIENT_CONFIG_GENERATIONS: LazyLock<Mutex<Vec<ClientConfigGenerationRecord>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Install one observer for actual worker-owned client configuration identities.
pub fn install_client_config_generation_observer<F>(observer: F) -> Result<(), SoakObserverError>
where
    F: Fn(ClientConfigGenerationEvent) + Send + Sync + 'static,
{
    CLIENT_CONFIG_GENERATION_OBSERVER
        .set(Box::new(observer))
        .map_err(|_| SoakObserverError::AlreadyInstalled)
}

pub(super) fn observe_client_config_generation(config: &Arc<ClientConfig>) -> Option<u64> {
    let observer = CLIENT_CONFIG_GENERATION_OBSERVER.get()?;
    let (generation_id, kind, strong_count) = {
        let mut generations = CLIENT_CONFIG_GENERATIONS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(record) = generations.iter_mut().find(|record| {
            record
                .config
                .upgrade()
                .is_some_and(|existing| Arc::ptr_eq(&existing, config))
        }) {
            record.observation_count += 1;
            (
                record.generation_id,
                ClientConfigGenerationEventKind::Shared,
                Arc::strong_count(config),
            )
        } else {
            let generation_id =
                NEXT_CLIENT_CONFIG_GENERATION_ID.fetch_add(1, Ordering::Relaxed);
            generations.push(ClientConfigGenerationRecord {
                generation_id,
                config: Arc::downgrade(config),
                observation_count: 1,
            });
            (
                generation_id,
                ClientConfigGenerationEventKind::Created,
                Arc::strong_count(config),
            )
        }
    };
    observer(ClientConfigGenerationEvent {
        generation_id,
        kind,
        strong_count,
    });
    Some(generation_id)
}

/// Resolve the generation ID for a live config observed by a worker.
pub fn client_config_generation_id(config: &Arc<ClientConfig>) -> Option<u64> {
    CLIENT_CONFIG_GENERATION_OBSERVER.get()?;
    CLIENT_CONFIG_GENERATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|record| {
            record
                .config
                .upgrade()
                .is_some_and(|existing| Arc::ptr_eq(&existing, config))
        })
        .map(|record| record.generation_id)
}

/// Sample every observed config generation, including reclaimed generations.
pub fn client_config_generation_samples() -> Vec<ClientConfigGenerationSample> {
    if CLIENT_CONFIG_GENERATION_OBSERVER.get().is_none() {
        return Vec::new();
    }
    CLIENT_CONFIG_GENERATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|record| {
            let strong_count = record.config.strong_count();
            ClientConfigGenerationSample {
                generation_id: record.generation_id,
                observation_count: record.observation_count,
                alive: strong_count != 0,
                strong_count,
            }
        })
        .collect()
}


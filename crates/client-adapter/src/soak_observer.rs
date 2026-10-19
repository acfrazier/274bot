//! Process-scoped, read-only observation of complete game WSS binary messages.
//!
//! This module is available only with the `test-support` feature. Callbacks run
//! synchronously on the WSS reactor thread and receive borrowed payload bytes;
//! they must not retain the slice after returning.

use std::sync::{Arc, LazyLock, Mutex, OnceLock, Weak};

use crate::control::BindingId;
use crate::ring::{IN_RING_BYTES, OUT_RING_BYTES, WS_MESSAGE_LIMIT};
use crate::shared::SharedConnection;
use crate::ws_endpoint::TlsIo;

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
    #[error("the WSS soak observer is already installed")]
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

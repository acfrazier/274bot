#![cfg(feature = "test-support")]

//! Authenticated post-handshake TLS control records through the real client
//! handshake lane and WSS endpoint. The server-side test writer uses secrets
//! extracted from its completed rustls connection; it never mutates plaintext
//! socket bytes or substitutes a test verifier.

#[path = "support/g5e_stack.rs"]
mod g5e_stack;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::cell::Cell;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::ffi::c_void;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::fmt::Write as FmtWrite;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::fs::OpenOptions;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::io::BufRead;
use std::io::{self, Read, Write};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::net::{Ipv4Addr, SocketAddr};
use std::net::{TcpListener, TcpStream};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::process::{Child, Command, Stdio};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use client_adapter::cancel::{AttemptRetire, CancelSource};
use client_adapter::control::BindingId;
use client_adapter::handoff::{
    Accepted, HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, WorkerOutcome,
};
use client_adapter::health::ReactorHealth;
use client_adapter::park::Park;
use client_adapter::shared::SharedConnection;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use client_adapter::soak_observer::wss_ledger_samples;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use client_adapter::soak_observer::{
    self, AllocationScopeEvent, AllocationScopeKind, AllocationScopePhase,
    ClientConfigGenerationEvent, ClientConfigGenerationEventKind, NetworkWorkerKind,
    WorkerObservationPoint,
};
use client_adapter::tcp::TcpStop;
use client_adapter::tls_handshake::{
    build_test_client_config_with_roots, GameTlsLane, GameTlsResource,
};
use client_adapter::ws_endpoint::{WssEndpoint, WssStep};
use mio::{Events, Interest, Poll, Token, Waker};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use rustls::client::{
    ClientSessionMemoryCache, ClientSessionStore, Resumption, Tls12ClientSessionValue,
    Tls13ClientSessionValue,
};
use rustls::crypto::cipher::{
    InboundOpaqueMessage, InboundPlainMessage, MessageDecrypter, MessageEncrypter, OutboundChunks,
    OutboundPlainMessage,
};
use rustls::crypto::tls13::OkmBlock;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use rustls::pki_types::ServerName;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use rustls::NamedGroup;
use rustls::{
    CipherSuite, ClientConfig, ConnectionTrafficSecrets, ContentType, ProtocolVersion,
    ServerConfig, ServerConnection, SupportedCipherSuite,
};
use tungstenite::handshake::derive_accept_key;

const WAKER_TOKEN: Token = Token(0);
const IO_TIMEOUT: Duration = Duration::from_secs(4);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const POLL_SLICE: Duration = Duration::from_millis(25);
const MAX_TLS_RECORD: usize = 18_432;
const BACKPRESSURE_GAME_RECORD: [u8; 512] = [0x5a; 512];
const MAX_BACKPRESSURE_RECORDS: usize = 8_192;
const SETTLE_RECORD: &[u8] = b"settle-the-backpressured-socket";
const MAX_SETTLE_ROUNDS: usize = 1_024;
const TLS12_SUITE: CipherSuite = CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256;
const TLS13_SUITE: CipherSuite = CipherSuite::TLS13_AES_128_GCM_SHA256;
const HELLO_REQUEST: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
const KEY_UPDATE_REQUESTED: [u8; 5] = [0x18, 0x00, 0x00, 0x01, 0x01];
const KEY_UPDATE_NOT_REQUESTED: [u8; 5] = [0x18, 0x00, 0x00, 0x01, 0x00];
const TLS_CLOSE_NOTIFY: [u8; 2] = [0x01, 0x00];

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_ALLOCATOR_LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_ALLOCATOR_PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct G5eCountingAllocator;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
// SAFETY: every allocator operation forwards its original pointer/layout to
// System; the atomics only account requested bytes and never allocate.
unsafe impl GlobalAlloc for G5eCountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplied the layout required by GlobalAlloc.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_g5e_scoped_allocation_request(layout.size());
            let live = G5E_ALLOCATOR_LIVE_BYTES
                .fetch_add(layout.size(), Ordering::Relaxed)
                .wrapping_add(layout.size());
            G5E_ALLOCATOR_PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplied the layout required by GlobalAlloc.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_g5e_scoped_allocation_request(layout.size());
            let live = G5E_ALLOCATOR_LIVE_BYTES
                .fetch_add(layout.size(), Ordering::Relaxed)
                .wrapping_add(layout.size());
            G5E_ALLOCATOR_PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller supplied the original allocation and layout;
        // System preserves GlobalAlloc's realloc contract.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            record_g5e_scoped_allocation_request(new_size);
            let live = if new_size >= layout.size() {
                G5E_ALLOCATOR_LIVE_BYTES
                    .fetch_add(new_size - layout.size(), Ordering::Relaxed)
                    .wrapping_add(new_size - layout.size())
            } else {
                G5E_ALLOCATOR_LIVE_BYTES
                    .fetch_sub(layout.size() - new_size, Ordering::Relaxed)
                    .saturating_sub(layout.size() - new_size)
            };
            G5E_ALLOCATOR_PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
        }
        replacement
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        G5E_ALLOCATOR_LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller supplied the pointer and matching layout returned
        // by this allocator.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[global_allocator]
static G5E_ALLOCATOR: G5eCountingAllocator = G5eCountingAllocator;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn reset_g5e_allocator_peak() {
    G5E_ALLOCATOR_PEAK_BYTES.store(
        G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire),
        Ordering::Release,
    );
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_allocator_snapshot() -> (usize, usize) {
    (
        G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire),
        G5E_ALLOCATOR_PEAK_BYTES.load(Ordering::Acquire),
    )
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const G5E_MAX_ALLOCATION_SCOPES: usize = 4096;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const G5E_MAX_SCOPE_DEPTH: usize = 16;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const G5E_MAX_STACK_SAMPLES: usize = 64;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const G5E_MAX_CONFIG_GENERATION_EVENTS: usize = 16;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy, Debug)]
struct G5eScopeSample {
    event: AllocationScopeEvent,
    completed: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy, Debug)]
struct G5eWorkerStackSample {
    kind: NetworkWorkerKind,
    point: WorkerObservationPoint,
    sample: g5e_stack::StackSample,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_SCOPED_ALLOCATOR_EXCLUSIVE_BYTES: [AtomicUsize; G5E_MAX_ALLOCATION_SCOPES] =
    [const { AtomicUsize::new(0) }; G5E_MAX_ALLOCATION_SCOPES];
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES: [AtomicUsize; G5E_MAX_ALLOCATION_SCOPES] =
    [const { AtomicUsize::new(0) }; G5E_MAX_ALLOCATION_SCOPES];
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_SCOPE_SAMPLES: OnceLock<Mutex<Vec<Option<G5eScopeSample>>>> = OnceLock::new();
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_CONFIG_GENERATION_EVENTS: OnceLock<Mutex<Vec<ClientConfigGenerationEvent>>> =
    OnceLock::new();
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_WORKER_STACK_SAMPLES: OnceLock<Mutex<Vec<G5eWorkerStackSample>>> = OnceLock::new();
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_SCOPE_OVERFLOW: AtomicBool = AtomicBool::new(false);
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_CONFIG_GENERATION_OVERFLOW: AtomicBool = AtomicBool::new(false);
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_WORKER_STACK_OVERFLOW: AtomicBool = AtomicBool::new(false);
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_WORKER_STACK_CAPTURE_FAILURES: AtomicUsize = AtomicUsize::new(0);
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
static G5E_OBSERVERS_INSTALLED: OnceLock<()> = OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
std::thread_local! {
    static G5E_ACTIVE_SCOPE_ID: Cell<u64> = const { Cell::new(0) };
    static G5E_ACTIVE_SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
    static G5E_SCOPE_PARENTS: Cell<[u64; G5E_MAX_SCOPE_DEPTH]> =
        const { Cell::new([0; G5E_MAX_SCOPE_DEPTH]) };
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_scope_index(scope_id: u64) -> Option<usize> {
    usize::try_from(scope_id)
        .ok()?
        .checked_sub(1)
        .filter(|index| *index < G5E_MAX_ALLOCATION_SCOPES)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_g5e_scoped_allocation_request(bytes: usize) {
    if bytes == 0 {
        return;
    }
    let scope_id = G5E_ACTIVE_SCOPE_ID.try_with(Cell::get).unwrap_or_default();
    if scope_id == 0 {
        return;
    }
    let Some(index) = g5e_scope_index(scope_id) else {
        G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
        return;
    };
    G5E_SCOPED_ALLOCATOR_EXCLUSIVE_BYTES[index].fetch_add(bytes, Ordering::Relaxed);
    G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES[index].fetch_add(bytes, Ordering::Relaxed);
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_g5e_allocation_scope(event: AllocationScopeEvent) {
    match event.phase {
        AllocationScopePhase::Begin => {
            if let Some(index) = g5e_scope_index(event.scope_id) {
                G5E_SCOPED_ALLOCATOR_EXCLUSIVE_BYTES[index].store(0, Ordering::Relaxed);
                G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES[index].store(0, Ordering::Relaxed);
                G5E_SCOPE_SAMPLES
                    .get()
                    .expect("allocation scope samples initialized")
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())[index] =
                    Some(G5eScopeSample {
                        event,
                        completed: false,
                    });
            } else {
                G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
            }

            let depth = G5E_ACTIVE_SCOPE_DEPTH.with(Cell::get);
            if depth >= G5E_MAX_SCOPE_DEPTH {
                G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
                return;
            }
            let parent = G5E_ACTIVE_SCOPE_ID.with(Cell::get);
            G5E_SCOPE_PARENTS.with(|parents| {
                let mut stack = parents.get();
                stack[depth] = parent;
                parents.set(stack);
            });
            G5E_ACTIVE_SCOPE_DEPTH.with(|active_depth| active_depth.set(depth + 1));
            G5E_ACTIVE_SCOPE_ID.with(|active_scope| active_scope.set(event.scope_id));
        }
        AllocationScopePhase::End => {
            let active_scope = G5E_ACTIVE_SCOPE_ID.with(Cell::get);
            if active_scope != event.scope_id {
                G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
            }
            let depth = G5E_ACTIVE_SCOPE_DEPTH.with(Cell::get);
            if depth == 0 {
                G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
                return;
            }
            let parent = G5E_SCOPE_PARENTS.with(|parents| {
                let mut stack = parents.get();
                let parent = stack[depth - 1];
                stack[depth - 1] = 0;
                parents.set(stack);
                parent
            });
            G5E_ACTIVE_SCOPE_DEPTH.with(|active_depth| active_depth.set(depth - 1));
            G5E_ACTIVE_SCOPE_ID.with(|active_id| active_id.set(parent));

            let Some(index) = g5e_scope_index(event.scope_id) else {
                G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
                return;
            };
            let inclusive_bytes =
                G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES[index].load(Ordering::Relaxed);
            let mut samples = G5E_SCOPE_SAMPLES
                .get()
                .expect("allocation scope samples initialized")
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(sample) = samples[index].as_mut() {
                sample.completed = true;
            } else {
                G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
            }
            drop(samples);
            if parent != 0 {
                if let Some(parent_index) = g5e_scope_index(parent) {
                    G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES[parent_index]
                        .fetch_add(inclusive_bytes, Ordering::Relaxed);
                } else {
                    G5E_SCOPE_OVERFLOW.store(true, Ordering::Relaxed);
                }
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_g5e_config_generation(event: ClientConfigGenerationEvent) {
    let mut events = G5E_CONFIG_GENERATION_EVENTS
        .get()
        .expect("config generation events initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if events.len() < events.capacity() {
        events.push(event);
    } else {
        G5E_CONFIG_GENERATION_OVERFLOW.store(true, Ordering::Relaxed);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_g5e_worker_stack(kind: NetworkWorkerKind, point: WorkerObservationPoint) {
    match g5e_stack::capture_current_thread_stack() {
        Ok(sample) => {
            let mut samples = G5E_WORKER_STACK_SAMPLES
                .get()
                .expect("worker stack samples initialized")
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if samples.len() < samples.capacity() {
                samples.push(G5eWorkerStackSample {
                    kind,
                    point,
                    sample,
                });
            } else {
                G5E_WORKER_STACK_OVERFLOW.store(true, Ordering::Relaxed);
            }
        }
        Err(_) => {
            G5E_WORKER_STACK_CAPTURE_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn install_g5e_observers() {
    G5E_OBSERVERS_INSTALLED.get_or_init(|| {
        G5E_SCOPE_SAMPLES
            .set(Mutex::new(vec![None; G5E_MAX_ALLOCATION_SCOPES]))
            .expect("initialize bounded allocation-scope sample slots once");
        G5E_CONFIG_GENERATION_EVENTS
            .set(Mutex::new(Vec::with_capacity(
                G5E_MAX_CONFIG_GENERATION_EVENTS,
            )))
            .expect("initialize bounded config-generation events once");
        G5E_WORKER_STACK_SAMPLES
            .set(Mutex::new(Vec::with_capacity(G5E_MAX_STACK_SAMPLES)))
            .expect("initialize bounded worker-stack sample slots once");
        soak_observer::install_allocation_scope_observer(record_g5e_allocation_scope)
            .expect("install bounded G5-E allocation-scope recorder once");
        soak_observer::install_client_config_generation_observer(record_g5e_config_generation)
            .expect("install bounded G5-E config-generation recorder once");
        soak_observer::install_worker_observer(record_g5e_worker_stack)
            .expect("install native stack capture on transport workers once");
    });
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn prove_g5e_scope_allocator_observes_known_allocation() -> usize {
    const KNOWN_BYTES: usize = 32 * 1024;
    let layout = Layout::from_size_align(KNOWN_BYTES, 64).expect("valid known allocation layout");
    let scope_id;
    {
        let scope = soak_observer::begin_allocation_scope(
            AllocationScopeKind::TestKnownAllocation,
            None,
            None,
            None,
        );
        scope_id = scope
            .scope_id()
            .expect("allocation observer assigns the known scope ID");
        // SAFETY: `layout` is nonzero and valid; this pointer is deallocated
        // below with the same layout before the scope ends.
        let pointer = unsafe { std::alloc::alloc(layout) };
        assert!(!pointer.is_null(), "known allocation must succeed");
        // SAFETY: the allocation contains exactly `layout.size()` writable bytes.
        unsafe { pointer.write_bytes(0, layout.size()) };
        std::hint::black_box(pointer);
        // SAFETY: `pointer` came from the global allocator with this layout.
        unsafe { std::alloc::dealloc(pointer, layout) };
    }
    let index = g5e_scope_index(scope_id).expect("known scope fits the bounded recorder");
    let requested = G5E_SCOPED_ALLOCATOR_EXCLUSIVE_BYTES[index].load(Ordering::Acquire);
    let samples = G5E_SCOPE_SAMPLES
        .get()
        .expect("allocation scope samples initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sample = samples[index]
        .as_ref()
        .expect("known allocation scope has an observed begin event");
    assert_eq!(sample.event.kind, AllocationScopeKind::TestKnownAllocation);
    assert!(sample.completed, "known allocation scope has an end event");
    let inclusive = G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES[index].load(Ordering::Acquire);
    assert_eq!(inclusive, KNOWN_BYTES);
    assert_eq!(requested, KNOWN_BYTES);
    requested
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn optional_u64_json(output: &mut String, value: Option<u64>) {
    if let Some(value) = value {
        write!(output, "{value}").expect("write optional JSON integer");
    } else {
        output.push_str("null");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn optional_usize_json(value: Option<usize>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_allocation_scope_samples_json() -> String {
    let mut output = String::from("[");
    let mut first = true;
    let samples = G5E_SCOPE_SAMPLES
        .get()
        .expect("allocation scope samples initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    for sample in samples.iter() {
        let index = g5e_scope_index(sample.event.scope_id)
            .expect("every recorded allocation scope is in the bounded table");
        if !first {
            output.push(',');
        }
        first = false;
        let event = sample.event;
        write!(
            output,
            "{{\"scope_id\":{},\"kind\":\"{:?}\",\"connection_id\":",
            event.scope_id, event.kind
        )
        .expect("write allocation scope identity");
        optional_u64_json(&mut output, event.connection_id);
        output.push_str(",\"binding\":");
        if let Some(binding) = event.binding {
            write!(
                output,
                "{{\"incarnation\":{},\"connection\":{},\"binding\":{},\"attempt\":{}}}",
                binding.incarnation, binding.connection, binding.binding, binding.attempt
            )
            .expect("write WSS binding identity");
        } else {
            output.push_str("null");
        }
        output.push_str(",\"config_generation_id\":");
        optional_u64_json(&mut output, event.config_generation_id);
        write!(
            output,
            ",\"requested_allocator_bytes_exclusive\":{},\"requested_allocator_bytes_inclusive\":{},\"completed\":{}}}",
            G5E_SCOPED_ALLOCATOR_EXCLUSIVE_BYTES[index].load(Ordering::Relaxed),
            G5E_SCOPED_ALLOCATOR_INCLUSIVE_BYTES[index].load(Ordering::Relaxed),
            sample.completed
        )
        .expect("write allocation scope byte counts");
    }
    output.push(']');
    output
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_config_generation_events_json() -> String {
    let mut output = String::from("[");
    let events = G5E_CONFIG_GENERATION_EVENTS
        .get()
        .expect("config generation events initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    for (index, event) in events.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"generation_id\":{},\"kind\":\"{:?}\",\"strong_count\":{}}}",
            event.generation_id, event.kind, event.strong_count
        )
        .expect("write config generation event");
    }
    output.push(']');
    output
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_config_generation_samples_json() -> String {
    let mut output = String::from("[");
    let samples = soak_observer::client_config_generation_samples();
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"generation_id\":{},\"observation_count\":{},\"alive\":{},\"strong_count\":{}}}",
            sample.generation_id, sample.observation_count, sample.alive, sample.strong_count
        )
        .expect("write client config generation sample");
    }
    output.push(']');
    output
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_worker_stack_samples_json() -> String {
    let mut output = String::from("[");
    let mut first = true;
    let samples = G5E_WORKER_STACK_SAMPLES
        .get()
        .expect("worker stack samples initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    for sample in samples.iter() {
        if !first {
            output.push(',');
        }
        first = false;
        let stack = sample.sample;
        write!(
            output,
            "{{\"worker\":\"{:?}\",\"point\":\"{:?}\",\"tid\":{},\"stack_low_address\":",
            sample.kind, sample.point, stack.tid
        )
        .expect("write native worker stack identity");
        optional_u64_json(&mut output, stack.stack_low_address);
        output.push_str(",\"stack_high_address_exclusive\":");
        optional_u64_json(&mut output, stack.stack_high_address_exclusive);
        output.push_str(",\"reserved_bytes\":");
        optional_u64_json(&mut output, stack.reserved_bytes);
        output.push_str(",\"committed_bytes\":");
        optional_u64_json(&mut output, stack.committed_bytes);
        output.push_str(",\"resident_bytes\":");
        optional_u64_json(&mut output, stack.resident_bytes);
        output.push_str(",\"guard_bytes\":");
        optional_u64_json(&mut output, stack.guard_bytes);
        write!(
            output,
            ",\"measurement_source\":\"{}\"}}",
            stack.measurement_source
        )
        .expect("write native stack measurement semantics");
    }
    output.push(']');
    output
}
struct ServerIdentity {
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl ServerIdentity {
    fn duplicate(&self) -> Self {
        Self {
            certificate: self.certificate.clone(),
            key: self.key.clone_key(),
        }
    }
}

fn common_name(name: &str) -> DistinguishedName {
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, name);
    subject
}

struct TestPki {
    root: CertificateDer<'static>,
    identity: ServerIdentity,
}

impl TestPki {
    fn new() -> Self {
        let root_key = KeyPair::generate().expect("generate test CA key");
        let mut root_params = CertificateParams::new(vec!["rustls-control-test-root".to_owned()])
            .expect("create test CA parameters");
        // The CA and the leaf need different subject names. With rcgen's shared
        // default subject, Windows CryptoAPI reads the leaf as self-signed and
        // rejects the chain (CERT_E_SIGNATURE, os error -2146869244).
        root_params.distinguished_name = common_name("rustls control test root");
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let root_certificate = root_params
            .self_signed(&root_key)
            .expect("self-sign test CA");
        let root = root_certificate.der().clone();

        let identity_key = KeyPair::generate().expect("generate test server key");
        let mut identity_params = CertificateParams::new(vec!["localhost".to_owned()])
            .expect("create test server certificate parameters");
        identity_params.distinguished_name = common_name("rustls control test server");
        identity_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        identity_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let identity_certificate = identity_params
            .signed_by(&identity_key, &root_certificate, &root_key)
            .expect("sign test server certificate");

        Self {
            root,
            identity: ServerIdentity {
                certificate: identity_certificate.der().clone(),
                key: PrivateKeyDer::Pkcs8(identity_key.serialize_der().into()),
            },
        }
    }

    fn client_config(&self) -> Arc<ClientConfig> {
        build_test_client_config_with_roots(std::slice::from_ref(&self.root))
            .expect("build OS-verifying client config with the test CA root")
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Debug, Default)]
struct TicketMetrics {
    inserted: AtomicUsize,
    taken: AtomicUsize,
    allocator_live_after_insert_high_water_bytes: AtomicUsize,
    allocator_live_growth_per_insert_high_water_bytes: AtomicUsize,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Debug)]
struct TicketCountingStore {
    inner: ClientSessionMemoryCache,
    metrics: Arc<TicketMetrics>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl ClientSessionStore for TicketCountingStore {
    fn set_kx_hint(&self, server_name: ServerName<'static>, group: NamedGroup) {
        self.inner.set_kx_hint(server_name, group);
    }

    fn kx_hint(&self, server_name: &ServerName<'_>) -> Option<NamedGroup> {
        self.inner.kx_hint(server_name)
    }

    fn set_tls12_session(&self, server_name: ServerName<'static>, value: Tls12ClientSessionValue) {
        self.inner.set_tls12_session(server_name, value);
    }

    fn tls12_session(&self, server_name: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        self.inner.tls12_session(server_name)
    }

    fn remove_tls12_session(&self, server_name: &ServerName<'static>) {
        self.inner.remove_tls12_session(server_name);
    }

    fn insert_tls13_ticket(
        &self,
        server_name: ServerName<'static>,
        value: Tls13ClientSessionValue,
    ) {
        let live_before = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Relaxed);
        self.inner.insert_tls13_ticket(server_name, value);
        let live_after = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Relaxed);
        self.metrics.inserted.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .allocator_live_after_insert_high_water_bytes
            .fetch_max(live_after, Ordering::Relaxed);
        self.metrics
            .allocator_live_growth_per_insert_high_water_bytes
            .fetch_max(live_after.saturating_sub(live_before), Ordering::Relaxed);
    }

    fn take_tls13_ticket(
        &self,
        server_name: &ServerName<'static>,
    ) -> Option<Tls13ClientSessionValue> {
        let ticket = self.inner.take_tls13_ticket(server_name);
        if ticket.is_some() {
            self.metrics.taken.fetch_add(1, Ordering::Relaxed);
        }
        ticket
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn client_config_with_ticket_metrics(
    root: &CertificateDer<'static>,
    metrics: Arc<TicketMetrics>,
) -> Arc<ClientConfig> {
    let mut config = build_test_client_config_with_roots(std::slice::from_ref(root))
        .expect("build OS-verifying client config with the test CA root");
    Arc::get_mut(&mut config)
        .expect("the test owns the OS-verifying client config before sharing it")
        .resumption = Resumption::store(Arc::new(TicketCountingStore {
        inner: ClientSessionMemoryCache::new(256),
        metrics,
    }));
    config
}

fn server_config(
    identity: ServerIdentity,
    version: ProtocolVersion,
    suite: CipherSuite,
) -> Arc<ServerConfig> {
    let mut provider = rustls::crypto::ring::default_provider();
    provider
        .cipher_suites
        .retain(|candidate| candidate.suite() == suite);
    assert_eq!(
        provider.cipher_suites.len(),
        1,
        "selected ring suite exists"
    );

    let builder = ServerConfig::builder_with_provider(Arc::new(provider));
    let builder = match version {
        ProtocolVersion::TLSv1_2 => builder
            .with_protocol_versions(&[&rustls::version::TLS12])
            .expect("configure TLS 1.2-only server"),
        ProtocolVersion::TLSv1_3 => builder
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("configure TLS 1.3-only server"),
        other => panic!("unsupported control-test TLS version: {other:?}"),
    };
    let mut config = builder
        .with_no_client_auth()
        .with_single_cert(vec![identity.certificate], identity.key)
        .expect("configure local rustls server identity");
    config.enable_secret_extraction = true;
    config.send_tls13_tickets = 0;
    Arc::new(config)
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn configure_tls13_with_tickets(identity: ServerIdentity) -> Arc<ServerConfig> {
    let mut config = configure_tls13(identity);
    Arc::get_mut(&mut config)
        .expect("the test owns its TLS 1.3 server config before sharing it")
        .send_tls13_tickets = 2;
    config
}

#[derive(Debug)]
struct HandshakeInfo {
    server_name: Option<String>,
    version: Option<ProtocolVersion>,
    suite: Option<CipherSuite>,
    failure: Option<String>,
}

#[derive(Debug)]
struct ObservedRecord {
    outer_type: ContentType,
    content_type: ContentType,
    payload: Vec<u8>,
}

enum ServerCommand {
    SendRecord {
        content_type: ContentType,
        payload: Vec<u8>,
        reply: SyncSender<Result<(), String>>,
    },
    ReadClientRecord {
        timeout: Duration,
        reply: SyncSender<Result<Option<ObservedRecord>, String>>,
    },
    Stop,
}

struct ControlServer {
    address: std::net::SocketAddr,
    commands: Sender<ServerCommand>,
    handshake: Receiver<HandshakeInfo>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl ControlServer {
    fn spawn(config: Arc<ServerConfig>) -> Self {
        Self::spawn_with_receive_buffer(config, None)
    }

    fn spawn_with_receive_buffer(config: Arc<ServerConfig>, receive_buffer: Option<usize>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local TLS server");
        let address = listener.local_addr().expect("read local TLS address");
        if let Some(size) = receive_buffer {
            // Reserve the receive buffer before any connection exists, so the
            // size is in force for the whole connection (window scale and
            // advertised window), not only after the accept.
            socket2::SockRef::from(&listener)
                .set_recv_buffer_size(size)
                .expect("bound the backpressure test server receive buffer");
        }
        let (command_tx, command_rx) = mpsc::channel();
        let (handshake_tx, handshake_rx) = mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(0);
        let server = thread::spawn(move || {
            ready_tx
                .send(())
                .expect("server fixture startup receiver remains available");
            serve_control_server(listener, config, command_rx, handshake_tx, receive_buffer)
        });
        ready_rx
            .recv_timeout(IO_TIMEOUT)
            .expect("local TLS fixture reaches its accept boundary");
        Self {
            address,
            commands: command_tx,
            handshake: handshake_rx,
            thread: Some(server),
        }
    }

    fn handshake(&self) -> HandshakeInfo {
        self.handshake
            .recv_timeout(IO_TIMEOUT)
            .expect("local rustls server reports the negotiated handshake")
    }

    fn send_record(&self, content_type: ContentType, payload: &[u8]) {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(ServerCommand::SendRecord {
                content_type,
                payload: payload.to_vec(),
                reply: reply_tx,
            })
            .expect("send authenticated TLS record command");
        reply_rx
            .recv_timeout(IO_TIMEOUT)
            .expect("server emitted the authenticated TLS record")
            .unwrap_or_else(|error| panic!("server failed to emit TLS record: {error}"));
    }

    fn begin_read_client_record(
        &self,
        timeout: Duration,
    ) -> Receiver<Result<Option<ObservedRecord>, String>> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(ServerCommand::ReadClientRecord {
                timeout,
                reply: reply_tx,
            })
            .expect("send client-record inspection command");
        reply_rx
    }

    fn read_client_record(&self, timeout: Duration) -> Option<ObservedRecord> {
        self.begin_read_client_record(timeout)
            .recv_timeout(IO_TIMEOUT)
            .expect("server completed client-record inspection")
            .unwrap_or_else(|error| panic!("server could not inspect client TLS record: {error}"))
    }

    fn stop(mut self) {
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(server) = self.thread.take() {
            server
                .join()
                .expect("local control server thread did not panic")
                .expect("local control server completed");
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(server) = self.thread.take() {
            let _ = server.join();
        }
    }
}

struct RecordEncrypter {
    sequence: u64,
    cipher: Box<dyn MessageEncrypter>,
}

impl RecordEncrypter {
    fn new(
        version: ProtocolVersion,
        suite: SupportedCipherSuite,
        secrets: (u64, ConnectionTrafficSecrets),
    ) -> Self {
        let (sequence, secrets) = secrets;
        let cipher = match (version, suite, secrets) {
            (
                ProtocolVersion::TLSv1_2,
                SupportedCipherSuite::Tls12(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => {
                let iv = iv.as_ref();
                assert_eq!(iv.len(), 12, "TLS 1.2 extracted GCM nonce has 12 bytes");
                suite.aead_alg.encrypter(key, &iv[..4], &iv[4..])
            }
            (
                ProtocolVersion::TLSv1_3,
                SupportedCipherSuite::Tls13(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => suite.aead_alg.encrypter(key, iv),
            _ => panic!("unexpected test cipher/traffic-secret combination"),
        };
        Self { sequence, cipher }
    }

    fn encrypt(&mut self, content_type: ContentType, payload: &[u8]) -> Result<Vec<u8>, String> {
        let message = OutboundPlainMessage {
            typ: content_type,
            // TLS 1.3 uses TLS 1.2 as the legacy record-layer version too.
            version: ProtocolVersion::TLSv1_2,
            payload: OutboundChunks::from(payload),
        };
        let record = self
            .cipher
            .encrypt(message, self.sequence)
            .map_err(|error| error.to_string())?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "TLS transmit sequence overflowed".to_owned())?;
        Ok(record.encode())
    }
}

#[derive(Debug, Default)]
struct ClientTrafficSecret(Mutex<Option<Vec<u8>>>);

impl rustls::KeyLog for ClientTrafficSecret {
    fn log(&self, label: &str, _client_random: &[u8], secret: &[u8]) {
        if label == "CLIENT_TRAFFIC_SECRET_0" {
            *self.0.lock().expect("local server traffic secret lock") = Some(secret.to_vec());
        }
    }

    fn will_log(&self, label: &str) -> bool {
        label == "CLIENT_TRAFFIC_SECRET_0"
    }
}

struct ReceiveKeyUpdate {
    suite: &'static rustls::Tls13CipherSuite,
    secret: OkmBlock,
}

impl ReceiveKeyUpdate {
    fn advance(&mut self) -> Box<dyn MessageDecrypter> {
        // RFC 8446 sections 7.2 and 7.3: HKDF-Expand-Label with empty context.
        let expander = self.suite.hkdf_provider.expander_for_okm(&self.secret);
        let length = (expander.hash_len() as u16).to_be_bytes();
        self.secret = expander.expand_block(&[&length, b"\x11tls13 traffic upd\x00"]);
        let expander = self.suite.hkdf_provider.expander_for_okm(&self.secret);
        let mut key = [0u8; 16];
        let mut iv = [0u8; 12];
        expander
            .expand_slice(&[b"\x00\x10\x09tls13 key\x00"], &mut key)
            .expect("AES-128 traffic key length");
        expander
            .expand_slice(&[b"\x00\x0c\x08tls13 iv\x00"], &mut iv)
            .expect("TLS traffic IV length");
        Box::new(UpdatedRecordDecrypter {
            key: ring::aead::LessSafeKey::new(
                ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &key)
                    .expect("AES-128 traffic key"),
            ),
            iv,
        })
    }
}

struct UpdatedRecordDecrypter {
    key: ring::aead::LessSafeKey,
    iv: [u8; 12],
}

impl MessageDecrypter for UpdatedRecordDecrypter {
    fn decrypt<'a>(
        &mut self,
        mut message: InboundOpaqueMessage<'a>,
        sequence: u64,
    ) -> Result<InboundPlainMessage<'a>, rustls::Error> {
        if message.typ != ContentType::ApplicationData
            || message.version != ProtocolVersion::TLSv1_2
        {
            return Err(rustls::Error::DecryptError);
        }
        let mut nonce = self.iv;
        for (byte, sequence_byte) in nonce[4..].iter_mut().zip(sequence.to_be_bytes()) {
            *byte ^= sequence_byte;
        }
        let length = (message.payload.len() as u16).to_be_bytes();
        let aad = [0x17, 0x03, 0x03, length[0], length[1]];
        let plaintext_len = self
            .key
            .open_in_place(
                ring::aead::Nonce::assume_unique_for_key(nonce),
                ring::aead::Aad::from(aad),
                &mut message.payload,
            )
            .map_err(|_| rustls::Error::DecryptError)?
            .len();
        message.payload.truncate(plaintext_len);
        message.into_tls13_unpadded_message()
    }
}

#[test]
fn updated_record_decrypter_authenticates_sequence_and_tag() {
    let make_key = || {
        ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &[0x42; 16]).expect("test key"),
        )
    };
    let iv = [0x31; 12];
    let mut nonce = iv;
    nonce[11] ^= 7;
    let mut ciphertext = b"next-key\x17\x00\x00".to_vec();
    let length = (ciphertext.len() + ring::aead::AES_128_GCM.tag_len()) as u16;
    let [hi, lo] = length.to_be_bytes();
    make_key()
        .seal_in_place_append_tag(
            ring::aead::Nonce::assume_unique_for_key(nonce),
            ring::aead::Aad::from([0x17, 0x03, 0x03, hi, lo]),
            &mut ciphertext,
        )
        .expect("encrypt authenticated next-key test record");
    let mut decrypter = UpdatedRecordDecrypter {
        key: make_key(),
        iv,
    };
    let mut valid = ciphertext.clone();
    let plaintext = decrypter
        .decrypt(
            InboundOpaqueMessage::new(
                ContentType::ApplicationData,
                ProtocolVersion::TLSv1_2,
                &mut valid,
            ),
            7,
        )
        .expect("authenticate and remove TLS inner padding");
    assert_eq!(plaintext.typ, ContentType::ApplicationData);
    assert_eq!(plaintext.payload, b"next-key");
    for (sequence, corrupt) in [(8, false), (7, true)] {
        let mut invalid = ciphertext.clone();
        if corrupt {
            invalid[0] ^= 1;
        }
        assert!(decrypter
            .decrypt(
                InboundOpaqueMessage::new(
                    ContentType::ApplicationData,
                    ProtocolVersion::TLSv1_2,
                    &mut invalid,
                ),
                sequence,
            )
            .is_err());
    }
}

struct RecordDecrypter {
    sequence: u64,
    cipher: Box<dyn MessageDecrypter>,
    key_update: Option<ReceiveKeyUpdate>,
}

impl RecordDecrypter {
    fn new(
        version: ProtocolVersion,
        suite: SupportedCipherSuite,
        secrets: (u64, ConnectionTrafficSecrets),
    ) -> Self {
        let (sequence, secrets) = secrets;
        let cipher = match (version, suite, secrets) {
            (
                ProtocolVersion::TLSv1_2,
                SupportedCipherSuite::Tls12(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => {
                let iv = iv.as_ref();
                assert_eq!(iv.len(), 12, "TLS 1.2 extracted GCM nonce has 12 bytes");
                suite.aead_alg.decrypter(key, &iv[..4])
            }
            (
                ProtocolVersion::TLSv1_3,
                SupportedCipherSuite::Tls13(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => suite.aead_alg.decrypter(key, iv),
            _ => panic!("unexpected test cipher/traffic-secret combination"),
        };
        Self {
            sequence,
            cipher,
            key_update: None,
        }
    }

    fn decrypt(&mut self, record: WireRecord) -> Result<ObservedRecord, String> {
        if record.version != [0x03, 0x03] {
            return Err(format!(
                "unexpected TLS legacy record version: {:?}",
                record.version
            ));
        }
        let outer_type = ContentType::from(record.content_type);
        let mut payload = record.payload;
        let plaintext = self
            .cipher
            .decrypt(
                InboundOpaqueMessage::new(outer_type, ProtocolVersion::TLSv1_2, &mut payload),
                self.sequence,
            )
            .map_err(|error| error.to_string())?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "TLS receive sequence overflowed".to_owned())?;
        let observed = ObservedRecord {
            outer_type,
            content_type: plaintext.typ,
            payload: plaintext.payload.to_vec(),
        };
        if observed.content_type == ContentType::Handshake
            && matches!(observed.payload.as_slice(), [0x18, 0, 0, 1, 0 | 1])
        {
            let update = self
                .key_update
                .as_mut()
                .ok_or_else(|| "TLS KeyUpdate has no receive traffic secret".to_owned())?;
            self.cipher = update.advance();
            self.sequence = 0;
        }
        Ok(observed)
    }
}

struct WireRecord {
    content_type: u8,
    version: [u8; 2],
    payload: Vec<u8>,
}

fn read_wire_record(stream: &mut TcpStream, timeout: Duration) -> io::Result<Option<WireRecord>> {
    stream.set_read_timeout(Some(timeout))?;
    let mut header = [0u8; 5];
    match stream.read(&mut header[..1]) {
        Ok(0) => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "peer closed before a TLS record header",
            ));
        }
        Ok(1) => {}
        Ok(_) => unreachable!("one-byte read cannot return more than one byte"),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    }
    stream.read_exact(&mut header[1..])?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    if length > MAX_TLS_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("TLS record length {length} exceeds the test bound"),
        ));
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    Ok(Some(WireRecord {
        content_type: header[0],
        version: [header[1], header[2]],
        payload,
    }))
}

fn serve_control_server(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    commands: Receiver<ServerCommand>,
    handshake_tx: SyncSender<HandshakeInfo>,
    receive_buffer: Option<usize>,
) -> io::Result<()> {
    let (mut socket, _) = listener.accept()?;
    if let Some(size) = receive_buffer {
        socket2::SockRef::from(&socket).set_recv_buffer_size(size)?;
    }
    socket.set_read_timeout(Some(IO_TIMEOUT))?;
    socket.set_write_timeout(Some(IO_TIMEOUT))?;
    socket.set_nodelay(true)?;

    let send_tls13_tickets = config.send_tls13_tickets;
    let traffic_secret = Arc::new(ClientTrafficSecret::default());
    let mut config = (*config).clone();
    config.key_log = traffic_secret.clone();
    let mut connection = ServerConnection::new(Arc::new(config))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut handshake_failure = None;
    while connection.is_handshaking() {
        if let Err(error) = connection.complete_io(&mut socket) {
            handshake_failure = Some(error.to_string());
            break;
        }
    }
    let info = HandshakeInfo {
        server_name: connection.server_name().map(str::to_owned),
        version: connection.protocol_version(),
        suite: connection
            .negotiated_cipher_suite()
            .map(|negotiated| negotiated.suite()),
        failure: handshake_failure,
    };
    let accepted_tls = info.failure.is_none();
    handshake_tx
        .send(info)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "handshake observer dropped"))?;
    if !accepted_tls {
        return Ok(());
    }

    let version = connection
        .protocol_version()
        .expect("successful test handshake has a protocol version");
    let suite = connection
        .negotiated_cipher_suite()
        .expect("successful test handshake has a cipher suite");
    let mut tls = rustls::StreamOwned::new(connection, socket);
    if send_tls13_tickets > 0 {
        tls.flush()?;
    }
    let request = read_http_head(&mut tls)?;
    let request = std::str::from_utf8(&request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let key = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("Sec-WebSocket-Key")
                .then_some(value.trim())
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing WebSocket key"))?;
    let accept = derive_accept_key(key.as_bytes());
    write!(
        tls,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n"
    )?;
    tls.flush()?;

    let (connection, mut socket) = tls.into_parts();
    let secrets = connection
        .dangerous_extract_secrets()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut tx = RecordEncrypter::new(version, suite, secrets.tx);
    let mut rx = RecordDecrypter::new(version, suite, secrets.rx);
    if let SupportedCipherSuite::Tls13(suite) = suite {
        let secret = traffic_secret
            .0
            .lock()
            .expect("local server traffic secret lock")
            .take()
            .expect("TLS 1.3 handshake logged its client traffic secret");
        rx.key_update = Some(ReceiveKeyUpdate {
            suite,
            secret: OkmBlock::new(&secret),
        });
    }

    while let Ok(command) = commands.recv() {
        match command {
            ServerCommand::SendRecord {
                content_type,
                payload,
                reply,
            } => {
                let result = tx
                    .encrypt(content_type, &payload)
                    .and_then(|record| socket.write_all(&record).map_err(|error| error.to_string()))
                    .and_then(|()| socket.flush().map_err(|error| error.to_string()));
                let _ = reply.send(result);
            }
            ServerCommand::ReadClientRecord { timeout, reply } => {
                let result = match read_wire_record(&mut socket, timeout) {
                    Ok(Some(record)) => rx.decrypt(record).map(Some),
                    Ok(None) => Ok(None),
                    Err(error) => Err(error.to_string()),
                };
                let _ = reply.send(result);
            }
            ServerCommand::Stop => break,
        }
    }
    Ok(())
}

fn read_http_head(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() == 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "test WSS request header exceeded 4096 bytes",
            ));
        }
        reader.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    Ok(head)
}

struct LaneAttempt {
    poll: Poll,
    events: Events,
    lane: Arc<GameTlsLane>,
    key: HandoffKey,
    token: client_adapter::cancel::CancelToken,
    binding: BindingId,
    shared: Arc<SharedConnection>,
    retirement: Arc<AttemptRetire>,
}

impl LaneAttempt {
    fn start_with_send_buffer(
        address: std::net::SocketAddr,
        config: Arc<ClientConfig>,
        send_buffer: Option<usize>,
    ) -> Self {
        static NEXT_CONNECTION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let poll = Poll::new().expect("create local TLS poller");
        let waker =
            Arc::new(Waker::new(poll.registry(), WAKER_TOKEN).expect("create handoff waker"));
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().expect("mint one TLS attempt token");
        let binding = BindingId {
            incarnation: 1,
            connection: NEXT_CONNECTION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            binding: 1,
            attempt: token.generation(),
        };
        let shared = SharedConnection::new(binding.connection, park);
        let lane = GameTlsLane::start_with_test_config(
            waker,
            Arc::new(ReactorHealth::new(Instant::now())),
            config,
        )
        .expect("start real game TLS handshake worker");
        assert!(lane.config().is_some(), "worker publishes its TLS config");

        let transfer_seq = lane
            .cell()
            .next_transfer_seq()
            .expect("initial transfer sequence is available");
        let key = HandoffKey {
            identity: HandoffIdentity {
                generation: token.generation(),
                binding,
            },
            transfer_seq,
        };
        let proof = Arc::new(QueueReleaseProof::new(binding, transfer_seq));
        let retirement = Arc::new(AttemptRetire::new(token.generation()));

        let socket = TcpStream::connect(address).expect("connect to local rustls server");
        if let Some(size) = send_buffer {
            socket2::SockRef::from(&socket)
                .set_send_buffer_size(size)
                .expect("bound the backpressure test client send buffer");
        }
        socket.set_nodelay(true).expect("set local TCP_NODELAY");
        socket
            .set_nonblocking(true)
            .expect("make handshake socket nonblocking");
        let mut socket = mio::net::TcpStream::from_std(socket);
        poll.registry()
            .register(
                &mut socket,
                Token(binding.connection as usize),
                Interest::READABLE,
            )
            .expect("register handshake socket");
        let resource = GameTlsResource::new(socket, "localhost".to_owned(), None);
        lane.cell()
            .submit(
                key,
                proof,
                token.clone(),
                Arc::clone(&retirement),
                Instant::now() + HANDSHAKE_TIMEOUT,
                resource,
            )
            .unwrap_or_else(|error| panic!("submit initial TLS resource: {error:?}"));

        Self {
            poll,
            events: Events::with_capacity(16),
            lane,
            key,
            token,
            binding,
            shared,
            retirement,
        }
    }

    fn wait_for_return(&mut self) -> Accepted<GameTlsResource> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            match self.lane.cell().phase() {
                HandoffPhase::Returned => {
                    return self
                        .lane
                        .cell()
                        .take_return(self.key)
                        .expect("take the completed TLS worker result");
                }
                HandoffPhase::Released => panic!("initial TLS attempt was revoked"),
                _ => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "initial TLS handshake timed out");
            self.poll
                .poll(&mut self.events, Some(remaining.min(POLL_SLICE)))
                .expect("poll local handshake socket");
            for event in &self.events {
                if event.token() == Token(self.binding.connection as usize) && event.is_readable() {
                    self.lane
                        .cell()
                        .publish_ready(self.key, client_adapter::handoff::ReadyDirection::Read);
                }
            }
        }
    }

    fn accept_success(
        &mut self,
        accepted: Accepted<GameTlsResource>,
        authority: &str,
    ) -> WssEndpoint {
        assert_eq!(accepted.key, self.key, "accept exact transfer identity");
        let mut resource = match accepted.outcome {
            WorkerOutcome::Success(resource) => resource,
            WorkerOutcome::Failure(failure) => panic!("expected trusted TLS, got {failure:?}"),
        };
        let connection = resource
            .connection
            .take()
            .expect("successful worker result carries rustls state");
        assert!(
            !connection.is_handshaking() && !connection.wants_write(),
            "worker flushes Client Finished before transfer"
        );
        self.poll
            .registry()
            .reregister(
                &mut resource.socket,
                Token(self.binding.connection as usize),
                Interest::READABLE.add(Interest::WRITABLE),
            )
            .expect("retain registered socket across TLS transfer");
        self.shared.accept_binding(self.binding);
        let endpoint = WssEndpoint::new(
            self.binding,
            authority,
            resource.socket,
            connection,
            Arc::clone(&self.shared),
            self.token.clone(),
            None,
            Arc::clone(&self.retirement),
        )
        .expect("construct real WSS endpoint from transferred TLS");
        assert!(self.retirement.ack(), "accepted transfer retires once");
        self.lane
            .cell()
            .finish_accept(self.key)
            .expect("finish accepted TLS transfer");
        endpoint
    }
}

impl Drop for LaneAttempt {
    fn drop(&mut self) {
        let _ = self.lane.shutdown();
    }
}

fn connect_wss(
    server: &ControlServer,
    config: Arc<ClientConfig>,
) -> (LaneAttempt, WssEndpoint, HandshakeInfo) {
    connect_wss_with_send_buffer(server, config, None)
}

fn connect_wss_with_send_buffer(
    server: &ControlServer,
    config: Arc<ClientConfig>,
    send_buffer: Option<usize>,
) -> (LaneAttempt, WssEndpoint, HandshakeInfo) {
    let mut attempt = LaneAttempt::start_with_send_buffer(server.address, config, send_buffer);
    let accepted = attempt.wait_for_return();
    let info = server.handshake();
    assert!(
        info.failure.is_none(),
        "test server accepts the trusted client: {info:?}"
    );
    assert_eq!(info.server_name.as_deref(), Some("localhost"));
    let authority = format!("localhost:{}", server.address.port());
    let mut endpoint = attempt.accept_success(accepted, &authority);
    complete_wss_upgrade(&mut attempt, &mut endpoint);
    assert!(
        endpoint.is_connected(),
        "the real WSS endpoint completes upgrade"
    );
    drain_local_work_to_idle(&mut attempt, &mut endpoint);
    (attempt, endpoint, info)
}

/// Run the endpoint's local continuation until its socket read returns a real
/// WouldBlock, as the product reactor does for every session that reports
/// local work. mio simulates edge-triggered readiness on Windows by re-arming
/// a socket only after an I/O call on it returns WouldBlock. A turn that stops
/// at local work (the upgrade response, or the last game record read) leaves
/// the socket disarmed there, so the next record the server sends raises no
/// readable event. Callers run this only while no record under test is in
/// flight, and every drain turn must read no game bytes, so it cannot consume
/// or hide one.
fn drain_local_work_to_idle(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint) {
    for _ in 0..8 {
        let turn = endpoint.drive().expect("drain local WSS work to idle");
        endpoint
            .apply_interest(attempt.poll.registry())
            .expect("apply WSS interest after the idle drain");
        assert_eq!(turn.read_bytes, 0, "the idle drain reads no game bytes");
        if turn.stop != TcpStop::LocalWork {
            assert_eq!(turn.stop, TcpStop::Quiescent);
            return;
        }
    }
    panic!("local WSS work did not reach idle");
}

fn complete_wss_upgrade(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint) {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let step = endpoint.begin_step(Instant::now());
        endpoint
            .apply_interest(attempt.poll.registry())
            .expect("apply WSS upgrade interest");
        if let WssStep::Failed(error) = step {
            panic!("local WSS upgrade failed: {error}");
        }
        if endpoint.is_connected() {
            assert!(attempt.shared.commit_session(attempt.binding));
            return;
        }
        if matches!(step, WssStep::Wait) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "local WSS upgrade timed out");
            attempt
                .poll
                .poll(&mut attempt.events, Some(remaining.min(POLL_SLICE)))
                .expect("poll local WSS upgrade");
            for event in &attempt.events {
                if event.token() == Token(attempt.binding.connection as usize) {
                    if let WssStep::Failed(error) =
                        endpoint.on_event(event.is_readable(), event.is_writable(), Instant::now())
                    {
                        panic!("local WSS upgrade failed on readiness: {error}");
                    }
                    endpoint
                        .apply_interest(attempt.poll.registry())
                        .expect("update WSS upgrade interest");
                }
            }
        }
    }
}

fn next_readable_turn(
    attempt: &mut LaneAttempt,
    endpoint: &mut WssEndpoint,
) -> io::Result<client_adapter::tcp::TcpTurnResult> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for authenticated TLS input"
        );
        attempt
            .poll
            .poll(&mut attempt.events, Some(remaining.min(POLL_SLICE)))?;
        let mut readable = false;
        for event in &attempt.events {
            if event.token() == Token(attempt.binding.connection as usize) && event.is_readable() {
                endpoint.on_event(true, event.is_writable(), Instant::now());
                readable = true;
            }
        }
        if readable {
            let turn = endpoint.drive();
            if turn.is_ok() {
                endpoint.apply_interest(attempt.poll.registry())?;
            }
            return turn;
        }
    }
}

fn take_game_payload(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint, expected: &[u8]) {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    while attempt.shared.inbound.len() < expected.len() {
        assert!(
            Instant::now() < deadline,
            "authenticated follow-up WebSocket payload timed out"
        );
        next_readable_turn(attempt, endpoint)
            .unwrap_or_else(|error| panic!("established WSS drive failed: {error}"));
    }
    // The last turn can end with local work still queued rather than at a real
    // WouldBlock; finish it so the next server record raises a readable event
    // on Windows too (see `drain_local_work_to_idle`).
    drain_local_work_to_idle(attempt, endpoint);
    let mut received = vec![0; expected.len()];
    let count = attempt.shared.take_inbound(&mut received);
    assert_eq!(count, expected.len());
    assert_eq!(
        received.as_slice(),
        expected,
        "current session preserves game bytes"
    );
}

fn server_binary_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.push(0x82); // FIN + binary, server frames are not masked.
    match payload.len() {
        length @ 0..=125 => frame.push(length as u8),
        length @ 126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => panic!("control-test binary frame is too large: {length}"),
    }
    frame.extend_from_slice(payload);
    frame
}

fn assert_negotiated(handshake: &HandshakeInfo, version: ProtocolVersion, suite: CipherSuite) {
    assert!(
        handshake.failure.is_none(),
        "TLS handshake succeeded: {handshake:?}"
    );
    assert_eq!(handshake.server_name.as_deref(), Some("localhost"));
    assert_eq!(handshake.version, Some(version));
    assert_eq!(handshake.suite, Some(suite));
}

fn configure_tls12(identity: ServerIdentity) -> Arc<ServerConfig> {
    server_config(identity, ProtocolVersion::TLSv1_2, TLS12_SUITE)
}

fn configure_tls13(identity: ServerIdentity) -> Arc<ServerConfig> {
    server_config(identity, ProtocolVersion::TLSv1_3, TLS13_SUITE)
}

#[test]
fn g2_m2_authenticated_hello_request_keeps_session_and_excess_is_fatal() {
    let pki = TestPki::new();
    let client_config = pki.client_config();

    let server = ControlServer::spawn(configure_tls12(ServerIdentity {
        certificate: pki.identity.certificate.clone(),
        key: pki.identity.key.clone_key(),
    }));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, Arc::clone(&client_config));
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);
    let initial_game = b"before-hello-request-game-data";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();

    // The server's extracted TX key and sequence authenticate this record.
    server.send_record(ContentType::Handshake, &HELLO_REQUEST);
    let turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("one authenticated HelloRequest is refused without renegotiation");
    assert_eq!(turn.read_bytes, 0, "HelloRequest yields no game bytes");
    assert!(
        endpoint.is_connected(),
        "one request leaves the current WSS session live"
    );
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(attempt.shared.inbound.is_empty());

    let response = server
        .read_client_record(IO_TIMEOUT)
        .expect("client emits its authenticated TLS response to HelloRequest");
    assert_eq!(response.outer_type, ContentType::Alert);
    assert_eq!(response.content_type, ContentType::Alert);
    assert_eq!(
        response.payload.as_slice(),
        &[0x01, 0x64][..],
        "TLS 1.2 no_renegotiation warning"
    );

    // A separately encrypted record, with the next TLS sequence number,
    // proves the original session continued after the refusal.
    let game = b"post-hello-request-game-data";
    server.send_record(ContentType::ApplicationData, &server_binary_frame(game));
    take_game_payload(&mut attempt, &mut endpoint, game);
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(
        attempt.shared.bytes_in(),
        bytes_in_before + game.len() as u64
    );
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "the client did not start a replacement handshake or emit game data"
    );
    endpoint.close(attempt.poll.registry());
    server.stop();

    // rustls permits one refusal response, but a second authenticated request
    // is an excessive-request protocol error that closes only this endpoint.
    let excessive_server = ControlServer::spawn(configure_tls12(pki.identity.duplicate()));
    let (mut excessive_attempt, mut excessive_endpoint, excessive_handshake) =
        connect_wss(&excessive_server, Arc::clone(&client_config));
    assert_negotiated(&excessive_handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);
    let sibling_server = ControlServer::spawn(configure_tls12(pki.identity.duplicate()));
    let (mut sibling_attempt, mut sibling_endpoint, sibling_handshake) =
        connect_wss(&sibling_server, Arc::clone(&client_config));
    assert_negotiated(&sibling_handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);
    let excessive_initial_game = b"before-excessive-hello-request-data";
    excessive_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(excessive_initial_game),
    );
    take_game_payload(
        &mut excessive_attempt,
        &mut excessive_endpoint,
        excessive_initial_game,
    );
    let bytes_in_before = excessive_attempt.shared.bytes_in();
    let bytes_out_before = excessive_attempt.shared.bytes_out();
    excessive_server.send_record(ContentType::Handshake, &HELLO_REQUEST);
    excessive_server.send_record(ContentType::Handshake, &HELLO_REQUEST);
    let error = loop {
        match next_readable_turn(&mut excessive_attempt, &mut excessive_endpoint) {
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let detail = error
        .get_ref()
        .expect("TLS protocol failure remains a typed error source")
        .to_string();
    assert!(
        detail.to_ascii_lowercase().contains("renegotiation"),
        "excessive HelloRequest failure is identified: {detail}"
    );
    assert_eq!(excessive_attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(excessive_attempt.shared.bytes_in(), bytes_in_before);
    assert!(excessive_attempt.shared.inbound.is_empty());
    assert!(excessive_attempt.shared.outbound.is_empty());
    if let Some(response) = excessive_server.read_client_record(Duration::from_millis(250)) {
        assert_eq!(response.content_type, ContentType::Alert);
        assert_eq!(
            response.payload.len(),
            2,
            "no handshake or game payload is emitted"
        );
    }
    excessive_endpoint.close(excessive_attempt.poll.registry());
    assert!(
        excessive_attempt.shared.is_closed(),
        "excessive stream closes locally"
    );

    let sibling_game = b"sibling-session-remains-healthy";
    sibling_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(sibling_game),
    );
    take_game_payload(&mut sibling_attempt, &mut sibling_endpoint, sibling_game);
    sibling_endpoint.close(sibling_attempt.poll.registry());
    excessive_server.stop();
    sibling_server.stop();
}

#[test]
fn g2_m2_authenticated_tls_close_notify_is_distinct_from_excessive_request() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls12(pki.identity));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, client_config);
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);

    let initial_game = b"before-peer-close-game-data";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    server.send_record(ContentType::Alert, &TLS_CLOSE_NOTIFY);
    let error = next_readable_turn(&mut attempt, &mut endpoint)
        .expect_err("authenticated TLS close_notify ends the peer's WSS stream");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        format!("{error:?}").contains("ResetWithoutClosingHandshake"),
        "expected an authenticated peer-close error, got {error:?}"
    );
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert_eq!(attempt.shared.bytes_out(), 0);
    assert!(attempt.shared.inbound.is_empty());

    endpoint.close(attempt.poll.registry());
    server.stop();
}

/// Publish complete authenticated WSS game records, one at a time, until a
/// real socket write returns WouldBlock and the TLS writer retains encrypted
/// output. Returns how many records it took; 0 when the writer was already
/// blocked by such an outcome.
fn fill_until_tls_blocked(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint) -> usize {
    fill_until_tls_blocked_observed(attempt, endpoint, || {})
}

fn fill_until_tls_blocked_observed(
    attempt: &mut LaneAttempt,
    endpoint: &mut WssEndpoint,
    mut observe: impl FnMut(),
) -> usize {
    let mut published = 0usize;
    while !endpoint.tls_output_blocked_for_test() {
        assert!(
            published < MAX_BACKPRESSURE_RECORDS,
            "the socket never refused a real encrypted game write"
        );
        attempt
            .shared
            .publish_record(attempt.binding, &BACKPRESSURE_GAME_RECORD)
            .expect("publish one bounded game record while the peer is not draining");
        endpoint.kick_output();
        let mut turn = endpoint.drive().expect("drive one real WSS game write");
        endpoint
            .apply_interest(attempt.poll.registry())
            .expect("apply WSS interest after a game write");
        for _ in 0..8 {
            if turn.stop != TcpStop::LocalWork {
                break;
            }
            turn = endpoint
                .drive()
                .expect("drain local work after one game write");
            endpoint
                .apply_interest(attempt.poll.registry())
                .expect("apply WSS interest after local game work");
        }
        assert_eq!(turn.stop, TcpStop::Quiescent);
        assert_eq!(turn.read_bytes, 0);
        assert!(attempt.shared.outbound.is_empty());
        published += 1;
    }
    observe();
    published
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Default)]
struct G5eLedgerPeaks {
    sample_count: usize,
    connection_count: usize,
    total_reserved_bytes: usize,
    pending_tls_output_high_water_bytes: usize,
    outbound_ring_high_water_bytes: usize,
    inbound_ring_high_water_bytes: usize,
    target_pending_tls_output_bytes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn sample_g5e_wss_ledger(
    peaks: &mut G5eLedgerPeaks,
    target: BindingId,
    expected_connections: usize,
) -> usize {
    let samples = wss_ledger_samples();
    assert_eq!(
        samples.len(),
        expected_connections,
        "ledger reports each live adapter WSS connection"
    );
    peaks.sample_count += 1;
    peaks.connection_count = samples.len();
    peaks.total_reserved_bytes = samples
        .iter()
        .map(|sample| sample.reserved_total_bytes)
        .sum();

    let mut target_pending = None;
    for sample in samples {
        assert!(!sample.overrun, "WSS ledger exceeded a named reservation");
        let pending = sample
            .pending_tls_output_bytes
            .expect("rustls exposes exact pending TLS output bytes");
        peaks.pending_tls_output_high_water_bytes =
            peaks.pending_tls_output_high_water_bytes.max(pending);
        peaks.outbound_ring_high_water_bytes = peaks
            .outbound_ring_high_water_bytes
            .max(sample.outbound_ring_bytes);
        peaks.inbound_ring_high_water_bytes = peaks
            .inbound_ring_high_water_bytes
            .max(sample.inbound_ring_bytes);
        if sample.binding == target {
            target_pending = Some(pending);
        }
    }
    let target_pending =
        target_pending.expect("ledger retains the backpressured connection identity");
    peaks.target_pending_tls_output_bytes = target_pending;
    target_pending
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy, Debug)]
struct G5eProcessResources {
    rss_bytes: u64,
    rss_high_water_bytes: Option<u64>,
    task_count: usize,
    file_descriptors: Option<usize>,
    process_handles: Option<usize>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn g5e_process_resources() -> G5eProcessResources {
    #[cfg(target_os = "linux")]
    {
        let rss_bytes = g5e_linux_rss_bytes().expect("read Linux process RSS");
        let task_count = fs::read_dir("/proc/self/task")
            .expect("Linux procfs exposes this test process task directory")
            .count();
        let file_descriptors = fs::read_dir("/proc/self/fd")
            .expect("Linux procfs exposes this test process fd directory")
            .count()
            .saturating_sub(1);
        G5eProcessResources {
            rss_bytes,
            rss_high_water_bytes: Some(
                g5e_linux_rss_high_water_bytes().expect("read Linux process RSS high-water"),
            ),
            task_count,
            file_descriptors: Some(file_descriptors),
            process_handles: None,
        }
    }
    #[cfg(target_os = "macos")]
    {
        let task_info = g5e_macos_task_info().expect("read macOS process task information");
        let file_descriptors = g5e_macos_fd_count();
        G5eProcessResources {
            rss_bytes: task_info.resident_size,
            rss_high_water_bytes: None,
            task_count: usize::try_from(task_info.thread_count)
                .expect("macOS process thread count is nonnegative"),
            file_descriptors,
            process_handles: None,
        }
    }
    #[cfg(target_os = "windows")]
    {
        let (rss_bytes, rss_high_water_bytes) =
            g5e_windows_rss_snapshot().expect("read Windows process working set");
        G5eProcessResources {
            rss_bytes,
            rss_high_water_bytes: Some(rss_high_water_bytes),
            task_count: g5e_windows_thread_count().expect("read Windows process thread count"),
            file_descriptors: None,
            process_handles: Some(
                g5e_windows_handle_count().expect("read Windows process handle count"),
            ),
        }
    }
}

#[cfg(target_os = "linux")]
fn g5e_linux_rss_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let resident_kib = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?;
    resident_kib.checked_mul(1024)
}

#[cfg(target_os = "linux")]
fn g5e_linux_rss_high_water_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let resident_kib = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?;
    resident_kib.checked_mul(1024)
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct G5eProcTaskInfo {
    _virtual_size: u64,
    resident_size: u64,
    _total_user: u64,
    _total_system: u64,
    _threads_user: u64,
    _threads_system: u64,
    _policy: i32,
    _faults: i32,
    _pageins: i32,
    _cow_faults: i32,
    _messages_sent: i32,
    _messages_received: i32,
    _syscalls_mach: i32,
    _syscalls_unix: i32,
    _context_switches: i32,
    thread_count: i32,
    _running_count: i32,
    _priority: i32,
}

#[cfg(target_os = "macos")]
fn g5e_macos_task_info() -> Option<G5eProcTaskInfo> {
    const PROC_PIDTASKINFO: i32 = 4;

    let pid = i32::try_from(std::process::id()).ok()?;
    let buffer_size = i32::try_from(std::mem::size_of::<G5eProcTaskInfo>()).ok()?;
    let mut information = std::mem::MaybeUninit::<G5eProcTaskInfo>::zeroed();
    // SAFETY: the buffer is writable and has the exact size of proc_taskinfo.
    let returned = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            information.as_mut_ptr().cast(),
            buffer_size,
        )
    };
    if returned != buffer_size {
        return None;
    }
    // SAFETY: proc_pidinfo returned the exact proc_taskinfo size.
    Some(unsafe { information.assume_init() })
}

#[cfg(target_os = "macos")]
#[test]
fn g5e_macos_process_census_matches_native_layout() {
    // macOS SDK sys/proc_info.h: six uint64_t fields and twelve int32_t fields.
    assert_eq!(std::mem::size_of::<G5eProcTaskInfo>(), 96);
    let information = g5e_macos_task_info().expect("read native proc_taskinfo");
    assert!(information.resident_size > 0);
    assert!(information.thread_count > 0);
    assert!(g5e_macos_fd_count().is_some_and(|count| count > 0));
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct G5eProcFdInfo {
    _descriptor: i32,
    _kind: u32,
}

#[cfg(target_os = "macos")]
fn g5e_macos_fd_count() -> Option<usize> {
    const PROC_PIDLISTFDS: i32 = 2;
    const MAX_PROCESS_FDS: usize = 4096;

    let pid = i32::try_from(std::process::id()).ok()?;
    let mut descriptors = [G5eProcFdInfo {
        _descriptor: 0,
        _kind: 0,
    }; MAX_PROCESS_FDS];
    let buffer_size = std::mem::size_of_val(&descriptors);
    let buffer_size_i32 = i32::try_from(buffer_size).ok()?;
    // SAFETY: `descriptors` is writable and the byte size matches its capacity.
    let returned = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDLISTFDS,
            0,
            descriptors.as_mut_ptr().cast(),
            buffer_size_i32,
        )
    };
    if returned < 0
        || returned as usize >= buffer_size
        || !(returned as usize).is_multiple_of(std::mem::size_of::<G5eProcFdInfo>())
    {
        return None;
    }
    Some(returned as usize / std::mem::size_of::<G5eProcFdInfo>())
}

#[cfg(target_os = "macos")]
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, buffersize: i32) -> i32;
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct G5eProcessMemoryCountersEx {
    _size: u32,
    _page_fault_count: u32,
    _peak_working_set_size: usize,
    working_set_size: usize,
    _quota_peak_paged_pool_usage: usize,
    _quota_paged_pool_usage: usize,
    _quota_peak_non_paged_pool_usage: usize,
    _quota_non_paged_pool_usage: usize,
    _peak_pagefile_usage: usize,
    _pagefile_usage: usize,
    _private_usage: usize,
}

#[cfg(target_os = "windows")]
fn g5e_windows_rss_snapshot() -> Option<(u64, u64)> {
    let mut counters = std::mem::MaybeUninit::<G5eProcessMemoryCountersEx>::zeroed();
    let size = u32::try_from(std::mem::size_of::<G5eProcessMemoryCountersEx>()).ok()?;
    // SAFETY: `counters` points to writable storage for the declared structure.
    unsafe { (*counters.as_mut_ptr())._size = size };
    // SAFETY: GetCurrentProcess returns the current process pseudo-handle.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `counters` is writable and its declared size matches the API buffer.
    let success = unsafe { GetProcessMemoryInfo(process, counters.as_mut_ptr(), size) };
    if success == 0 {
        return None;
    }
    // SAFETY: GetProcessMemoryInfo succeeded and filled the complete structure.
    let counters = unsafe { counters.assume_init() };
    Some((
        u64::try_from(counters.working_set_size).ok()?,
        u64::try_from(counters._peak_working_set_size).ok()?,
    ))
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy)]
struct G5eThreadEntry32 {
    _size: u32,
    _usage: u32,
    _thread_id: u32,
    owner_process_id: u32,
    _base_priority: i32,
    _delta_priority: i32,
    _flags: u32,
}

#[cfg(target_os = "windows")]
fn g5e_windows_thread_count() -> Option<usize> {
    const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
    const ERROR_NO_MORE_FILES: u32 = 18;
    let entry_size = u32::try_from(std::mem::size_of::<G5eThreadEntry32>()).ok()?;

    // SAFETY: CreateToolhelp32Snapshot takes a valid thread-snapshot flag.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot.is_null() || snapshot as isize == -1 {
        return None;
    }
    // SAFETY: GetCurrentProcessId is a read-only query of this process identity.
    let process_id = unsafe { GetCurrentProcessId() };
    let mut entry = std::mem::MaybeUninit::<G5eThreadEntry32>::zeroed();
    // SAFETY: `entry` points to writable storage for the documented structure.
    unsafe { (*entry.as_mut_ptr())._size = entry_size };
    // SAFETY: snapshot is a valid thread snapshot and entry is writable.
    let mut has_entry = unsafe { Thread32First(snapshot, entry.as_mut_ptr()) };
    let mut count = 0usize;
    while has_entry != 0 {
        // SAFETY: Thread32First/Thread32Next succeeded for this iteration.
        let owner_process_id = unsafe { entry.assume_init_ref().owner_process_id };
        if owner_process_id == process_id {
            count = count.saturating_add(1);
        }
        // SAFETY: `entry` remains writable and the API requires its size field.
        unsafe { (*entry.as_mut_ptr())._size = entry_size };
        // SAFETY: snapshot is valid and entry points to writable structure storage.
        has_entry = unsafe { Thread32Next(snapshot, entry.as_mut_ptr()) };
    }
    // SAFETY: GetLastError reads the calling thread's last Win32 error.
    let last_error = unsafe { GetLastError() };
    // SAFETY: `snapshot` is the live handle returned by CreateToolhelp32Snapshot.
    unsafe { CloseHandle(snapshot) };
    if last_error != ERROR_NO_MORE_FILES {
        return None;
    }
    Some(count)
}

#[cfg(target_os = "windows")]
fn g5e_windows_handle_count() -> Option<usize> {
    let mut count = 0u32;
    // SAFETY: GetCurrentProcess returns this process's pseudo-handle and `count`
    // is a writable DWORD output.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `count` points to writable storage for one DWORD.
    let success = unsafe { GetProcessHandleCount(process, &mut count) };
    (success != 0).then_some(count as usize)
}

#[cfg(target_os = "windows")]
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(
        process: *mut c_void,
        counters: *mut G5eProcessMemoryCountersEx,
        size: u32,
    ) -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn GetProcessHandleCount(process: *mut c_void, count: *mut u32) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut c_void;
    fn Thread32First(snapshot: *mut c_void, entry: *mut G5eThreadEntry32) -> i32;
    fn Thread32Next(snapshot: *mut c_void, entry: *mut G5eThreadEntry32) -> i32;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const G5E_RESOURCE_SEMANTICS_JSON: &str = if cfg!(target_os = "linux") {
    "{\"process_rss\":\"Linux /proc/self/status VmRSS\",\"process_threads\":\"Linux /proc/self/task entries\",\"file_descriptors\":\"Linux /proc/self/fd entries; excludes the directory opened for measurement\",\"process_handles\":\"not applicable\"}"
} else if cfg!(target_os = "macos") {
    "{\"process_rss\":\"macOS proc_pidinfo(PROC_PIDTASKINFO).pti_resident_size\",\"process_threads\":\"macOS proc_pidinfo(PROC_PIDTASKINFO).pti_threadnum\",\"file_descriptors\":\"macOS proc_pidinfo(PROC_PIDLISTFDS); null on query failure or when the bounded 4096-entry buffer may truncate\",\"process_handles\":\"not applicable\"}"
} else {
    "{\"process_rss\":\"Windows GetProcessMemoryInfo.WorkingSetSize\",\"process_threads\":\"Windows Toolhelp32 thread snapshot filtered by owner process ID\",\"file_descriptors\":\"unavailable on this fixture; Windows process handles are not file descriptors\",\"process_handles\":\"Windows GetProcessHandleCount; process handles, not file descriptors\"}"
};

#[test]
fn g2_m3_authenticated_requested_key_update_waits_for_real_writable_recovery() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn_with_receive_buffer(
        configure_tls13(pki.identity.duplicate()),
        Some(4 * 1024),
    );
    let sibling_server = ControlServer::spawn(configure_tls13(pki.identity.duplicate()));
    let (mut attempt, mut endpoint, handshake) =
        connect_wss_with_send_buffer(&server, Arc::clone(&client_config), Some(16 * 1024));
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
    let (mut sibling_attempt, mut sibling_endpoint, sibling_handshake) =
        connect_wss(&sibling_server, client_config);
    assert_negotiated(&sibling_handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);

    let initial_game = b"before-requested-key-update";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    // Fill the real loopback TCP path with complete authenticated WSS game
    // writes while the peer's application does not read, until a real socket
    // write returns WouldBlock with encrypted output still queued in the TLS
    // writer. Readiness events do not establish this: BSD kqueue raises its
    // writable edge at a low-water mark before send(2) refuses a write, and
    // either stack frees send-buffer space whenever it processes the peer's
    // ACKs, so a poll of a duplicate descriptor can flip after the fact.
    let filled = fill_until_tls_blocked(&mut attempt, &mut endpoint);
    assert!(filled > 0, "the real socket never refused a game write");
    // Settle the stack. TCP delivers the server's bytes in order, so once the
    // server's next record is readable here, every ACK and window update its
    // stack produced for the client's earlier writes has been processed. Any
    // space those freed is refilled; the stack has nothing more to free when
    // a turn that reads such a record finds the TLS writer still blocked.
    let mut settle_rounds = 0usize;
    loop {
        assert!(
            settle_rounds < MAX_SETTLE_ROUNDS,
            "the client socket never settled into stable backpressure"
        );
        settle_rounds += 1;
        server.send_record(
            ContentType::ApplicationData,
            &server_binary_frame(SETTLE_RECORD),
        );
        take_game_payload(&mut attempt, &mut endpoint, SETTLE_RECORD);
        if fill_until_tls_blocked(&mut attempt, &mut endpoint) == 0 {
            break;
        }
    }
    assert!(endpoint.tls_output_blocked_for_test());
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();

    // Rustls processes the authenticated request while the socket cannot
    // accept a response. No peer input, endpoint read, or new game write is
    // performed while the server drains older outbound records.
    server.send_record(ContentType::Handshake, &KEY_UPDATE_REQUESTED);
    let turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("process the authenticated requested KeyUpdate");
    assert_eq!(turn.read_bytes, 0);
    assert!(endpoint.is_connected());
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(attempt.shared.inbound.is_empty());
    assert!(
        endpoint.tls_output_blocked_for_test(),
        "authenticated KeyUpdate reply must remain queued after the real TLS write WouldBlock"
    );

    // B progresses independently while A waits for the socket to become
    // writable again.
    let sibling_game = b"sibling-progresses-during-key-update";
    sibling_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(sibling_game),
    );
    take_game_payload(&mut sibling_attempt, &mut sibling_endpoint, sibling_game);

    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut observed_game_records = 0usize;
    let mut writable_events = 0usize;
    let mut response = None;
    while response.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for writable KeyUpdate reply after {observed_game_records} game records and {writable_events} writable events"
        );
        let pending_read = server.begin_read_client_record(remaining.min(IO_TIMEOUT));
        let mut read_one_record = false;
        while !read_one_record && response.is_none() {
            match pending_read.recv_timeout(POLL_SLICE) {
                Ok(Ok(Some(record))) => {
                    if record.content_type == ContentType::Handshake {
                        response = Some(record);
                    } else {
                        assert_eq!(
                            record.content_type,
                            ContentType::ApplicationData,
                            "only prior game records precede the KeyUpdate reply"
                        );
                        observed_game_records += 1;
                    }
                    read_one_record = true;
                }
                Ok(Ok(None)) => {
                    panic!("server timed out before receiving the KeyUpdate reply")
                }
                Ok(Err(error)) => panic!(
                    "server could not decrypt client output: {error}; observed game records {observed_game_records}, writable events {writable_events}"
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("server read task ended before the KeyUpdate reply")
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // The server is waiting for client output that only the
                    // writable event can release; service the socket without
                    // blocking, then wait on the server again.
                    attempt
                        .poll
                        .poll(&mut attempt.events, Some(Duration::ZERO))
                        .expect("poll for the real client writable event");
                    for event in &attempt.events {
                        if event.token() == WAKER_TOKEN {
                            continue;
                        }
                        assert_eq!(event.token(), Token(attempt.binding.connection as usize));
                        assert!(
                            event.is_writable(),
                            "backpressure recovery is writable-only"
                        );
                        assert!(
                            !event.is_readable(),
                            "no further peer input is delivered during recovery"
                        );
                        assert!(matches!(
                            endpoint.on_event(false, true, Instant::now()),
                            WssStep::Ready
                        ));
                        let mut turn = endpoint
                            .drive()
                            .expect("flush queued TLS control output on writable readiness");
                        endpoint
                            .apply_interest(attempt.poll.registry())
                            .expect("apply WSS interest after writable recovery");
                        for _ in 0..8 {
                            if turn.stop != TcpStop::LocalWork {
                                break;
                            }
                            turn = endpoint
                                .drive()
                                .expect("finish locally runnable TLS ciphertext flush");
                            endpoint
                                .apply_interest(attempt.poll.registry())
                                .expect("apply WSS interest after a local flush");
                        }
                        assert_ne!(
                            turn.stop,
                            TcpStop::LocalWork,
                            "the bounded local TLS work must settle before another OS event"
                        );
                        assert_eq!(turn.read_bytes, 0);
                        assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
                        assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
                        writable_events += 1;
                    }
                }
            }
        }
    }

    assert!(observed_game_records > 0);
    assert!(writable_events > 0, "reply follows writable-only recovery");
    let response = response.expect("authenticated KeyUpdate reply was observed");
    assert_eq!(response.outer_type, ContentType::ApplicationData);
    assert_eq!(response.content_type, ContentType::Handshake);
    assert_eq!(
        response.payload.as_slice(),
        &[0x18, 0x00, 0x00, 0x01, 0x00][..]
    );
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "exactly one KeyUpdate reply is emitted"
    );

    endpoint.close(attempt.poll.registry());
    sibling_endpoint.close(sibling_attempt.poll.registry());
    server.stop();
    sibling_server.stop();
}

/// Run with a fresh, disjoint receipt path:
///
/// ```text
/// BOT_T14_G5E_ADAPTER_OUTPUT=/abs/path/receipt.json cargo test -p client-adapter --features test-support --test rustls_authenticated_control g5e_tls13_wss_keyupdate_backpressure_memory_receipt -- --ignored --test-threads=1
/// ```
///
/// The receipt covers two local adapter WSS lanes, including the actual
/// authenticated KeyUpdate response recovered from writable-only readiness.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
#[ignore = "opt-in G5-E memory receipt for the actual adapter TLS/WSS path"]
fn g5e_tls13_wss_keyupdate_backpressure_memory_receipt() {
    const EMPTY_BINARY_TURNS: usize = 32;
    const QUEUED_BACKPRESSURE_RECORDS: usize = 8;

    install_g5e_observers();
    let known_allocation_scope_bytes = prove_g5e_scope_allocator_observes_known_allocation();
    let output_path = std::env::var_os("BOT_T14_G5E_ADAPTER_OUTPUT")
        .expect("set a disjoint BOT_T14_G5E_ADAPTER_OUTPUT receipt path");
    let pki = TestPki::new();
    let ticket_metrics = Arc::new(TicketMetrics::default());
    let shared_config_before = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    reset_g5e_allocator_peak();
    let client_config = client_config_with_ticket_metrics(&pki.root, Arc::clone(&ticket_metrics));
    let (shared_config_after, shared_config_peak) = g5e_allocator_snapshot();
    let server_config = configure_tls13_with_tickets(pki.identity.duplicate());
    let server =
        ControlServer::spawn_with_receive_buffer(Arc::clone(&server_config), Some(4 * 1024));
    let sibling_server = ControlServer::spawn(Arc::clone(&server_config));

    let allocator_before_connect = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    let resources_setup = g5e_process_resources();
    reset_g5e_allocator_peak();

    let (mut attempt, mut endpoint, handshake) =
        connect_wss_with_send_buffer(&server, Arc::clone(&client_config), Some(16 * 1024));
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
    assert_eq!(
        ticket_metrics.inserted.load(Ordering::Acquire),
        0,
        "the first lane disables resumption and does not store server tickets"
    );
    let target_binding = attempt.binding;
    let mut ledger = G5eLedgerPeaks::default();
    sample_g5e_wss_ledger(&mut ledger, target_binding, 1);
    let (sibling_attempt, sibling_endpoint, sibling_handshake) =
        connect_wss(&sibling_server, Arc::clone(&client_config));
    assert_negotiated(&sibling_handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
    assert_ne!(attempt.binding, sibling_attempt.binding);
    assert_eq!(
        ticket_metrics.inserted.load(Ordering::Acquire),
        0,
        "the sibling lane also disables resumption and stores no tickets"
    );

    let lane_config = attempt
        .lane
        .config()
        .expect("first real TLS lane retains its worker config generation");
    let sibling_lane_config = sibling_attempt
        .lane
        .config()
        .expect("sibling real TLS lane retains its worker config generation");
    assert!(!Arc::ptr_eq(&client_config, &lane_config));
    assert!(!Arc::ptr_eq(&client_config, &sibling_lane_config));
    assert!(
        !Arc::ptr_eq(&lane_config, &sibling_lane_config),
        "each start_with_test_config lane builds a distinct Arc generation"
    );
    let lane_config_generation_id = soak_observer::client_config_generation_id(&lane_config)
        .expect("first worker config generation was observed");
    let sibling_config_generation_id =
        soak_observer::client_config_generation_id(&sibling_lane_config)
            .expect("sibling worker config generation was observed");
    assert_ne!(lane_config_generation_id, sibling_config_generation_id);
    let config_generation_samples = soak_observer::client_config_generation_samples();
    assert_eq!(config_generation_samples.len(), 2);
    assert!(config_generation_samples.iter().all(|sample| {
        sample.observation_count == 1 && sample.alive && sample.strong_count > 0
    }));
    let config_generation_samples_after_connect = g5e_config_generation_samples_json();
    drop(lane_config);
    drop(sibling_lane_config);
    let input_config_strong_count = Arc::strong_count(&client_config);
    assert_eq!(
        input_config_strong_count, 1,
        "test caller retains only its source config after worker clones are built"
    );

    sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
    let (allocator_after_connect, allocator_connect_peak) = g5e_allocator_snapshot();
    let resources_after_connect = g5e_process_resources();

    let allocator_before_backpressure = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    reset_g5e_allocator_peak();
    let filled = fill_until_tls_blocked_observed(&mut attempt, &mut endpoint, || {
        let pending = sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
        assert!(pending > 0, "blocked TLS writer retains encrypted output");
    });
    assert!(
        filled > 0,
        "the real WSS socket refused an encrypted game write"
    );
    let mut settle_rounds = 0usize;
    loop {
        assert!(
            settle_rounds < MAX_SETTLE_ROUNDS,
            "the client socket never settled into stable backpressure"
        );
        settle_rounds += 1;
        server.send_record(
            ContentType::ApplicationData,
            &server_binary_frame(SETTLE_RECORD),
        );
        take_game_payload(&mut attempt, &mut endpoint, SETTLE_RECORD);
        if fill_until_tls_blocked_observed(&mut attempt, &mut endpoint, || {
            let pending = sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
            assert!(pending > 0, "stable blocked writer retains TLS output");
        }) == 0
        {
            break;
        }
    }
    assert!(endpoint.tls_output_blocked_for_test());
    let bytes_in_before = attempt.shared.bytes_in();
    for _ in 0..QUEUED_BACKPRESSURE_RECORDS {
        attempt
            .shared
            .publish_record(attempt.binding, &BACKPRESSURE_GAME_RECORD)
            .expect("bounded WSS output ring accepts the queued memory sample");
    }
    endpoint.kick_output();
    let bytes_out_before = attempt.shared.bytes_out();
    let queued_pending = sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
    assert!(
        queued_pending > 0,
        "TLS output remains blocked with queued game data"
    );
    assert!(
        ledger.outbound_ring_high_water_bytes
            >= QUEUED_BACKPRESSURE_RECORDS * BACKPRESSURE_GAME_RECORD.len(),
        "ledger samples queued application bytes before the endpoint drains them"
    );

    for _ in 0..EMPTY_BINARY_TURNS {
        server.send_record(ContentType::ApplicationData, &server_binary_frame(&[]));
        let turn = next_readable_turn(&mut attempt, &mut endpoint)
            .expect("process an authenticated empty binary message under backpressure");
        assert_eq!(
            turn.read_bytes, 0,
            "empty binary messages carry no game bytes"
        );
        assert!(attempt.shared.inbound.is_empty());
        assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
        assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
        assert!(
            endpoint.tls_output_blocked_for_test(),
            "the empty binary turn does not clear real TLS socket backpressure"
        );
        sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
    }
    let (allocator_while_blocked, allocator_backpressure_peak) = g5e_allocator_snapshot();
    let resources_while_blocked = g5e_process_resources();

    let allocator_before_key_update = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    reset_g5e_allocator_peak();
    server.send_record(ContentType::Handshake, &KEY_UPDATE_REQUESTED);
    let key_update_turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("process the authenticated TLS 1.3 KeyUpdate request");
    assert_eq!(key_update_turn.read_bytes, 0);
    assert!(endpoint.is_connected());
    assert!(endpoint.tls_output_blocked_for_test());
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert!(attempt.shared.inbound.is_empty());
    let pending_after_key_update = sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
    assert!(
        pending_after_key_update > 0,
        "KeyUpdate response remains queued behind the real blocked TLS writer"
    );
    let (allocator_after_key_update, allocator_key_update_peak) = g5e_allocator_snapshot();
    let resources_after_key_update = g5e_process_resources();
    let recovery_ledger_samples_before = ledger.sample_count;
    let allocator_before_recovery = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    reset_g5e_allocator_peak();
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut recovered_game_records = 0usize;
    let mut writable_recovery_events = 0usize;
    let mut key_update_response = None;
    while key_update_response.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out recovering KeyUpdate reply after {recovered_game_records} game records and {writable_recovery_events} writable events"
        );
        let pending_read = server.begin_read_client_record(remaining.min(IO_TIMEOUT));
        let mut read_one_record = false;
        while !read_one_record && key_update_response.is_none() {
            match pending_read.recv_timeout(POLL_SLICE) {
                Ok(Ok(Some(record))) => {
                    if record.content_type == ContentType::Handshake {
                        key_update_response = Some(record);
                    } else {
                        assert_eq!(
                            record.content_type,
                            ContentType::ApplicationData,
                            "only prior game records precede the KeyUpdate reply"
                        );
                        recovered_game_records += 1;
                    }
                    read_one_record = true;
                    sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
                }
                Ok(Ok(None)) => {
                    panic!("server timed out before receiving the recovered KeyUpdate reply")
                }
                Ok(Err(error)) => panic!(
                    "server could not decrypt recovered client output: {error}; recovered game records {recovered_game_records}, writable events {writable_recovery_events}"
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("server read task ended before the recovered KeyUpdate reply")
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    attempt
                        .poll
                        .poll(&mut attempt.events, Some(Duration::ZERO))
                        .expect("poll for the real client writable recovery event");
                    for event in &attempt.events {
                        if event.token() == WAKER_TOKEN {
                            continue;
                        }
                        assert_eq!(event.token(), Token(attempt.binding.connection as usize));
                        assert!(
                            event.is_writable(),
                            "backpressure recovery is writable-only"
                        );
                        assert!(
                            !event.is_readable(),
                            "no further peer input is delivered during recovery"
                        );
                        assert!(matches!(
                            endpoint.on_event(false, true, Instant::now()),
                            WssStep::Ready
                        ));
                        let mut turn = endpoint
                            .drive()
                            .expect("flush queued TLS control output on writable readiness");
                        endpoint
                            .apply_interest(attempt.poll.registry())
                            .expect("apply WSS interest after writable recovery");
                        for _ in 0..8 {
                            if turn.stop != TcpStop::LocalWork {
                                break;
                            }
                            turn = endpoint
                                .drive()
                                .expect("finish locally runnable TLS ciphertext flush");
                            endpoint
                                .apply_interest(attempt.poll.registry())
                                .expect("apply WSS interest after a local flush");
                        }
                        assert_ne!(
                            turn.stop,
                            TcpStop::LocalWork,
                            "the bounded local TLS work must settle before another OS event"
                        );
                        assert_eq!(turn.read_bytes, 0);
                        assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
                        assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
                        writable_recovery_events += 1;
                    }
                    sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
                }
            }
        }
    }
    assert!(recovered_game_records > 0);
    assert!(
        writable_recovery_events > 0,
        "reply follows writable-only recovery"
    );
    let response = key_update_response.expect("authenticated KeyUpdate reply was observed");
    assert_eq!(response.outer_type, ContentType::ApplicationData);
    assert_eq!(response.content_type, ContentType::Handshake);
    assert_eq!(
        response.payload.as_slice(),
        &[0x18, 0x00, 0x00, 0x01, 0x00][..]
    );
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(!endpoint.tls_output_blocked_for_test());
    let pending_after_recovery = sample_g5e_wss_ledger(&mut ledger, target_binding, 2);
    assert_eq!(pending_after_recovery, 0);
    let recovery_ledger_sample_count = ledger
        .sample_count
        .saturating_sub(recovery_ledger_samples_before);
    assert!(recovery_ledger_sample_count > 0);
    let drain_deadline = Instant::now() + IO_TIMEOUT;
    while let Some(record) = server.read_client_record(Duration::from_millis(250)) {
        assert_eq!(
            record.content_type,
            ContentType::ApplicationData,
            "exactly one KeyUpdate reply is emitted; later records are queued game data"
        );
        assert!(
            Instant::now() < drain_deadline,
            "queued game records must drain within the fixture deadline"
        );
    }
    let (allocator_after_recovery, allocator_recovery_peak) = g5e_allocator_snapshot();
    let resources_after_recovery = g5e_process_resources();

    let tickets_inserted = ticket_metrics.inserted.load(Ordering::Relaxed);
    let tickets_taken = ticket_metrics.taken.load(Ordering::Relaxed);
    let ticket_live_high_water = ticket_metrics
        .allocator_live_after_insert_high_water_bytes
        .load(Ordering::Relaxed);
    let ticket_growth_high_water = ticket_metrics
        .allocator_live_growth_per_insert_high_water_bytes
        .load(Ordering::Relaxed);
    let allocator_before_cleanup = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    reset_g5e_allocator_peak();
    endpoint.close(attempt.poll.registry());
    sibling_endpoint.close(sibling_attempt.poll.registry());
    drop(attempt);
    drop(sibling_attempt);
    server.stop();
    sibling_server.stop();
    drop(server_config);
    drop(client_config);
    drop(ticket_metrics);
    drop(pki);
    let (allocator_after_cleanup, allocator_cleanup_peak) = g5e_allocator_snapshot();
    let active_connections_after_cleanup = wss_ledger_samples().len();
    assert_eq!(
        active_connections_after_cleanup, 0,
        "test teardown retires both WSS ledger registrations"
    );
    assert_eq!(tickets_inserted, 0);
    assert_eq!(tickets_taken, 0);
    assert_eq!(ticket_live_high_water, 0);
    assert_eq!(ticket_growth_high_water, 0);

    let allocation_scope_samples = G5E_SCOPE_SAMPLES
        .get()
        .expect("allocation scope samples initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    assert!(!allocation_scope_samples.is_empty());
    assert!(allocation_scope_samples
        .iter()
        .all(|sample| sample.completed));
    let game_tls_connection_scopes = allocation_scope_samples
        .iter()
        .filter(|sample| sample.event.kind == AllocationScopeKind::GameTlsConnection)
        .collect::<Vec<_>>();
    assert_eq!(game_tls_connection_scopes.len(), 2);
    assert!(game_tls_connection_scopes.iter().all(|sample| {
        matches!(
            sample.event.config_generation_id,
            Some(id) if id == lane_config_generation_id || id == sibling_config_generation_id
        )
    }));
    let game_tls_verification_scopes = allocation_scope_samples
        .iter()
        .filter(|sample| sample.event.kind == AllocationScopeKind::GameTlsVerification)
        .collect::<Vec<_>>();
    assert!(game_tls_verification_scopes.len() >= 2);
    assert!(game_tls_verification_scopes.iter().all(|sample| {
        matches!(
            sample.event.config_generation_id,
            Some(id) if id == lane_config_generation_id || id == sibling_config_generation_id
        )
    }));
    assert!(!G5E_SCOPE_OVERFLOW.load(Ordering::Acquire));

    let config_generation_events = G5E_CONFIG_GENERATION_EVENTS
        .get()
        .expect("config generation events initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(config_generation_events.len(), 2);
    assert!(config_generation_events.iter().all(|event| {
        event.kind == ClientConfigGenerationEventKind::Created && event.strong_count == 1
    }));
    assert!(config_generation_events
        .iter()
        .any(|event| event.generation_id == lane_config_generation_id));
    assert!(config_generation_events
        .iter()
        .any(|event| event.generation_id == sibling_config_generation_id));
    assert!(!G5E_CONFIG_GENERATION_OVERFLOW.load(Ordering::Acquire));

    let worker_stack_samples = G5E_WORKER_STACK_SAMPLES
        .get()
        .expect("worker stack samples initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let game_tls_worker_tids = worker_stack_samples
        .iter()
        .filter(|sample| {
            sample.kind == NetworkWorkerKind::GameTls
                && sample.point == WorkerObservationPoint::Started
        })
        .map(|sample| sample.sample.tid)
        .collect::<Vec<_>>();
    assert_eq!(game_tls_worker_tids.len(), 2);
    for tid in game_tls_worker_tids {
        assert!(worker_stack_samples.iter().any(|sample| {
            sample.kind == NetworkWorkerKind::GameTls
                && sample.point == WorkerObservationPoint::BeforeVerify
                && sample.sample.tid == tid
        }));
        assert!(worker_stack_samples.iter().any(|sample| {
            sample.kind == NetworkWorkerKind::GameTls
                && sample.point == WorkerObservationPoint::AfterVerify
                && sample.sample.tid == tid
        }));
    }
    assert!(worker_stack_samples.iter().all(|sample| {
        sample.sample.tid != 0
            && sample.sample.stack_low_address.is_some()
            && sample.sample.stack_high_address_exclusive.is_some()
    }));
    if cfg!(target_os = "windows") {
        assert!(worker_stack_samples.iter().all(|sample| {
            sample.sample.reserved_bytes.is_some()
                && sample.sample.committed_bytes.is_some()
                && sample.sample.guard_bytes.is_some()
        }));
    }
    assert!(!G5E_WORKER_STACK_OVERFLOW.load(Ordering::Acquire));
    assert_eq!(G5E_WORKER_STACK_CAPTURE_FAILURES.load(Ordering::Acquire), 0);

    let allocation_scope_samples_json = g5e_allocation_scope_samples_json();
    let config_generation_events_json = g5e_config_generation_events_json();
    let config_generation_samples_after_cleanup_json = g5e_config_generation_samples_json();
    let worker_stack_samples_json = g5e_worker_stack_samples_json();
    let allocation_scope_sample_count = allocation_scope_samples.len();
    let config_generation_event_count = config_generation_events.len();
    let worker_stack_sample_count = worker_stack_samples.len();

    let file_descriptors_setup_json = optional_usize_json(resources_setup.file_descriptors);
    let file_descriptors_after_connect_json =
        optional_usize_json(resources_after_connect.file_descriptors);
    let file_descriptors_while_blocked_json =
        optional_usize_json(resources_while_blocked.file_descriptors);
    let file_descriptors_after_key_update_json =
        optional_usize_json(resources_after_key_update.file_descriptors);
    let file_descriptors_after_recovery_json =
        optional_usize_json(resources_after_recovery.file_descriptors);
    let process_handles_setup_json = optional_usize_json(resources_setup.process_handles);
    let process_handles_after_connect_json =
        optional_usize_json(resources_after_connect.process_handles);
    let process_handles_while_blocked_json =
        optional_usize_json(resources_while_blocked.process_handles);
    let process_handles_after_key_update_json =
        optional_usize_json(resources_after_key_update.process_handles);
    let process_handles_after_recovery_json =
        optional_usize_json(resources_after_recovery.process_handles);
    let receipt = format!(
        concat!(
            "{{\"schema\":\"t1.4-g5e-rustls-adapter-receipt-v3\",",
            "\"target_os\":\"{}\",\"target_arch\":\"{}\",",
            "\"measurement_subject\":\"actual client-adapter GameTlsLane and WssEndpoint with local TLS 1.3 fixture\",",
            "\"client_verifier\":\"platform verifier with generated fixture CA root\",",
            "\"connection_count\":2,\"client_config_identity_count\":{config_identity_count},",
            "\"shared_config_process_requested_before_build_bytes\":{shared_config_before},",
            "\"shared_config_process_requested_after_build_bytes\":{shared_config_after},",
            "\"shared_config_process_requested_peak_during_build_bytes\":{shared_config_peak},",
            "\"shared_config_process_requested_live_delta_bytes\":{shared_config_delta},",
            "\"shared_config_process_requested_peak_delta_bytes\":{shared_config_peak_delta},",
            "\"shared_config_measurement\":\"one caller source ClientConfig including fixture trust roots, platform verifier and ticket store; measured after PKI creation and before server fixture setup; process-wide requested bytes, not retained bytes owned by this config\",",
            "\"initial_backpressure_records\":{},\"stable_backpressure_settle_rounds\":{},",
            "\"client_config_generation_count\":{config_generation_count},\"source_config_strong_count_after_connect\":{},",
            "\"client_config_generation_ids\":[{lane_config_generation_id},{sibling_config_generation_id}],",
            "\"client_config_generation_events\":{config_generation_events_json},",
            "\"client_config_generation_samples_after_connect\":{config_generation_samples_after_connect},",
            "\"client_config_generation_samples_after_cleanup\":{config_generation_samples_after_cleanup_json},",
            "\"tls13_tickets_configured_per_connection\":2,\"tls13_tickets_inserted\":{},",
            "\"tls13_tickets_taken_for_resumption\":{},",
            "\"allocator_live_after_ticket_insert_high_water_bytes\":{},",
            "\"allocator_live_growth_per_ticket_insert_high_water_bytes\":{},",
            "\"allocator_requested_live_before_connect_bytes\":{},",
            "\"allocator_requested_live_after_connect_bytes\":{},",
            "\"allocator_requested_peak_during_connect_bytes\":{},",
            "\"allocator_peak_delta_during_connect_bytes\":{},",
            "\"allocator_requested_live_while_blocked_bytes\":{},",
            "\"allocator_requested_peak_during_backpressure_bytes\":{},",
            "\"allocator_peak_delta_during_backpressure_bytes\":{},",
            "\"allocator_requested_live_after_key_update_bytes\":{},",
            "\"allocator_requested_peak_during_key_update_bytes\":{},",
            "\"allocator_peak_delta_during_key_update_bytes\":{},",
            "\"allocator_requested_live_after_recovery_bytes\":{},",
            "\"allocator_requested_peak_during_recovery_bytes\":{},",
            "\"allocator_peak_delta_during_recovery_bytes\":{},",
            "\"allocator_requested_live_before_cleanup_bytes\":{},",
            "\"allocator_requested_live_after_cleanup_bytes\":{},",
            "\"allocator_peak_delta_during_cleanup_bytes\":{},",
            "\"rss_current_setup_bytes\":{},\"rss_current_after_connect_bytes\":{},",
            "\"rss_current_while_blocked_bytes\":{},\"rss_current_after_key_update_bytes\":{},",
            "\"rss_current_after_recovery_bytes\":{},",
            "\"tasks_setup\":{},\"tasks_after_connect\":{},\"tasks_while_blocked\":{},",
            "\"tasks_after_key_update\":{},\"tasks_after_recovery\":{},",
            "\"fds_setup\":{},\"fds_after_connect\":{},\"fds_while_blocked\":{},",
            "\"fds_after_key_update\":{},\"fds_after_recovery\":{},",
            "\"process_handles_setup\":{process_handles_setup_json},",
            "\"process_handles_after_connect\":{process_handles_after_connect_json},",
            "\"process_handles_while_blocked\":{process_handles_while_blocked_json},",
            "\"process_handles_after_key_update\":{process_handles_after_key_update_json},",
            "\"process_handles_after_recovery\":{process_handles_after_recovery_json},",
            "\"resource_semantics\":{resource_semantics_json},",
            "\"wss_ledger_sample_count\":{},\"wss_ledger_active_connections\":{},",
            "\"wss_ledger_reserved_total_bytes\":{},",
            "\"wss_pending_tls_output_high_water_bytes\":{},",
            "\"wss_target_pending_tls_output_after_key_update_bytes\":{},",
            "\"wss_target_pending_tls_output_after_recovery_bytes\":{},",
            "\"wss_outbound_ring_high_water_bytes\":{},",
            "\"wss_inbound_ring_high_water_bytes\":{},",
            "\"wss_ledger_overrun\":false,\"empty_binary_turns_while_blocked\":{},",
            "\"key_update_request_processed_while_blocked\":true,",
            "\"key_update_response_recovery_observed\":true,",
            "\"key_update_recovery_game_records_observed\":{},",
            "\"key_update_recovery_writable_events\":{},",
            "\"wss_recovery_ledger_sample_count\":{},",
            "\"active_connections_after_cleanup\":{},",
            "\"allocation_scope_sample_count\":{allocation_scope_sample_count},",
            "\"allocator_scope_known_request_bytes\":{known_allocation_scope_bytes},",
            "\"allocation_scope_observer_overflow\":{allocation_scope_overflow},",
            "\"allocation_scope_samples\":{allocation_scope_samples_json},",
            "\"client_config_generation_event_count\":{config_generation_event_count},",
            "\"client_config_generation_observer_overflow\":{config_generation_overflow},",
            "\"worker_stack_sample_count\":{worker_stack_sample_count},",
            "\"worker_stack_observer_overflow\":{worker_stack_overflow},",
            "\"worker_stack_capture_failures\":{worker_stack_capture_failures},",
            "\"worker_stack_samples\":{worker_stack_samples_json},",
            "\"stack_measurement\":\"per-worker OS-reported stack interval; each measurement_source states reserved, committed, resident, and guard availability\",",
            "\"scope_measurement\":\"gross successful alloc, alloc_zeroed, and realloc request bytes while an operation scope is active; exclusive direct and inclusive nested; not retained-by-scope live bytes\",",
            "\"v8_metrics\":\"not_applicable_in_client_adapter_test_process\",",
            "\"allocation_measurement\":\"global requested live/peak includes the whole test process; scoped requests are not allocation ownership or usable capacity\",",
            "\"configuration_scope\":\"caller config cloned into two distinct GameTlsLane worker generations; lane resumption disabled; server tickets are neither stored nor resumed\"}}\n"
        ),
        std::env::consts::OS,
        std::env::consts::ARCH,
        filled,
        settle_rounds,
        input_config_strong_count,
        tickets_inserted,
        tickets_taken,
        ticket_live_high_water,
        ticket_growth_high_water,
        allocator_before_connect,
        allocator_after_connect,
        allocator_connect_peak,
        allocator_connect_peak.saturating_sub(allocator_before_connect),
        allocator_while_blocked,
        allocator_backpressure_peak,
        allocator_backpressure_peak.saturating_sub(allocator_before_backpressure),
        allocator_after_key_update,
        allocator_key_update_peak,
        allocator_key_update_peak.saturating_sub(allocator_before_key_update),
        allocator_after_recovery,
        allocator_recovery_peak,
        allocator_recovery_peak.saturating_sub(allocator_before_recovery),
        allocator_before_cleanup,
        allocator_after_cleanup,
        allocator_cleanup_peak.saturating_sub(allocator_before_cleanup),
        resources_setup.rss_bytes,
        resources_after_connect.rss_bytes,
        resources_while_blocked.rss_bytes,
        resources_after_key_update.rss_bytes,
        resources_after_recovery.rss_bytes,
        resources_setup.task_count,
        resources_after_connect.task_count,
        resources_while_blocked.task_count,
        resources_after_key_update.task_count,
        resources_after_recovery.task_count,
        file_descriptors_setup_json,
        file_descriptors_after_connect_json,
        file_descriptors_while_blocked_json,
        file_descriptors_after_key_update_json,
        file_descriptors_after_recovery_json,
        ledger.sample_count,
        ledger.connection_count,
        ledger.total_reserved_bytes,
        ledger.pending_tls_output_high_water_bytes,
        pending_after_key_update,
        pending_after_recovery,
        ledger.outbound_ring_high_water_bytes,
        ledger.inbound_ring_high_water_bytes,
        EMPTY_BINARY_TURNS,
        recovered_game_records,
        writable_recovery_events,
        recovery_ledger_sample_count,
        active_connections_after_cleanup,
        config_identity_count = config_generation_samples.len(),
        shared_config_before = shared_config_before,
        shared_config_after = shared_config_after,
        shared_config_peak = shared_config_peak,
        shared_config_delta = shared_config_after as i128 - shared_config_before as i128,
        shared_config_peak_delta = shared_config_peak.saturating_sub(shared_config_before),
        config_generation_count = config_generation_samples.len(),
        lane_config_generation_id = lane_config_generation_id,
        sibling_config_generation_id = sibling_config_generation_id,
        config_generation_events_json = config_generation_events_json,
        config_generation_samples_after_connect = config_generation_samples_after_connect,
        config_generation_samples_after_cleanup_json = config_generation_samples_after_cleanup_json,
        allocation_scope_sample_count = allocation_scope_sample_count,
        allocation_scope_overflow = G5E_SCOPE_OVERFLOW.load(Ordering::Acquire),
        allocation_scope_samples_json = allocation_scope_samples_json,
        config_generation_event_count = config_generation_event_count,
        config_generation_overflow = G5E_CONFIG_GENERATION_OVERFLOW.load(Ordering::Acquire),
        worker_stack_sample_count = worker_stack_sample_count,
        worker_stack_overflow = G5E_WORKER_STACK_OVERFLOW.load(Ordering::Acquire),
        worker_stack_capture_failures = G5E_WORKER_STACK_CAPTURE_FAILURES.load(Ordering::Acquire),
        worker_stack_samples_json = worker_stack_samples_json,
        known_allocation_scope_bytes = known_allocation_scope_bytes,
        process_handles_setup_json = process_handles_setup_json,
        process_handles_after_connect_json = process_handles_after_connect_json,
        process_handles_while_blocked_json = process_handles_while_blocked_json,
        process_handles_after_key_update_json = process_handles_after_key_update_json,
        process_handles_after_recovery_json = process_handles_after_recovery_json,
        resource_semantics_json = G5E_RESOURCE_SEMANTICS_JSON,
    );
    let mut receipt_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .expect("create a unique G5-E adapter receipt file");
    receipt_file
        .write_all(receipt.as_bytes())
        .expect("write the G5-E adapter receipt");
    println!("G5E_MEMORY_RECEIPT {receipt}");
}

/// Differential test-only receipt. Both server and client execute in this
/// process, so a server-side post-accept allocation is still an explicit
/// confounder; the fitted slope is not silently relabeled client-only memory.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
#[ignore = "opt-in actual-adapter WSS 1/2/4/8 difference receipt"]
fn g5e_adapter_wss_connection_difference_receipt() {
    const LANES: usize = 8;
    const HANDSHAKE_REPEATS: usize = 5;

    let output_path = std::env::var_os("BOT_T14_G5E_DIFFERENCE_OUTPUT")
        .expect("set an absent BOT_T14_G5E_DIFFERENCE_OUTPUT receipt path");
    install_g5e_observers();
    let mut servers = Vec::with_capacity(LANES);
    let mut lanes = Vec::with_capacity(LANES);
    let mut points = Vec::with_capacity(LANES);
    let mut handshakes = Vec::with_capacity(HANDSHAKE_REPEATS);
    let process_before_setup = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    let pki = TestPki::new();
    let ticket_metrics = Arc::new(TicketMetrics::default());
    let client_config = client_config_with_ticket_metrics(&pki.root, Arc::clone(&ticket_metrics));
    let server_config = configure_tls13_with_tickets(pki.identity.duplicate());
    let process_after_shared_setup = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    for _ in 0..LANES {
        servers.push(ControlServer::spawn(Arc::clone(&server_config)));
    }
    let fixture_only_live = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    let fixture_setup_delta = fixture_only_live as i128 - process_after_shared_setup as i128;
    for (index, server) in servers.iter().enumerate() {
        let (attempt, endpoint, handshake) = connect_wss(server, Arc::clone(&client_config));
        assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
        drop(handshake);
        lanes.push((attempt, endpoint));
        assert_eq!(wss_ledger_samples().len(), index + 1);
        let live = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
        let corrected = live as i128 - process_before_setup as i128 - fixture_setup_delta;
        let resources = g5e_process_resources();
        points.push((index + 1, live, corrected, resources));
    }
    for ((attempt, endpoint), server) in lanes.into_iter().zip(servers) {
        endpoint.close(attempt.poll.registry());
        drop(attempt);
        server.stop();
    }
    assert!(wss_ledger_samples().is_empty());

    for _ in 0..HANDSHAKE_REPEATS {
        let server = ControlServer::spawn(Arc::clone(&server_config));
        let before = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
        reset_g5e_allocator_peak();
        let (attempt, endpoint, handshake) = connect_wss(&server, Arc::clone(&client_config));
        assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
        drop(handshake);
        let (after, peak) = g5e_allocator_snapshot();
        handshakes.push((before, after, peak, peak.saturating_sub(before.max(after))));
        endpoint.close(attempt.poll.registry());
        drop(attempt);
        server.stop();
        assert!(wss_ledger_samples().is_empty());
    }
    assert_eq!(ticket_metrics.inserted.load(Ordering::Acquire), 0);
    assert_eq!(ticket_metrics.taken.load(Ordering::Acquire), 0);
    assert!(!G5E_SCOPE_OVERFLOW.load(Ordering::Acquire));
    assert!(!G5E_CONFIG_GENERATION_OVERFLOW.load(Ordering::Acquire));
    assert!(!G5E_WORKER_STACK_OVERFLOW.load(Ordering::Acquire));
    assert_eq!(G5E_WORKER_STACK_CAPTURE_FAILURES.load(Ordering::Acquire), 0);

    let selected = [points[0], points[1], points[3], points[7]];
    let sum_x = 15.0_f64;
    let sum_xx = 85.0_f64;
    let sum_y: f64 = selected
        .iter()
        .map(|(_, _, corrected, _)| *corrected as f64)
        .sum();
    let sum_xy: f64 = selected
        .iter()
        .map(|(count, _, corrected, _)| *count as f64 * *corrected as f64)
        .sum();
    let slope = (4.0 * sum_xy - sum_x * sum_y) / (4.0 * sum_xx - sum_x * sum_x);
    let intercept = (sum_y - slope * sum_x) / 4.0;
    let residuals = selected
        .map(|(count, _, corrected, _)| corrected as f64 - intercept - slope * count as f64);
    let spread_min = handshakes.iter().map(|sample| sample.3).min().unwrap();
    let spread_max = handshakes.iter().map(|sample| sample.3).max().unwrap();
    let mut receipt = format!(
        "{{\"schema\":\"t1.4-g5e-adapter-difference-v1\",\"target_os\":\"{}\",\"target_arch\":\"{}\",\"measurement_subject\":\"actual GameTlsLane and WssEndpoint against eight prestarted in-process TLS server fixtures\",\"process_live_before_setup_bytes\":{},\"process_live_after_shared_setup_bytes\":{},\"fixture_only_live_before_connections_bytes\":{},\"fixture_setup_live_delta_bytes\":{},\"fit\":\"ordinary least squares on N=1,2,4,8 after subtracting process-before-setup and eight server-fixture setup delta\",\"retained_slope_bytes_per_connection\":{},\"retained_shared_intercept_bytes\":{},\"residuals_bytes\":[{},{},{},{}],\"server_post_accept_cost\":\"unresolved: TLS/WebSocket server connections remain live in this counting process; fixture-only pre-accept baseline cannot subtract their per-connection cost\",\"idle_steady_points\":[",
        std::env::consts::OS,
        std::env::consts::ARCH,
        process_before_setup,
        process_after_shared_setup,
        fixture_only_live,
        fixture_setup_delta,
        slope,
        intercept,
        residuals[0],
        residuals[1],
        residuals[2],
        residuals[3],
    );
    for (index, (count, live, corrected, resources)) in points.iter().enumerate() {
        if index != 0 {
            receipt.push(',');
        }
        write!(
            receipt,
            "{{\"connections\":{count},\"process_live_requested_bytes\":{live},\"fixture_setup_corrected_bytes\":{corrected},\"rss_current_bytes\":{},\"tasks\":{},\"fds\":{},\"process_handles\":{}}}",
            resources.rss_bytes,
            resources.task_count,
            optional_usize_json(resources.file_descriptors),
            optional_usize_json(resources.process_handles),
        )
        .expect("write observed idle WSS point");
    }
    receipt.push_str("],\"isolated_handshake_runs\":[");
    for (index, (before, after, peak, excess)) in handshakes.iter().enumerate() {
        if index != 0 {
            receipt.push(',');
        }
        write!(
            receipt,
            "{{\"iteration\":{},\"before_live_bytes\":{before},\"after_live_bytes\":{after},\"peak_live_bytes\":{peak},\"peak_above_max_endpoint_bytes\":{excess}}}",
            index + 1
        )
        .expect("write isolated handshake sample");
    }
    writeln!(
        receipt,
        "],\"handshake_peak_excess_spread_bytes\":{{\"min\":{spread_min},\"max\":{spread_max},\"range\":{}}},\"ticket_store_insertions\":0,\"ticket_parse_cost\":\"unresolved: the excess includes client handshake, server post-accept and upgrade allocations; no ticket-parse-specific scope\",\"worker_stack_capture_count\":{},\"worker_config_generation_count\":{},\"allocation_scope_count\":{},\"sample_semantics\":\"GlobalAlloc requested-layout live bytes, not usable capacity; source-known 250760-byte logical reserve is never an actual-cost oracle\"}}",
        spread_max - spread_min,
        G5E_WORKER_STACK_SAMPLES
            .get()
            .expect("worker stack observer initialized")
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len(),
        soak_observer::client_config_generation_samples().len(),
        G5E_SCOPE_SAMPLES
            .get()
            .expect("allocation scope observer initialized")
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .flatten()
            .count(),
    )
    .expect("write difference receipt summary");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .expect("create unique G5-E difference receipt");
    file.write_all(receipt.as_bytes())
        .expect("write G5-E difference receipt");
    file.sync_all().expect("sync G5-E difference receipt");
}

/// The child process owns all server TLS and WebSocket allocations. Its
/// certificate authority is transmitted as public DER; no server key enters
/// the measured client process.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct ExternalG5eFixture {
    child: Child,
    stopped: bool,
    _stdout: io::BufReader<std::process::ChildStdout>,
    root: CertificateDer<'static>,
    addresses: Vec<SocketAddr>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl ExternalG5eFixture {
    fn start() -> Self {
        let mut child = Command::new(std::env::current_exe().expect("locate this test binary"))
            .args([
                "g5e_external_server_fixture",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BOT_T14_G5E_CHILD_SERVER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the isolated local server fixture");
        let mut stdout = io::BufReader::new(child.stdout.take().expect("capture fixture stdout"));
        let mut line = String::new();
        let marker = loop {
            line.clear();
            assert_ne!(
                stdout.read_line(&mut line).expect("read fixture startup"),
                0,
                "isolated server fixture exited without readiness"
            );
            if let Some((_, marker)) = line.split_once("G5E_FIXTURE_READY ") {
                break marker.trim_end().to_owned();
            }
        };
        let (root_hex, ports) = marker
            .split_once('|')
            .expect("fixture readiness includes root DER and ports");
        let mut pairs = root_hex.as_bytes().chunks_exact(2);
        let root = pairs
            .by_ref()
            .map(|pair| {
                u8::from_str_radix(
                    std::str::from_utf8(pair).expect("root DER is ASCII hex"),
                    16,
                )
                .expect("root DER contains hex bytes")
            })
            .collect::<Vec<_>>();
        assert!(!root.is_empty());
        assert!(pairs.remainder().is_empty(), "root DER hex has even length");
        let addresses = ports
            .split(',')
            .map(|port| {
                SocketAddr::from((
                    Ipv4Addr::LOCALHOST,
                    port.parse::<u16>().expect("fixture port is numeric"),
                ))
            })
            .collect::<Vec<_>>();
        assert_eq!(addresses.len(), 13, "eight live and five transient lanes");
        Self {
            child,
            stopped: false,
            _stdout: stdout,
            root: CertificateDer::from(root),
            addresses,
        }
    }

    fn stop(mut self) {
        let mut stdin = self.child.stdin.take().expect("fixture stdin is open");
        stdin.write_all(b"stop\n").expect("stop fixture");
        drop(stdin);
        let status = self.child.wait().expect("reap isolated fixture");
        self.stopped = true;
        assert!(
            status.success(),
            "isolated fixture completed without a server failure"
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Drop for ExternalG5eFixture {
    fn drop(&mut self) {
        // On an assertion failure a prestarted listener may still be waiting
        // in accept(). Kill only our test child rather than orphaning it.
        if !self.stopped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
#[ignore = "internal child fixture, launched by the client-only G5-E receipt"]
fn g5e_external_server_fixture() {
    assert_eq!(
        std::env::var("BOT_T14_G5E_CHILD_SERVER")
            .expect("only the parent receipt launches this fixture"),
        "1"
    );
    let pki = TestPki::new();
    let config = configure_tls13_with_tickets(pki.identity.duplicate());
    let servers = (0..13)
        .map(|_| ControlServer::spawn(Arc::clone(&config)))
        .collect::<Vec<_>>();
    let mut marker = String::from("G5E_FIXTURE_READY ");
    for byte in pki.root.as_ref() {
        write!(marker, "{byte:02x}").expect("encode public test CA root");
    }
    marker.push('|');
    for (index, server) in servers.iter().enumerate() {
        if index != 0 {
            marker.push(',');
        }
        write!(marker, "{}", server.address.port()).expect("encode local fixture port");
    }
    println!("{marker}");
    io::stdout().flush().expect("publish fixture readiness");
    let mut command = String::new();
    let length = io::stdin()
        .read_line(&mut command)
        .expect("read parent fixture command");
    if length == 0 || command.trim() != "stop" {
        // Never join a listener that has not accepted when the parent aborted.
        std::process::exit(1);
    }
    for server in servers {
        server.stop();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn connect_external_g5e_wss(
    address: SocketAddr,
    config: Arc<ClientConfig>,
) -> (LaneAttempt, WssEndpoint) {
    let mut attempt = LaneAttempt::start_with_send_buffer(address, config, None);
    let accepted = attempt.wait_for_return();
    let mut endpoint = attempt.accept_success(accepted, &format!("localhost:{}", address.port()));
    complete_wss_upgrade(&mut attempt, &mut endpoint);
    assert!(
        endpoint.is_connected(),
        "the isolated WSS upgrade completed"
    );
    drain_local_work_to_idle(&mut attempt, &mut endpoint);
    (attempt, endpoint)
}

/// Test-only client process measurement. The server fixture is another
/// process; the in-process difference receipt remains the combined upper
/// bound, not an attempted per-allocation ownership oracle.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
#[ignore = "opt-in client-only actual-adapter WSS 1/2/4/8 difference receipt"]
fn g5e_client_only_wss_connection_difference_receipt() {
    const LANES: usize = 8;
    const HANDSHAKE_REPEATS: usize = 5;

    let output_path = std::env::var_os("BOT_T14_G5E_CLIENT_ONLY_OUTPUT")
        .expect("set an absent BOT_T14_G5E_CLIENT_ONLY_OUTPUT receipt path");
    install_g5e_observers();
    let fixture = ExternalG5eFixture::start();
    let fixture_pid = fixture.child.id();
    assert_ne!(fixture_pid, std::process::id());
    let mut lanes = Vec::with_capacity(LANES);
    let mut points = Vec::with_capacity(LANES);
    let mut handshakes = Vec::with_capacity(HANDSHAKE_REPEATS);
    let baseline_resources = g5e_process_resources();
    let process_before_setup = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    let ticket_metrics = Arc::new(TicketMetrics::default());
    let client_config =
        client_config_with_ticket_metrics(&fixture.root, Arc::clone(&ticket_metrics));
    let process_after_shared_setup = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
    for (index, &address) in fixture.addresses[..LANES].iter().enumerate() {
        lanes.push(connect_external_g5e_wss(
            address,
            Arc::clone(&client_config),
        ));
        assert_eq!(wss_ledger_samples().len(), index + 1);
        let live = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
        let corrected = live as i128 - process_before_setup as i128;
        let resources = g5e_process_resources();
        points.push((index + 1, live, corrected, resources));
    }
    for (attempt, endpoint) in lanes {
        endpoint.close(attempt.poll.registry());
        drop(attempt);
    }
    assert!(wss_ledger_samples().is_empty());

    for &address in &fixture.addresses[LANES..LANES + HANDSHAKE_REPEATS] {
        let before = G5E_ALLOCATOR_LIVE_BYTES.load(Ordering::Acquire);
        reset_g5e_allocator_peak();
        let (attempt, endpoint) = connect_external_g5e_wss(address, Arc::clone(&client_config));
        let (after, peak) = g5e_allocator_snapshot();
        let resources = g5e_process_resources();
        handshakes.push((
            before,
            after,
            peak,
            peak.saturating_sub(before.max(after)),
            resources,
        ));
        endpoint.close(attempt.poll.registry());
        drop(attempt);
        assert!(wss_ledger_samples().is_empty());
    }
    assert_eq!(ticket_metrics.inserted.load(Ordering::Acquire), 0);
    assert_eq!(ticket_metrics.taken.load(Ordering::Acquire), 0);
    assert!(!G5E_SCOPE_OVERFLOW.load(Ordering::Acquire));
    assert!(!G5E_CONFIG_GENERATION_OVERFLOW.load(Ordering::Acquire));
    assert!(!G5E_WORKER_STACK_OVERFLOW.load(Ordering::Acquire));
    assert_eq!(G5E_WORKER_STACK_CAPTURE_FAILURES.load(Ordering::Acquire), 0);
    fixture.stop();

    let selected = [points[0], points[1], points[3], points[7]];
    let sum_x = 15.0_f64;
    let sum_xx = 85.0_f64;
    let sum_y: f64 = selected
        .iter()
        .map(|(_, _, corrected, _)| *corrected as f64)
        .sum();
    let sum_xy: f64 = selected
        .iter()
        .map(|(count, _, corrected, _)| *count as f64 * *corrected as f64)
        .sum();
    let slope = (4.0 * sum_xy - sum_x * sum_y) / (4.0 * sum_xx - sum_x * sum_x);
    let intercept = (sum_y - slope * sum_x) / 4.0;
    let residuals = selected
        .map(|(count, _, corrected, _)| corrected as f64 - intercept - slope * count as f64);
    let spread_min = handshakes.iter().map(|sample| sample.3).min().unwrap();
    let spread_max = handshakes.iter().map(|sample| sample.3).max().unwrap();
    let observed_rss_max = points
        .iter()
        .map(|(_, _, _, resources)| resources.rss_bytes)
        .max()
        .unwrap();
    let mut receipt = format!(
        "{{\"schema\":\"t1.4-g5e-client-only-difference-v1\",\"target_os\":\"{}\",\"target_arch\":\"{}\",\"measurement_subject\":\"actual GameTlsLane and WssEndpoint in client process; thirteen TLS server fixtures in a separate child process\",\"client_process_pid\":{},\"server_process_pid\":{},\"process_live_before_shared_setup_bytes\":{},\"process_live_after_shared_setup_bytes\":{},\"shared_setup_live_delta_bytes\":{},\"fit\":\"ordinary least squares on N=1,2,4,8 client-process GlobalAlloc live requested bytes minus post-fixture client-process baseline\",\"retained_slope_bytes_per_connection\":{},\"retained_shared_intercept_bytes\":{},\"residuals_bytes\":[{},{},{},{}],\"server_post_accept_cost\":\"excluded from the measured client process; the separate child owns all server TLS and WebSocket state\",\"points\":[",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id(),
        fixture_pid,
        process_before_setup,
        process_after_shared_setup,
        process_after_shared_setup as i128 - process_before_setup as i128,
        slope,
        intercept,
        residuals[0],
        residuals[1],
        residuals[2],
        residuals[3],
    );
    for (index, (count, live, corrected, resources)) in points.iter().enumerate() {
        if index != 0 {
            receipt.push(',');
        }
        write!(
            receipt,
            "{{\"connections\":{count},\"client_process_live_requested_bytes\":{live},\"client_process_baseline_corrected_bytes\":{corrected},\"rss_current_bytes\":{},\"rss_high_water_bytes\":{},\"tasks\":{},\"fds\":{},\"process_handles\":{}}}",
            resources.rss_bytes,
            resources
                .rss_high_water_bytes
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
            resources.task_count,
            optional_usize_json(resources.file_descriptors),
            optional_usize_json(resources.process_handles),
        )
        .expect("write observed client-only WSS point");
    }
    receipt.push_str("],\"isolated_handshake_runs\":[");
    for (index, (before, after, peak, excess, resources)) in handshakes.iter().enumerate() {
        if index != 0 {
            receipt.push(',');
        }
        write!(
            receipt,
            "{{\"iteration\":{},\"before_live_bytes\":{before},\"after_live_bytes\":{after},\"peak_live_bytes\":{peak},\"peak_above_max_endpoint_bytes\":{excess},\"rss_current_bytes\":{},\"rss_high_water_bytes\":{}}}",
            index + 1,
            resources.rss_bytes,
            resources
                .rss_high_water_bytes
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        )
        .expect("write isolated client handshake sample");
    }
    writeln!(
        receipt,
        "],\"handshake_peak_excess_spread_bytes\":{{\"min\":{spread_min},\"max\":{spread_max},\"range\":{}}},\"client_rss_current_before_setup_bytes\":{},\"client_rss_high_water_before_setup_bytes\":{},\"client_rss_observed_max_at_live_points_bytes\":{observed_rss_max},\"ticket_store_insertions\":0,\"ticket_parse_cost\":\"not separately attributable: the excess includes client handshake and upgrade allocations; no TLS ticket store insertion observed\",\"worker_stack_capture_count\":{},\"worker_config_generation_count\":{},\"allocation_scope_count\":{},\"sample_semantics\":\"client-process GlobalAlloc requested-layout live bytes, not usable capacity; source-known 250760-byte logical reserve is never an actual-cost oracle\"}}",
        spread_max - spread_min,
        baseline_resources.rss_bytes,
        baseline_resources
            .rss_high_water_bytes
            .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        G5E_WORKER_STACK_SAMPLES
            .get()
            .expect("worker stack observer initialized")
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len(),
        soak_observer::client_config_generation_samples().len(),
        G5E_SCOPE_SAMPLES
            .get()
            .expect("allocation scope observer initialized")
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .flatten()
            .count(),
    )
    .expect("write client-only difference summary");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .expect("create unique G5-E client-only difference receipt");
    file.write_all(receipt.as_bytes())
        .expect("write G5-E client-only difference receipt");
    file.sync_all()
        .expect("sync G5-E client-only difference receipt");
}

#[test]
fn g2_m3_authenticated_idle_requested_key_update_emits_exact_reply() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls13(pki.identity));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, client_config);
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);

    let initial_game = b"before-idle-requested-key-update";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();

    // Drain the returned game message's local continuation until the real
    // socket reaches WouldBlock, leaving the established WSS endpoint idle.
    let mut turn = endpoint.drive().expect("drain local WSS work to idle");
    endpoint
        .apply_interest(attempt.poll.registry())
        .expect("apply WSS interest after idle drain");
    for _ in 0..8 {
        if turn.stop != TcpStop::LocalWork {
            break;
        }
        turn = endpoint.drive().expect("finish local WSS idle drain");
        endpoint
            .apply_interest(attempt.poll.registry())
            .expect("apply WSS interest after idle continuation");
    }
    assert_eq!(turn.stop, TcpStop::Quiescent);
    assert_eq!(turn.read_bytes, 0);
    assert!(attempt.shared.inbound.is_empty());

    server.send_record(ContentType::Handshake, &KEY_UPDATE_REQUESTED);
    let turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("process requested KeyUpdate on idle WSS");
    assert_eq!(turn.read_bytes, 0);
    assert!(endpoint.is_connected());
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert!(attempt.shared.inbound.is_empty());

    let response = server
        .read_client_record(IO_TIMEOUT)
        .expect("rustls emits the requested KeyUpdate reply");
    assert_eq!(response.outer_type, ContentType::ApplicationData);
    assert_eq!(response.content_type, ContentType::Handshake);
    assert_eq!(
        response.payload.as_slice(),
        &[0x18, 0x00, 0x00, 0x01, 0x00][..]
    );
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "no duplicate reply or WebSocket game output follows"
    );

    endpoint.close(attempt.poll.registry());
    server.stop();
}

#[test]
fn g2_m3_authenticated_unrequested_key_update_emits_no_reply() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls13(pki.identity));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, client_config);
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);

    let initial_game = b"before-unrequested-key-update";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();

    server.send_record(ContentType::Handshake, &KEY_UPDATE_NOT_REQUESTED);
    let turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("process authenticated update_not_requested");
    assert_eq!(turn.read_bytes, 0);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "update_not_requested does not invent a KeyUpdate reply"
    );

    endpoint.close(attempt.poll.registry());
    server.stop();
}

#[test]
fn g2_m3_authenticated_queued_application_record_survives_key_update() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls13(pki.identity));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, client_config);
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);

    let queued_game = b"application-record-before-key-update";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(queued_game),
    );
    server.send_record(ContentType::Handshake, &KEY_UPDATE_NOT_REQUESTED);
    let turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("process queued application data and authenticated KeyUpdate");
    assert_eq!(turn.read_bytes, queued_game.len());
    take_game_payload(&mut attempt, &mut endpoint, queued_game);
    assert_eq!(attempt.shared.bytes_in(), queued_game.len() as u64);
    assert_eq!(attempt.shared.bytes_out(), 0);
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "queued application data is preserved without an invented reply"
    );

    endpoint.close(attempt.poll.registry());
    server.stop();
}

#[test]
fn g2_m6_authenticated_post_handshake_certificate_request_fails_closed() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls13(pki.identity.duplicate()));
    let sibling_server = ControlServer::spawn(configure_tls13(pki.identity.duplicate()));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, Arc::clone(&client_config));
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
    let initial_game = b"before-certificate-request-game-data";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();
    let (mut sibling_attempt, mut sibling_endpoint, sibling_handshake) =
        connect_wss(&sibling_server, Arc::clone(&client_config));
    assert_negotiated(&sibling_handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);

    // RFC 8446 CertificateRequest with a unique nonempty post-handshake
    // context and a valid signature_algorithms extension (ecdsa_secp256r1_sha256).
    let certificate_request = [
        0x0d, 0x00, 0x00, 0x10, // HandshakeType + 16-byte body
        0x05, b'r', b'7', b'e', b'-', b'm', // nonempty request context
        0x00, 0x08, // extension vector
        0x00, 0x0d, 0x00, 0x04, // signature_algorithms extension
        0x00, 0x02, 0x04, 0x03, // one TLS 1.3-compatible scheme
    ];
    server.send_record(ContentType::Handshake, &certificate_request);
    let error = next_readable_turn(&mut attempt, &mut endpoint)
        .expect_err("post-handshake CertificateRequest is a local TLS failure");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let detail = error
        .get_ref()
        .expect("TLS failure retains its typed rustls source")
        .to_string();
    assert!(!detail.is_empty());
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(attempt.shared.inbound.is_empty());
    assert!(attempt.shared.outbound.is_empty());

    // rustls may emit a fatal TLS alert. If it does, authenticate and inspect
    // it: no client Certificate, success data, or renewed server handshake is
    // allowed to leave this established session.
    if let Some(response) = server.read_client_record(Duration::from_millis(500)) {
        assert_eq!(response.content_type, ContentType::Alert);
        assert_eq!(response.payload.len(), 2);
        assert_eq!(
            response.payload[0], 0x02,
            "only a fatal TLS alert is emitted"
        );
        assert!(
            server
                .read_client_record(Duration::from_millis(250))
                .is_none(),
            "no client-auth or game record follows the TLS failure"
        );
    }

    endpoint.close(attempt.poll.registry());
    assert!(
        attempt.shared.is_closed(),
        "CertificateRequest failure closes locally"
    );
    server.stop();
    let sibling_game = b"certificate-request-failure-does-not-outage-b";
    sibling_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(sibling_game),
    );
    take_game_payload(&mut sibling_attempt, &mut sibling_endpoint, sibling_game);
    sibling_endpoint.close(sibling_attempt.poll.registry());
    sibling_server.stop();
}

//! G5-E receipt combining a scoped direct-WSS control and an actual adapter WSS run.
//!
//! Run the ignored `rustls_authenticated_control` tests
//! `g5e_tls13_wss_keyupdate_backpressure_memory_receipt` and
//! `g5e_adapter_wss_connection_difference_receipt` separately with disjoint
//! `BOT_T14_G5E_ADAPTER_OUTPUT` and `BOT_T14_G5E_DIFFERENCE_OUTPUT` paths.
//! Then run this ignored test with `BOT_T14_G5E_ADAPTER_INPUT` set to the first
//! completed receipt and `BOT_T14_G5E_OUTPUT` set to another absent path.
//! Select each exact test by name; use
//! `--features test-support -- --exact --ignored --test-threads=1`.
//! The test processes have disjoint allocator baselines; do not add their peaks.
//!
//! G5-E in `/srv/dev/scratch/274bot-evidence/TRACK-T1/T1-GATE/r7e/T1-GATE.md`
//! (especially lines 684-717) distinguishes logical reservation, allocator-
//! visible requested bytes, usable capacity, OS/verifier memory, RSS, stack,
//! and kernel resources. The five source-known reservations below are only
//! context; this test never treats their 250,760-byte sum as measured memory or
//! as a passing oracle.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(target_os = "linux")]
use std::mem::MaybeUninit;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection};
use sha2::{Digest, Sha256};
use tungstenite::client::client as websocket_client;
use tungstenite::protocol::Message;

const MAX_EVENTS: usize = 32_768;
const HTTP_HEAD_LIMIT: usize = 4_096;
const FRAGMENT_BYTES: usize = 8_192;
const INBOUND_BYTES: usize = FRAGMENT_BYTES * 2;
const OUTBOUND_BYTES: usize = 4_096;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const NO_PHASE: u8 = 0;

const RESERVATIONS: [(&str, usize); 5] = [
    ("rustls_received_plaintext", 16 * 1024),
    ("tls_total_pending_output", 128 * 1024),
    ("handshake_deframer", 64 * 1024),
    ("outbound_ring", 5_000),
    ("inbound_ring", 32 * 1024),
];
const RESERVED_TOTAL_BYTES: usize = 250_760;

#[derive(Clone, Copy)]
#[repr(u8)]
enum Phase {
    FixtureSetup = 1,
    WssConnectAndUpgrade = 2,
    EstablishedIdle = 3,
    FragmentedReadWrite = 4,
    Shutdown = 5,
}

impl Phase {
    const COUNT: usize = 5;

    fn name(self) -> &'static str {
        match self {
            Self::FixtureSetup => "fixture_setup_before_no_wss_baseline",
            Self::WssConnectAndUpgrade => "wss_tcp_tls_and_websocket_upgrade",
            Self::EstablishedIdle => "established_idle",
            Self::FragmentedReadWrite => "fragmented_read_and_binary_write",
            Self::Shutdown => "websocket_and_tls_shutdown_and_drop",
        }
    }

    fn index(self) -> usize {
        self as usize - 1
    }

    fn from_id(id: u8) -> &'static str {
        match id {
            1 => Self::FixtureSetup.name(),
            2 => Self::WssConnectAndUpgrade.name(),
            3 => Self::EstablishedIdle.name(),
            4 => Self::FragmentedReadWrite.name(),
            5 => Self::Shutdown.name(),
            _ => "invalid_phase",
        }
    }
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum Operation {
    Alloc = 1,
    AllocZeroed = 2,
    Realloc = 3,
    Dealloc = 4,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Alloc => "alloc",
            Self::AllocZeroed => "alloc_zeroed",
            Self::Realloc => "realloc",
            Self::Dealloc => "dealloc",
        }
    }

    fn from_id(id: u8) -> &'static str {
        match id {
            1 => Self::Alloc.name(),
            2 => Self::AllocZeroed.name(),
            3 => Self::Realloc.name(),
            4 => Self::Dealloc.name(),
            _ => "invalid_operation",
        }
    }
}

#[derive(Clone, Copy)]
struct AllocationEvent {
    phase: u8,
    operation: u8,
    thread_id: u64,
    requested_alignment_bytes: usize,
    old_requested_bytes: usize,
    new_requested_bytes: usize,
    succeeded: bool,
    process_live_requested_bytes_after: usize,
}

const EMPTY_EVENT: AllocationEvent = AllocationEvent {
    phase: 0,
    operation: 0,
    thread_id: 0,
    requested_alignment_bytes: 0,
    old_requested_bytes: 0,
    new_requested_bytes: 0,
    succeeded: false,
    process_live_requested_bytes_after: 0,
};

struct EventSlot(UnsafeCell<AllocationEvent>);

// SAFETY: each slot is written once after a unique atomic event-index reservation,
// and is read only after measurement is disabled and all fixture threads are joined.
unsafe impl Sync for EventSlot {}

static EVENTS: [EventSlot; MAX_EVENTS] =
    [const { EventSlot(UnsafeCell::new(EMPTY_EVENT)) }; MAX_EVENTS];
static EVENT_COUNT: AtomicUsize = AtomicUsize::new(0);
static EVENT_OVERFLOW: AtomicBool = AtomicBool::new(false);
static CURRENT_REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_PHASE: AtomicU8 = AtomicU8::new(NO_PHASE);
static PHASE_PEAKS: [AtomicUsize; Phase::COUNT] = [const { AtomicUsize::new(0) }; Phase::COUNT];
static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static THREAD_ID: Cell<u64> = const { Cell::new(0) };
}

fn current_thread_id() -> u64 {
    THREAD_ID
        .try_with(|thread_id| {
            let existing = thread_id.get();
            if existing != 0 {
                existing
            } else {
                let assigned = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
                thread_id.set(assigned);
                assigned
            }
        })
        .unwrap_or(0)
}

fn update_phase_peak(phase: u8, current: usize) {
    if phase != NO_PHASE {
        let index = usize::from(phase - 1);
        if index < PHASE_PEAKS.len() {
            PHASE_PEAKS[index].fetch_max(current, Ordering::Relaxed);
        }
    }
}

fn record_event(
    operation: Operation,
    alignment: usize,
    old_bytes: usize,
    new_bytes: usize,
    succeeded: bool,
    current: usize,
) {
    let phase = ACTIVE_PHASE.load(Ordering::Acquire);
    if phase == NO_PHASE {
        return;
    }
    update_phase_peak(phase, current);
    let index = EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
    if index >= EVENTS.len() {
        EVENT_OVERFLOW.store(true, Ordering::Relaxed);
        return;
    }

    let event = AllocationEvent {
        phase,
        operation: operation as u8,
        thread_id: current_thread_id(),
        requested_alignment_bytes: alignment,
        old_requested_bytes: old_bytes,
        new_requested_bytes: new_bytes,
        succeeded,
        process_live_requested_bytes_after: current,
    };
    // SAFETY: `index` is unique and in bounds; the slot is not read until all
    // measurement threads have stopped and the active phase is cleared.
    unsafe { EVENTS[index].0.get().write(event) };
}

// `allocation-counter` is thread-local; this receipt needs process-wide live
// bytes and captures the separate local fixture worker.
struct CountingAllocator;

// SAFETY: every allocator method forwards the original pointer/layout to
// `System`; the counters and fixed event slots do not allocate.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplied the layout required by GlobalAlloc.
        let pointer = unsafe { System.alloc(layout) };
        let succeeded = !pointer.is_null();
        let current = if succeeded {
            CURRENT_REQUESTED_BYTES
                .fetch_add(layout.size(), Ordering::Relaxed)
                .wrapping_add(layout.size())
        } else {
            CURRENT_REQUESTED_BYTES.load(Ordering::Relaxed)
        };
        record_event(
            Operation::Alloc,
            layout.align(),
            0,
            layout.size(),
            succeeded,
            current,
        );
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplied the layout required by GlobalAlloc.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        let succeeded = !pointer.is_null();
        let current = if succeeded {
            CURRENT_REQUESTED_BYTES
                .fetch_add(layout.size(), Ordering::Relaxed)
                .wrapping_add(layout.size())
        } else {
            CURRENT_REQUESTED_BYTES.load(Ordering::Relaxed)
        };
        record_event(
            Operation::AllocZeroed,
            layout.align(),
            0,
            layout.size(),
            succeeded,
            current,
        );
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller supplied the original allocation and layout as
        // required by GlobalAlloc; `System` preserves the realloc contract.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        let succeeded = !replacement.is_null();
        let current = if succeeded {
            let old_size = layout.size();
            if new_size >= old_size {
                CURRENT_REQUESTED_BYTES
                    .fetch_add(new_size - old_size, Ordering::Relaxed)
                    .wrapping_add(new_size - old_size)
            } else {
                CURRENT_REQUESTED_BYTES
                    .fetch_sub(old_size - new_size, Ordering::Relaxed)
                    .saturating_sub(old_size - new_size)
            }
        } else {
            CURRENT_REQUESTED_BYTES.load(Ordering::Relaxed)
        };
        record_event(
            Operation::Realloc,
            layout.align(),
            layout.size(),
            new_size,
            succeeded,
            current,
        );
        replacement
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let current = CURRENT_REQUESTED_BYTES
            .fetch_sub(layout.size(), Ordering::Relaxed)
            .saturating_sub(layout.size());
        record_event(
            Operation::Dealloc,
            layout.align(),
            layout.size(),
            0,
            true,
            current,
        );
        // SAFETY: the caller supplied the pointer and matching layout required
        // by GlobalAlloc; this allocator returns the same blocks from `System`.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn begin_phase(phase: Phase) {
    let current = CURRENT_REQUESTED_BYTES.load(Ordering::Acquire);
    PHASE_PEAKS[phase.index()].store(current, Ordering::Release);
    ACTIVE_PHASE.store(phase as u8, Ordering::Release);
}

#[derive(Clone, Copy)]
struct PhaseSnapshot {
    phase: Phase,
    sample_name: &'static str,
    reference_name: &'static str,
    current_requested_bytes: usize,
    peak_requested_bytes: usize,
    current_delta_from_reference: i128,
    peak_delta_from_reference: i128,
    process_peak_rss_bytes: Option<u64>,
    process_peak_rss_delta_from_reference: Option<i128>,
}

fn signed_delta(value: usize, reference: usize) -> i128 {
    value as i128 - reference as i128
}

fn snapshot(phase: Phase, reference_current: usize, reference_rss: Option<u64>) -> PhaseSnapshot {
    let current = CURRENT_REQUESTED_BYTES.load(Ordering::Acquire);
    let peak = PHASE_PEAKS[phase.index()].load(Ordering::Acquire);
    let rss = process_peak_rss_bytes();
    PhaseSnapshot {
        phase,
        sample_name: match phase {
            Phase::FixtureSetup => "fixture_ready_before_wss_baseline",
            Phase::WssConnectAndUpgrade => "connected_after_tls_and_ws_upgrade",
            Phase::EstablishedIdle => "established_idle",
            Phase::FragmentedReadWrite => "after_fragmented_read_and_write",
            Phase::Shutdown => "after_close_notify_before_client_drop",
        },
        reference_name: match phase {
            Phase::FixtureSetup => "pre_fixture_warmup",
            _ => "no_wss_baseline",
        },
        current_requested_bytes: current,
        peak_requested_bytes: peak,
        current_delta_from_reference: signed_delta(current, reference_current),
        peak_delta_from_reference: signed_delta(peak, reference_current),
        process_peak_rss_bytes: rss,
        process_peak_rss_delta_from_reference: rss
            .zip(reference_rss)
            .map(|(observed, reference)| i128::from(observed) - i128::from(reference)),
    }
}

#[cfg(target_os = "linux")]
fn process_peak_rss_bytes() -> Option<u64> {
    let mut usage = MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` initializes the supplied `rusage` on success.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the successful call above initialized all fields.
    let usage = unsafe { usage.assume_init() };
    u64::try_from(usage.ru_maxrss).ok()?.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn process_peak_rss_bytes() -> Option<u64> {
    None
}

fn allocator_warmup() {
    let mut warmup = Vec::with_capacity(64);
    warmup.extend_from_slice(b"allocator warmup");
    drop(warmup);
    let _ = current_thread_id();
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ServerCommand {
    SendFragments,
    Close,
}

fn io_error(kind: io::ErrorKind, message: &'static str) -> io::Error {
    io::Error::new(kind, message)
}

fn server_start(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    startup: SyncSender<u64>,
    ready: SyncSender<()>,
    commands: Receiver<ServerCommand>,
    data_read: SyncSender<io::Result<()>>,
) -> io::Result<()> {
    startup
        .send(current_thread_id())
        .map_err(|_| io_error(io::ErrorKind::BrokenPipe, "driver dropped server startup"))?;
    let (socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(IO_TIMEOUT))?;
    socket.set_write_timeout(Some(IO_TIMEOUT))?;
    socket.set_nodelay(true)?;
    let connection = ServerConnection::new(config)
        .map_err(|_| io_error(io::ErrorKind::InvalidData, "server TLS connection setup"))?;
    let mut tls = rustls::StreamOwned::new(connection, socket);

    let mut request = [0u8; HTTP_HEAD_LIMIT];
    let mut request_len = 0;
    loop {
        if request_len == request.len() {
            return Err(io_error(
                io::ErrorKind::InvalidData,
                "WSS request header too large",
            ));
        }
        tls.read_exact(&mut request[request_len..request_len + 1])?;
        request_len += 1;
        if request_len >= 4 && request[..request_len].ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let accept = websocket_accept(&request[..request_len])?;
    write!(
        tls,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    tls.flush()?;
    ready
        .send(())
        .map_err(|_| io_error(io::ErrorKind::BrokenPipe, "driver dropped WSS-ready signal"))?;

    match commands
        .recv()
        .map_err(|_| io_error(io::ErrorKind::BrokenPipe, "driver dropped fragment command"))?
    {
        ServerCommand::SendFragments => send_fragmented_binary(&mut tls)?,
        ServerCommand::Close => {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                "server received close before data phase",
            ));
        }
    }

    let data_result = read_and_check_client_binary(&mut tls);
    data_read
        .send(data_result.as_ref().map(|_| ()).map_err(|_| {
            io_error(
                io::ErrorKind::InvalidData,
                "client binary frame differed from fixture",
            )
        }))
        .map_err(|_| io_error(io::ErrorKind::BrokenPipe, "driver dropped binary result"))?;
    data_result?;

    match commands
        .recv()
        .map_err(|_| io_error(io::ErrorKind::BrokenPipe, "driver dropped close command"))?
    {
        ServerCommand::Close => finish_websocket_and_tls(&mut tls),
        ServerCommand::SendFragments => Err(io_error(
            io::ErrorKind::InvalidInput,
            "server received a second fragment command",
        )),
    }
}

fn websocket_accept(request: &[u8]) -> io::Result<String> {
    let request = std::str::from_utf8(request)
        .map_err(|_| io_error(io::ErrorKind::InvalidData, "WSS request was not UTF-8"))?;
    for line in request.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Sec-WebSocket-Key") {
                let key = value.trim();
                if !key.is_empty() {
                    return Ok(tungstenite::handshake::derive_accept_key(key.as_bytes()));
                }
            }
        }
    }
    Err(io_error(
        io::ErrorKind::InvalidData,
        "WSS request omitted Sec-WebSocket-Key",
    ))
}

fn frame_payload_byte(index: usize) -> u8 {
    ((index.wrapping_mul(31).wrapping_add(7)) % 251) as u8
}

fn send_fragmented_binary(stream: &mut impl Write) -> io::Result<()> {
    let mut frames = [0u8; 2 * (4 + FRAGMENT_BYTES)];
    frames[0] = 0x02; // non-final binary data frame
    frames[1] = 126; // 16-bit payload length
    frames[2] = (FRAGMENT_BYTES >> 8) as u8;
    frames[3] = FRAGMENT_BYTES as u8;
    for offset in 0..FRAGMENT_BYTES {
        frames[4 + offset] = frame_payload_byte(offset);
    }

    let continuation = 4 + FRAGMENT_BYTES;
    frames[continuation] = 0x80; // final continuation frame
    frames[continuation + 1] = 126;
    frames[continuation + 2] = (FRAGMENT_BYTES >> 8) as u8;
    frames[continuation + 3] = FRAGMENT_BYTES as u8;
    for offset in 0..FRAGMENT_BYTES {
        frames[continuation + 4 + offset] = frame_payload_byte(FRAGMENT_BYTES + offset);
    }
    stream.write_all(&frames)?;
    stream.flush()
}

fn read_client_frame(stream: &mut impl Read, payload: &mut [u8]) -> io::Result<(u8, bool, usize)> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header)?;
    let final_frame = header[0] & 0x80 != 0;
    let opcode = header[0] & 0x0f;
    if header[0] & 0x70 != 0 || header[1] & 0x80 == 0 {
        return Err(io_error(
            io::ErrorKind::InvalidData,
            "client frame has reserved bits or is not masked",
        ));
    }

    let mut length = usize::from(header[1] & 0x7f);
    match length {
        126 => {
            let mut extended = [0u8; 2];
            stream.read_exact(&mut extended)?;
            length = usize::from(u16::from_be_bytes(extended));
        }
        127 => {
            let mut extended = [0u8; 8];
            stream.read_exact(&mut extended)?;
            length = usize::try_from(u64::from_be_bytes(extended))
                .map_err(|_| io_error(io::ErrorKind::InvalidData, "frame length overflow"))?;
        }
        _ => {}
    }
    if length > payload.len() {
        return Err(io_error(
            io::ErrorKind::InvalidData,
            "client frame exceeds fixed fixture buffer",
        ));
    }

    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask)?;
    stream.read_exact(&mut payload[..length])?;
    for (index, byte) in payload[..length].iter_mut().enumerate() {
        *byte ^= mask[index % mask.len()];
    }
    Ok((opcode, final_frame, length))
}

fn read_and_check_client_binary(stream: &mut impl Read) -> io::Result<()> {
    let mut payload = [0u8; OUTBOUND_BYTES];
    let (opcode, final_frame, length) = read_client_frame(stream, &mut payload)?;
    if opcode != 0x02 || !final_frame || length != OUTBOUND_BYTES {
        return Err(io_error(
            io::ErrorKind::InvalidData,
            "client did not send one complete binary frame",
        ));
    }
    for (index, byte) in payload[..length].iter().enumerate() {
        if *byte != frame_payload_byte(index ^ 0x55) {
            return Err(io_error(
                io::ErrorKind::InvalidData,
                "client binary payload differed from fixture",
            ));
        }
    }
    Ok(())
}

fn finish_websocket_and_tls(stream: &mut impl ReadWrite) -> io::Result<()> {
    let mut close_payload = [0u8; 125];
    let (opcode, final_frame, length) = read_client_frame(stream, &mut close_payload)?;
    if opcode != 0x08 || !final_frame || length > 125 {
        return Err(io_error(
            io::ErrorKind::InvalidData,
            "client did not send a complete WebSocket close frame",
        ));
    }
    stream.write_all(&[0x88, 0x00])?;
    stream.flush()?;

    // Keep the fixture socket open until the client sends TLS close_notify.
    // This makes the client's protocol shutdown and TLS shutdown distinct.
    let mut scratch = [0u8; 1];
    match stream.read(&mut scratch) {
        Ok(0) => Ok(()),
        Ok(_) => Err(io_error(
            io::ErrorKind::InvalidData,
            "unexpected application data after WebSocket close",
        )),
        Err(error) => Err(error),
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn make_tls_configs() -> (Arc<ClientConfig>, Arc<ServerConfig>) {
    let identity = generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("generate local WSS identity");
    let certificate = identity.cert.der().clone();
    let private_key = PrivateKeyDer::Pkcs8(identity.key_pair.serialize_der().into());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let protocol_versions = [&rustls::version::TLS13, &rustls::version::TLS12];

    let server = ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&protocol_versions)
        .expect("server TLS protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], private_key)
        .expect("server TLS identity");

    let mut roots = RootCertStore::empty();
    roots.add(certificate).expect("add local certificate root");
    let client = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&protocol_versions)
        .expect("client TLS protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    (Arc::new(client), Arc::new(server))
}

fn thread_net_delta(phase: Phase, thread_id: u64, event_count: usize) -> i128 {
    let mut delta = 0i128;
    for slot in EVENTS.iter().take(event_count.min(EVENTS.len())) {
        // SAFETY: a complete allocator event was written to each retained
        // slot, and measurement has stopped before receipt generation.
        let event = unsafe { *slot.0.get() };
        if event.phase == phase as u8 && event.thread_id == thread_id && event.succeeded {
            let event_delta = match event.operation {
                1 | 2 => event.new_requested_bytes as i128,
                3 => event.new_requested_bytes as i128 - event.old_requested_bytes as i128,
                4 => -(event.old_requested_bytes as i128),
                _ => 0,
            };
            delta = delta.saturating_add(event_delta);
        }
    }
    delta
}

fn phase_event_count(phase: Phase, event_count: usize) -> usize {
    let mut count = 0;
    for slot in EVENTS.iter().take(event_count.min(EVENTS.len())) {
        // SAFETY: a complete allocator event was written to each retained
        // slot, and measurement has stopped before receipt generation.
        let event = unsafe { *slot.0.get() };
        if event.phase == phase as u8 {
            count += 1;
        }
    }
    count
}

fn write_optional_u64(output: &mut String, value: Option<u64>) {
    match value {
        Some(value) => write!(output, "{value}").expect("write number into receipt"),
        None => output.push_str("null"),
    }
}

fn write_optional_i128(output: &mut String, value: Option<i128>) {
    match value {
        Some(value) => write!(output, "{value}").expect("write number into receipt"),
        None => output.push_str("null"),
    }
}

fn write_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control < '\u{20}' => {
                write!(output, "\\u{:04x}", control as u32).expect("escape JSON control character");
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

fn write_snapshot(
    output: &mut String,
    snapshot: &PhaseSnapshot,
    driver_thread: u64,
    fixture_thread: u64,
    event_count: usize,
) {
    let driver_delta = thread_net_delta(snapshot.phase, driver_thread, event_count);
    let fixture_delta = thread_net_delta(snapshot.phase, fixture_thread, event_count);
    let phase_events = phase_event_count(snapshot.phase, event_count);
    write!(
        output,
        "{{\"phase\":\"{}\",\"sample\":\"{}\",\"reference\":\"{}\",\"allocator_event_count\":{},\"process_live_requested_bytes\":{},\"phase_peak_live_requested_bytes\":{},\"current_delta_from_reference_bytes\":{},\"phase_peak_delta_from_reference_bytes\":{},\"unidentified_process_live_requested_bytes\":{{\"current\":{},\"phase_peak\":{}}},\"driver_thread_net_requested_bytes\":{},\"fixture_thread_net_requested_bytes\":{},\"process_peak_rss_bytes\":",
        snapshot.phase.name(),
        snapshot.sample_name,
        snapshot.reference_name,
        phase_events,
        snapshot.current_requested_bytes,
        snapshot.peak_requested_bytes,
        snapshot.current_delta_from_reference,
        snapshot.peak_delta_from_reference,
        snapshot.current_requested_bytes,
        snapshot.peak_requested_bytes,
        driver_delta,
        fixture_delta,
    )
    .expect("write snapshot into receipt");
    write_optional_u64(output, snapshot.process_peak_rss_bytes);
    output.push_str(",\"process_peak_rss_delta_from_reference_bytes\":");
    write_optional_i128(output, snapshot.process_peak_rss_delta_from_reference);
    output.push('}');
}

fn make_receipt(
    baseline_current: usize,
    baseline_rss: Option<u64>,
    snapshots: &[PhaseSnapshot; 6],
    driver_thread: u64,
    fixture_thread: u64,
    adapter_receipt_path: &str,
    adapter_receipt: &str,
    adapter_receipt_sha256: &[u8],
) -> String {
    let event_count = EVENT_COUNT.load(Ordering::Acquire);
    assert!(
        !EVENT_OVERFLOW.load(Ordering::Acquire) && event_count <= EVENTS.len(),
        "bounded allocator event recorder overflowed; refusing incomplete receipt"
    );
    let rss_available = baseline_rss.is_some()
        && snapshots
            .iter()
            .all(|snapshot| snapshot.process_peak_rss_bytes.is_some());
    if cfg!(target_os = "linux") {
        assert!(rss_available, "Linux getrusage RSS samples are required");
    }
    let root_lock_digest = Sha256::digest(include_bytes!("../../../Cargo.lock"));
    let fork_lock_digest = Sha256::digest(include_bytes!("../../../vendor/fr-client-rust/Cargo.lock"));
    let mut output = String::with_capacity(4_096 + event_count * 256 + adapter_receipt.len());
    output.push_str("{\n  \"schema\": \"t1-g5e-allocator-receipt-v2\",\n  \"gate\": \"T1-GATE r7e G5-E (sections 8 and 8.1, lines 684-717)\",\n");
    output.push_str("  \"root_lock_sha256\": \"");
    for byte in root_lock_digest {
        write!(output, "{byte:02x}").expect("write lock digest");
    }
    output.push_str("\",\n  \"fork_standalone_lock_sha256\": \"");
    for byte in fork_lock_digest {
        write!(output, "{byte:02x}").expect("write fork lock digest");
    }
    output.push_str("\",\n  \"locked_dependency_configuration\": {\n    \"rustls\": \"0.23.45; manifest features std,ring,tls12; local private root; TLS 1.3/1.2\",\n    \"tungstenite\": \"0.24.0; default-features=false; handshake\",\n    \"resolved_root_features\": \"unresolved in binary alone: capture cargo tree -p client-adapter -e features for the exact root lock and target\"\n  },\n");
    output.push_str("  \"measurement_subject\": \"direct rustls+tungstenite control plus embedded independently measured actual client-adapter endpoint; allocator baselines are disjoint\",\n");
    output.push_str("  \"target\": {\n    \"os\": \"");
    output.push_str(std::env::consts::OS);
    output.push_str("\",\n    \"arch\": \"");
    output.push_str(std::env::consts::ARCH);
    output.push_str("\"\n  },\n");
    output.push_str("  \"connection_count\": 1,\n  \"client_transport\": \"one TCP + rustls client connection + tungstenite WSS upgrade\",\n  \"warmup_policy\": \"allocator and fixture/config initialization are recorded separately; no network warmup; this is the only client WSS connection\",\n");
    output.push_str("  \"baseline_without_wss\": {\"process_live_requested_bytes\":");
    write!(output, "{baseline_current}").expect("write baseline");
    output.push_str(",\"process_peak_rss_bytes\":");
    write_optional_u64(&mut output, baseline_rss);
    output.push_str("},\n  \"phase_samples\": [\n");

    for (index, snapshot) in snapshots.iter().enumerate() {
        output.push_str("    ");
        write_snapshot(
            &mut output,
            snapshot,
            driver_thread,
            fixture_thread,
            event_count,
        );
        if index + 1 != snapshots.len() {
            output.push(',');
        }
        output.push('\n');
    }

    output.push_str("  ],\n  \"scoped_allocation_events\": {\n    \"event_count\": ");
    write!(output, "{event_count}").expect("write event count");
    output.push_str(",\n    \"events\": [\n");
    for (index, slot) in EVENTS.iter().take(event_count).enumerate() {
        // SAFETY: all retained slots were written by unique reservations,
        // and all fixture threads have stopped before this read.
        let event = unsafe { *slot.0.get() };
        write!(
            output,
            "      {{\"phase\":\"{}\",\"operation\":\"{}\",\"thread_id\":{},\"requested_alignment_bytes\":{},\"old_requested_bytes\":{},\"new_requested_bytes\":{},\"succeeded\":{},\"process_live_requested_bytes_after\":{}}}",
            Phase::from_id(event.phase),
            Operation::from_id(event.operation),
            event.thread_id,
            event.requested_alignment_bytes,
            event.old_requested_bytes,
            event.new_requested_bytes,
            event.succeeded,
            event.process_live_requested_bytes_after,
        )
        .expect("write allocator event");
        if index + 1 != event_count {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str("    ]\n  },\n");

    output.push_str("  \"allocator_recorder\": {\"storage\": \"static process-global event slots; hooks do not allocate\", \"capacity_events\":");
    write!(output, "{}", EVENTS.len()).expect("write recorder capacity");
    output.push_str(",\"static_storage_bytes\":");
    write!(output, "{}", std::mem::size_of_val(&EVENTS)).expect("write recorder storage size");
    output.push_str("},\n");
    output.push_str(
        "  \"source_known_reservations\": {\n    \"measured\": false,\n    \"components_bytes\": {",
    );
    for (index, (name, bytes)) in RESERVATIONS.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(output, "\"{name}\":{bytes}").expect("write reservation");
    }
    output.push_str("},\n    \"logical_sum_bytes\":");
    write!(output, "{RESERVED_TOTAL_BYTES}").expect("write reservation summary");
    output.push_str(",\n    \"adapter_rings_measured\": false,\n    \"used_as_measurement_oracle\": false\n  },\n");
    output.push_str("  \"actual_adapter_wss_capture\": {\"source_path\":");
    write_json_string(&mut output, adapter_receipt_path);
    output.push_str(",\"sha256\":\"");
    for byte in adapter_receipt_sha256 {
        write!(output, "{byte:02x}").expect("write adapter receipt digest");
    }
    output.push_str("\",\"receipt\":");
    output.push_str(adapter_receipt);
    output.push_str("},\n");
    output.push_str("  \"gate_measurement_fields\": {\n");
    output.push_str("    \"actual_adapter_connection_count\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.connection_count\"},\n");
    output.push_str("    \"process_allocator_high_water\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.allocator_requested_peak_during_*_bytes; includes fixture and both lanes\"},\n");
    output.push_str("    \"per_connection_allocator_high_water\": {\"status\":\"unresolved\",\"reason\":\"global live-requested peak includes server fixture and two live WSS lanes; allocation scopes count gross request bytes, not retained per-connection ownership\"},\n");
    output.push_str("    \"tls_session_request_transient\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.allocation_scope_samples filtered by GameTlsConnection and GameTlsVerification; gross requested bytes only\"},\n");
    output.push_str("    \"tls_session_retained_transient\": {\"status\":\"unresolved\",\"reason\":\"scope events do not retain allocation pointer ownership through cross-thread frees or expose private rustls capacities\"},\n");
    output.push_str("    \"client_ticket_store\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.tls13_tickets_inserted and allocator_live_growth_per_ticket_insert_high_water_bytes; resumption disabled\"},\n");
    output.push_str("    \"ticket_parse_transient\": {\"status\":\"unresolved\",\"reason\":\"client ticket insert and resumption are disabled, but incoming ticket parsing may allocate inside a larger TLS scope without separate attribution\"},\n");
    output.push_str("    \"shared_config_once_per_process\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.shared_config_process_requested_*_bytes; process live and peak deltas, not exact retained config ownership\"},\n");
    output.push_str("    \"worker_config_generations\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.client_config_generation_samples_after_connect and client_config_generation_events\"},\n");
    output.push_str("    \"worker_stack_reserved_committed_resident_guard\": {\"status\":\"measured_or_unresolved_per_field\",\"source\":\"actual_adapter_wss_capture.receipt.worker_stack_samples; null means OS/API cannot provide that field, inspect measurement_source and platform-specific evaluator\"},\n");
    output.push_str("    \"process_rss\": {\"status\":\"measured\",\"source\":\"actual_adapter_wss_capture.receipt.rss_current_*_bytes at five phases; not a continuous high-water\"},\n");
    output.push_str("    \"per_connection_os_rss_high_water\": {\"status\":\"unresolved\",\"reason\":\"process RSS includes two WSS lanes, server fixture, shared config, kernel and allocator overhead; no independent per-connection attribution\"},\n");
    output.push_str("    \"unexplained_difference_from_logical_ledger\": {\"status\":\"unresolved\",\"reason\":\"logical 250760-byte reserve is not actual allocation; component ownership is unavailable, so no honest exact subtraction exists\"}\n");
    output.push_str("  },\n");

    output.push_str("  \"limitations\": {\n    \"requested_layout_bytes_are_not_allocator_usable_bytes\": true,\n    \"private_field_sizes_inferred\": false,\n    \"allocation_scope\": \"all Rust GlobalAlloc requests in this dedicated integration-test process while a named phase is active; exact process live-requested current/peak, including local fixture and test code\",\n    \"allocation_component_attribution\": \"unknown: GlobalAlloc reports layouts, not rustls/tungstenite callsites or allocation owners; per-thread event net is operation-thread attribution only and cross-thread frees remain ambiguous\",\n    \"allocator_event_order\": \"slot-reservation order; concurrent operations may be listed in a different order than the atomic counter updates\",\n    \"unidentified_process_bytes\": \"the process-wide current and phase peak fields are reported separately and must not be relabeled as client-only bytes\",\n    \"adapter_scope\": \"the production WssEndpoint, reactor and SharedConnection rings are not instantiated; no full client-adapter total is claimed\",\n    \"unmeasured_reservation_owners\": \"the source-known inbound/outbound ring compartments are listed as logical reserves only; adapter ring allocations are outside this direct stack test\",\n    \"shared_tls_config_cost\": \"client root/config and server fixture are built before the no-WSS baseline; aggregate fixture_setup sample only, not isolated shared-config attribution\",\n    \"handshake_ticket_cost\": \"not measured: the adapter handshake ticket and worker path are not instantiated\",\n    \"realloc_peak_semantics\": \"requested live bytes are sampled after successful realloc; underlying allocator transient/usable capacity is not observed\",\n    \"native_os_verifier_memory\": \"not exercised: client trusts the generated local certificate through an explicit RootCertStore\",\n    \"rss_metric\": \"process lifetime maximum RSS from getrusage sampled at phase boundaries; process-wide high-water, not per-client/current RSS\",\n    \"rss_includes_static_recorder_pages\": true,\n    \"rss_available\": ");
    output.push_str(if rss_available { "true\n" } else { "false\n" });
    output.push_str("  }\n}\n");
    output
}

#[test]
#[ignore = "manual combined G5-E receipt; requires BOT_T14_G5E_ADAPTER_INPUT and BOT_T14_G5E_OUTPUT"]
fn g5e_actual_adapter_and_direct_control_receipt() {
    let output_path = std::env::var_os("BOT_T14_G5E_OUTPUT")
        .map(PathBuf::from)
        .expect("set BOT_T14_G5E_OUTPUT to an absent receipt path");
    allocator_warmup();
    let driver_thread = current_thread_id();
    let before_fixture = CURRENT_REQUESTED_BYTES.load(Ordering::Acquire);
    let before_fixture_rss = process_peak_rss_bytes();

    begin_phase(Phase::FixtureSetup);
    let (client_config, server_config) = make_tls_configs();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local WSS fixture");
    let address = listener.local_addr().expect("read local fixture address");
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (command_tx, command_rx) = mpsc::sync_channel(1);
    let (data_tx, data_rx) = mpsc::sync_channel(1);
    let fixture_thread = thread::spawn(move || {
        server_start(
            listener,
            server_config,
            startup_tx,
            ready_tx,
            command_rx,
            data_tx,
        )
    });
    let fixture_thread_id = startup_rx
        .recv_timeout(IO_TIMEOUT)
        .expect("local WSS fixture thread starts");
    let mut expected_inbound = Vec::with_capacity(INBOUND_BYTES);
    expected_inbound.extend((0..INBOUND_BYTES).map(frame_payload_byte));
    let mut outbound = Vec::with_capacity(OUTBOUND_BYTES);
    outbound.extend((0..OUTBOUND_BYTES).map(|index| frame_payload_byte(index ^ 0x55)));
    let fixture_setup_current = CURRENT_REQUESTED_BYTES.load(Ordering::Acquire);
    let fixture_setup_rss = process_peak_rss_bytes();
    let setup = snapshot(Phase::FixtureSetup, before_fixture, before_fixture_rss);
    ACTIVE_PHASE.store(NO_PHASE, Ordering::Release);

    // The fixture thread is blocked before accepting, the TLS identities/configs
    // are ready, and no client socket exists: this is the no-WSS baseline.
    let baseline_current = fixture_setup_current;
    let baseline_rss = fixture_setup_rss;

    begin_phase(Phase::WssConnectAndUpgrade);
    let mut socket = TcpStream::connect(address).expect("connect local fixture");
    socket
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("set client read timeout");
    socket
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("set client write timeout");
    socket.set_nodelay(true).expect("set client TCP_NODELAY");
    let server_name = ServerName::try_from("localhost".to_owned()).expect("server name");
    let mut tls = ClientConnection::new(Arc::clone(&client_config), server_name)
        .expect("create local-root TLS client");
    while tls.is_handshaking() {
        tls.complete_io(&mut socket)
            .expect("complete client TLS handshake");
    }
    let tls_stream = rustls::StreamOwned::new(tls, socket);
    let uri = format!("wss://localhost:{}/", address.port());
    let (mut websocket, response) =
        websocket_client(uri, tls_stream).expect("complete tungstenite WSS upgrade");
    assert_eq!(response.status().as_u16(), 101);
    drop(response);
    ready_rx
        .recv_timeout(IO_TIMEOUT)
        .expect("fixture completed WSS upgrade response");
    let connected = snapshot(Phase::WssConnectAndUpgrade, baseline_current, baseline_rss);

    begin_phase(Phase::EstablishedIdle);
    let idle = snapshot(Phase::EstablishedIdle, baseline_current, baseline_rss);

    begin_phase(Phase::FragmentedReadWrite);
    command_tx
        .send(ServerCommand::SendFragments)
        .expect("send fragmented-frame command");
    match websocket.read().expect("read fragmented fixture message") {
        Message::Binary(payload) => assert_eq!(payload, expected_inbound),
        other => panic!("expected binary fixture message, received {other:?}"),
    }
    websocket
        .send(Message::Binary(outbound))
        .expect("write one binary message to WSS fixture");
    data_rx
        .recv_timeout(IO_TIMEOUT)
        .expect("fixture observed client WebSocket frame")
        .expect("fixture verified client binary frame");
    let read_write = snapshot(Phase::FragmentedReadWrite, baseline_current, baseline_rss);

    begin_phase(Phase::Shutdown);
    command_tx
        .send(ServerCommand::Close)
        .expect("send close command to fixture");
    websocket.close(None).expect("send WebSocket close frame");
    match websocket.read().expect("read fixture close response") {
        Message::Close(_) => {}
        other => panic!("expected close response, received {other:?}"),
    }
    let stream = websocket.get_mut();
    stream.conn.send_close_notify();
    while stream.conn.wants_write() {
        let written = stream
            .conn
            .write_tls(&mut stream.sock)
            .expect("write client TLS close_notify");
        assert_ne!(written, 0, "TLS close_notify made no progress");
    }
    fixture_thread
        .join()
        .expect("fixture server thread did not panic")
        .expect("fixture completed close and TLS shutdown");
    let shutdown = snapshot(Phase::Shutdown, baseline_current, baseline_rss);
    drop(websocket);
    let mut retired = snapshot(Phase::Shutdown, baseline_current, baseline_rss);
    retired.sample_name = "after_client_websocket_drop";
    ACTIVE_PHASE.store(NO_PHASE, Ordering::Release);

    // The fixture worker may free baseline setup allocations during connect;
    // a positive process-wide net is not a valid per-WSS allocation oracle.
    assert!(
        connected.peak_requested_bytes >= connected.current_requested_bytes,
        "connection phase must include a live-requested high-water"
    );
    assert!(
        read_write.peak_requested_bytes >= read_write.current_requested_bytes,
        "fragmented read/write phase must include a live-requested high-water"
    );
    let measured_event_count = EVENT_COUNT.load(Ordering::Acquire);
    assert!(
        measured_event_count > 0,
        "scoped allocator recorded no layouts"
    );
    assert!(
        phase_event_count(Phase::WssConnectAndUpgrade, measured_event_count) > 0,
        "WSS connection phase recorded no allocator events"
    );
    assert!(
        phase_event_count(Phase::FragmentedReadWrite, measured_event_count) > 0,
        "fragmented WebSocket read/write phase recorded no allocator events"
    );
    assert_eq!(
        RESERVATIONS.iter().map(|(_, bytes)| bytes).sum::<usize>(),
        RESERVED_TOTAL_BYTES,
        "source-known G5 compartments changed; they remain non-measurements"
    );

    let adapter_input_path = std::env::var_os("BOT_T14_G5E_ADAPTER_INPUT")
        .map(PathBuf::from)
        .expect("set BOT_T14_G5E_ADAPTER_INPUT to the completed actual-adapter receipt");
    assert!(
        adapter_input_path.is_absolute(),
        "the adapter receipt path must be absolute"
    );
    let adapter_bytes = fs::read(&adapter_input_path).expect("read actual-adapter WSS receipt");
    let adapter_receipt = std::str::from_utf8(&adapter_bytes)
        .expect("actual-adapter receipt is UTF-8")
        .trim();
    assert!(
        adapter_receipt.starts_with("{\"schema\":\"t1.4-g5e-rustls-adapter-receipt-v3\",")
            && adapter_receipt.ends_with('}'),
        "expected a complete actual-adapter WSS receipt, not the direct control"
    );
    assert!(
        adapter_receipt.contains(&format!("\"target_os\":\"{}\"", std::env::consts::OS))
            && adapter_receipt.contains(&format!("\"target_arch\":\"{}\"", std::env::consts::ARCH))
            && adapter_receipt.contains("\"connection_count\":2")
            && adapter_receipt.contains("\"key_update_response_recovery_observed\":true"),
        "actual-adapter receipt target and required WSS operation do not match this run"
    );
    let adapter_digest = Sha256::digest(&adapter_bytes);

    let receipt = make_receipt(
        baseline_current,
        baseline_rss,
        &[setup, connected, idle, read_write, shutdown, retired],
        driver_thread,
        fixture_thread_id,
        &adapter_input_path.to_string_lossy(),
        adapter_receipt,
        &adapter_digest,
    );
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .unwrap_or_else(|error| {
            panic!("create G5-E receipt at {}: {error}", output_path.display())
        });
    file.write_all(receipt.as_bytes())
        .expect("write complete G5-E JSON receipt");
    file.sync_all().expect("sync G5-E JSON receipt");
}

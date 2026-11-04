//! Reactor-owned TLS/WebSocket upgrade and bounded established WSS turns.
//!
//! The handshake worker transfers the already-authenticated rustls connection
//! and its original registered Mio socket. This module never performs the TLS
//! handshake or certificate verification.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

#[cfg(feature = "test-support")]
use super::soak_observer::{self, SoakDirection};
use mio::net::TcpStream;
use mio::{Interest, Registry};
use rustls::ClientConnection;
use tungstenite::client::IntoClientRequest;
use tungstenite::error::Error as WsError;
use tungstenite::handshake::client::{generate_request, Response};
use tungstenite::handshake::derive_accept_key;
use tungstenite::handshake::machine::{HandshakeMachine, RoundResult, StageResult};
use tungstenite::protocol::{Role, WebSocket, WebSocketConfig};
use tungstenite::Message;

use super::budget::{Budgeted, StopReason, TURN_BYTE_LIMIT, TURN_IO_ATTEMPTS};
use super::cancel::{AttemptRetire, CancelToken};
use super::control::{BindingId, RejectedCommand};
use super::park::WriterBinding;
use super::ring::{OUT_RING_BYTES, WS_MESSAGE_LIMIT};
use super::shared::{SharedConnection, StreamExitReason};
use super::tcp::{TcpStop, TcpTurnResult};
use super::tcp_endpoint::RebindBoundary;
use super::ws_frames::{admit_message, admit_pending, FrameFault, WsReceive};

const TLS_OUTPUT_PAUSE: usize = 64 * 1024;
const TLS_OUTPUT_CEILING: usize = 128 * 1024;
// One maximum TLS record plus overhead bounds automatic output until write_tls
// drains it or a later safe IoState sample includes it.
const TLS_CONTROL_OUTPUT_RESERVE: usize = 16 * 1024 + 256;
const HTTP_AUTHORITY_LIMIT: usize = 512;
const WS_WRITE_BUFFER_SIZE: usize = 8 * 1024;
const WS_WRITE_BUFFER_LIMIT: usize = 32 * 1024;

/// Result of advancing the nonblocking HTTP upgrade. `Ready` means the
/// authenticated TLS connection has also completed the WebSocket upgrade.
#[derive(Debug)]
pub enum WssStep {
    Ready,
    Progress,
    Wait,
    Failed(io::Error),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum WorkStop {
    #[default]
    Returned,
    SocketWouldBlock,
    Budget,
    HighWater,
    Closed,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum ClosePhase {
    #[default]
    Open,
    NotifyQueued,
    WriteHalfClosed,
}

#[derive(Debug, Default)]
struct TlsTurnBudget {
    plaintext_read: usize,
    plaintext_written: usize,
    rustls_operations: usize,
    read_stop: WorkStop,
    write_stop: WorkStop,
}

impl TlsTurnBudget {
    fn reset(&mut self) {
        *self = Self::default();
        self.read_stop = WorkStop::Returned;
        self.write_stop = WorkStop::Returned;
    }

    fn can_read_plaintext(&self) -> bool {
        self.plaintext_read < TURN_BYTE_LIMIT && self.rustls_operations < TURN_IO_ATTEMPTS
    }

    fn can_write_plaintext(&self) -> bool {
        self.plaintext_written < TURN_BYTE_LIMIT && self.rustls_operations < TURN_IO_ATTEMPTS
    }

    fn can_process_input(&self) -> bool {
        self.rustls_operations.saturating_add(4) <= TURN_IO_ATTEMPTS
    }
}

#[derive(Debug)]
struct TlsFailure(String);

impl std::fmt::Display for TlsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TlsFailure {}

/// The one TLS/socket pair. Tungstenite's handshake consumes its stream on a
/// protocol error, so an Arc keeps the *same* stream available to the reactor
/// for deregistration and physical retirement; all access remains reactor-only.
#[derive(Debug)]
pub(super) struct TlsIo {
    socket: Budgeted<TcpStream>,
    conn: ClientConnection,
    budget: TlsTurnBudget,
    pending_tls_output: usize,
    unmeasured_tls_output: usize,
    peer_tls_closed: bool,
    established: bool,
}

impl TlsIo {
    // rustls owns its input deframer and allocates its initial 4 KiB on read.
    // It grows to accept complete TLS records (up to 16 KiB plaintext plus
    // record overhead); this is not an eager per-service reservation.
    fn new(socket: TcpStream, mut conn: ClientConnection) -> Self {
        conn.set_buffer_limit(Some(TLS_OUTPUT_PAUSE));
        Self {
            socket: Budgeted::new(socket),
            conn,
            budget: TlsTurnBudget::default(),
            unmeasured_tls_output: 0,
            pending_tls_output: 0,
            peer_tls_closed: false,
            established: true,
        }
    }

    fn reset_turn(&mut self) {
        self.socket.reset_turn();
        self.budget.reset();
    }

    fn pending_output(&self) -> usize {
        self.pending_tls_output
            .saturating_add(self.unmeasured_tls_output)
    }
    #[cfg(feature = "test-support")]
    pub(super) fn soak_ledger_snapshot(&self) -> (Option<usize>, bool) {
        (
            (self.unmeasured_tls_output == 0).then_some(self.pending_tls_output),
            self.established,
        )
    }

    fn wants_write(&self) -> bool {
        self.conn.wants_write()
    }

    fn is_clean_tls_boundary(&self) -> bool {
        self.conn.wants_read()
    }

    fn process_packets(&mut self) -> io::Result<()> {
        self.budget.rustls_operations += 1;
        let state = self.conn.process_new_packets().map_err(tls_io_error)?;
        self.pending_tls_output = state.tls_bytes_to_write();
        self.unmeasured_tls_output = 0;
        self.peer_tls_closed = state.peer_has_closed();
        if self.pending_tls_output > TLS_OUTPUT_CEILING {
            return Err(tls_capacity_error(
                "TLS pending output exceeded the 128 KiB WSS ceiling",
            ));
        }
        if self.established && self.conn.is_handshaking() {
            return Err(tls_failure_io_error(
                io::ErrorKind::InvalidData,
                "rustls attempted a post-transfer handshake",
            ));
        }
        Ok(())
    }

    fn process_input(&mut self) -> io::Result<()> {
        self.process_packets()?;
        self.budget.rustls_operations += 1;
        let written = self.conn.writer().write(&[])?;
        debug_assert_eq!(written, 0, "an empty TLS write must not consume app bytes");
        if self.pending_output() >= TLS_OUTPUT_PAUSE {
            if self.conn.wants_write() {
                self.unmeasured_tls_output = TLS_CONTROL_OUTPUT_RESERVE;
                if self.pending_output() > TLS_OUTPUT_CEILING {
                    return Err(tls_capacity_error(
                        "TLS pending output exceeded the 128 KiB WSS ceiling",
                    ));
                }
            }
            return Ok(());
        }
        self.process_packets()
    }

    fn read_plaintext(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        if dst.is_empty() {
            return Ok(0);
        }

        loop {
            if self.pending_output() >= TLS_OUTPUT_PAUSE {
                self.budget.read_stop = WorkStop::HighWater;
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let remaining = TURN_BYTE_LIMIT.saturating_sub(self.budget.plaintext_read);
            if remaining == 0 || !self.budget.can_read_plaintext() {
                self.budget.read_stop = WorkStop::Budget;
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let allowed = dst.len().min(remaining);
            self.budget.rustls_operations += 1;
            match self.conn.reader().read(&mut dst[..allowed]) {
                Ok(0) => {
                    self.budget.read_stop = WorkStop::Closed;
                    return Ok(0);
                }
                Ok(read) => {
                    self.budget.plaintext_read += read;
                    self.budget.read_stop = WorkStop::Returned;
                    return Ok(read);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if self.peer_tls_closed {
                        self.budget.read_stop = WorkStop::Closed;
                        return Ok(0);
                    }
                }
                Err(error) => {
                    self.budget.read_stop = WorkStop::Returned;
                    return Err(error);
                }
            }

            if !self.budget.can_process_input() {
                self.budget.read_stop = WorkStop::Budget;
                return Err(io::ErrorKind::WouldBlock.into());
            }

            self.budget.rustls_operations += 1;
            let encrypted = match self.conn.read_tls(&mut self.socket) {
                Ok(0) => {
                    self.budget.read_stop = WorkStop::Closed;
                    return Err(tls_failure_io_error(
                        io::ErrorKind::UnexpectedEof,
                        "WSS peer closed the TCP stream without TLS close_notify",
                    ));
                }
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.budget.read_stop = match self.socket.budget().read_stop() {
                        StopReason::SocketWouldBlock => WorkStop::SocketWouldBlock,
                        StopReason::Budget => WorkStop::Budget,
                        StopReason::Returned => WorkStop::SocketWouldBlock,
                    };
                    return Err(error);
                }
                Err(error) => {
                    self.budget.read_stop = WorkStop::Returned;
                    return Err(error);
                }
            };
            debug_assert_ne!(encrypted, 0);
            self.process_input()?;
        }
    }

    fn write_plaintext(&mut self, src: &[u8]) -> io::Result<usize> {
        if src.is_empty() {
            return Ok(0);
        }
        let pending_output = self.pending_output();
        if pending_output > TLS_OUTPUT_CEILING {
            return Err(tls_capacity_error(
                "TLS pending output exceeded the 128 KiB WSS ceiling",
            ));
        }
        if pending_output >= TLS_OUTPUT_PAUSE {
            self.budget.write_stop = WorkStop::HighWater;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let remaining = TURN_BYTE_LIMIT.saturating_sub(self.budget.plaintext_written);
        if remaining == 0 || !self.budget.can_write_plaintext() {
            self.budget.write_stop = WorkStop::Budget;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        if self.budget.rustls_operations.saturating_add(2) > TURN_IO_ATTEMPTS {
            self.budget.write_stop = WorkStop::Budget;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        // Leave one maximum TLS record plus overhead for automatic control
        // output so a large WebSocket write cannot exceed the aggregate cap.
        let output_allowance = TLS_OUTPUT_CEILING
            .saturating_sub(pending_output)
            .saturating_sub(TLS_CONTROL_OUTPUT_RESERVE);
        if output_allowance == 0 {
            self.budget.write_stop = WorkStop::HighWater;
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let limit = src.len().min(remaining).min(output_allowance);
        self.budget.rustls_operations += 1;
        let written = match self.conn.writer().write(&src[..limit]) {
            Ok(written) => written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.budget.write_stop = WorkStop::HighWater;
                return Err(error);
            }
            Err(error) => {
                self.budget.write_stop = WorkStop::Returned;
                return Err(error);
            }
        };
        self.budget.plaintext_written += written;
        self.process_packets()?;
        self.budget.write_stop = WorkStop::Returned;
        Ok(written)
    }

    fn flush_tls(&mut self) -> io::Result<()> {
        while self.conn.wants_write() {
            if self.budget.rustls_operations >= TURN_IO_ATTEMPTS {
                self.budget.write_stop = WorkStop::Budget;
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.budget.rustls_operations += 1;
            match self.conn.write_tls(&mut self.socket) {
                Ok(0) => {
                    if self.conn.wants_write() {
                        self.budget.write_stop = WorkStop::Returned;
                        return Err(io::ErrorKind::WriteZero.into());
                    }
                }
                Ok(written) => {
                    let known = written.min(self.pending_tls_output);
                    self.pending_tls_output -= known;
                    self.unmeasured_tls_output =
                        self.unmeasured_tls_output.saturating_sub(written - known);
                    self.budget.write_stop = WorkStop::Returned;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.budget.write_stop = match self.socket.budget().write_stop() {
                        StopReason::SocketWouldBlock => WorkStop::SocketWouldBlock,
                        StopReason::Budget => WorkStop::Budget,
                        StopReason::Returned => WorkStop::SocketWouldBlock,
                    };
                    return Err(error);
                }
                Err(error) => {
                    self.budget.write_stop = WorkStop::Returned;
                    return Err(error);
                }
            }
        }
        self.pending_tls_output = 0;
        self.unmeasured_tls_output = 0;
        self.budget.write_stop = WorkStop::Returned;
        Ok(())
    }
}

impl TlsIo {
    // The stopped session discards all peer records. The local terminator
    // sends TCP FIN without a reciprocal TLS close_notify.
    fn drain_after_close(&mut self) -> io::Result<bool> {
        if self.peer_tls_closed {
            return Ok(true);
        }
        let mut discard = [0u8; 4096];
        loop {
            if self.budget.rustls_operations >= TURN_IO_ATTEMPTS
                || self.budget.plaintext_read >= TURN_BYTE_LIMIT
            {
                self.budget.read_stop = WorkStop::Budget;
                return Ok(false);
            }
            let allowed = discard
                .len()
                .min(TURN_BYTE_LIMIT - self.budget.plaintext_read);
            self.budget.rustls_operations += 1;
            match self.conn.reader().read(&mut discard[..allowed]) {
                Ok(0) => {
                    if self.peer_tls_closed {
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Ok(read) => {
                    self.budget.plaintext_read += read;
                    continue;
                }
                Err(error) => return Err(error),
            }
            if self.budget.rustls_operations >= TURN_IO_ATTEMPTS {
                self.budget.read_stop = WorkStop::Budget;
                return Ok(false);
            }
            self.budget.rustls_operations += 1;
            match self.conn.read_tls(&mut self.socket) {
                Ok(0) => {
                    self.budget.read_stop = WorkStop::Closed;
                    return Ok(true);
                }
                Ok(_) => {
                    self.process_packets()?;
                    if self.peer_tls_closed {
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.budget.read_stop = match self.socket.budget().read_stop() {
                        StopReason::Budget => WorkStop::Budget,
                        _ => WorkStop::SocketWouldBlock,
                    };
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TlsStreamHandle(Arc<Mutex<TlsIo>>);

impl TlsStreamHandle {
    fn lock(&self) -> MutexGuard<'_, TlsIo> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Read for TlsStreamHandle {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        self.lock().read_plaintext(dst)
    }
}

impl Write for TlsStreamHandle {
    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        self.lock().write_plaintext(src)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.lock().flush_tls()
    }
}

#[derive(Debug)]
enum UpgradeState {
    Writing(HandshakeMachine<TlsStreamHandle>),
    Reading(HandshakeMachine<TlsStreamHandle>),
}

/// Reactor-only endpoint for a transferred, established TLS connection.
#[derive(Debug)]
pub struct WssEndpoint {
    binding: BindingId,
    #[cfg(feature = "test-support")]
    local_port: Option<u16>,
    io: Option<Arc<Mutex<TlsIo>>>,
    upgrade: Option<UpgradeState>,
    setup_error: Option<io::Error>,
    websocket: Option<WebSocket<TlsStreamHandle>>,
    shared: Arc<SharedConnection>,
    cancel: Option<CancelToken>,
    deadline: Option<Instant>,
    retire: Arc<AttemptRetire>,
    expected_accept: Option<String>,
    setup_local: bool,
    read_local: bool,
    write_local: bool,
    read_suspended: bool,
    tls_read_paused: bool,
    pending_inbound: Option<Vec<u8>>,
    ws_write_pending: bool,
    graceful_ws_started: bool,
    close_phase: ClosePhase,
    peer_closed: bool,
    clean_rebind: bool,
    boundary_dirty: bool,
    rebind_draining: bool,
    registered: bool,
    last_interest: Option<Interest>,
    retired: bool,
}

impl WssEndpoint {
    /// Begin the nonblocking HTTP upgrade over the transferred TLS connection.
    /// `host` is the URI authority (including a non-default port, if any).
    /// The socket remains the original registered Mio socket.
    // The handoff keeps all eight independently owned inputs; avoid allocating
    // a temporary argument bundle on every WSS transfer only for lint shape.
    #[expect(
        clippy::too_many_arguments,
        reason = "A WSS transfer retains explicit binding, TLS, and resource owners"
    )]
    pub fn new(
        binding: BindingId,
        host: &str,
        socket: TcpStream,
        conn: ClientConnection,
        shared: Arc<SharedConnection>,
        cancel: CancelToken,
        deadline: Option<Instant>,
        retire: Arc<AttemptRetire>,
    ) -> io::Result<Self> {
        #[cfg(feature = "test-support")]
        let allocation_scope = soak_observer::begin_allocation_scope(
            soak_observer::AllocationScopeKind::WssEndpoint,
            Some(binding.connection),
            Some(binding),
            None,
        );
        #[cfg(feature = "test-support")]
        let local_port = socket.local_addr().ok().map(|address| address.port());
        let mut endpoint = Self {
            binding,
            #[cfg(feature = "test-support")]
            local_port,
            io: Some(Arc::new(Mutex::new(TlsIo::new(socket, conn)))),
            upgrade: None,
            setup_error: None,
            websocket: None,
            shared,
            cancel: Some(cancel),
            deadline,
            retire,
            expected_accept: None,
            setup_local: true,
            read_local: false,
            write_local: false,
            read_suspended: false,
            tls_read_paused: false,
            pending_inbound: None,
            ws_write_pending: false,
            graceful_ws_started: false,
            close_phase: ClosePhase::Open,
            peer_closed: false,
            clean_rebind: false,
            boundary_dirty: false,
            rebind_draining: false,
            registered: true,
            last_interest: Some(Interest::READABLE.add(Interest::WRITABLE)),
            retired: false,
        };

        let invalid_identity = binding.incarnation == 0
            || binding.connection == 0
            || binding.binding == 0
            || binding.attempt == 0
            || usize::try_from(binding.connection).is_err()
            || endpoint.shared.control.id != binding.connection
            || endpoint
                .cancel
                .as_ref()
                .is_none_or(|token| token.generation() != endpoint.retire.generation());
        if invalid_identity {
            endpoint.setup_error = Some(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid WSS attempt identity",
            ));
            return Ok(endpoint);
        }
        if host.is_empty() || host.len() > HTTP_AUTHORITY_LIMIT {
            endpoint.setup_error = Some(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid WSS HTTP authority",
            ));
            return Ok(endpoint);
        }
        if endpoint.with_tls(|tls| tls.conn.is_handshaking()) {
            endpoint.setup_error = Some(io::Error::new(
                io::ErrorKind::InvalidData,
                "GameTlsLane transferred a handshaking rustls connection",
            ));
            return Ok(endpoint);
        }

        let request = build_upgrade_request(host);
        match request {
            Ok((request, accept_key)) => {
                let stream = TlsStreamHandle(Arc::clone(endpoint.io.as_ref().expect("TLS owner")));
                let machine = HandshakeMachine::start_write(stream, request);
                endpoint.setup_local = true;
                endpoint.write_local = true;
                endpoint.expected_accept = Some(accept_key);
                endpoint.upgrade = Some(UpgradeState::Writing(machine));
            }
            Err(error) => endpoint.setup_error = Some(error),
        }
        #[cfg(feature = "test-support")]
        drop(allocation_scope);
        #[cfg(feature = "test-support")]
        if let Some(tls) = endpoint.io.as_ref() {
            soak_observer::register_wss_ledger(binding, endpoint.local_port, tls, &endpoint.shared);
        }
        Ok(endpoint)
    }

    pub fn binding(&self) -> BindingId {
        self.binding
    }

    #[cfg(feature = "test-support")]
    fn allocation_scope(
        &self,
        kind: soak_observer::AllocationScopeKind,
    ) -> soak_observer::AllocationScopeGuard {
        soak_observer::begin_allocation_scope(
            kind,
            Some(self.binding.connection),
            Some(self.binding),
            None,
        )
    }

    /// Connected is published only after the 101 response and binary protocol
    /// have both been authenticated by the tungstenite-compatible verifier.
    pub fn is_connected(&self) -> bool {
        self.websocket.is_some()
    }

    /// Read-only integration-test witness: the last TLS flush reached a real
    /// socket WouldBlock and retained encrypted output for writable recovery.
    #[cfg(feature = "test-support")]
    pub fn tls_output_blocked_for_test(&self) -> bool {
        self.with_tls(|tls| {
            tls.budget.write_stop == WorkStop::SocketWouldBlock && tls.conn.wants_write()
        })
    }

    fn desired_interest(&self) -> Option<Interest> {
        if self.setup_error.is_some() {
            return None;
        }
        let read = if self.close_phase == ClosePhase::WriteHalfClosed {
            true
        } else if self.is_connected() {
            !self.tls_read_paused
                && !self.read_suspended
                && !self.peer_closed
                && !self.shared.rebind_disabled()
        } else {
            !self.tls_read_paused
        };
        let write = if self.close_phase == ClosePhase::WriteHalfClosed {
            false
        } else if self.is_connected() {
            self.has_write_work() && !self.write_local
        } else {
            !self.write_local && (self.handshake_write_pending() || self.tls_wants_write())
        };
        match (read, write) {
            (false, false) => None,
            (true, false) => Some(Interest::READABLE),
            (false, true) => Some(Interest::WRITABLE),
            (true, true) => Some(Interest::READABLE.add(Interest::WRITABLE)),
        }
    }

    /// Keep the socket registered only for readiness that can advance this
    /// connection. An idle WSS session is never writable-polled.
    pub fn apply_interest(&mut self, registry: &Registry) -> io::Result<()> {
        let desired = self.desired_interest();
        if desired == self.last_interest {
            return Ok(());
        }
        let registered = self.registered;
        let token = mio::Token(self.binding.connection as usize);
        match desired {
            Some(interest) => {
                self.with_tls_mut(|tls| {
                    let socket = tls.socket.inner_mut();
                    if registered {
                        registry.reregister(socket, token, interest)
                    } else {
                        registry.register(socket, token, interest)
                    }
                })?;
                self.registered = true;
                self.last_interest = Some(interest);
            }
            None if registered => {
                self.with_tls_mut(|tls| registry.deregister(tls.socket.inner_mut()))?;
                self.registered = false;
                self.last_interest = None;
            }
            None => {
                self.last_interest = None;
            }
        }
        Ok(())
    }

    /// Apply readiness and advance at most one resumable HTTP-upgrade step.
    /// For an established endpoint this only records the readiness hint; the
    /// reactor invokes `drive` for its one fair session turn.
    pub fn on_event(&mut self, readable: bool, writable: bool, now: Instant) -> WssStep {
        if readable
            && (self.close_phase == ClosePhase::WriteHalfClosed
                || (!self.tls_read_paused
                    && !self.read_suspended
                    && !self.shared.rebind_disabled()))
        {
            self.read_local = true;
            self.clean_rebind = false;
        }
        if writable {
            self.write_local = true;
            self.clean_rebind = false;
        }
        if self.is_connected() {
            return WssStep::Ready;
        }
        if !self.setup_local && !self.read_local && !self.write_local {
            return WssStep::Wait;
        }
        self.advance_upgrade(now)
    }
    /// Run the initial local HTTP-upgrade step without waiting for a new
    /// readiness event. TLS transfer can include already-buffered bytes.
    pub fn begin_step(&mut self, now: Instant) -> WssStep {
        if self.is_connected() {
            return WssStep::Ready;
        }
        if !self.setup_local && !self.read_local && !self.write_local {
            return WssStep::Wait;
        }
        self.advance_upgrade(now)
    }

    pub fn resume_read(&mut self) {
        self.clean_rebind = false;
        if self.shared.rebind_disabled() {
            return;
        }
        if let Some(pending) = self.pending_inbound.take() {
            match admit_pending(pending, &self.shared.inbound) {
                WsReceive::Binary(_) => {
                    self.read_suspended = false;
                }
                WsReceive::Backpressured(pending) => {
                    self.pending_inbound = Some(pending);
                    self.read_suspended = true;
                }
                _ => unreachable!("pending binary admission only returns binary/backpressure"),
            }
        }
        if self.pending_inbound.is_none() && self.shared.inbound.free() > 0 {
            self.read_suspended = false;
        }
        if !self.read_suspended && self.shared.inbound.free() > 0 {
            self.read_local = true;
        }
    }

    pub fn kick_output(&mut self) {
        self.clean_rebind = false;
        self.write_local = true;
    }

    /// Advance at most one WSS read, one semantic record, and their bounded
    /// explicit flushes. A returned message is always locally requeued because
    /// tungstenite may retain additional plaintext or frames privately.
    pub fn drive(&mut self) -> io::Result<TcpTurnResult> {
        if !self.is_connected() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WSS upgrade is not complete",
            ));
        }
        self.clean_rebind = false;
        self.with_tls_mut(TlsIo::reset_turn);
        let rebind_read_before = if self.rebind_draining {
            Some(self.with_tls(|tls| (tls.budget.plaintext_read, tls.socket.budget().read_bytes())))
        } else {
            None
        };

        let mut read_bytes = 0usize;
        let mut written_bytes = 0usize;
        let mut returned_message = false;
        let mut read_error = None;
        let may_read = self.read_local
            && !self.read_suspended
            && !self.peer_closed
            && (!self.shared.rebind_disabled() || self.rebind_draining)
            && (self.rebind_draining || self.shared.inbound.free() > 0);
        if self.read_local && !self.rebind_draining && self.shared.inbound.free() == 0 {
            self.read_local = false;
            self.read_suspended = true;
        }

        if may_read {
            let result = {
                #[cfg(feature = "test-support")]
                let _allocation_scope =
                    self.allocation_scope(soak_observer::AllocationScopeKind::WssMessageStaging);
                self.websocket.as_mut().expect("connected WSS").read()
            };
            match result {
                Ok(message) => {
                    #[cfg(feature = "test-support")]
                    if let Message::Binary(payload) = &message {
                        soak_observer::observe(
                            self.binding,
                            self.local_port,
                            SoakDirection::Inbound,
                            payload,
                        );
                    }
                    returned_message = true;
                    self.read_local = true;
                    if matches!(&message, Message::Ping(_) | Message::Close(_)) {
                        self.ws_write_pending = true;
                    }
                    if self.rebind_draining {
                        self.boundary_dirty = true;
                        if matches!(&message, Message::Close(_)) {
                            if let Err(error) = self.receive_peer_close(message, &mut read_bytes) {
                                read_error = Some(error);
                            }
                        }
                    } else {
                        match admit_message(message, &self.shared.inbound) {
                            Ok(WsReceive::Binary(count)) => read_bytes = count,
                            Ok(WsReceive::Backpressured(pending)) => {
                                self.pending_inbound = Some(pending);
                                self.read_suspended = true;
                                self.read_local = false;
                            }
                            Ok(WsReceive::Control) => {}
                            Ok(WsReceive::PeerClosed { code, reason }) => {
                                self.peer_closed = true;
                                self.read_suspended = true;
                                self.read_local = false;
                                self.shared
                                    .publish_exit_reason(StreamExitReason::WebSocketClose {
                                        code,
                                        reason,
                                    });
                            }
                            Err(fault) => read_error = Some(frame_fault_error(fault)),
                        }
                    }
                }
                Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    let stop = self.with_tls(|tls| tls.budget.read_stop);
                    match stop {
                        WorkStop::SocketWouldBlock => self.read_local = false,
                        WorkStop::Budget => self.read_local = true,
                        WorkStop::HighWater => self.read_local = false,
                        WorkStop::Closed => {
                            self.peer_closed = true;
                            self.read_suspended = true;
                            self.read_local = false;
                        }
                        WorkStop::Returned => self.read_local = false,
                    }
                }
                Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {
                    self.peer_closed = true;
                    self.read_suspended = true;
                    self.read_local = false;
                }
                Err(error) => read_error = Some(websocket_io_error(error)),
            }
        }
        if may_read && !returned_message {
            let (plain, wire) =
                self.with_tls(|tls| (tls.budget.plaintext_read, tls.socket.budget().read_bytes()));
            if plain != 0 || wire != 0 {
                self.boundary_dirty = true;
            }
        }
        if let Some((before_plain, before_wire)) = rebind_read_before {
            let (plain, wire) =
                self.with_tls(|tls| (tls.budget.plaintext_read, tls.socket.budget().read_bytes()));
            if plain != before_plain || wire != before_wire {
                self.boundary_dirty = true;
            }
        }

        // One queued semantic record at a time. The full record is moved from
        // the ring into tungstenite once; WouldBlock leaves it in tungstenite's
        // bounded output buffer and it is never submitted a second time.
        if !self.peer_closed
            && read_error.is_none()
            && self.write_local
            && !self.ws_write_pending
            && self.with_tls(|tls| tls.pending_output()) < TLS_OUTPUT_PAUSE
        {
            if let Some(length) = self.shared.records.front().map(usize::from) {
                if length == 0 || length > OUT_RING_BYTES || length > WS_MESSAGE_LIMIT {
                    let error =
                        io::Error::new(io::ErrorKind::InvalidData, "invalid WSS record length");
                    self.publish_error(&error);
                    return Err(error);
                }
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(length)
                    .map_err(|error| io::Error::new(io::ErrorKind::OutOfMemory, error))?;
                bytes.resize(length, 0);
                if self.shared.outbound.pop(&mut bytes) != length {
                    let error = io::Error::new(
                        io::ErrorKind::InvalidData,
                        "WSS record bytes are incomplete",
                    );
                    self.publish_error(&error);
                    return Err(error);
                }
                #[cfg(feature = "test-support")]
                soak_observer::observe(
                    self.binding,
                    self.local_port,
                    SoakDirection::Outbound,
                    &bytes,
                );
                let result = {
                    #[cfg(feature = "test-support")]
                    let _allocation_scope = self
                        .allocation_scope(soak_observer::AllocationScopeKind::WssMessageStaging);
                    self.websocket
                        .as_mut()
                        .expect("connected WSS")
                        .write(Message::Binary(bytes))
                };
                match result {
                    Ok(()) => {
                        self.shared.records.pop_front();
                        self.shared.control.park.publish_freed_space(WriterBinding {
                            connection: self.binding.connection,
                            binding: self.binding.binding,
                        });
                        written_bytes = length;
                        self.ws_write_pending = true;
                    }
                    Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                        // tungstenite retains a frame once `write` has begun;
                        // retry only its flush, never the semantic message.
                        self.shared.records.pop_front();
                        self.shared.control.park.publish_freed_space(WriterBinding {
                            connection: self.binding.connection,
                            binding: self.binding.binding,
                        });
                        written_bytes = length;
                        self.ws_write_pending = true;
                    }
                    Err(WsError::WriteBufferFull(_)) => {
                        let error = io::Error::new(
                            io::ErrorKind::OutOfMemory,
                            "bounded WSS output buffer is full",
                        );
                        self.publish_error(&error);
                        return Err(error);
                    }
                    Err(error) => {
                        let error = websocket_io_error(error);
                        self.publish_error(&error);
                        return Err(error);
                    }
                }
            }
        }

        // Flush after every tungstenite read attempt, including WouldBlock.
        // TLS KeyUpdate output remains eligible without another readable edge.
        let need_flush =
            may_read || self.ws_write_pending || self.with_tls(|tls| tls.wants_write());
        if need_flush {
            if let Err(error) = self.flush_websocket() {
                self.publish_error(&error);
                return Err(error);
            }
        }
        if let Some(error) = read_error {
            self.publish_error(&error);
            return Err(error);
        }
        let (read_stop, write_stop, pending_output, wants_write) = self.with_tls(|tls| {
            (
                tls.budget.read_stop,
                tls.budget.write_stop,
                tls.pending_output(),
                tls.wants_write(),
            )
        });
        if !matches!(read_stop, WorkStop::HighWater) {
            if matches!(
                read_stop,
                WorkStop::SocketWouldBlock | WorkStop::Budget | WorkStop::Closed
            ) || (read_stop == WorkStop::Returned && returned_message)
            {
                self.tls_read_paused = false;
            }
        } else {
            self.tls_read_paused = true;
        }
        if !self.read_suspended && !self.peer_closed && !self.shared.rebind_disabled() {
            match read_stop {
                WorkStop::SocketWouldBlock | WorkStop::Closed => self.read_local = false,
                WorkStop::HighWater => self.read_local = pending_output < TLS_OUTPUT_PAUSE,
                WorkStop::Budget => self.read_local = true,
                WorkStop::Returned if returned_message => self.read_local = true,
                WorkStop::Returned => {}
            }
        }
        if self.peer_closed {
            self.read_local = false;
            self.tls_read_paused = false;
        } else if self.tls_read_paused
            && pending_output < TLS_OUTPUT_PAUSE
            && !self.read_suspended
            && !self.shared.rebind_disabled()
        {
            self.read_local = true;
            self.tls_read_paused = false;
        }
        let output_pending =
            self.ws_write_pending || wants_write || self.shared.records.front().is_some();
        match write_stop {
            WorkStop::SocketWouldBlock => self.write_local = false,
            WorkStop::Budget => self.write_local = true,
            WorkStop::HighWater => self.write_local = pending_output < TLS_OUTPUT_PAUSE,
            WorkStop::Closed => self.write_local = false,
            WorkStop::Returned => {
                if output_pending && (written_bytes != 0 || returned_message || self.write_local) {
                    self.write_local = true;
                } else if !output_pending {
                    self.write_local = false;
                }
            }
        }
        if pending_output >= TLS_OUTPUT_PAUSE
            && wants_write
            && write_stop == WorkStop::SocketWouldBlock
        {
            self.write_local = false;
        }

        let stop = if self.peer_closed {
            if self.ws_write_pending || wants_write {
                if write_stop == WorkStop::SocketWouldBlock {
                    TcpStop::Quiescent
                } else {
                    TcpStop::LocalWork
                }
            } else {
                TcpStop::PeerClosed
            }
        } else if self.read_local || self.write_local {
            TcpStop::LocalWork
        } else if self.read_suspended {
            TcpStop::HighWater
        } else {
            TcpStop::Quiescent
        };
        Ok(self.turn_result(stop, read_bytes, written_bytes))
    }

    /// Flush accepted records and WebSocket Close, then send TLS close_notify.
    /// Half-close the write side and drain peer TLS records until an alert or
    /// TCP FIN. The reactor bounds flushing and draining separately.
    pub fn flush_for_close(&mut self) -> io::Result<(bool, bool)> {
        self.read_suspended = true;
        self.read_local = false;
        if !self.shared.outbound.is_empty() || self.shared.records.front().is_some() {
            self.write_local = true;
            let turn = self.drive()?;
            if turn.stop == TcpStop::PeerClosed {
                return Ok((true, false));
            }
            let records_drained = self.shared.outbound.is_empty()
                && self.shared.records.front().is_none()
                && !self.ws_write_pending;
            return Ok((false, records_drained || turn.write_ready));
        }

        self.with_tls_mut(TlsIo::reset_turn);
        if self.close_phase == ClosePhase::WriteHalfClosed {
            return self.drain_close_turn();
        }
        if !self.graceful_ws_started {
            let close_result = {
                #[cfg(feature = "test-support")]
                let _allocation_scope =
                    self.allocation_scope(soak_observer::AllocationScopeKind::WssMessageStaging);
                self.websocket.as_mut().expect("connected WSS").close(None)
            };
            match close_result {
                Ok(()) => self.graceful_ws_started = true,
                Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.graceful_ws_started = true;
                    self.ws_write_pending = true;
                    let local = self.write_work_locally_runnable();
                    self.write_local = local;
                    return Ok((false, local));
                }
                Err(error) => return Err(websocket_io_error(error)),
            }
            self.ws_write_pending = true;
        }

        if self.with_tls(|tls| tls.pending_output()) >= TLS_OUTPUT_PAUSE {
            match self.flush_tls() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let local = self.write_work_locally_runnable();
                    self.write_local = local;
                    return Ok((false, local));
                }
                Err(error) => return Err(error),
            }
        }
        let flush_result = {
            #[cfg(feature = "test-support")]
            let _allocation_scope =
                self.allocation_scope(soak_observer::AllocationScopeKind::WssMessageStaging);
            self.websocket.as_mut().expect("connected WSS").flush()
        };
        match flush_result {
            Ok(()) => self.ws_write_pending = false,
            Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                self.ws_write_pending = true;
                let local = self.write_work_locally_runnable();
                self.write_local = local;
                return Ok((false, local));
            }
            Err(error) => return Err(websocket_io_error(error)),
        }

        if self.close_phase == ClosePhase::Open {
            if !self
                .with_tls(|tls| tls.budget.rustls_operations.saturating_add(1) <= TURN_IO_ATTEMPTS)
            {
                self.write_local = true;
                return Ok((false, true));
            }
            self.with_tls_mut(|tls| {
                tls.conn.send_close_notify();
                tls.budget.rustls_operations += 1;
                if tls.conn.wants_write() {
                    tls.unmeasured_tls_output = TLS_CONTROL_OUTPUT_RESERVE;
                }
            });
            if self.with_tls(|tls| tls.pending_output() > TLS_OUTPUT_CEILING) {
                let error =
                    tls_capacity_error("TLS pending output exceeded the 128 KiB WSS ceiling");
                self.publish_error(&error);
                return Err(error);
            }
            self.close_phase = ClosePhase::NotifyQueued;
        }
        if self.close_phase == ClosePhase::NotifyQueued {
            match self.flush_tls() {
                Ok(()) => {
                    self.with_tls_mut(|tls| tls.socket.inner_mut().shutdown(Shutdown::Write))?;
                    self.close_phase = ClosePhase::WriteHalfClosed;
                    self.write_local = false;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let local = self.write_work_locally_runnable();
                    self.write_local = local;
                    return Ok((false, local));
                }
                Err(error) => return Err(error),
            }
        }
        self.drain_close_turn()
    }

    fn drain_close_turn(&mut self) -> io::Result<(bool, bool)> {
        let complete = self.with_tls_mut(TlsIo::drain_after_close)?;
        self.read_local =
            !complete && self.with_tls(|tls| tls.budget.read_stop == WorkStop::Budget);
        Ok((complete, self.read_local))
    }

    pub fn draining_for_close(&self) -> bool {
        self.close_phase == ClosePhase::WriteHalfClosed
    }

    pub fn attempt_failure(&self, now: Instant) -> Option<io::ErrorKind> {
        self.revoked(now).map(|error| error.kind())
    }

    /// Detach login authority only. The live TLS/WS parsers, pending records,
    /// rings and socket are deliberately unchanged.
    pub fn seal(&mut self, identity: BindingId, now: Instant) -> Result<Instant, RejectedCommand> {
        if identity != self.binding {
            return Err(RejectedCommand::StaleIdentity);
        }
        if !self.is_connected() || self.shared.is_closed() {
            return Err(RejectedCommand::Closed);
        }
        let token = self.cancel.as_ref().ok_or(RejectedCommand::StaleIdentity)?;
        if token.is_cancelled() {
            return Err(RejectedCommand::Revoked);
        }
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            return Err(RejectedCommand::Deadline);
        }
        self.cancel = None;
        self.deadline = None;
        Ok(now)
    }

    /// login. The parser is never inspected: any interrupted WS/TLS input is
    /// dirty, and the final probe must reach a real socket WouldBlock without
    /// consuming new bytes or queuing control output.
    pub fn drain_for_rebind(&mut self) -> io::Result<RebindBoundary> {
        self.clean_rebind = false;
        if !self.is_connected() || self.peer_closed || self.shared.is_closed() {
            return Ok(RebindBoundary::PeerClosed);
        }
        if !self.shared.rebind_disabled() || self.shared.session_binding() != self.binding.binding {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "old WSS session was not quiesced for Rebind",
            ));
        }
        if self.boundary_dirty || self.pending_inbound.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WSS receive boundary is not provably clean",
            ));
        }
        if !self.shared.outbound.is_empty()
            || self.shared.records.front().is_some()
            || self.ws_write_pending
        {
            self.rebind_draining = true;
            self.write_local = true;
            let turn = self.drive();
            self.rebind_draining = false;
            let turn = turn?;
            if self.boundary_dirty {
                return Ok(RebindBoundary::Pending);
            }
            if turn.stop == TcpStop::PeerClosed {
                return Ok(RebindBoundary::PeerClosed);
            }
            if !self.shared.outbound.is_empty()
                || self.shared.records.front().is_some()
                || self.ws_write_pending
            {
                return Ok(if !turn.write_ready {
                    RebindBoundary::WaitWritable
                } else {
                    RebindBoundary::Pending
                });
            }
        }
        if self
            .shared
            .inbound
            .consume_with(|span| Ok(span.len().min(TURN_BYTE_LIMIT)))?
            != 0
        {
            return Ok(RebindBoundary::Pending);
        }

        self.rebind_draining = true;
        self.with_tls_mut(TlsIo::reset_turn);
        let before_plain = self.with_tls(|tls| tls.budget.plaintext_read);
        let before_wire = self.with_tls(|tls| tls.socket.budget().read_bytes());
        let result = {
            #[cfg(feature = "test-support")]
            let _allocation_scope =
                self.allocation_scope(soak_observer::AllocationScopeKind::WssMessageStaging);
            self.websocket.as_mut().expect("connected WSS").read()
        };
        if matches!(&result, Ok(Message::Ping(_)) | Ok(Message::Close(_))) {
            self.ws_write_pending = true;
        }
        let flush_result = self.flush_websocket();
        self.rebind_draining = false;
        if let Err(error) = flush_result {
            self.publish_error(&error);
            return Err(error);
        }
        let (plain, wire, stop, write_stop, pending_output, wants_write) = self.with_tls(|tls| {
            (
                tls.budget.plaintext_read,
                tls.socket.budget().read_bytes(),
                tls.budget.read_stop,
                tls.budget.write_stop,
                tls.pending_output(),
                tls.wants_write(),
            )
        });
        match result {
            Ok(message) => {
                #[cfg(feature = "test-support")]
                if let Message::Binary(payload) = &message {
                    soak_observer::observe(
                        self.binding,
                        self.local_port,
                        SoakDirection::Inbound,
                        payload,
                    );
                }
                self.boundary_dirty = true;
                match admit_message(message, &self.shared.inbound) {
                    Ok(WsReceive::PeerClosed { code, reason }) => {
                        self.peer_closed = true;
                        self.read_suspended = true;
                        self.read_local = false;
                        self.shared
                            .publish_exit_reason(StreamExitReason::WebSocketClose { code, reason });
                        Ok(RebindBoundary::PeerClosed)
                    }
                    Ok(WsReceive::Backpressured(pending)) => {
                        self.pending_inbound = Some(pending);
                        Ok(RebindBoundary::Pending)
                    }
                    Ok(_) => Ok(RebindBoundary::Pending),
                    Err(fault) => {
                        let error = frame_fault_error(fault);
                        self.publish_error(&error);
                        Err(error)
                    }
                }
            }
            Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                if stop == WorkStop::SocketWouldBlock
                    && plain == before_plain
                    && wire == before_wire
                    && pending_output == 0
                    && !wants_write
                    && !self.ws_write_pending
                    && self.with_tls(|tls| tls.is_clean_tls_boundary())
                {
                    self.clean_rebind = true;
                    Ok(RebindBoundary::Clean)
                } else if write_stop == WorkStop::SocketWouldBlock
                    || (stop == WorkStop::HighWater && wants_write)
                {
                    self.write_local = false;
                    Ok(RebindBoundary::WaitWritable)
                } else {
                    if stop == WorkStop::SocketWouldBlock
                        && (plain != before_plain || wire != before_wire)
                    {
                        self.boundary_dirty = true;
                    }
                    if write_stop == WorkStop::Budget || stop == WorkStop::Budget {
                        self.write_local = true;
                    }
                    Ok(RebindBoundary::Pending)
                }
            }
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {
                self.peer_closed = true;
                self.read_suspended = true;
                self.read_local = false;
                Ok(RebindBoundary::PeerClosed)
            }
            Err(error) => {
                let error = websocket_io_error(error);
                self.publish_error(&error);
                Err(error)
            }
        }
    }

    pub fn rebind(
        &mut self,
        expected: BindingId,
        next: BindingId,
        deadline: Instant,
        now: Instant,
        token: CancelToken,
        retire: Arc<AttemptRetire>,
    ) -> Result<(), RejectedCommand> {
        if expected != self.binding
            || next.incarnation != expected.incarnation
            || next.connection != expected.connection
            || next.binding == expected.binding
            || next.binding == 0
            || next.attempt == expected.attempt
            || next.attempt == 0
            || token.generation() != retire.generation()
        {
            return Err(RejectedCommand::StaleIdentity);
        }
        if !self.is_connected() || self.shared.is_closed() {
            return Err(RejectedCommand::Closed);
        }
        if token.is_cancelled() {
            return Err(RejectedCommand::Revoked);
        }
        if now >= deadline {
            return Err(RejectedCommand::Deadline);
        }
        if !self.clean_rebind
            || self.cancel.is_some()
            || self.boundary_dirty
            || self.pending_inbound.is_some()
            || !self.shared.rebind_disabled()
            || self.shared.session_binding() != expected.binding
            || !self.shared.inbound.is_empty()
            || !self.shared.outbound.is_empty()
            || self.shared.records.front().is_some()
            || self.ws_write_pending
        {
            return Err(RejectedCommand::DirtyReceiveBoundary);
        }
        self.clean_rebind = false;
        self.read_suspended = false;
        self.tls_read_paused = false;
        self.read_local = true;
        self.write_local = false;
        self.binding = next;
        self.cancel = Some(token);
        self.deadline = Some(deadline);
        self.retire = retire;
        self.shared.accept_binding(next);
        #[cfg(feature = "test-support")]
        soak_observer::rebind_wss_ledger(expected, next);
        Ok(())
    }

    /// Physical retirement. An abort skips both WebSocket Close and TLS
    /// close_notify; a deliberate close reaches here only after its drain.
    pub fn close(mut self, registry: &Registry) {
        self.drop_socket(Some(registry));
        self.retire.ack();
        self.shared.publish_closed();
        self.retired = true;
    }

    fn advance_upgrade(&mut self, now: Instant) -> WssStep {
        if let Some(error) = self.setup_error.take() {
            return WssStep::Failed(error);
        }
        if let Some(error) = self.revoked(now) {
            return WssStep::Failed(error);
        }
        let Some(state) = self.upgrade.take() else {
            return if self.is_connected() {
                WssStep::Ready
            } else {
                WssStep::Wait
            };
        };
        self.with_tls_mut(TlsIo::reset_turn);
        let result = match state {
            UpgradeState::Writing(machine) => match machine.single_round::<Response>() {
                Ok(RoundResult::Incomplete(machine)) => {
                    self.upgrade = Some(UpgradeState::Writing(machine));
                    self.setup_local = true;
                    WssStep::Progress
                }
                Ok(RoundResult::WouldBlock(machine)) => {
                    self.upgrade = Some(UpgradeState::Writing(machine));
                    self.setup_blocked(false);
                    self.step_from_block()
                }
                Ok(RoundResult::StageFinished(StageResult::DoneWriting(stream))) => {
                    self.upgrade =
                        Some(UpgradeState::Reading(HandshakeMachine::start_read(stream)));
                    self.setup_local = true;
                    WssStep::Progress
                }
                Ok(RoundResult::StageFinished(StageResult::DoneReading { .. })) => {
                    WssStep::Failed(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected WSS upgrade read state",
                    ))
                }
                Err(error) => WssStep::Failed(websocket_io_error(error)),
            },
            UpgradeState::Reading(machine) => match machine.single_round::<Response>() {
                Ok(RoundResult::Incomplete(machine)) => {
                    self.upgrade = Some(UpgradeState::Reading(machine));
                    self.setup_local = true;
                    WssStep::Progress
                }
                Ok(RoundResult::WouldBlock(machine)) => {
                    self.upgrade = Some(UpgradeState::Reading(machine));
                    self.setup_blocked(true);
                    self.step_from_block()
                }
                Ok(RoundResult::StageFinished(StageResult::DoneReading {
                    result,
                    stream,
                    tail,
                })) => {
                    let expected = self.expected_accept.as_deref().unwrap_or_default();
                    if let Err(error) = verify_upgrade_response(&result, expected) {
                        WssStep::Failed(error)
                    } else {
                        let tail_dirty = !tail.is_empty();
                        let websocket = {
                            #[cfg(feature = "test-support")]
                            let _allocation_scope = self
                                .allocation_scope(soak_observer::AllocationScopeKind::WssEndpoint);
                            WebSocket::from_partially_read(
                                stream,
                                tail,
                                Role::Client,
                                Some(websocket_config()),
                            )
                        };
                        self.websocket = Some(websocket);
                        self.upgrade = None;
                        self.expected_accept = None;
                        self.setup_local = false;
                        self.boundary_dirty =
                            tail_dirty || !self.with_tls(|tls| tls.is_clean_tls_boundary());
                        self.read_local = true;
                        self.write_local = self.with_tls(|tls| tls.wants_write());
                        WssStep::Ready
                    }
                }
                Ok(RoundResult::StageFinished(StageResult::DoneWriting(_))) => {
                    WssStep::Failed(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected WSS upgrade write state",
                    ))
                }
                Err(error) => WssStep::Failed(websocket_io_error(error)),
            },
        };

        // A TLS KeyUpdate can be processed while tungstenite is still reading
        // its HTTP response. Flush it under the same shared wire/TLS budgets.
        if self.with_tls(|tls| tls.wants_write()) {
            match self.flush_tls() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.write_local =
                        self.with_tls(|tls| tls.budget.write_stop) != WorkStop::SocketWouldBlock;
                }
                Err(error) => return WssStep::Failed(error),
            }
        }
        if !self.with_tls(|tls| tls.wants_write()) {
            self.write_local = false;
        }
        if self.tls_read_paused && self.with_tls(|tls| tls.pending_output() < TLS_OUTPUT_PAUSE) {
            self.read_local = true;
            self.tls_read_paused = false;
        }
        if matches!(&result, WssStep::Wait)
            && (self.setup_local || self.read_local || self.write_local)
        {
            WssStep::Progress
        } else {
            result
        }
    }

    fn setup_blocked(&mut self, reading: bool) {
        let stop = self.with_tls(|tls| {
            if reading {
                tls.budget.read_stop
            } else {
                tls.budget.write_stop
            }
        });
        match stop {
            WorkStop::SocketWouldBlock => {
                if reading {
                    self.read_local = false;
                    self.tls_read_paused = false;
                } else {
                    self.write_local = false;
                    self.read_local = false;
                }
                self.setup_local = false;
            }
            WorkStop::Budget => {
                if reading {
                    self.read_local = true;
                    self.tls_read_paused = false;
                } else {
                    self.write_local = true;
                }
                self.setup_local = true;
            }
            WorkStop::HighWater => {
                self.read_local = false;
                self.setup_local = false;
                self.write_local = false;
                if reading {
                    self.tls_read_paused = true;
                }
            }
            WorkStop::Closed => {
                self.setup_local = false;
                self.read_local = false;
                self.write_local = false;
                self.tls_read_paused = false;
            }
            WorkStop::Returned => {
                self.setup_local = true;
                if reading {
                    self.read_local = true;
                    self.tls_read_paused = false;
                } else {
                    self.write_local = true;
                }
            }
        }
    }

    fn step_from_block(&self) -> WssStep {
        if self.setup_local || self.read_local || self.write_local {
            WssStep::Progress
        } else {
            WssStep::Wait
        }
    }

    fn receive_peer_close(&mut self, message: Message, _read_bytes: &mut usize) -> io::Result<()> {
        match admit_message(message, &self.shared.inbound) {
            Ok(WsReceive::PeerClosed { code, reason }) => {
                self.peer_closed = true;
                self.read_suspended = true;
                self.read_local = false;
                self.shared
                    .publish_exit_reason(StreamExitReason::WebSocketClose { code, reason });
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(fault) => Err(frame_fault_error(fault)),
        }
    }

    fn turn_result(&self, stop: TcpStop, read_bytes: usize, written_bytes: usize) -> TcpTurnResult {
        TcpTurnResult {
            stop,
            read_bytes,
            written_bytes,
            read_ready: self.read_local,
            write_ready: self.write_local,
        }
    }

    fn has_write_work(&self) -> bool {
        self.ws_write_pending
            || self.with_tls(|tls| tls.wants_write())
            || self.shared.records.front().is_some()
    }
    fn write_work_locally_runnable(&self) -> bool {
        matches!(
            self.with_tls(|tls| tls.budget.write_stop),
            WorkStop::Budget | WorkStop::HighWater
        )
    }

    fn handshake_write_pending(&self) -> bool {
        matches!(&self.upgrade, Some(UpgradeState::Writing(_)))
    }

    fn tls_wants_write(&self) -> bool {
        self.with_tls(|tls| tls.wants_write())
    }

    fn with_tls<R>(&self, f: impl FnOnce(&TlsIo) -> R) -> R {
        let io = self.io.as_ref().expect("live WSS TLS owner");
        let guard = io.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&guard)
    }

    fn with_tls_mut<R>(&self, f: impl FnOnce(&mut TlsIo) -> R) -> R {
        let io = self.io.as_ref().expect("live WSS TLS owner");
        let mut guard = io.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }

    fn flush_tls(&self) -> io::Result<()> {
        self.with_tls_mut(TlsIo::flush_tls)
    }
    fn flush_websocket(&mut self) -> io::Result<()> {
        if self.with_tls(|tls| tls.pending_output()) >= TLS_OUTPUT_PAUSE
            && self.with_tls(|tls| tls.wants_write())
        {
            match self.flush_tls() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.ws_write_pending = true;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        if self.with_tls(|tls| tls.pending_output()) < TLS_OUTPUT_PAUSE {
            let flush_result = {
                #[cfg(feature = "test-support")]
                let _allocation_scope =
                    self.allocation_scope(soak_observer::AllocationScopeKind::WssMessageStaging);
                self.websocket.as_mut().expect("connected WSS").flush()
            };
            match flush_result {
                Ok(()) => self.ws_write_pending = false,
                Err(WsError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.ws_write_pending = true;
                }
                Err(error) => return Err(websocket_io_error(error)),
            }
        } else {
            self.ws_write_pending = true;
        }
        Ok(())
    }

    fn revoked(&self, now: Instant) -> Option<io::Error> {
        if self.cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
            Some(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "game attempt cancelled",
            ))
        } else if self.deadline.is_some_and(|deadline| now >= deadline) {
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "game login deadline passed",
            ))
        } else {
            None
        }
    }

    fn publish_error(&self, error: &io::Error) {
        if let Some(tls) = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<TlsFailure>())
        {
            self.shared
                .publish_exit_reason(StreamExitReason::Tls(tls.0.clone()));
        } else {
            self.shared
                .publish_exit_reason(StreamExitReason::WebSocket(error.to_string()));
        }
    }

    fn drop_socket(&mut self, registry: Option<&Registry>) {
        self.upgrade.take();
        self.websocket.take();
        if let Some(io) = self.io.take() {
            match Arc::try_unwrap(io) {
                Ok(mutex) => {
                    let mut tls = mutex
                        .into_inner()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let socket = tls.socket.inner_mut();
                    let _ = socket.shutdown(Shutdown::Both);
                    if let Some(registry) = registry {
                        let _ = registry.deregister(socket);
                    }
                    drop(tls);
                }
                Err(io) => {
                    let mut tls = io.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let socket = tls.socket.inner_mut();
                    let _ = socket.shutdown(Shutdown::Both);
                    if let Some(registry) = registry {
                        let _ = registry.deregister(socket);
                    }
                    drop(tls);
                    drop(io);
                }
            }
        }
    }
}

impl Drop for WssEndpoint {
    fn drop(&mut self) {
        #[cfg(feature = "test-support")]
        soak_observer::retire_wss_ledger(self.binding);
        if !self.retired {
            self.drop_socket(None);
            self.retire.ack();
            self.shared.publish_closed();
        }
    }
}
fn build_upgrade_request(host: &str) -> io::Result<(Vec<u8>, String)> {
    let uri = format!("wss://{host}/");
    let mut request = uri
        .into_client_request()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        tungstenite::http::HeaderValue::from_static("binary"),
    );
    let (bytes, key) = generate_request(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    Ok((bytes, derive_accept_key(key.as_bytes())))
}

fn header_has_token(headers: &tungstenite::http::HeaderMap, name: &str, expected: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        value.to_str().ok().is_some_and(|value| {
            value.split(',').any(|token| {
                token
                    .trim_matches(|character| character == ' ' || character == '\t')
                    .eq_ignore_ascii_case(expected)
            })
        })
    })
}

fn has_one_header_value(
    headers: &tungstenite::http::HeaderMap,
    name: &str,
    expected: &[u8],
) -> bool {
    let mut values = headers.get_all(name).iter();
    matches!((values.next(), values.next()), (Some(value), None) if value.as_bytes() == expected)
}

fn verify_upgrade_response(response: &Response, expected_accept: &str) -> io::Result<()> {
    let headers = response.headers();
    if response.version() != tungstenite::http::Version::HTTP_11
        || response.status() != tungstenite::http::StatusCode::SWITCHING_PROTOCOLS
        || !header_has_token(headers, "Upgrade", "websocket")
        || !header_has_token(headers, "Connection", "Upgrade")
        || !has_one_header_value(headers, "Sec-WebSocket-Accept", expected_accept.as_bytes())
        || !has_one_header_value(headers, "Sec-WebSocket-Protocol", b"binary")
        || headers.contains_key("Sec-WebSocket-Extensions")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WSS HTTP upgrade response rejected",
        ));
    }
    Ok(())
}

// tungstenite 0.24 retains the upgrade's input Vec as frame input storage
// and uses a separate 4 KiB read chunk. Both are dependency-private. Keep
// them: reducing read chunks changes I/O throughput, and discarding an
// upgrade tail can lose the first game frame. The 16 KiB message/frame bounds
// below are adapter admission bounds, not substitutes for TLS record buffering.
fn websocket_config() -> WebSocketConfig {
    WebSocketConfig {
        write_buffer_size: WS_WRITE_BUFFER_SIZE,
        max_write_buffer_size: WS_WRITE_BUFFER_LIMIT,
        max_message_size: Some(WS_MESSAGE_LIMIT),
        max_frame_size: Some(WS_MESSAGE_LIMIT),
        ..WebSocketConfig::default()
    }
}

fn frame_fault_error(fault: FrameFault) -> io::Error {
    match fault {
        FrameFault::MessageTooLarge { length, limit } => io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!("WSS message exceeds {limit} bytes ({length} bytes)"),
        ),
        other => io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid WSS game frame: {other:?}"),
        ),
    }
}

fn websocket_io_error(error: WsError) -> io::Error {
    match error {
        WsError::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}

fn tls_failure_io_error(kind: io::ErrorKind, message: impl Into<String>) -> io::Error {
    io::Error::new(kind, TlsFailure(message.into()))
}

fn tls_capacity_error(message: &'static str) -> io::Error {
    tls_failure_io_error(io::ErrorKind::OutOfMemory, message)
}

fn tls_io_error(error: rustls::Error) -> io::Error {
    tls_failure_io_error(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName};
    use std::net::{TcpListener, TcpStream as StdTcpStream};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    enum TestServerCommand {
        SendFrames {
            frames: Vec<Vec<u8>>,
            done: Sender<io::Result<usize>>,
        },
        DrainBinaries {
            expected_binary: Vec<u8>,
            expected_binary_count: usize,
            done: Sender<io::Result<usize>>,
        },
        WaitForPong {
            expected: Vec<u8>,
            expected_binary: Vec<u8>,
            expected_binary_count: usize,
            done: Sender<io::Result<bool>>,
        },
        Stop,
    }

    struct WssFixture {
        endpoint: WssEndpoint,
        shared: Arc<SharedConnection>,
        binding: BindingId,
        commands: Sender<TestServerCommand>,
        poll: mio::Poll,
        server: Option<JoinHandle<io::Result<()>>>,
    }

    impl WssFixture {
        fn new() -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let (commands, receiver) = mpsc::channel();
            let server = thread::spawn(move || serve_wss_test_connection(listener, receiver));
            let mut socket = StdTcpStream::connect(address).unwrap();
            let name = ServerName::try_from(String::from("localhost")).unwrap();
            let mut connection = ClientConnection::new(test_client_config(), name).unwrap();
            while connection.is_handshaking() {
                connection.complete_io(&mut socket).unwrap();
            }
            socket.set_nonblocking(true).unwrap();

            let park = Arc::new(super::super::park::Park::new());
            let source = super::super::cancel::CancelSource::new(Arc::clone(&park));
            let token = source.child().unwrap();
            let binding = BindingId {
                incarnation: 1,
                connection: 1,
                binding: 1,
                attempt: token.generation(),
            };
            let retire = Arc::new(AttemptRetire::new(token.generation()));
            let shared = SharedConnection::new(binding.connection, park);
            shared.accept_binding(binding);
            let poll = mio::Poll::new().unwrap();
            let mut socket = TcpStream::from_std(socket);
            poll.registry()
                .register(
                    &mut socket,
                    mio::Token(binding.connection as usize),
                    Interest::READABLE.add(Interest::WRITABLE),
                )
                .unwrap();
            let endpoint = WssEndpoint::new(
                binding,
                "localhost",
                socket,
                connection,
                Arc::clone(&shared),
                token,
                None,
                retire,
            )
            .unwrap();
            let mut fixture = Self {
                endpoint,
                shared,
                binding,
                commands,
                poll,
                server: Some(server),
            };
            fixture.complete_upgrade();
            fixture
        }

        fn complete_upgrade(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut events = mio::Events::with_capacity(8);
            while Instant::now() < deadline {
                let step = self.endpoint.begin_step(Instant::now());
                self.endpoint.apply_interest(self.poll.registry()).unwrap();
                if let WssStep::Failed(error) = step {
                    panic!("test WSS upgrade failed: {error}");
                }
                if self.endpoint.is_connected() {
                    assert_eq!(self.endpoint.last_interest, Some(Interest::READABLE));
                    return;
                }
                if matches!(step, WssStep::Wait) {
                    self.poll
                        .poll(&mut events, Some(Duration::from_millis(20)))
                        .unwrap();
                    for event in &events {
                        if let WssStep::Failed(error) = self.endpoint.on_event(
                            event.is_readable(),
                            event.is_writable(),
                            Instant::now(),
                        ) {
                            panic!("test WSS upgrade failed on readiness: {error}");
                        }
                        self.endpoint.apply_interest(self.poll.registry()).unwrap();
                    }
                }
            }
            panic!("test WSS upgrade did not complete within 2s");
        }

        fn on_event(&mut self, readable: bool, writable: bool) -> WssStep {
            let step = self.endpoint.on_event(readable, writable, Instant::now());
            self.endpoint.apply_interest(self.poll.registry()).unwrap();
            step
        }

        fn drive(&mut self) -> TcpTurnResult {
            let turn = self.endpoint.drive().unwrap();
            self.endpoint.apply_interest(self.poll.registry()).unwrap();
            turn
        }
        fn wait_for_event(&mut self, deadline: Instant) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "test WSS readiness deadline expired");
            let mut events = mio::Events::with_capacity(8);
            self.poll.poll(&mut events, Some(remaining)).unwrap();
            assert!(!events.is_empty(), "test WSS readiness deadline expired");
            for event in &events {
                if let WssStep::Failed(error) =
                    self.endpoint
                        .on_event(event.is_readable(), event.is_writable(), Instant::now())
                {
                    panic!("test WSS readiness failed: {error}");
                }
            }
            self.endpoint.apply_interest(self.poll.registry()).unwrap();
        }

        fn drive_until_inbound(&mut self, deadline: Instant) -> TcpTurnResult {
            loop {
                assert!(
                    Instant::now() < deadline,
                    "test WSS inbound deadline expired"
                );
                let turn = self.endpoint.drive().unwrap();
                self.endpoint.apply_interest(self.poll.registry()).unwrap();
                if turn.read_bytes != 0 {
                    return turn;
                }
                match turn.stop {
                    TcpStop::LocalWork => {}
                    TcpStop::Quiescent => self.wait_for_event(deadline),
                    stop => panic!("WSS stopped before binary admission: {stop:?}"),
                }
            }
        }

        fn drive_until_peer_closed(&mut self, deadline: Instant) -> TcpTurnResult {
            loop {
                assert!(Instant::now() < deadline, "test WSS close deadline expired");
                let turn = self.endpoint.drive().unwrap();
                self.endpoint.apply_interest(self.poll.registry()).unwrap();
                if self.endpoint.peer_closed {
                    return turn;
                }
                match turn.stop {
                    TcpStop::LocalWork => {}
                    TcpStop::Quiescent => self.wait_for_event(deadline),
                    stop => panic!("WSS stopped before peer close: {stop:?}"),
                }
            }
        }

        fn drive_until_error(&mut self, deadline: Instant) -> io::Error {
            loop {
                assert!(Instant::now() < deadline, "test WSS error deadline expired");
                match self.endpoint.drive() {
                    Err(error) => {
                        self.endpoint.apply_interest(self.poll.registry()).unwrap();
                        return error;
                    }
                    Ok(turn) => {
                        self.endpoint.apply_interest(self.poll.registry()).unwrap();
                        match turn.stop {
                            TcpStop::LocalWork => {}
                            TcpStop::Quiescent => self.wait_for_event(deadline),
                            stop => panic!("WSS stopped before expected protocol error: {stop:?}"),
                        }
                    }
                }
            }
        }

        fn start_drain_binaries(
            &self,
            expected_binary: Vec<u8>,
            expected_binary_count: usize,
        ) -> Receiver<io::Result<usize>> {
            let (done, result) = mpsc::channel();
            self.commands
                .send(TestServerCommand::DrainBinaries {
                    expected_binary,
                    expected_binary_count,
                    done,
                })
                .unwrap();
            result
        }

        fn wait_for_pong(
            &self,
            expected: Vec<u8>,
            expected_binary: Vec<u8>,
            expected_binary_count: usize,
        ) {
            let (done, result) = mpsc::channel();
            self.commands
                .send(TestServerCommand::WaitForPong {
                    expected,
                    expected_binary,
                    expected_binary_count,
                    done,
                })
                .unwrap();
            assert!(server_result(result));
        }

        /// Have the server write `frames`, one TLS record each, and return
        /// once the client's own socket holds every byte. The server's write
        /// acknowledgement alone does not say that: a kernel may deliver
        /// loopback bytes after the writer returns (macOS does), and a drive
        /// that runs first reads nothing and misses the first turn.
        fn send_frames(&mut self, frames: Vec<Vec<u8>>) {
            let (done, result) = mpsc::channel();
            self.commands
                .send(TestServerCommand::SendFrames { frames, done })
                .unwrap();
            let wire = server_result(result);
            self.wait_for_client_bytes(wire);
        }

        fn wait_for_client_bytes(&mut self, wire: usize) {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut probe = vec![0_u8; wire];
            let mut events = mio::Events::with_capacity(8);
            loop {
                // Peeking leaves the bytes for the endpoint's own read.
                match self
                    .endpoint
                    .with_tls(|tls| tls.socket.inner().peek(&mut probe))
                {
                    Ok(queued) if queued >= wire => return,
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("peek the test WSS client socket: {error}"),
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "the server's {wire} ciphertext bytes never reached the client socket"
                );
                self.poll.poll(&mut events, Some(remaining)).unwrap();
            }
        }
    }

    impl Drop for WssFixture {
        fn drop(&mut self) {
            self.endpoint.drop_socket(Some(self.poll.registry()));
            let _ = self.commands.send(TestServerCommand::Stop);
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
        }
    }

    fn test_client_config() -> Arc<rustls::ClientConfig> {
        let root = CertificateDer::from_pem_slice(include_bytes!(
            "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/ca.pem"
        ))
        .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(root).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    fn test_server_config() -> Arc<rustls::ServerConfig> {
        let certificate = CertificateDer::from_pem_slice(include_bytes!(
            "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.pem"
        ))
        .unwrap();
        let key = PrivateKeyDer::from_pem_slice(include_bytes!(
            "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.key"
        ))
        .unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key)
                .unwrap(),
        )
    }

    /// The server's socket, counting the ciphertext bytes it has written so a
    /// test can tell when the client's kernel queue holds them all.
    struct CountingSocket {
        inner: StdTcpStream,
        written: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Read for CountingSocket {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Write for CountingSocket {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let count = self.inner.write(buf)?;
            self.written
                .fetch_add(count, std::sync::atomic::Ordering::SeqCst);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    fn serve_wss_test_connection(
        listener: TcpListener,
        commands: Receiver<TestServerCommand>,
    ) -> io::Result<()> {
        let (mut socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut connection = rustls::ServerConnection::new(test_server_config())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        while connection.is_handshaking() {
            connection.complete_io(&mut socket)?;
        }
        let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut tls = rustls::StreamOwned::new(
            connection,
            CountingSocket {
                inner: socket,
                written: Arc::clone(&written),
            },
        );
        let request = read_http_head(&mut tls)?;
        let request = std::str::from_utf8(&request)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let key = request
            .lines()
            .find_map(|line| line.strip_prefix("Sec-WebSocket-Key:").map(str::trim))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing WebSocket key"))?;
        let accept = derive_accept_key(key.as_bytes());
        write!(
            tls,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n"
        )?;
        tls.flush()?;
        let mut websocket = WebSocket::from_raw_socket(tls, Role::Server, Some(websocket_config()));

        while let Ok(command) = commands.recv() {
            match command {
                TestServerCommand::SendFrames { frames, done } => {
                    let mut result = Ok(());
                    let wire_before = written.load(std::sync::atomic::Ordering::SeqCst);
                    // Each flush emits one TLS application record for the next
                    // test frame, while the client receives no new event hint.
                    for frame in frames {
                        if let Err(error) = websocket.get_mut().write_all(&frame) {
                            result = Err(error);
                            break;
                        }
                        if let Err(error) = websocket.get_mut().flush() {
                            result = Err(error);
                            break;
                        }
                    }
                    let wire = written.load(std::sync::atomic::Ordering::SeqCst) - wire_before;
                    let _ = done.send(result.map(|()| wire));
                }
                TestServerCommand::DrainBinaries {
                    expected_binary,
                    expected_binary_count,
                    done,
                } => {
                    let mut binary_count = 0;
                    let result = loop {
                        if binary_count == expected_binary_count {
                            break Ok(binary_count);
                        }
                        match websocket.read() {
                            Ok(Message::Binary(payload)) => {
                                if payload.as_slice() != expected_binary.as_slice() {
                                    break Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        "unexpected outbound WSS binary payload",
                                    ));
                                }
                                binary_count += 1;
                            }
                            Ok(_) => {
                                break Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "unexpected outbound WSS control frame",
                                ));
                            }
                            Err(error) => break Err(websocket_io_error(error)),
                        }
                    };
                    let _ = done.send(result);
                }
                TestServerCommand::WaitForPong {
                    expected,
                    expected_binary,
                    expected_binary_count,
                    done,
                } => {
                    let mut binary_count = 0;
                    let mut pong_seen = false;
                    let result = loop {
                        match websocket.read() {
                            Ok(Message::Binary(payload)) => {
                                if pong_seen
                                    || binary_count == expected_binary_count
                                    || payload.as_slice() != expected_binary.as_slice()
                                {
                                    break Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        "unexpected outbound WSS binary payload",
                                    ));
                                }
                                binary_count += 1;
                            }
                            Ok(Message::Pong(payload)) => {
                                if pong_seen
                                    || payload.as_slice() != expected.as_slice()
                                    || binary_count != expected_binary_count
                                {
                                    break Ok(false);
                                }
                                pong_seen = true;
                            }
                            Ok(_) => {}
                            Err(WsError::Io(error))
                                if matches!(
                                    error.kind(),
                                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                                ) =>
                            {
                                break Ok(pong_seen && binary_count == expected_binary_count);
                            }
                            Err(error) => break Err(websocket_io_error(error)),
                        }
                    };
                    let _ = done.send(result);
                }
                TestServerCommand::Stop => break,
            }
        }
        Ok(())
    }

    fn read_http_head(reader: &mut impl Read) -> io::Result<Vec<u8>> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() == 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "test HTTP request header too large",
                ));
            }
            reader.read_exact(&mut byte)?;
            head.push(byte[0]);
        }
        Ok(head)
    }

    fn raw_ws_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(payload.len() + 10);
        frame.push(if fin { 0x80 | opcode } else { opcode });
        match payload.len() {
            0..=125 => frame.push(payload.len() as u8),
            126..=65_535 => {
                frame.push(126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            _ => {
                frame.push(127);
                frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(payload);
        frame
    }

    fn server_result<T>(result: Receiver<io::Result<T>>) -> T {
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn two_tls_records_keep_second_wss_message_locally_runnable() {
        let mut fixture = WssFixture::new();
        fixture.send_frames(vec![
            raw_ws_frame(true, 2, b"one"),
            raw_ws_frame(true, 2, b"two"),
        ]);

        let first = fixture.drive();
        assert_eq!(first.read_bytes, 3);
        assert_eq!(first.stop, TcpStop::LocalWork);
        let second = fixture.drive();
        assert_eq!(second.read_bytes, 3);

        let mut received = [0u8; 6];
        assert_eq!(fixture.shared.take_inbound(&mut received), received.len());
        assert_eq!(&received, b"onetwo");
    }

    #[test]
    fn fragmented_wss_message_resumes_across_the_plaintext_turn_budget() {
        let mut fixture = WssFixture::new();
        let first_fragment = vec![0x2a; WS_MESSAGE_LIMIT - 2];
        fixture.send_frames(vec![raw_ws_frame(false, 2, &first_fragment)]);

        let first_turn = fixture.drive();
        assert_eq!(first_turn.stop, TcpStop::LocalWork);
        assert_eq!(
            fixture.endpoint.with_tls(|tls| tls.budget.read_stop),
            WorkStop::Budget
        );
        assert_eq!(fixture.shared.inbound.len(), 0);

        // TLS may deliver the 16 KiB fragment in more than two bounded
        // plaintext turns. Keep consuming local work without an extra edge
        // until the incomplete message reaches a real socket WouldBlock.
        let mut turn = fixture.drive();
        for _ in 0..8 {
            if turn.stop == TcpStop::Quiescent {
                break;
            }
            assert_eq!(turn.stop, TcpStop::LocalWork);
            turn = fixture.drive();
        }
        assert_eq!(turn.stop, TcpStop::Quiescent);
        assert_eq!(fixture.shared.inbound.len(), 0);

        fixture.send_frames(vec![raw_ws_frame(true, 0, &[])]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));
        let completed = fixture.drive();
        assert_eq!(completed.read_bytes, first_fragment.len());
        let mut received = vec![0; first_fragment.len()];
        assert_eq!(fixture.shared.take_inbound(&mut received), received.len());
        assert_eq!(received, first_fragment);
    }

    #[test]
    fn automatic_pong_survives_socket_write_would_block() {
        let mut fixture = WssFixture::new();
        fixture.send_frames(vec![raw_ws_frame(true, 9, b"ping")]);
        fixture.endpoint.read_local = false;

        let record = vec![0x55; OUT_RING_BYTES];
        let mut socket_blocked = false;
        let mut published_binary_count = 0;
        for _ in 0..4096 {
            fixture
                .shared
                .publish_record(fixture.binding, &record)
                .unwrap();
            fixture.endpoint.kick_output();
            published_binary_count += 1;
            for _ in 0..4 {
                let turn = fixture.drive();
                if fixture.endpoint.with_tls(|tls| tls.budget.write_stop)
                    == WorkStop::SocketWouldBlock
                {
                    socket_blocked = true;
                    break;
                }
                if turn.stop != TcpStop::LocalWork {
                    break;
                }
            }
            if socket_blocked {
                break;
            }
        }
        assert!(
            socket_blocked,
            "loopback send buffer did not reach WouldBlock"
        );

        // The kernel frees send-buffer space on its own schedule (ACKs, window
        // updates, autotuning), so by now the real socket may accept the Pong
        // after all. Refuse the Pong's first socket write so that it meets a
        // WouldBlock whatever the kernel has done meanwhile; the backlog the
        // real socket refused is still queued ahead of it.
        fixture
            .endpoint
            .with_tls_mut(|tls| tls.socket.refuse_one_socket_write_for_test());
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));
        fixture.drive();
        assert!(fixture.endpoint.ws_write_pending);
        assert_eq!(
            fixture.endpoint.with_tls(|tls| tls.budget.write_stop),
            WorkStop::SocketWouldBlock
        );

        let drained = fixture.start_drain_binaries(record, published_binary_count);
        // The peer drains and the kernel reports the socket writable: only
        // that may release the queued Pong. Wait for the real readiness event
        // rather than spinning.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut pong_flushed = false;
        while Instant::now() < deadline {
            let turn = fixture.drive();
            if !fixture.endpoint.ws_write_pending
                && !fixture.endpoint.with_tls(|tls| tls.wants_write())
            {
                pong_flushed = true;
                break;
            }
            if turn.stop != TcpStop::LocalWork {
                fixture.wait_for_event(deadline);
            }
        }
        assert!(
            pong_flushed,
            "WSS control output remained blocked after the peer drained"
        );
        assert_eq!(server_result(drained), published_binary_count);
        fixture.wait_for_pong(b"ping".to_vec(), Vec::new(), 0);
    }

    #[test]
    fn ping_interleaved_with_fragmented_binary_stays_control_only() {
        let mut fixture = WssFixture::new();
        fixture.send_frames(vec![raw_ws_frame(false, 2, b"before")]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));
        let mut first_fragment_turn = fixture.drive();
        for _ in 0..8 {
            if first_fragment_turn.stop == TcpStop::Quiescent {
                break;
            }
            assert_eq!(first_fragment_turn.stop, TcpStop::LocalWork);
            first_fragment_turn = fixture.drive();
        }
        assert_eq!(first_fragment_turn.stop, TcpStop::Quiescent);
        assert_eq!(fixture.shared.inbound.len(), 0);

        fixture.send_frames(vec![raw_ws_frame(true, 9, b"ping")]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));
        let ping_turn = fixture.drive();
        assert_eq!(ping_turn.read_bytes, 0);
        fixture.wait_for_pong(b"ping".to_vec(), Vec::new(), 0);
        assert_eq!(fixture.shared.inbound.len(), 0);

        fixture.send_frames(vec![raw_ws_frame(true, 0, b"after")]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));
        let deadline = Instant::now() + Duration::from_secs(2);
        let completed = fixture.drive_until_inbound(deadline);
        assert_eq!(completed.read_bytes, 11);
        let mut received = [0; 11];
        assert_eq!(fixture.shared.take_inbound(&mut received), received.len());
        assert_eq!(&received, b"beforeafter");
        fixture.send_frames(vec![raw_ws_frame(true, 10, b"unsolicited")]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));
        let pong_turn = fixture.drive();
        assert_eq!(pong_turn.read_bytes, 0);
        assert_eq!(fixture.shared.inbound.len(), 0);
    }

    #[test]
    fn idle_ping_flushes_its_pong_without_another_data_frame() {
        let mut fixture = WssFixture::new();
        assert_eq!(fixture.shared.records.len(), 0, "no outbound game bytes");
        assert!(!fixture.endpoint.ws_write_pending);
        fixture
            .endpoint
            .with_tls_mut(|tls| tls.socket.refuse_one_socket_write_for_test());
        fixture.send_frames(vec![raw_ws_frame(true, 9, b"idle-ping")]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));

        let blocked = fixture.drive();
        assert_eq!(blocked.read_bytes, 0);
        assert!(fixture.endpoint.ws_write_pending);
        assert_eq!(
            fixture.endpoint.with_tls(|tls| tls.budget.write_stop),
            WorkStop::SocketWouldBlock,
            "the first explicit idle-Ping flush reached a blocked wire write"
        );
        assert_eq!(
            fixture.shared.records.len(),
            0,
            "no game write rescued the Pong"
        );
        for _ in 0..8 {
            assert!(matches!(fixture.on_event(false, true), WssStep::Ready));
            fixture.drive();
            if !fixture.endpoint.ws_write_pending
                && !fixture.endpoint.with_tls(|tls| tls.wants_write())
            {
                break;
            }
        }
        assert!(!fixture.endpoint.ws_write_pending);
        assert!(!fixture.endpoint.with_tls(|tls| tls.wants_write()));
        fixture.wait_for_pong(b"idle-ping".to_vec(), Vec::new(), 0);
        assert_eq!(fixture.shared.inbound.len(), 0);
    }

    #[test]
    fn close_control_does_not_enter_the_binary_ring() {
        let mut fixture = WssFixture::new();
        fixture.send_frames(vec![
            raw_ws_frame(false, 2, b"incomplete"),
            raw_ws_frame(true, 8, &[]),
        ]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));

        let deadline = Instant::now() + Duration::from_secs(2);
        fixture.drive_until_peer_closed(deadline);
        assert!(fixture.endpoint.peer_closed);
        assert_eq!(fixture.shared.inbound.len(), 0);
        assert!(matches!(
            fixture.shared.take_exit_reason(),
            Some(StreamExitReason::WebSocketClose { code: None, reason }) if reason.is_empty()
        ));
    }

    #[test]
    fn text_frame_fails_without_entering_the_binary_ring() {
        let mut fixture = WssFixture::new();
        fixture.send_frames(vec![raw_ws_frame(true, 1, b"not-game-data")]);
        assert!(matches!(fixture.on_event(true, false), WssStep::Ready));

        let deadline = Instant::now() + Duration::from_secs(2);
        let error = fixture.drive_until_error(deadline);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fixture.shared.inbound.len(), 0);
        assert!(matches!(
            fixture.shared.take_exit_reason(),
            Some(StreamExitReason::WebSocket(_))
        ));
    }

    #[test]
    fn upgrade_request_targets_root_and_binary_subprotocol() {
        let (request, _) = build_upgrade_request("game.example:8443").unwrap();
        let request = std::str::from_utf8(&request).unwrap();
        assert!(request.starts_with("GET / HTTP/1.1\r\n"));
        assert!(request.contains("Host: game.example:8443\r\n"));
        assert!(request.contains("Sec-WebSocket-Protocol: binary\r\n"));
    }

    #[test]
    fn upgrade_requires_the_binary_protocol_and_exact_accept_key() {
        let expected = "test-accept";
        let valid = tungstenite::http::Response::builder()
            .status(101)
            .header("Upgrade", "websocket")
            .header("Connection", "keep-alive, Upgrade")
            .header("Sec-WebSocket-Accept", expected)
            .header("Sec-WebSocket-Protocol", "binary")
            .body(None)
            .unwrap();
        assert!(verify_upgrade_response(&valid, expected).is_ok());

        let wrong_protocol = tungstenite::http::Response::builder()
            .status(101)
            .header("Upgrade", "websocket")
            .header("Connection", "keep-alive, Upgrade")
            .header("Sec-WebSocket-Accept", expected)
            .header("Sec-WebSocket-Protocol", "other")
            .body(None)
            .unwrap();
        assert_eq!(
            verify_upgrade_response(&wrong_protocol, expected)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn oversized_wss_message_maps_to_capacity_error() {
        let error = frame_fault_error(FrameFault::MessageTooLarge {
            length: 2,
            limit: 1,
        });
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    }

    #[test]
    fn websocket_message_and_frame_limits_match_game_record_limit() {
        let config = websocket_config();
        assert_eq!(config.max_message_size, Some(WS_MESSAGE_LIMIT));
        assert_eq!(config.max_frame_size, Some(WS_MESSAGE_LIMIT));
        assert!(config.max_write_buffer_size > config.write_buffer_size + OUT_RING_BYTES);
    }
}

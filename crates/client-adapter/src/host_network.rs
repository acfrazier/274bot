//! Process-owned implementation of the fork's byte-stream hook.
//!
//! Queue authority is prepared on the slot before K1 charges a start. A
//! scoped one-shot handoff lets the fork's synchronous connector consume that
//! exact reservation without exposing adapter types to the client fork.

use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use client::client::client::AssetFetchError;
use client::io::client_stream::LoginCancel;
use client::io::host_network::{
    HostCloseDisposition, HostGameStream, HostHttpFetch, HostLookupAddresses, HostLookupFailure,
    HostLookupTicket, HostNetwork, HostStreamExit, SealReceipt,
};
use client::Transport;

use crate::cancel::{AttemptRetire, CancelSource, CancelToken};
use crate::control::{
    BindingId, CloseReason, ConnectCommand, PrimaryCommand, PrimaryReply, SubmitError,
};
use crate::https_handshake::HttpsLane;
use crate::park::{Park, ParkOutcome};
use crate::reactor::TransportService;
use crate::resolver::{
    ResolveIdentity, ResolvePoll, ResolveTicket, ResolvedAddresses, Resolver,
    ResolverAdmissionError, ResolverFailure, ResolverWake, SlotKey,
};
use crate::shared::StreamExitReason;
use crate::stream_handle::ConnectionHandle;
static NEXT_FETCH_RESOLVE_SLOT: AtomicU64 = AtomicU64::new(1_u64 << 63);

thread_local! {
    static GAME_ATTEMPT: RefCell<Option<PreparedGameAttempt>> = const { RefCell::new(None) };
}

/// Host selection without exposing fork types in the adapter's public API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameTransport {
    Tcp,
    Wss,
}

impl GameTransport {
    fn hook(self) -> Transport {
        match self {
            Self::Tcp => Transport::Tcp,
            Self::Wss => Transport::Wss,
        }
    }

    fn from_hook(mode: Transport) -> Self {
        match mode {
            Transport::Tcp => Self::Tcp,
            Transport::Wss => Self::Wss,
        }
    }
}

struct GameAttemptProof {
    queue_owner: u64,
    binding: BindingId,
    generation: u64,
    transfer_seq: Option<u64>,
    cancel: CancelToken,
    retirement: Arc<AttemptRetire>,
}

struct AdapterStreamIdentity {
    service: Arc<TransportService>,
    shared: Arc<crate::shared::SharedConnection>,
    binding: BindingId,
    transport: GameTransport,
    host: String,
    port: u16,
}

/// Reserve a connection and its bounded control/ring cells before K1 starts
/// the flight. A dropped unsubmitted reservation has never owned a socket.
pub struct PreparedGameAttempt {
    service: Arc<TransportService>,
    binding: BindingId,
    shared: Arc<crate::shared::SharedConnection>,
    token: CancelToken,
    retirement: Arc<AttemptRetire>,
    transport: GameTransport,
    host: String,
    port: u16,
    addresses: ResolvedAddresses,
    submitted: bool,
    rebind_expected: Option<BindingId>,
    queue_release: Option<Arc<crate::handoff::QueueReleaseProof>>,
    tls_ticket: Option<crate::tls_handshake::GameTlsTicket>,
}

impl PreparedGameAttempt {
    #[expect(
        clippy::too_many_arguments,
        reason = "Attempt identity and each independently owned connection resource stay explicit"
    )]
    pub fn new(
        service: Arc<TransportService>,
        park: Arc<Park>,
        token: CancelToken,
        retirement: Arc<AttemptRetire>,
        transport: GameTransport,
        host: &str,
        port: u16,
        addresses: ResolvedAddresses,
    ) -> io::Result<Self> {
        if addresses.is_empty() || host.is_empty() || token.generation() != retirement.generation()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid game attempt",
            ));
        }
        let transfer_seq = match transport {
            GameTransport::Tcp => None,
            GameTransport::Wss => Some(service.tls_transfer_seq()?),
        };
        let (binding, shared) = service.reserve(park, token.generation())?;
        let queue_release = transfer_seq
            .map(|sequence| Arc::new(crate::handoff::QueueReleaseProof::new(binding, sequence)));
        Ok(Self {
            service,
            binding,
            shared,
            token,
            retirement,
            transport,
            host: host.to_owned(),
            port,
            addresses,
            submitted: false,
            rebind_expected: None,
            queue_release,
            tls_ticket: None,
        })
    }

    /// Mint the next attempt on an already committed socket. No second
    /// descriptor, ring or connection charge enters this reservation.
    #[expect(
        clippy::too_many_arguments,
        reason = "Rebind preserves separate existing identity, cancellation, and retirement resources"
    )]
    pub fn new_for_rebind(
        service: Arc<TransportService>,
        previous: &client::io::host_network::HostStreamIdentity,
        park: Arc<Park>,
        token: CancelToken,
        retirement: Arc<AttemptRetire>,
        transport: GameTransport,
        host: &str,
        port: u16,
        addresses: ResolvedAddresses,
    ) -> io::Result<Self> {
        let old = previous
            .downcast_ref::<AdapterStreamIdentity>()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "foreign adopted stream identity",
                )
            })?;
        if !Arc::ptr_eq(&old.service, &service)
            || !Arc::ptr_eq(&old.shared.control.park, &park)
            || old.transport != transport
            || old.host != host
            || old.port != port
            || addresses.is_empty()
            || token.generation() != retirement.generation()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "adopted endpoint or attempt changed",
            ));
        }
        let (binding, shared) = service.reserve_rebind(old.binding, token.generation())?;
        if !Arc::ptr_eq(&shared, &old.shared) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "adopted socket custody changed",
            ));
        }
        Ok(Self {
            service,
            binding,
            shared,
            token,
            retirement,
            transport,
            host: host.to_owned(),
            port,
            addresses,
            submitted: false,
            rebind_expected: Some(old.binding),
            queue_release: None,
            tls_ticket: None,
        })
    }

    pub fn binding(&self) -> BindingId {
        self.binding
    }
    pub fn token(&self) -> CancelToken {
        self.token.clone()
    }
    pub fn retirement(&self) -> Arc<AttemptRetire> {
        Arc::clone(&self.retirement)
    }
    pub fn needs_initial_tls(&self) -> bool {
        self.transport == GameTransport::Wss && self.rebind_expected.is_none()
    }
    pub fn game_tls_waiter(
        &self,
        park: Arc<Park>,
        deadline: Instant,
    ) -> io::Result<crate::tls_handshake::GameTlsTicket> {
        self.service
            .game_tls_waiter(park, deadline, self.token.clone())
    }
    pub fn take_tls_ticket(&mut self) -> Option<crate::tls_handshake::GameTlsTicket> {
        self.tls_ticket.take()
    }

    pub fn endpoint(&self) -> (&str, u16) {
        (&self.host, self.port)
    }

    /// Share the process loss latch with prequeue waits without borrowing a slot.
    pub fn network_health(&self) -> Arc<crate::health::ReactorHealth> {
        self.service.health()
    }

    /// Mint the fork-hook carrier from the queue's exact flight.
    pub fn hook_identity(
        &self,
        queue_owner: u64,
        transfer_seq: Option<u64>,
    ) -> client::io::host_network::HostAttemptIdentity {
        client::io::host_network::HostAttemptIdentity::new(GameAttemptProof {
            queue_owner,
            binding: self.binding,
            generation: self.token.generation(),
            transfer_seq,
            cancel: self.token.clone(),
            retirement: Arc::clone(&self.retirement),
        })
    }

    /// Transfer the exclusive socket-free lane grant into this exact
    /// prepared attempt; the reactor releases it only after worker submission.
    pub fn set_tls_ticket(
        &mut self,
        ticket: crate::tls_handshake::GameTlsTicket,
    ) -> io::Result<()> {
        if self.transport != GameTransport::Wss
            || self.rebind_expected.is_some()
            || self.submitted
            || self.tls_ticket.is_some()
            || !ticket.matches_token(&self.token)
            || !ticket.reserved()
            || !self.service.owns_tls_ticket(&ticket)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WSS lane ticket does not match prepared attempt",
            ));
        }
        self.tls_ticket = Some(ticket);
        Ok(())
    }

    /// Queue waiting owns no TLS lane or socket. Mint its transfer proof at
    /// the actual eligible grant, not while waiting: intervening completed
    /// handshakes must not leave this attempt with a stale sequence.
    pub fn refresh_wss_transfer(&mut self) -> io::Result<()> {
        if self.transport == GameTransport::Wss && self.rebind_expected.is_none() {
            if self.submitted {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "game attempt already submitted",
                ));
            }
            if self
                .tls_ticket
                .as_ref()
                .is_none_or(|ticket| !ticket.reserved())
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "WSS lane not reserved",
                ));
            }
            let sequence = self.service.tls_transfer_seq()?;
            self.queue_release = Some(Arc::new(crate::handoff::QueueReleaseProof::new(
                self.binding,
                sequence,
            )));
        }
        Ok(())
    }

    /// WSS's pre-accept release receipt is minted from this exact binding.
    pub fn queue_release_proof(&self) -> Option<Arc<crate::handoff::QueueReleaseProof>> {
        self.queue_release.as_ref().map(Arc::clone)
    }
}

impl Drop for PreparedGameAttempt {
    fn drop(&mut self) {
        if !self.submitted
            && (self.rebind_expected.is_some() || self.service.abandon_unused(self.binding))
        {
            self.retirement.ack();
        }
    }
}

/// The closure runs on this slot thread only. Panic or early return drops a
/// never-submitted reservation rather than leaving its queue flight charged.
pub fn with_prepared_game_attempt<R>(attempt: PreparedGameAttempt, run: impl FnOnce() -> R) -> R {
    GAME_ATTEMPT.with(|cell| {
        let mut pending = cell.borrow_mut();
        assert!(pending.is_none(), "nested game attempt scope");
        *pending = Some(attempt);
    });
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            GAME_ATTEMPT.with(|cell| {
                cell.borrow_mut().take();
            });
        }
    }
    let clear = Clear;
    let result = run();
    drop(clear);
    result
}

struct ResolveWake {
    ready: AtomicBool,
    park: Arc<Park>,
}
impl ResolverWake for ResolveWake {
    fn wake(&self) {
        self.ready.store(true, Ordering::Release);
        self.park.publish();
    }
}

/// Resolve before K1 grant/start. A late result is still fenced by the
/// caller's captured world/identity and exact generation after the wait.
// These are the eight independently owned values at the DNS admission boundary;
// bundling them would only move the same arguments into a disposable struct.
#[expect(
    clippy::too_many_arguments,
    reason = "DNS admission owns eight distinct address, identity, and cancellation values"
)]
pub fn resolve_game(
    resolver: &Resolver,
    service: &TransportService,
    slot: SlotKey,
    generation: u64,
    host: &str,
    port: u16,
    park: Arc<Park>,
    cancel: &CancelToken,
) -> io::Result<ResolvedAddresses> {
    if service.health().loss().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "network service unavailable",
        ));
    }
    let wake = Arc::new(ResolveWake {
        ready: AtomicBool::new(false),
        park: Arc::clone(&park),
    });
    let ticket = resolver
        .resolve(
            ResolveIdentity { slot, generation },
            host,
            port,
            wake.clone(),
        )
        .map_err(|error| io::Error::other(format!("DNS admission: {error:?}")))?;
    loop {
        if cancel.is_cancelled() {
            ticket.cancel();
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "DNS attempt revoked",
            ));
        }
        match ticket.poll() {
            ResolvePoll::Ready(Ok(addresses)) => {
                if service.health().loss().is_some() || resolver.health().unavailable.is_some() {
                    ticket.cancel();
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "network service unavailable",
                    ));
                }
                return Ok(addresses);
            }
            ResolvePoll::Ready(Err(ResolverFailure::Cancelled)) => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "DNS request cancelled",
                ))
            }
            ResolvePoll::Ready(Err(ResolverFailure::TimedOut)) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "DNS request timed out",
                ))
            }
            ResolvePoll::Ready(Err(ResolverFailure::Unavailable(_))) => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "DNS resolver unavailable",
                ))
            }
            ResolvePoll::Ready(Err(ResolverFailure::Lookup(_))) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "DNS lookup failed",
                ))
            }
            ResolvePoll::Pending => {}
        }
        // Resolver health takes its own state mutex; sample it before K2.
        // Resolver terminal publication wakes this Park for the next turn.
        let resolver_unavailable = resolver.health().unavailable.is_some();
        let outcome = park.wait(
            ticket.deadline(),
            Instant::now,
            || service.health().loss().is_some() || resolver_unavailable,
            || false,
            || cancel.is_cancelled(),
            || wake.ready.swap(false, Ordering::AcqRel).then_some(()),
            false,
            None,
        );
        if matches!(
            outcome,
            ParkOutcome::Deadline | ParkOutcome::Revoked | ParkOutcome::Unavailable
        ) {
            ticket.cancel();
            return Err(io::Error::new(
                if matches!(outcome, ParkOutcome::Deadline) {
                    io::ErrorKind::TimedOut
                } else {
                    io::ErrorKind::ConnectionAborted
                },
                "DNS wait ended without an address",
            ));
        }
    }
}

struct ColdFillLookup {
    ticket: ResolveTicket,
    wake: Arc<ResolveWake>,
    park: Arc<Park>,
    resolver: Arc<Resolver>,
    service: Arc<TransportService>,
}

impl HostLookupTicket for ColdFillLookup {
    fn poll(&mut self, until: Instant) -> Result<Option<HostLookupAddresses>, HostLookupFailure> {
        loop {
            match self.ticket.poll() {
                ResolvePoll::Ready(Ok(addresses)) => {
                    if self.service.health().loss().is_some()
                        || self.resolver.health().unavailable.is_some()
                    {
                        return Err(HostLookupFailure::Unavailable);
                    }
                    let (inline, count) = inline_http_addresses(&addresses);
                    return Ok(Some(HostLookupAddresses::from_slice(&inline[..count])));
                }
                ResolvePoll::Ready(Err(ResolverFailure::Lookup(_))) => {
                    return Err(HostLookupFailure::Lookup);
                }
                ResolvePoll::Ready(Err(ResolverFailure::TimedOut)) => {
                    return Err(HostLookupFailure::TimedOut);
                }
                ResolvePoll::Ready(Err(ResolverFailure::Cancelled)) => {
                    return Err(HostLookupFailure::Cancelled);
                }
                ResolvePoll::Ready(Err(ResolverFailure::Unavailable(_))) => {
                    return Err(HostLookupFailure::Unavailable);
                }
                ResolvePoll::Pending => {}
            }
            let deadline = until.min(self.ticket.deadline());
            let resolver_unavailable = self.resolver.health().unavailable.is_some();
            match self.park.wait(
                deadline,
                Instant::now,
                || self.service.health().loss().is_some() || resolver_unavailable,
                || false,
                || false,
                || self.wake.ready.swap(false, Ordering::AcqRel).then_some(()),
                false,
                None,
            ) {
                ParkOutcome::Ready(()) => {}
                ParkOutcome::Deadline if deadline == self.ticket.deadline() => {
                    // A terminal published before the boundary retains its
                    // result even when the wake races the wait timeout.
                    if !matches!(self.ticket.poll(), ResolvePoll::Pending) {
                        continue;
                    }
                    self.ticket.cancel();
                    return Err(HostLookupFailure::TimedOut);
                }
                ParkOutcome::Deadline => return Ok(None),
                ParkOutcome::Unavailable | ParkOutcome::SelectedIntent | ParkOutcome::Revoked => {
                    self.ticket.cancel();
                    return Err(HostLookupFailure::Unavailable);
                }
            }
        }
    }

    fn backend_timing(&self) -> Option<(Instant, Instant)> {
        self.ticket.backend_timing()
    }
}

impl Drop for ColdFillLookup {
    fn drop(&mut self) {
        self.ticket.cancel();
    }
}

pub struct AdapterNetwork {
    service: Arc<TransportService>,
    resolver: Arc<Resolver>,
    drivers: Arc<std::sync::atomic::AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    https: Arc<HttpsLane>,
}
impl AdapterNetwork {
    pub(crate) fn new(
        service: Arc<TransportService>,
        resolver: Arc<Resolver>,
        drivers: Arc<std::sync::atomic::AtomicUsize>,
        shutdown: Arc<AtomicBool>,
        https: Arc<HttpsLane>,
    ) -> Arc<Self> {
        Arc::new(Self {
            service,
            resolver,
            drivers,
            shutdown,
            https,
        })
    }

    fn require_driver(&self) -> io::Result<()> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "network service shut down",
            ));
        }
        if self.drivers.load(Ordering::Acquire) == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "network maintenance driver not registered",
            ));
        }
        if self.service.health().loss().is_some() || self.resolver.health().unavailable.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "network service unavailable",
            ));
        }
        Ok(())
    }
}

fn inline_http_addresses(resolved: &ResolvedAddresses) -> ([SocketAddr; 8], usize) {
    // The resolver's admitted result contains no more than eight addresses.
    let mut addresses = [SocketAddr::from(([0, 0, 0, 0], 0)); 8];
    for (slot, address) in addresses.iter_mut().zip(resolved.iter()) {
        *slot = address;
    }
    (addresses, resolved.len())
}

/// A stepped HTTP request owns a DNS ticket, never a DNS backend call. Its
/// first poll waits only until the caller's ordinary unit boundary, then the
/// same wrapper delegates to the established HTTP fetch.
struct ResolvingHttpFetch {
    ticket: Option<ResolveTicket>,
    wake: Arc<ResolveWake>,
    park: Arc<Park>,
    resolver: Arc<Resolver>,
    service: Arc<TransportService>,
    https: Arc<HttpsLane>,
    mode: Transport,
    host: String,
    path: String,
    inner: Option<Box<dyn HostHttpFetch>>,
}

impl HostHttpFetch for ResolvingHttpFetch {
    fn poll(&mut self, until: Instant) -> Result<Option<Vec<u8>>, AssetFetchError> {
        if let Some(inner) = self.inner.as_mut() {
            return inner.poll(until);
        }
        loop {
            if self.service.health().loss().is_some()
                || self.resolver.health().unavailable.is_some()
            {
                if let Some(ticket) = self.ticket.take() {
                    ticket.cancel();
                }
                return Err(AssetFetchError::Connection);
            }
            let ticket = self.ticket.as_ref().expect("DNS ticket before HTTP fetch");
            match ticket.poll() {
                ResolvePoll::Ready(Ok(resolved)) => {
                    if self.service.health().loss().is_some()
                        || self.resolver.health().unavailable.is_some()
                    {
                        self.ticket.take();
                        return Err(AssetFetchError::Connection);
                    }
                    self.ticket.take();
                    let (addresses, count) = inline_http_addresses(&resolved);
                    self.inner = Some(crate::http::start_fetch(
                        self.mode,
                        &self.host,
                        &addresses[..count],
                        &self.path,
                        &self.https,
                    )?);
                    return self
                        .inner
                        .as_mut()
                        .expect("just installed HTTP fetch")
                        .poll(until);
                }
                ResolvePoll::Ready(Err(_)) => {
                    self.ticket.take();
                    return Err(AssetFetchError::Connection);
                }
                ResolvePoll::Pending => {}
            }
            let deadline = ticket.deadline();
            let wake = &self.wake;
            let resolver_unavailable = self.resolver.health().unavailable.is_some();
            let outcome = self.park.wait(
                deadline.min(until),
                Instant::now,
                || resolver_unavailable,
                || false,
                || false,
                || wake.ready.swap(false, Ordering::AcqRel).then_some(()),
                false,
                None,
            );
            match outcome {
                ParkOutcome::Ready(()) => {}
                ParkOutcome::Deadline if Instant::now() < deadline => return Ok(None),
                _ => {
                    if let Some(ticket) = self.ticket.take() {
                        ticket.cancel();
                    }
                    return Err(AssetFetchError::Connection);
                }
            }
        }
    }
}

impl Drop for ResolvingHttpFetch {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            ticket.cancel();
        }
    }
}

struct AdapterGameStream {
    service: Arc<TransportService>,
    shared: Arc<crate::shared::SharedConnection>,
    handle: ConnectionHandle,
    login_deadline: Option<Instant>,
    committed: bool,
    confirmed_at: Option<Instant>,
    transport: GameTransport,
    host: String,
    port: u16,
    rebind_cancel_attached: bool,
}
struct AdapterSealReceipt {
    binding: BindingId,
    confirmed_at: Instant,
    service_incarnation: u64,
}

impl AdapterGameStream {
    fn from_prepared(
        mut prepared: PreparedGameAttempt,
        deadline: Option<Instant>,
        auto_commit: bool,
    ) -> io::Result<Self> {
        if prepared.rebind_expected.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "adopted stream requires Rebind",
            ));
        }

        let service = Arc::clone(&prepared.service);
        let binding = prepared.binding;
        let shared = Arc::clone(&prepared.shared);
        let retirement = Arc::clone(&prepared.retirement);
        let park = Arc::clone(&shared.control.park);
        let mut handle = ConnectionHandle::new(
            shared.clone(),
            service.bus(),
            service.health(),
            binding,
            deadline,
            Some(prepared.token.clone()),
            None,
        );
        let command = ConnectCommand {
            binding,
            transport: prepared.transport.hook(),
            host: prepared.host.clone(),
            port: prepared.port,
            addresses: prepared.addresses,
            cancel: prepared.token.clone(),
            deadline,
            retire: Arc::clone(&retirement),
            queue_release: prepared.queue_release_proof(),
            tls_ticket: prepared.tls_ticket.take(),
        };
        match service.submit_connect(command) {
            Ok(()) | Err(SubmitError::Wake(_)) => prepared.submitted = true,
            Err(error) => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!("Connect admission: {error:?}"),
                ))
            }
        }
        // The original deadline is never extended by readiness or spurious wakes.
        let until = deadline.unwrap_or_else(|| Instant::now() + crate::stream_handle::READ_TIMEOUT);
        let response = park.wait(
            until,
            Instant::now,
            || service.health().loss().is_some(),
            || false,
            || prepared.token.is_cancelled(),
            || shared.control.has_primary_reply().then_some(()),
            false,
            None,
        );
        if !matches!(response, ParkOutcome::Ready(())) {
            handle.close();
            return Err(terminal_error(response));
        }
        match shared.control.take_primary_reply() {
            Some(PrimaryReply::Connected { binding: accepted }) if accepted == binding => {
                handle.confirm_binding(accepted)?;
                let mut stream = Self {
                    service,
                    shared,
                    handle,
                    login_deadline: deadline,
                    committed: false,
                    confirmed_at: None,
                    transport: prepared.transport,
                    host: prepared.host.clone(),
                    port: prepared.port,
                    rebind_cancel_attached: false,
                };
                if auto_commit {
                    let receipt = stream.seal_login()?;
                    stream.commit_login(receipt)?;
                    retirement.ack();
                }
                Ok(stream)
            }
            Some(PrimaryReply::ConnectFailed {
                binding: failed,
                kind,
                errno,
            }) if failed == binding => Err(match errno {
                Some(errno) => io::Error::from_raw_os_error(errno),
                None => io::Error::new(kind, "game connect failed"),
            }),
            Some(PrimaryReply::Rejected {
                binding: rejected,
                reason,
            }) if rejected == binding => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                format!("game connect rejected: {reason:?}"),
            )),
            _ => {
                handle.close();
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "wrong game connect receipt",
                ))
            }
        }
    }
    fn rebind_prepared(
        &mut self,
        mut prepared: PreparedGameAttempt,
        deadline: Option<Instant>,
    ) -> io::Result<()> {
        let expected = prepared.rebind_expected.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "adopted connection missing")
        })?;
        let until = deadline.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Rebind deadline missing")
        })?;
        if !self.rebind_cancel_attached
            || expected != self.handle.identity()
            || !Arc::ptr_eq(&self.shared, &prepared.shared)
            || !Arc::ptr_eq(&self.service, &prepared.service)
            || self.transport != prepared.transport
            || self.host != prepared.host
            || self.port != prepared.port
            || !self.committed
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "adopted stream does not own old session",
            ));
        }
        self.handle.disable_for_rebind();
        let next = prepared.binding;
        match self.service.submit_rebind(
            expected,
            next,
            until,
            prepared.token.clone(),
            Arc::clone(&prepared.retirement),
        ) {
            Ok(()) | Err(SubmitError::Wake(_)) => prepared.submitted = true,
            Err(error) => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!("Rebind admission: {error:?}"),
                ))
            }
        }
        let response = self.handle.park().wait(
            until,
            Instant::now,
            || self.service.health().loss().is_some(),
            || false,
            || prepared.token.is_cancelled(),
            || self.shared.control.has_primary_reply().then_some(()),
            false,
            None,
        );
        if !matches!(response, ParkOutcome::Ready(())) {
            self.handle.close();
            return Err(terminal_error(response));
        }
        match self.shared.control.take_primary_reply() {
            Some(PrimaryReply::Rebound { binding }) if binding == next => {
                self.handle
                    .confirm_rebind(next, until, prepared.token.clone(), None)?;
                self.login_deadline = deadline;
                self.committed = false;
                self.confirmed_at = None;
                self.rebind_cancel_attached = false;
                Ok(())
            }
            Some(PrimaryReply::Rejected { binding, reason }) if binding == next => {
                Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!("Rebind rejected: {reason:?}"),
                ))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "wrong Rebind receipt",
            )),
        }
    }
}

impl Drop for AdapterGameStream {
    fn drop(&mut self) {
        if self.committed {
            self.handle.close_with_reason(CloseReason::GracefulSession {
                by: Instant::now() + Duration::from_secs(1),
            });
        }
        // An uncommitted ConnectionHandle drop aborts the exact attempt.
    }
}

fn terminal_error<T>(outcome: ParkOutcome<T>) -> io::Error {
    match outcome {
        ParkOutcome::Deadline => {
            io::Error::new(io::ErrorKind::TimedOut, "game login deadline passed")
        }
        ParkOutcome::Revoked | ParkOutcome::SelectedIntent => {
            io::Error::new(io::ErrorKind::ConnectionAborted, "game login revoked")
        }
        ParkOutcome::Unavailable => {
            io::Error::new(io::ErrorKind::ConnectionAborted, "game reactor unavailable")
        }
        ParkOutcome::Ready(_) => {
            io::Error::new(io::ErrorKind::InvalidData, "unexpected game readiness")
        }
    }
}

impl HostGameStream for AdapterGameStream {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        self.handle.read_some(dst)
    }
    fn available(&mut self) -> io::Result<usize> {
        self.handle.available().map(|len| len.max(0) as usize)
    }
    fn wait_available(&mut self, need: usize, until: Instant) {
        if need == 0 {
            return;
        }
        let park = self.handle.park();
        let _ = park.wait(
            until,
            Instant::now,
            || self.service.health().loss().is_some(),
            || false,
            || false,
            || {
                self.handle
                    .available()
                    .ok()
                    .filter(|&len| len as usize >= need)
                    .map(|_| ())
            },
            false,
            None,
        );
    }
    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        self.handle.write(record, record.len())
    }
    fn set_deadline(&mut self, deadline: Option<Instant>) -> io::Result<()> {
        if deadline.is_none() && !self.committed {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "login not committed",
            ));
        }
        self.handle.set_deadline(deadline);
        self.login_deadline = deadline;
        Ok(())
    }
    fn binding_identity(&self) -> client::io::host_network::HostStreamIdentity {
        client::io::host_network::HostStreamIdentity::new(AdapterStreamIdentity {
            service: Arc::clone(&self.service),
            shared: Arc::clone(&self.shared),
            binding: self.handle.identity(),
            transport: self.transport,
            host: self.host.clone(),
            port: self.port,
        })
    }
    fn attach_cancel(
        &mut self,
        cancel: &LoginCancel,
        attempt: &client::io::host_network::HostAttemptIdentity,
    ) -> io::Result<()> {
        let proof = attempt
            .downcast_ref::<GameAttemptProof>()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "foreign game attempt"))?;
        if proof.binding == self.handle.identity()
            && proof.generation == proof.cancel.generation()
            && proof.generation == proof.retirement.generation()
        {
            let revoke = proof.cancel.clone();
            cancel.register_cancel_callback(move || revoke.cancel());
            return Ok(());
        }
        let matches = GAME_ATTEMPT.with(|cell| {
            let pending = cell.borrow();
            pending.as_ref().is_some_and(|prepared| {
                prepared.rebind_expected == Some(self.handle.identity())
                    && prepared.binding == proof.binding
                    && prepared.token.same_attempt(&proof.cancel)
                    && Arc::ptr_eq(&prepared.retirement, &proof.retirement)
                    && Arc::ptr_eq(&prepared.shared, &self.shared)
                    && Arc::ptr_eq(&prepared.service, &self.service)
                    && proof.transfer_seq
                        == prepared.queue_release_proof().map(|p| p.transfer_seq())
            })
        });
        if !matches {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "captured Rebind attempt changed",
            ));
        }
        let revoke = proof.cancel.clone();
        cancel.register_cancel_callback(move || revoke.cancel());
        self.rebind_cancel_attached = true;
        Ok(())
    }
    fn rebind(
        &mut self,
        attempt: &client::io::host_network::HostAttemptIdentity,
        deadline: Option<Instant>,
    ) -> io::Result<()> {
        let proof = attempt
            .downcast_ref::<GameAttemptProof>()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "foreign game attempt"))?;
        if !self.rebind_cancel_attached
            || proof.binding.connection != self.handle.identity().connection
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "adopted stream not attached",
            ));
        }
        let prepared = GAME_ATTEMPT
            .with(|cell| cell.borrow_mut().take())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "Rebind attempt not prepared")
            })?;
        if proof.binding != prepared.binding
            || proof.generation != prepared.token.generation()
            || !proof.cancel.same_attempt(&prepared.token)
            || !Arc::ptr_eq(&proof.retirement, &prepared.retirement)
            || proof.transfer_seq != prepared.queue_release_proof().map(|p| p.transfer_seq())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Rebind identity changed",
            ));
        }
        self.rebind_prepared(prepared, deadline)
    }
    fn seal_login(&mut self) -> io::Result<SealReceipt> {
        if self.committed || self.confirmed_at.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "login already sealed",
            ));
        }
        let binding = self.handle.identity();
        let park = self.handle.park();
        let shared = &self.shared;
        let deadline = self
            .login_deadline
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "login deadline missing"))?;
        shared
            .control
            .post_primary(PrimaryCommand::Seal { binding }, &self.service.bus())
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("Seal admission: {error:?}"),
                )
            })?;
        match park.wait(
            deadline,
            Instant::now,
            || self.service.health().loss().is_some(),
            || false,
            || false,
            || shared.control.has_primary_reply().then_some(()),
            true,
            None,
        ) {
            ParkOutcome::Ready(()) => {}
            terminal => {
                self.handle.close();
                return Err(terminal_error(terminal));
            }
        }
        let confirmed_at = match shared.control.take_primary_reply() {
            Some(PrimaryReply::Sealed {
                binding: sealed,
                confirmed_at,
            }) if sealed == binding && confirmed_at < deadline => confirmed_at,
            _ => {
                self.handle.close();
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Seal rejected or late",
                ));
            }
        };
        self.confirmed_at = Some(confirmed_at);
        Ok(Box::new(AdapterSealReceipt {
            binding,
            confirmed_at,
            service_incarnation: self.service.incarnation(),
        }))
    }
    fn commit_login(&mut self, receipt: SealReceipt) -> io::Result<()> {
        let receipt = receipt
            .downcast::<AdapterSealReceipt>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "foreign Seal receipt"))?;
        if self.committed
            || receipt.binding != self.handle.identity()
            || receipt.service_incarnation != self.service.incarnation()
            || self.confirmed_at != Some(receipt.confirmed_at)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stale Seal receipt",
            ));
        }
        self.handle.commit_session(receipt.binding)?;
        self.committed = true;
        self.confirmed_at = None;
        // The queue's physical acknowledgement runs after the K4 commit.
        Ok(())
    }
    fn close(&mut self, disposition: HostCloseDisposition) {
        let reason = match disposition {
            HostCloseDisposition::AbortAttempt => CloseReason::Abort,
            HostCloseDisposition::GracefulSession => CloseReason::GracefulSession {
                by: Instant::now() + Duration::from_secs(1),
            },
        };
        self.handle.close_with_reason(reason);
    }
    fn take_exit_reason(&mut self) -> Option<HostStreamExit> {
        self.handle.take_exit_reason().map(|reason| match reason {
            StreamExitReason::WebSocketClose { code, reason } => {
                HostStreamExit::WebSocketClose { code, reason }
            }
            StreamExitReason::Tls(message) => HostStreamExit::Tls(message),
            StreamExitReason::WebSocket(message) => HostStreamExit::WebSocket(message),
        })
    }
}

impl HostNetwork for AdapterNetwork {
    fn connect_game(
        &self,
        mode: Transport,
        host: &str,
        port: u16,
        deadline: Option<Instant>,
        cancel: Option<&LoginCancel>,
        attempt: &client::io::host_network::HostAttemptIdentity,
    ) -> io::Result<Box<dyn HostGameStream>> {
        self.require_driver()?;
        let prepared = GAME_ATTEMPT
            .with(|cell| cell.borrow_mut().take())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "game attempt not prepared before queue start",
                )
            })?;
        let expected = attempt.downcast_ref::<GameAttemptProof>().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "wrong game attempt identity type",
            )
        })?;
        if expected.queue_owner == 0
            || expected.binding != prepared.binding
            || expected.generation != prepared.token.generation()
            || !expected.cancel.same_attempt(&prepared.token)
            || !Arc::ptr_eq(&expected.retirement, &prepared.retirement)
            || expected.transfer_seq
                != prepared
                    .queue_release_proof()
                    .map(|proof| proof.transfer_seq())
            || prepared.transport.hook() != mode
            || prepared.host != host
            || prepared.port != port
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "captured game attempt changed",
            ));
        }
        let cancel = cancel.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "hosted game login needs cancellation",
            )
        })?;
        let token = prepared.token.clone();
        cancel.register_cancel_callback(move || token.cancel());
        AdapterGameStream::from_prepared(prepared, deadline, false)
            .map(|stream| Box::new(stream) as Box<dyn HostGameStream>)
    }

    fn connect_ondemand(
        &self,
        mode: Transport,
        host: &str,
        port: u16,
        resolved: Option<&[SocketAddr]>,
        cancel: Option<&LoginCancel>,
    ) -> io::Result<Box<dyn HostGameStream>> {
        self.require_driver()?;
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source
            .child()
            .ok_or_else(|| io::Error::other("OnDemand generation exhausted"))?;
        if let Some(cancel) = cancel {
            let revoke = token.clone();
            cancel.register_cancel_callback(move || revoke.cancel());
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        let addresses = match resolved {
            Some(resolved) => {
                let mut addresses = ResolvedAddresses::new();
                for &address in resolved {
                    addresses.push(address);
                }
                addresses
            }
            None => {
                let slot = NEXT_FETCH_RESOLVE_SLOT
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                        next.checked_add(1)
                    })
                    .map_err(|_| io::Error::other("OnDemand slot identity exhausted"))?;
                resolve_game(
                    &self.resolver,
                    &self.service,
                    SlotKey(slot),
                    token.generation(),
                    host,
                    port,
                    Arc::clone(&park),
                    &token,
                )?
            }
        };
        let ticket = if mode == Transport::Wss {
            let ticket =
                self.service
                    .game_tls_waiter(Arc::clone(&park), deadline, token.clone())?;
            let outcome = park.wait(
                deadline,
                Instant::now,
                || self.service.health().loss().is_some(),
                || false,
                || token.is_cancelled(),
                || {
                    let state = ticket.poll(Instant::now());
                    (!matches!(state, crate::tls_handshake::GameTlsWait::Waiting)).then_some(state)
                },
                false,
                None,
            );
            match outcome {
                ParkOutcome::Ready(crate::tls_handshake::GameTlsWait::Ready) => Some(ticket),
                ParkOutcome::Ready(
                    crate::tls_handshake::GameTlsWait::InsufficientRemainingTime { .. },
                ) => {
                    ticket.release_for_insufficient_time();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "insufficient WSS start time",
                    ));
                }
                _ => return Err(terminal_error(outcome)),
            }
        } else {
            None
        };
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let mut prepared = PreparedGameAttempt::new(
            Arc::clone(&self.service),
            park,
            token,
            retire,
            GameTransport::from_hook(mode),
            host,
            port,
            addresses,
        )?;
        if let Some(ticket) = ticket {
            prepared.set_tls_ticket(ticket)?;
        }
        AdapterGameStream::from_prepared(prepared, Some(deadline), true)
            .map(|stream| Box::new(stream) as Box<dyn HostGameStream>)
    }

    fn start_cold_fill_lookup(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn HostLookupTicket>, HostLookupFailure> {
        self.require_driver()
            .map_err(|_| HostLookupFailure::Unavailable)?;
        let slot = NEXT_FETCH_RESOLVE_SLOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| HostLookupFailure::Busy)?;
        let park = Arc::new(Park::new());
        let wake = Arc::new(ResolveWake {
            ready: AtomicBool::new(false),
            park: Arc::clone(&park),
        });
        let ticket = self
            .resolver
            .resolve(
                ResolveIdentity {
                    slot: SlotKey(slot),
                    generation: 1,
                },
                host,
                port,
                Arc::clone(&wake) as Arc<dyn ResolverWake>,
            )
            .map_err(|error| match error {
                ResolverAdmissionError::InvalidHost => HostLookupFailure::Lookup,
                ResolverAdmissionError::Unavailable(_) => HostLookupFailure::Unavailable,
                ResolverAdmissionError::Busy
                | ResolverAdmissionError::SlotAlreadyWaiting
                | ResolverAdmissionError::DeadlineOverflow => HostLookupFailure::Busy,
            })?;
        Ok(Box::new(ColdFillLookup {
            ticket,
            wake,
            park,
            resolver: Arc::clone(&self.resolver),
            service: Arc::clone(&self.service),
        }))
    }

    fn start_http(
        &self,
        mode: Transport,
        host: &str,
        port: u16,
        path: &str,
    ) -> Result<Box<dyn HostHttpFetch>, AssetFetchError> {
        self.require_driver()
            .map_err(|_| AssetFetchError::Connection)?;
        let slot = NEXT_FETCH_RESOLVE_SLOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| AssetFetchError::Connection)?;
        let park = Arc::new(Park::new());
        let wake = Arc::new(ResolveWake {
            ready: AtomicBool::new(false),
            park: Arc::clone(&park),
        });
        let ticket = self
            .resolver
            .resolve(
                ResolveIdentity {
                    slot: SlotKey(slot),
                    generation: 1,
                },
                host,
                port,
                Arc::clone(&wake) as Arc<dyn ResolverWake>,
            )
            .map_err(|_| AssetFetchError::Connection)?;
        Ok(Box::new(ResolvingHttpFetch {
            ticket: Some(ticket),
            wake,
            park,
            resolver: Arc::clone(&self.resolver),
            service: Arc::clone(&self.service),
            https: Arc::clone(&self.https),
            mode,
            host: host.to_owned(),
            path: path.to_owned(),
            inner: None,
        }))
    }

    fn start_http_resolved(
        &self,
        mode: Transport,
        host: &str,
        addrs: &[SocketAddr],
        path: &str,
    ) -> Result<Box<dyn HostHttpFetch>, AssetFetchError> {
        self.require_driver()
            .map_err(|_| AssetFetchError::Connection)?;
        crate::http::start_fetch(mode, host, addrs, path, &self.https)
    }

    fn fetch_http_blocking_resolved(
        &self,
        mode: Transport,
        host: &str,
        addrs: &[SocketAddr],
        path: &str,
        deadline: Option<Instant>,
        cancel: Option<&LoginCancel>,
    ) -> Result<Vec<u8>, AssetFetchError> {
        self.require_driver()
            .map_err(|_| AssetFetchError::Connection)?;
        crate::http::fetch_blocking(mode, host, addrs, path, deadline, &self.https, cancel)
    }

    fn fetch_http_blocking(
        &self,
        mode: Transport,
        host: &str,
        port: u16,
        path: &str,
        deadline: Option<Instant>,
        cancel: Option<&LoginCancel>,
    ) -> Result<Vec<u8>, AssetFetchError> {
        self.require_driver()
            .map_err(|_| AssetFetchError::Connection)?;
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().ok_or(AssetFetchError::Connection)?;
        if let Some(cancel) = cancel {
            let revoke = token.clone();
            cancel.register_cancel_callback(move || revoke.cancel());
        }
        let slot = NEXT_FETCH_RESOLVE_SLOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| AssetFetchError::Connection)?;
        let resolved = resolve_game(
            &self.resolver,
            &self.service,
            SlotKey(slot),
            token.generation(),
            host,
            port,
            park,
            &token,
        )
        .map_err(|_| AssetFetchError::Connection)?;
        self.require_driver()
            .map_err(|_| AssetFetchError::Connection)?;
        if token.is_cancelled() {
            return Err(AssetFetchError::Connection);
        }
        let (addresses, count) = inline_http_addresses(&resolved);
        crate::http::fetch_blocking(
            mode,
            host,
            &addresses[..count],
            path,
            deadline,
            &self.https,
            cancel,
        )
    }
}

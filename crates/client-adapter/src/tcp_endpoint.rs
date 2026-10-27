//! Reactor-owned nonblocking TCP setup and finite game-I/O turns.
//!
//! Each setup invocation tries at most one resolved address. The original
//! registered mio socket stays with this owner until physical close or the
//! checked initial-TLS handoff; no socket clone or slot-side OS poll exists.

use std::io::{self, Read};
use std::sync::Arc;
use std::time::Instant;

use mio::net::TcpStream;
use mio::{Interest, Registry, Token};

use super::budget::{Budgeted, StopReason, TURN_BYTE_LIMIT};
use super::cancel::{AttemptRetire, CancelToken};
use super::control::{BindingId, RejectedCommand};
use super::park::WriterBinding;
use super::resolver::ResolvedAddresses;
use super::shared::SharedConnection;
use super::tcp::{TcpTurn, TcpTurnResult};

#[derive(Debug)]
pub enum TcpStep {
    /// One address failed; schedule one more finite setup step.
    Retry,
    /// A registered socket awaits a writable connection completion.
    Connecting,
    /// The registered socket is connected, but no login reply was issued.
    Ready,
    /// No socket remains and no address can be tried.
    Failed(io::Error),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RebindBoundary {
    /// Old output, a prior inbound span, or kernel bytes need another turn.
    Pending,
    /// Old output remains, but the registered socket needs writable readiness.
    WaitWritable,
    /// The old ring is empty and a native read reached real WouldBlock.
    Clean,
    /// The old connection ended before a clean handoff.
    PeerClosed,
}

/// Only the reactor may call these methods. `retire` belongs to the physical
/// attempt and is acknowledged on close, not at a positive TCP connect or
/// after an initial-TLS custody transfer.
#[derive(Debug)]
pub struct TcpEndpoint {
    binding: BindingId,
    addresses: ResolvedAddresses,
    next_address: usize,
    socket: Option<Budgeted<TcpStream>>,
    connecting: bool,
    last_error: Option<io::Error>,
    turn: TcpTurn,
    clean_rebind: bool,
    read_suspended: bool,
    shared: Arc<SharedConnection>,
    cancel: Option<CancelToken>,
    deadline: Option<Instant>,
    retire: Arc<AttemptRetire>,
    /// Set after normal close or ownership transfer; unwinding must retire
    /// an abandoned attempt that still owns its native socket.
    retired: bool,
}

impl TcpEndpoint {
    pub fn new(
        binding: BindingId,
        addresses: ResolvedAddresses,
        shared: Arc<SharedConnection>,
        cancel: CancelToken,
        deadline: Option<Instant>,
        retire: Arc<AttemptRetire>,
    ) -> io::Result<Self> {
        if binding.incarnation == 0
            || binding.connection == 0
            || binding.binding == 0
            || binding.attempt == 0
            || usize::try_from(binding.connection).is_err()
            || addresses.is_empty()
            || shared.control.id != binding.connection
            || cancel.generation() != retire.generation()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid TCP attempt identity",
            ));
        }
        Ok(Self {
            binding,
            addresses,
            next_address: 0,
            socket: None,
            connecting: false,
            last_error: None,
            turn: TcpTurn::default(),
            clean_rebind: false,
            read_suspended: false,
            shared,
            cancel: Some(cancel),
            deadline,
            retire,
            retired: false,
        })
    }

    pub fn binding(&self) -> BindingId {
        self.binding
    }

    pub fn is_connected(&self) -> bool {
        self.socket.is_some() && !self.connecting
    }

    /// Start the next address without blocking or looping over candidates.
    /// A failing address keeps the exact attempt/capacity charge until the
    /// reactor calls this method again or closes the endpoint.
    pub fn begin_step(&mut self, registry: &Registry, now: Instant) -> TcpStep {
        self.begin_step_with_connector(registry, now, TcpStream::connect)
    }

    fn begin_step_with_connector(
        &mut self,
        registry: &Registry,
        now: Instant,
        connect: impl FnOnce(std::net::SocketAddr) -> io::Result<TcpStream>,
    ) -> TcpStep {
        if let Some(error) = self.revoked(now) {
            return TcpStep::Failed(error);
        }
        if self.socket.is_some() {
            return if self.connecting {
                TcpStep::Connecting
            } else {
                TcpStep::Ready
            };
        }
        let Some(address) = self.addresses.get(self.next_address) else {
            return TcpStep::Failed(self.last_error.take().unwrap_or_else(|| {
                io::Error::new(io::ErrorKind::AddrNotAvailable, "no reachable game address")
            }));
        };
        self.next_address += 1;
        match connect(address) {
            Ok(mut socket) => {
                let token = Token(self.binding.connection as usize);
                match registry.register(
                    &mut socket,
                    token,
                    Interest::READABLE.add(Interest::WRITABLE),
                ) {
                    Ok(()) => {
                        self.socket = Some(Budgeted::new(socket));
                        self.connecting = true;
                        TcpStep::Connecting
                    }
                    Err(error) => self.address_failure(error),
                }
            }
            Err(error) => self.address_failure(error),
        }
    }

    fn address_failure(&mut self, error: io::Error) -> TcpStep {
        if self.next_address < self.addresses.len() {
            self.last_error = Some(error);
            TcpStep::Retry
        } else {
            TcpStep::Failed(error)
        }
    }

    /// Mio events are hints. A writable event checks the socket error and
    /// peer association before allowing any game bytes or WSS setup.
    pub fn on_event(
        &mut self,
        registry: &Registry,
        readable: bool,
        writable: bool,
        now: Instant,
    ) -> TcpStep {
        if readable {
            self.clean_rebind = false;
        }
        if let Some(error) = self.revoked(now) {
            return TcpStep::Failed(error);
        }
        if !self.connecting {
            if readable && !self.read_suspended && !self.shared.rebind_disabled() {
                self.turn.on_readable();
            }
            if writable {
                self.turn.on_writable();
            }
            return TcpStep::Ready;
        }
        if !readable && !writable {
            return TcpStep::Connecting;
        }
        let socket = self
            .socket
            .as_mut()
            .expect("connecting owns the registered socket");
        let error = match socket.inner().take_error() {
            Ok(Some(error)) => Some(error),
            Ok(None) => match socket.inner().peer_addr() {
                Ok(_) => None,
                Err(error) if error.kind() == io::ErrorKind::NotConnected => {
                    return TcpStep::Connecting;
                }
                Err(error) => Some(error),
            },
            Err(error) => Some(error),
        };
        if let Some(error) = error {
            self.drop_socket(registry);
            return self.address_failure(error);
        }
        // A failed nonblocking connect can make setsockopt return EINVAL on
        // macOS. Preserve SO_ERROR above before configuring a connected socket.
        if let Err(error) = socket.inner().set_nodelay(true) {
            // A reset can arrive after peer_addr succeeded. Prefer that socket
            // error to an option error from configuring the now-failed socket.
            let error = socket.inner().take_error().ok().flatten().unwrap_or(error);
            self.drop_socket(registry);
            return self.address_failure(error);
        }
        self.connecting = false;
        if readable {
            self.turn.on_readable();
        }
        if writable {
            self.turn.on_writable();
        }
        TcpStep::Ready
    }

    pub fn resume_read(&mut self) {
        self.clean_rebind = false;
        if self.shared.rebind_disabled() {
            return;
        }
        if self.shared.inbound.free() > 0 {
            self.read_suspended = false;
        }
        if !self.read_suspended {
            self.turn.resume_read();
        }
    }

    pub fn kick_output(&mut self) {
        self.clean_rebind = false;
        self.turn.kick_output();
    }

    /// The reactor calls this once per fair round, only after the phase that
    /// publishes the positive TCP login connection reply. It never calls the
    /// socket on a slot or TLS-handshake worker thread.
    pub fn drive(&mut self) -> io::Result<TcpTurnResult> {
        self.clean_rebind = false;
        assert!(
            self.is_connected(),
            "cannot drive an unfinished TCP connect"
        );
        let socket = self.socket.as_mut().expect("connected socket");
        socket.reset_turn();
        let result = self.turn.drive(
            socket,
            &self.shared.inbound,
            &self.shared.outbound,
            &self.shared.records,
            &self.shared.control.park,
            WriterBinding {
                connection: self.binding.connection,
                binding: self.binding.binding,
            },
        )?;
        self.read_suspended = self.shared.inbound.free() == 0;
        Ok(result)
    }

    /// A graceful logout stops inbound delivery and flushes already accepted
    /// semantic records. The reactor enforces the caller's one-second bound.
    /// The second result requests writable readiness rather than local spin.
    pub fn flush_for_close(&mut self) -> io::Result<(bool, bool)> {
        self.turn.suppress_read_for_rebind();
        if self.shared.outbound.is_empty() && self.shared.records.front().is_none() {
            return Ok((true, false));
        }
        let turn = self.drive()?;
        let finished = self.shared.outbound.is_empty() && self.shared.records.front().is_none();
        Ok((finished, !finished && turn.write_ready))
    }

    pub fn attempt_failure(&self, now: Instant) -> Option<io::ErrorKind> {
        self.revoked(now).map(|error| error.kind())
    }

    /// Phase R changes only authority, not socket, rings, or byte order. A
    /// positive receipt captures a time strictly before the original D and
    /// permanently detaches this attempt's token and deadline. Phase C still
    /// decides whether the slot may use the session.
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

    /// Quiesce a committed TCP session for Rebind. The slot disables ring
    /// reads before posting the command. The reactor may then dispose the
    /// old-generation inbound span, flush old output, and probe the kernel
    /// until a *real* WouldBlock proves the current receive boundary. A
    /// positive read always takes another fair turn, even if it was short.
    pub fn drain_for_rebind(&mut self) -> io::Result<RebindBoundary> {
        self.clean_rebind = false;
        if !self.is_connected() || self.shared.is_closed() {
            return Ok(RebindBoundary::PeerClosed);
        }
        if !self.shared.rebind_disabled() || self.shared.session_binding() != self.binding.binding {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "old session was not quiesced for Rebind",
            ));
        }
        if !self.shared.outbound.is_empty() || self.shared.records.front().is_some() {
            self.turn.suppress_read_for_rebind();
            let turn = self.drive()?;
            if turn.stop == super::tcp::TcpStop::PeerClosed {
                return Ok(RebindBoundary::PeerClosed);
            }
            if !self.shared.outbound.is_empty() && !turn.write_ready {
                return Ok(RebindBoundary::WaitWritable);
            }
            return Ok(RebindBoundary::Pending);
        }
        if self
            .shared
            .inbound
            .consume_with(|span| Ok(span.len().min(TURN_BYTE_LIMIT)))?
            != 0
        {
            return Ok(RebindBoundary::Pending);
        }
        let socket = self.socket.as_mut().expect("connected socket");
        socket.reset_turn();
        let mut old_bytes = [0u8; TURN_BYTE_LIMIT];
        match socket.read(&mut old_bytes) {
            Ok(0) => Ok(RebindBoundary::PeerClosed),
            Ok(_) => Ok(RebindBoundary::Pending),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if socket.budget().read_stop() == StopReason::SocketWouldBlock {
                    self.turn.suppress_read_for_rebind();
                    self.clean_rebind = true;
                    Ok(RebindBoundary::Clean)
                } else {
                    Ok(RebindBoundary::Pending)
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Install the next attempt only after `drain_for_rebind` returns Clean.
    /// The caller retains the old binding on rejection and acknowledges the
    /// new retirement cell when no socket was transferred.
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
            || !self.shared.rebind_disabled()
            || self.shared.session_binding() != expected.binding
            || !self.shared.inbound.is_empty()
            || !self.shared.outbound.is_empty()
            || self.shared.records.front().is_some()
        {
            return Err(RejectedCommand::DirtyReceiveBoundary);
        }
        self.clean_rebind = false;
        self.read_suspended = false;
        self.binding = next;
        self.cancel = Some(token);
        self.deadline = Some(deadline);
        self.retire = retire;
        self.turn = TcpTurn::default();
        self.shared.accept_binding(next);
        Ok(())
    }

    /// Move the exactly registered socket into the fixed initial-TLS worker.
    /// The reactor retains selector authority and deregisters on failure or
    /// eventual close; no clone or second socket owner is created.
    pub fn take_registered(mut self) -> (TcpStream, Arc<AttemptRetire>) {
        assert!(self.is_connected(), "initial TLS requires connected TCP");
        let socket = self.socket.take().expect("connected socket").into_inner();
        self.retired = true;
        (socket, Arc::clone(&self.retire))
    }

    /// Transfer only after deregistration succeeds. A failed deregistration
    /// closes the socket and retires the attempt rather than letting a second
    /// owner operate a descriptor still registered with the reactor.
    pub fn transfer(mut self, registry: &Registry) -> io::Result<(TcpStream, Arc<AttemptRetire>)> {
        assert!(self.is_connected(), "initial TLS requires connected TCP");
        let mut socket = self.socket.take().expect("connected socket").into_inner();
        if let Err(error) = registry.deregister(&mut socket) {
            drop(socket);
            self.retire.ack();
            self.shared.publish_closed();
            self.retired = true;
            return Err(error);
        }
        self.retired = true;
        Ok((socket, Arc::clone(&self.retire)))
    }

    /// Physical close precedes the exact attempt's retirement acknowledgement.
    pub fn close(mut self, registry: &Registry) {
        self.drop_socket(registry);
        self.retire.ack();
        self.shared.publish_closed();
        self.retired = true;
    }

    fn drop_socket(&mut self, registry: &Registry) {
        if let Some(mut socket) = self.socket.take() {
            let _ = registry.deregister(socket.inner_mut());
            drop(socket);
        }
        self.connecting = false;
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
}
impl Drop for TcpEndpoint {
    fn drop(&mut self) {
        if !self.retired {
            drop(self.socket.take());
            self.retire.ack();
            self.shared.publish_closed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::cancel::CancelSource;
    use super::super::park::Park;
    use super::*;
    use std::net::SocketAddr;
    use std::net::TcpListener;
    use std::time::Duration;

    fn connected(
        deadline: Instant,
    ) -> (
        TcpEndpoint,
        std::net::TcpStream,
        mio::Poll,
        Arc<SharedConnection>,
        CancelSource,
        Arc<AttemptRetire>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let socket = std::net::TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut socket = TcpStream::from_std(socket);
        let poll = mio::Poll::new().unwrap();
        poll.registry()
            .register(
                &mut socket,
                Token(1),
                Interest::READABLE.add(Interest::WRITABLE),
            )
            .unwrap();
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let shared = SharedConnection::new(1, park);
        let binding = BindingId {
            incarnation: 1,
            connection: 1,
            binding: 1,
            attempt: 1,
        };
        let mut addresses = ResolvedAddresses::new();
        assert!(addresses.push(address));
        let mut endpoint = TcpEndpoint::new(
            binding,
            addresses,
            Arc::clone(&shared),
            token,
            Some(deadline),
            Arc::clone(&retire),
        )
        .unwrap();
        endpoint.socket = Some(Budgeted::new(socket));
        endpoint.connecting = false;
        shared.accept_binding(binding);
        (endpoint, server, poll, shared, source, retire)
    }

    #[test]
    fn seal_detaches_deadline_and_token_only_before_original_deadline() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        let (mut endpoint, _server, poll, shared, _source, retire) = connected(deadline);
        let id = endpoint.binding();
        let old = endpoint.cancel.as_ref().unwrap().clone();
        let just_before_deadline = deadline
            .checked_sub(Duration::from_nanos(1))
            .expect("test deadline is in the future");
        assert_eq!(
            endpoint.seal(id, just_before_deadline),
            Ok(just_before_deadline)
        );
        old.cancel();
        assert!(matches!(
            endpoint.on_event(poll.registry(), false, false, deadline),
            TcpStep::Ready
        ));
        assert!(!shared.commit_session(BindingId { binding: 2, ..id }));
        assert!(shared.commit_session(id));
        assert!(
            !retire.acknowledged(),
            "Phase R does not acknowledge physical retirement"
        );
        assert!(retire.ack(), "Phase C acknowledges after the K4 commit");
        endpoint.close(poll.registry());
        assert_eq!(retire.ack_count(), 1);

        let (mut late, _server, poll, _shared, _source, _retire) = connected(deadline);
        assert_eq!(
            late.seal(late.binding(), deadline),
            Err(RejectedCommand::Deadline)
        );
        late.cancel.as_ref().unwrap().cancel();
        assert_eq!(
            late.seal(late.binding(), deadline),
            Err(RejectedCommand::Revoked)
        );
        late.close(poll.registry());
    }

    #[test]
    fn adopted_tcp_binding_disposes_old_ring_before_a_new_hello() {
        let now = Instant::now();
        let (mut endpoint, _server, poll, shared, source, retire) =
            connected(now + Duration::from_secs(5));
        let old = endpoint.binding();
        endpoint.seal(old, now).unwrap();
        assert!(shared.commit_session(old));
        assert!(retire.ack());
        assert!(shared.inbound.push_all(b"old-session"));
        shared.disable_for_rebind();
        assert_eq!(
            endpoint.drain_for_rebind().unwrap(),
            RebindBoundary::Pending
        );
        assert_eq!(shared.inbound.len(), 0);
        assert_eq!(endpoint.drain_for_rebind().unwrap(), RebindBoundary::Clean);
        let token = source.child().unwrap();
        let next_retire = Arc::new(AttemptRetire::new(token.generation()));
        let next = BindingId {
            binding: 2,
            attempt: 2,
            ..old
        };
        endpoint
            .rebind(
                old,
                next,
                now + Duration::from_secs(10),
                now,
                token,
                Arc::clone(&next_retire),
            )
            .unwrap();
        assert_eq!(shared.binding(), 2);
        assert_eq!(shared.session_binding(), 0);
        assert_eq!(shared.inbound.len(), 0);
        endpoint.close(poll.registry());
        assert_eq!(next_retire.ack_count(), 1);
    }

    #[test]
    fn one_candidate_per_step_keeps_later_address_available() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let no_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bad = no_listener.local_addr().unwrap();
        drop(no_listener);
        let mut addresses = ResolvedAddresses::new();
        assert!(addresses.push(bad));
        assert!(addresses.push(address));
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let binding = BindingId {
            incarnation: 1,
            connection: 1,
            binding: 1,
            attempt: 1,
        };
        let shared = SharedConnection::new(1, park);
        let mut endpoint =
            TcpEndpoint::new(binding, addresses, shared, token, None, Arc::clone(&retire)).unwrap();
        let poll = mio::Poll::new().unwrap();
        assert!(matches!(
            endpoint.begin_step(poll.registry(), Instant::now()),
            TcpStep::Connecting | TcpStep::Retry
        ));
        assert!(!retire.acknowledged());
        endpoint.close(poll.registry());
        assert!(retire.acknowledged());
    }

    #[test]
    fn completed_refusal_keeps_socket_error_before_setting_options() {
        let no_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = no_listener.local_addr().unwrap();
        drop(no_listener);
        let mut addresses = ResolvedAddresses::new();
        assert!(addresses.push(address));
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let shared = SharedConnection::new(1, park);
        let binding = BindingId {
            incarnation: 1,
            connection: 1,
            binding: 1,
            attempt: 1,
        };
        let mut endpoint =
            TcpEndpoint::new(binding, addresses, shared, token, None, retire).unwrap();
        let poll = mio::Poll::new().unwrap();
        let step = endpoint.begin_step_with_connector(poll.registry(), Instant::now(), |address| {
            let mut socket = TcpStream::connect(address)?;
            // Hold only the connector return until the real refusal is ready.
            // Do not consume SO_ERROR: the production endpoint must read it.
            let mut barrier = mio::Poll::new()?;
            barrier.registry().register(
                &mut socket,
                Token(1),
                Interest::READABLE.add(Interest::WRITABLE),
            )?;
            let mut events = mio::Events::with_capacity(4);
            barrier.poll(&mut events, Some(Duration::from_secs(2)))?;
            assert!(!events.is_empty(), "refused socket must become ready");
            barrier.registry().deregister(&mut socket)?;
            Ok(socket)
        });
        assert!(matches!(step, TcpStep::Connecting), "{step:?}");
        let step = endpoint.on_event(poll.registry(), true, true, Instant::now());
        let TcpStep::Failed(error) = step else {
            panic!("refused connect must fail: {step:?}");
        };
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert!(error.raw_os_error().is_some());
        assert!(endpoint.socket.is_none());
    }

    #[test]
    fn closed_or_timed_out_attempt_never_starts_a_socket() {
        let mut addresses = ResolvedAddresses::new();
        assert!(addresses.push("127.0.0.1:1".parse::<SocketAddr>().unwrap()));
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let shared = SharedConnection::new(1, park);
        let binding = BindingId {
            incarnation: 1,
            connection: 1,
            binding: 1,
            attempt: 1,
        };
        let mut endpoint = TcpEndpoint::new(
            binding,
            addresses,
            shared,
            token,
            Some(Instant::now()),
            retire,
        )
        .unwrap();
        let poll = mio::Poll::new().unwrap();
        assert!(
            matches!(endpoint.begin_step(poll.registry(), Instant::now()), TcpStep::Failed(error) if error.kind() == io::ErrorKind::TimedOut)
        );
        assert!(endpoint.socket.is_none());
    }
}

//! One owner of registered game sockets, control cells and fair local turns.
//! Slot threads hold rings and identity-scoped handles, never native sockets.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mio::{Events, Poll, Token, Waker};

use super::cancel::{AttemptRetire, CancelToken};
use super::control::{
    BindingId, CloseCommand, CloseReason, CommandBus, ConnectCommand, PrimaryCommand, PrimaryReply,
    RejectedCommand, SubmitError,
};
use super::handoff::{
    HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, ReadyDirection, RevocationReason,
    TakeReturnError, WorkerFailure, WorkerOutcome,
};
use super::health::{ReactorHealth, ReactorLoss, REACTOR_POLL_MAX};
use super::ledger::{ConnectionCharge, ResourceLedger, MAX_CONNECTIONS};
use super::park::Park;
use super::shared::SharedConnection;
use super::tcp::TcpStop;
use super::tcp_endpoint::{RebindBoundary, TcpEndpoint, TcpStep};
use super::tls_handshake::{GameTlsLane, GameTlsResource, GameTlsTicket};
use super::ws_endpoint::{WssEndpoint, WssStep};
use client::Transport;

const WAKE_TOKEN: Token = Token(0);
// The existing close flush has one second. The captured server-first WS
// closes returned the client FIN in 47 and 84 ms (F9, 2026-11-04 review).
// Allow a separate second for a delayed peer TLS alert or TCP FIN.
const GRACEFUL_PEER_DRAIN: Duration = Duration::from_secs(1);
static NEXT_INCARNATION: AtomicU64 = AtomicU64::new(1);
static PROCESS_TRANSPORT: LazyLock<Result<Arc<TransportService>, io::ErrorKind>> =
    LazyLock::new(|| TransportService::start().map_err(|error| error.kind()));

struct Registration {
    id: u64,
    initial: BindingId,
    shared: Arc<SharedConnection>,
    submitted: AtomicBool,
    charge: Mutex<Option<ConnectionCharge>>,
}

impl Registration {
    /// Sender Arc clones must not keep physical capacity charged after close.
    fn release_charge(&self) {
        self.charge
            .lock()
            .expect("TLS connection charge mutex is not poisoned")
            .take();
    }

    /// During the initial TLS worker call the registered socket and its
    /// physical capacity charge move together, including after queue release.
    fn take_charge(&self) -> Option<ConnectionCharge> {
        self.charge
            .lock()
            .expect("TLS connection charge mutex is not poisoned")
            .take()
    }

    fn restore_charge(&self, charge: Option<ConnectionCharge>) {
        let mut slot = self
            .charge
            .lock()
            .expect("TLS connection charge mutex is not poisoned");
        assert!(slot.is_none(), "returned TLS charge already registered");
        *slot = charge;
    }
}

struct Registered {
    // Sparse modulo-slot storage preserves identity checks without empty entries.
    entries: HashMap<usize, Arc<Registration>>,
}

impl Registered {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn get(&self, id: u64) -> Option<Arc<Registration>> {
        self.entries
            .get(&index(id))
            .filter(|entry| entry.id == id)
            .cloned()
    }

    fn remove(&mut self, id: u64) -> Option<Arc<Registration>> {
        let slot = index(id);
        if self.entries.get(&slot).is_some_and(|entry| entry.id == id) {
            self.entries.remove(&slot)
        } else {
            None
        }
    }
}

fn index(id: u64) -> usize {
    (id.saturating_sub(1) % MAX_CONNECTIONS as u64) as usize
}

/// Created before any game transport admission; its only native socket owner
/// is `net-reactor`. The host keeps this owner alive until every slot stops.
pub struct TransportService {
    incarnation: u64,
    registry: Arc<Mutex<Registered>>,
    bus: Arc<CommandBus>,
    ledger: Arc<ResourceLedger>,
    health: Arc<ReactorHealth>,
    loss_notified: AtomicBool,
    next_id: AtomicU64,
    shutdown: Arc<AtomicBool>,
    tls_lane: Arc<Mutex<Option<Arc<GameTlsLane>>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TransportService {
    /// One process service for Play and standalone clients alike. A failed
    /// startup stays failed instead of silently spawning a replacement.
    pub fn process_shared() -> io::Result<Arc<Self>> {
        match &*PROCESS_TRANSPORT {
            Ok(service) => Ok(Arc::clone(service)),
            Err(kind) => Err(io::Error::new(*kind, "game reactor startup failed")),
        }
    }

    pub fn start() -> io::Result<Arc<Self>> {
        let incarnation = NEXT_INCARNATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| io::Error::other("transport incarnation exhausted"))?;
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), WAKE_TOKEN)?);
        let registry = Arc::new(Mutex::new(Registered::new()));
        let bus = Arc::new(CommandBus::new(waker));
        let ledger = ResourceLedger::new();
        let health = Arc::new(ReactorHealth::new(Instant::now()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let tls_lane = Arc::new(Mutex::new(None));
        let reactor = Reactor::new(
            poll,
            Arc::clone(&registry),
            Arc::clone(&bus),
            Arc::clone(&ledger),
            Arc::clone(&health),
            Arc::clone(&shutdown),
            Arc::clone(&tls_lane),
        );
        let service = Arc::new(Self {
            incarnation,
            registry,
            bus,
            ledger,
            health: Arc::clone(&health),
            loss_notified: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            shutdown,
            tls_lane,
            worker: Mutex::new(None),
        });
        match thread::Builder::new()
            .name("net-reactor".to_owned())
            .spawn(move || {
                #[cfg(feature = "test-support")]
                super::soak_observer::observe_worker(
                    super::soak_observer::NetworkWorkerKind::Reactor,
                    super::soak_observer::WorkerObservationPoint::Started,
                );
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reactor.run()));
                if outcome.is_err() {
                    health.latch(ReactorLoss::Panicked);
                }
            }) {
            Ok(worker) => {
                *service
                    .worker
                    .lock()
                    .expect("reactor worker mutex is not poisoned") = Some(worker)
            }
            Err(_) => service.health.latch(ReactorLoss::StartFailed),
        }
        Ok(service)
    }

    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }

    pub fn bus(&self) -> Arc<CommandBus> {
        Arc::clone(&self.bus)
    }

    pub fn health(&self) -> Arc<ReactorHealth> {
        Arc::clone(&self.health)
    }
    /// Composition drives this independently of status consumers. The first
    /// service loss explicitly wakes every admitted slot after releasing the
    /// registration lock; physical charges stay with their actual owners.
    pub fn maintain(&self, now: Instant) -> Option<ReactorLoss> {
        let loss = self.health.maintain(now)?;
        if !self.loss_notified.swap(true, Ordering::AcqRel) {
            let parks = self
                .registry
                .lock()
                .expect("connection registry mutex is not poisoned")
                .entries
                .values()
                .map(|entry| Arc::clone(&entry.shared.control.park))
                .collect::<Vec<_>>();
            for park in parks {
                park.publish();
            }
        }
        Some(loss)
    }

    /// Install the fixed handshake owner before the host hook can admit TCP.
    pub fn install_tls_lane(&self, lane: Arc<GameTlsLane>) {
        let mut slot = self
            .tls_lane
            .lock()
            .expect("game TLS lane mutex is not poisoned");
        assert!(slot.is_none(), "the process has only one game TLS worker");
        *slot = Some(lane);
        drop(slot);
        let _ = self.bus.wake();
    }
    /// Reserve the immutable transfer sequence before the FIFO's physical
    /// WSS attempt starts; the handoff cell checks it again on submission.
    pub fn tls_transfer_seq(&self) -> io::Result<u64> {
        let lane = self
            .tls_lane
            .lock()
            .expect("game TLS lane mutex is not poisoned")
            .clone()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "game TLS worker unavailable")
            })?;
        lane.cell()
            .next_transfer_seq()
            .ok_or_else(|| io::Error::other("game TLS transfer sequence exhausted"))
    }

    /// Register a socket-free FIFO WSS demand before entering login-queue
    /// admission. The ticket retains exclusive lane eligibility through the
    /// actual reactor handoff, not merely through a capacity observation.
    pub fn game_tls_waiter(
        &self,
        park: Arc<Park>,
        deadline: Instant,
        token: CancelToken,
    ) -> io::Result<GameTlsTicket> {
        if self.shutdown.load(Ordering::Acquire) || self.health.loss().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "game TLS service unavailable",
            ));
        }
        let lane = self
            .tls_lane
            .lock()
            .expect("game TLS lane mutex is not poisoned")
            .clone()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "game TLS worker unavailable")
            })?;
        lane.waiter(park, deadline, token)
    }

    pub fn owns_tls_ticket(&self, ticket: &GameTlsTicket) -> bool {
        self.tls_lane
            .lock()
            .expect("game TLS lane mutex is not poisoned")
            .as_ref()
            .is_some_and(|lane| lane.owns_ticket(ticket))
    }

    /// Close new admission without waiting for an opaque platform verifier.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.bus.wake();
    }

    pub fn ledger(&self) -> Arc<ResourceLedger> {
        Arc::clone(&self.ledger)
    }

    /// Reservation allocates the fixed rings and both command/reply cells.
    /// A failed charge or exhausted ID creates no half-registered connection.
    pub fn reserve(
        &self,
        park: Arc<Park>,
        attempt_generation: u64,
    ) -> io::Result<(BindingId, Arc<SharedConnection>)> {
        if attempt_generation == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero attempt generation",
            ));
        }
        if self.health.loss().is_some() || self.shutdown.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "game reactor unavailable",
            ));
        }
        let known_bytes =
            SharedConnection::known_struct_bytes() + std::mem::size_of::<Registration>();
        let charge = self
            .ledger
            .admit_connection(known_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "game connection cap"))?;
        let mut registry = self
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned");
        for _ in 0..MAX_CONNECTIONS {
            let id = self
                .next_id
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .map_err(|_| io::Error::other("game connection identity exhausted"))?;
            if usize::try_from(id).is_err() {
                return Err(io::Error::other("game connection identity exhausted"));
            }
            let slot = index(id);
            if registry.entries.contains_key(&slot) {
                continue;
            }
            let shared = SharedConnection::new(id, park);
            let binding = BindingId {
                incarnation: self.incarnation,
                connection: id,
                binding: 1,
                attempt: attempt_generation,
            };
            registry.entries.insert(
                slot,
                Arc::new(Registration {
                    id,
                    initial: binding,
                    shared: Arc::clone(&shared),
                    submitted: AtomicBool::new(false),
                    charge: Mutex::new(Some(charge)),
                }),
            );
            return Ok((binding, shared));
        }
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "game connection slots exhausted",
        ))
    }

    /// Mint a new attempt on an already committed socket. No new connection,
    /// ring or descriptor is reserved; the reactor confirms the boundary
    /// before any new login byte can use the old physical connection.
    pub fn reserve_rebind(
        &self,
        expected: BindingId,
        attempt_generation: u64,
    ) -> io::Result<(BindingId, Arc<SharedConnection>)> {
        if expected.incarnation != self.incarnation
            || attempt_generation == 0
            || attempt_generation == expected.attempt
            || self.shutdown.load(Ordering::Acquire)
            || self.health.loss().is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "old binding unavailable",
            ));
        }
        let entry = self
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned")
            .get(expected.connection)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "old connection not registered")
            })?;
        if entry.shared.is_closed()
            || entry.shared.binding() != expected.binding
            || entry.shared.session_binding() != expected.binding
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "old session not committed",
            ));
        }
        let generation = expected
            .binding
            .checked_add(1)
            .ok_or_else(|| io::Error::other("binding generation exhausted"))?;
        Ok((
            BindingId {
                incarnation: self.incarnation,
                connection: expected.connection,
                binding: generation,
                attempt: attempt_generation,
            },
            Arc::clone(&entry.shared),
        ))
    }

    /// The service accepts each physical Connect exactly once. A failed
    /// notification leaves the command in reactor custody. The caller must
    /// not retry it or acknowledge its retirement cell. For pre-admission
    /// refusal the caller owns the retirement cell and the unused reservation.
    pub fn submit_connect(&self, command: ConnectCommand) -> Result<(), SubmitError> {
        if command.binding.incarnation != self.incarnation
            || self.health.loss().is_some()
            || self.shutdown.load(Ordering::Acquire)
        {
            return Err(SubmitError::Unavailable);
        }
        if command.binding.attempt != command.cancel.generation()
            || command.cancel.generation() != command.retire.generation()
            || match command.transport {
                Transport::Tcp => command.queue_release.is_some() || command.tls_ticket.is_some(),
                Transport::Wss => {
                    command.deadline.is_none()
                        || command.queue_release.as_ref().is_none_or(|proof| {
                            proof.identity() != command.binding || proof.transfer_seq() == 0
                        })
                        || command.tls_ticket.as_ref().is_none_or(|ticket| {
                            !ticket.reserved()
                                || !ticket.matches_token(&command.cancel)
                                || !self.owns_tls_ticket(ticket)
                                || command
                                    .deadline
                                    .is_none_or(|deadline| deadline > ticket.deadline())
                        })
                }
            }
        {
            return Err(SubmitError::StaleIdentity);
        }
        let registry = self
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned");
        let Some(registration) = registry.get(command.binding.connection) else {
            return Err(SubmitError::StaleIdentity);
        };
        if registration.initial != command.binding {
            return Err(SubmitError::StaleIdentity);
        }
        if registration.submitted.swap(true, Ordering::AcqRel) {
            return Err(SubmitError::PrimaryOccupied);
        }
        // Publish while registry removal is excluded. A simultaneous Close
        // rejects this exact pending Connect before it can drop custody.
        let result = registration
            .shared
            .control
            .post_primary(PrimaryCommand::Connect(command), &self.bus);
        if !matches!(&result, Ok(()) | Err(SubmitError::Wake(_))) {
            registration.submitted.store(false, Ordering::Release);
        }
        result
    }

    /// Submit one Rebind without allocating a second connection or bypassing
    /// the same primary reply cell used by Connect and Seal.
    pub fn submit_rebind(
        &self,
        expected: BindingId,
        next: BindingId,
        deadline: Instant,
        cancel: CancelToken,
        retire: Arc<AttemptRetire>,
    ) -> Result<(), SubmitError> {
        if self.health.loss().is_some() || self.shutdown.load(Ordering::Acquire) {
            return Err(SubmitError::Unavailable);
        }
        if next.incarnation != self.incarnation
            || next.connection != expected.connection
            || next.binding != expected.binding.checked_add(1).unwrap_or(0)
            || next.attempt != cancel.generation()
            || next.attempt != retire.generation()
            || cancel.is_cancelled()
            || Instant::now() >= deadline
        {
            return Err(SubmitError::StaleIdentity);
        }
        let entry = self
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned")
            .get(expected.connection)
            .ok_or(SubmitError::StaleIdentity)?;
        if entry.shared.binding() != expected.binding
            || entry.shared.session_binding() != expected.binding
            || entry.shared.is_closed()
        {
            return Err(SubmitError::StaleIdentity);
        }
        entry.shared.control.post_primary(
            PrimaryCommand::Rebind {
                expected,
                next,
                deadline,
                cancel,
                retire,
            },
            &self.bus,
        )
    }

    /// An unused reservation has no socket. Release it before abandoning the
    /// queue grant. A submitted Connect must instead finish through reactor
    /// close/retirement, even when its slot loses interest.
    pub fn abandon_unused(&self, binding: BindingId) -> bool {
        if binding.incarnation != self.incarnation {
            return false;
        }
        let registration = {
            let mut registry = self
                .registry
                .lock()
                .expect("connection registry mutex is not poisoned");
            let Some(registration) = registry.get(binding.connection) else {
                return false;
            };
            if registration.initial != binding
                || registration.submitted.swap(true, Ordering::AcqRel)
            {
                return false;
            }
            registry.remove(binding.connection)
        };
        if let Some(registration) = registration {
            registration.release_charge();
            registration.shared.publish_closed();
            true
        } else {
            false
        }
    }
}

impl Drop for TransportService {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.bus.wake();
        let worker = self
            .worker
            .lock()
            .expect("reactor worker mutex is not poisoned")
            .take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

struct RebindPending {
    expected: BindingId,
    next: BindingId,
    deadline: Instant,
    token: CancelToken,
    retire: Arc<AttemptRetire>,
}

/// While initial TLS holds the registered socket the registration retains
/// only its exact attempt metadata; the socket and capacity charge travel
/// together in GameTlsResource, never in two owners.
struct TlsFlight {
    host: String,
    port: u16,
    token: CancelToken,
    deadline: Instant,
    proof: Arc<QueueReleaseProof>,
    ticket: Option<GameTlsTicket>,
}

enum Endpoint {
    Tcp(TcpEndpoint),
    InitialTls(HandoffKey),
    Wss(WssEndpoint),
}

impl Endpoint {
    fn binding(&self) -> BindingId {
        match self {
            Self::Tcp(endpoint) => endpoint.binding(),
            Self::InitialTls(key) => key.identity.binding,
            Self::Wss(endpoint) => endpoint.binding(),
        }
    }

    fn resume_read(&mut self) {
        match self {
            Self::Tcp(endpoint) => endpoint.resume_read(),
            Self::Wss(endpoint) => endpoint.resume_read(),
            Self::InitialTls(_) => {}
        }
    }

    fn kick_output(&mut self) {
        match self {
            Self::Tcp(endpoint) => endpoint.kick_output(),
            Self::Wss(endpoint) => endpoint.kick_output(),
            Self::InitialTls(_) => {}
        }
    }

    fn attempt_failure(&self, now: Instant) -> Option<io::ErrorKind> {
        match self {
            Self::Tcp(endpoint) => endpoint.attempt_failure(now),
            Self::Wss(endpoint) => endpoint.attempt_failure(now),
            Self::InitialTls(_) => None,
        }
    }

    fn seal(&mut self, binding: BindingId, now: Instant) -> Result<Instant, RejectedCommand> {
        match self {
            Self::Tcp(endpoint) => endpoint.seal(binding, now),
            Self::Wss(endpoint) => endpoint.seal(binding, now),
            Self::InitialTls(_) => Err(RejectedCommand::WorkerOwned),
        }
    }

    fn flush_for_close(&mut self) -> io::Result<(bool, bool)> {
        match self {
            Self::Tcp(endpoint) => endpoint.flush_for_close(),
            Self::Wss(endpoint) => endpoint.flush_for_close(),
            Self::InitialTls(_) => Ok((true, false)),
        }
    }

    fn draining_for_close(&self) -> bool {
        matches!(self, Self::Wss(endpoint) if endpoint.draining_for_close())
    }

    fn drive(&mut self) -> io::Result<super::tcp::TcpTurnResult> {
        match self {
            Self::Tcp(endpoint) => endpoint.drive(),
            Self::Wss(endpoint) => endpoint.drive(),
            Self::InitialTls(_) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "initial TLS owns socket",
            )),
        }
    }

    fn drain_for_rebind(&mut self) -> io::Result<RebindBoundary> {
        match self {
            Self::Tcp(endpoint) => endpoint.drain_for_rebind(),
            Self::Wss(endpoint) => endpoint.drain_for_rebind(),
            Self::InitialTls(_) => Ok(RebindBoundary::WaitWritable),
        }
    }

    fn rebind(
        &mut self,
        expected: BindingId,
        next: BindingId,
        deadline: Instant,
        now: Instant,
        token: CancelToken,
        retire: Arc<AttemptRetire>,
    ) -> Result<(), RejectedCommand> {
        match self {
            Self::Tcp(endpoint) => endpoint.rebind(expected, next, deadline, now, token, retire),
            Self::Wss(endpoint) => endpoint.rebind(expected, next, deadline, now, token, retire),
            Self::InitialTls(_) => Err(RejectedCommand::WorkerOwned),
        }
    }

    fn close(self, registry: &mio::Registry, lane: Option<&GameTlsLane>, reason: RevocationReason) {
        match self {
            Self::Tcp(endpoint) => endpoint.close(registry),
            Self::Wss(endpoint) => endpoint.close(registry),
            Self::InitialTls(key) => {
                if let Some(lane) = lane {
                    let _ = lane.cell().revoke(key, reason);
                }
            }
        }
    }
}

struct Live {
    registration: Arc<Registration>,
    endpoint: Endpoint,
    connected_reply: bool,
    queued: bool,
    rebind: Option<RebindPending>,
    graceful_by: Option<Instant>,
    close_ack: Option<BindingId>,
    tls: Option<TlsFlight>,
}

#[derive(Clone, Copy)]
enum TurnOutcome {
    Wait,
    Local,
    Close,
}

enum EventStep {
    Local,
    Wait,
    Failed(io::Error),
}

enum ConnectProgress {
    Connected,
    Local,
    Wait,
    Failed(io::ErrorKind, Option<i32>),
}

struct Reactor {
    poll: Poll,
    events: Events,
    ready: Vec<(u64, bool, bool)>,
    registry: Arc<Mutex<Registered>>,
    bus: Arc<CommandBus>,
    ledger: Arc<ResourceLedger>,
    health: Arc<ReactorHealth>,
    shutdown: Arc<AtomicBool>,
    tls_lane: Arc<Mutex<Option<Arc<GameTlsLane>>>>,
    // Sparse modulo-slot storage keeps token lookup compatible with ID reuse.
    // This saves the empty 744-byte-per-slot table at small occupancies. Hash
    // lookups and rehash copies cost more than indexing; at roughly 449 live
    // connections bucket rounding can exceed the old table's retained bytes.
    live: HashMap<usize, Live>,
    active: Vec<u64>,
    local: VecDeque<u64>,
}

impl Reactor {
    fn new(
        poll: Poll,
        registry: Arc<Mutex<Registered>>,
        bus: Arc<CommandBus>,
        ledger: Arc<ResourceLedger>,
        health: Arc<ReactorHealth>,
        shutdown: Arc<AtomicBool>,
        tls_lane: Arc<Mutex<Option<Arc<GameTlsLane>>>>,
    ) -> Self {
        Self {
            poll,
            events: Events::with_capacity(MAX_CONNECTIONS + 1),
            // Reserve once: collecting a new readiness peak must not allocate.
            ready: Vec::with_capacity(MAX_CONNECTIONS),
            registry,
            bus,
            ledger,
            health,
            shutdown,
            tls_lane,
            live: HashMap::new(),
            active: Vec::new(),
            // Reserve once: I/O, deadlines and TLS returns enqueue on the hot path.
            local: VecDeque::with_capacity(MAX_CONNECTIONS),
        }
    }

    /// Advance one local connection step. TCP connect, initial TLS custody
    /// and the HTTP upgrade are distinct phases; only the final protocol-ready
    /// boundary may publish the positive Connected reply.
    fn advance_connect(
        live: &mut Live,
        registry: &mio::Registry,
        lane: Option<&GameTlsLane>,
    ) -> ConnectProgress {
        let now = Instant::now();
        match &mut live.endpoint {
            Endpoint::Tcp(endpoint) => match endpoint.begin_step(registry, now) {
                TcpStep::Retry => ConnectProgress::Local,
                TcpStep::Connecting => ConnectProgress::Wait,
                TcpStep::Failed(error) => {
                    ConnectProgress::Failed(error.kind(), error.raw_os_error())
                }
                TcpStep::Ready if live.tls.is_none() => ConnectProgress::Connected,
                TcpStep::Ready => {
                    let Some(lane) = lane else {
                        return ConnectProgress::Failed(io::ErrorKind::NotConnected, None);
                    };
                    if lane.config().is_none() {
                        return ConnectProgress::Failed(io::ErrorKind::InvalidData, None);
                    }
                    if !lane.cell().can_admit() {
                        return ConnectProgress::Failed(io::ErrorKind::WouldBlock, None);
                    }
                    let flight = live.tls.as_mut().expect("WSS has a TLS flight");
                    if flight
                        .ticket
                        .as_ref()
                        .is_none_or(|ticket| !ticket.reserved())
                    {
                        return ConnectProgress::Failed(io::ErrorKind::ConnectionAborted, None);
                    }
                    let key = HandoffKey {
                        identity: HandoffIdentity {
                            generation: flight.token.generation(),
                            binding: endpoint.binding(),
                        },
                        transfer_seq: flight.proof.transfer_seq(),
                    };
                    let old = std::mem::replace(&mut live.endpoint, Endpoint::InitialTls(key));
                    let Endpoint::Tcp(old) = old else {
                        unreachable!("connected TCP endpoint")
                    };
                    let (socket, retire) = old.take_registered();
                    let charge = live.registration.take_charge();
                    let resource =
                        GameTlsResource::new(socket, std::mem::take(&mut flight.host), charge);
                    match lane.cell().submit(
                        key,
                        Arc::clone(&flight.proof),
                        flight.token.clone(),
                        Arc::clone(&retire),
                        flight.deadline,
                        resource,
                    ) {
                        Ok(_) => {
                            // The socket was registered before the worker
                            // claim; seed its writable sequence without
                            // depending on a second selector edge.
                            drop(flight.ticket.take());
                            lane.cell().publish_ready(key, ReadyDirection::Write);
                            ConnectProgress::Wait
                        }
                        Err(rejected) => {
                            let mut resource = rejected.into_resource();
                            let _ = registry.deregister(&mut resource.socket);
                            drop(resource);
                            retire.ack();
                            drop(flight.ticket.take());
                            ConnectProgress::Failed(io::ErrorKind::WouldBlock, None)
                        }
                    }
                }
            },
            Endpoint::InitialTls(key) => {
                let Some(lane) = lane else {
                    return ConnectProgress::Failed(io::ErrorKind::NotConnected, None);
                };
                match lane.cell().take_return(*key) {
                    Err(TakeReturnError::NotReturned) => ConnectProgress::Wait,
                    Err(_) => ConnectProgress::Failed(io::ErrorKind::ConnectionAborted, None),
                    Ok(accepted) => {
                        let key = accepted.key;
                        match accepted.outcome {
                            WorkerOutcome::Failure(failure) => {
                                let _ = lane.cell().finish_accept(key);
                                live.registration.shared.publish_exit_reason(
                                    super::shared::StreamExitReason::Tls(format!("{failure:?}")),
                                );
                                let kind = match failure {
                                    WorkerFailure::Verification | WorkerFailure::Handshake => {
                                        io::ErrorKind::InvalidData
                                    }
                                    _ => io::ErrorKind::ConnectionAborted,
                                };
                                ConnectProgress::Failed(kind, None)
                            }
                            WorkerOutcome::Success(mut resource) => {
                                let Some(connection) = resource.connection.take() else {
                                    let _ = registry.deregister(&mut resource.socket);
                                    drop(resource);
                                    accepted.retirement.ack();
                                    let _ = lane.cell().finish_accept(key);
                                    return ConnectProgress::Failed(
                                        io::ErrorKind::InvalidData,
                                        None,
                                    );
                                };
                                live.registration.restore_charge(resource.charge.take());
                                let flight = live.tls.as_ref().expect("returned TLS flight");
                                let mut authority = resource.host;
                                if flight.port != 443 {
                                    authority.push(':');
                                    authority.push_str(&flight.port.to_string());
                                }
                                match WssEndpoint::new(
                                    key.identity.binding,
                                    &authority,
                                    resource.socket,
                                    connection,
                                    Arc::clone(&live.registration.shared),
                                    flight.token.clone(),
                                    Some(flight.deadline),
                                    accepted.retirement,
                                ) {
                                    Ok(mut endpoint) => {
                                        let interest = endpoint.apply_interest(registry);
                                        live.endpoint = Endpoint::Wss(endpoint);
                                        let finished = lane.cell().finish_accept(key);
                                        if interest.is_err() || finished.is_err() {
                                            ConnectProgress::Failed(
                                                io::ErrorKind::ConnectionAborted,
                                                None,
                                            )
                                        } else {
                                            ConnectProgress::Local
                                        }
                                    }
                                    Err(error) => {
                                        let _ = lane.cell().finish_accept(key);
                                        ConnectProgress::Failed(error.kind(), error.raw_os_error())
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Endpoint::Wss(endpoint) => {
                let step = endpoint.begin_step(now);
                if let Err(error) = endpoint.apply_interest(registry) {
                    return ConnectProgress::Failed(error.kind(), error.raw_os_error());
                }
                match step {
                    WssStep::Ready => ConnectProgress::Connected,
                    WssStep::Progress => ConnectProgress::Local,
                    WssStep::Wait => ConnectProgress::Wait,
                    WssStep::Failed(error) => {
                        ConnectProgress::Failed(error.kind(), error.raw_os_error())
                    }
                }
            }
        }
    }

    /// A worker completion wakes the shared Mio waker, not a per-connection
    /// event. Schedule its exact pending binding on the next reactor turn.
    fn enqueue_tls_return(&mut self) {
        let Some(lane) = self
            .tls_lane
            .lock()
            .expect("game TLS lane mutex is not poisoned")
            .clone()
        else {
            return;
        };
        lane.notify_capacity();
        if !matches!(
            lane.cell().phase(),
            HandoffPhase::Returned | HandoffPhase::Released
        ) {
            return;
        }
        for pos in 0..self.active.len() {
            let id = self.active[pos];
            if self.live.get(&index(id)).is_some_and(|live| {
                live.registration.id == id && matches!(&live.endpoint, Endpoint::InitialTls(_))
            }) {
                self.enqueue(id);
                break;
            }
        }
    }

    fn run(mut self) {
        while !self.shutdown.load(Ordering::Acquire) && self.health.loss().is_none() {
            self.health.progress(Instant::now());
            self.process_commands();
            self.scan_deadlines(Instant::now());
            self.enqueue_tls_return();
            self.drive_local();
            let timeout = if self.local.is_empty() {
                REACTOR_POLL_MAX
            } else {
                Duration::ZERO
            };
            self.health.progress(Instant::now());
            if let Err(error) = self.poll.poll(&mut self.events, Some(timeout)) {
                if error.kind() != io::ErrorKind::Interrupted {
                    self.health.latch(ReactorLoss::ProgressLost);
                }
                continue;
            }
            self.health.progress(Instant::now());
            self.ready.clear();
            for event in &self.events {
                if event.token() != WAKE_TOKEN {
                    if self.ready.len() == MAX_CONNECTIONS {
                        self.health.latch(ReactorLoss::ProgressLost);
                        break;
                    }
                    self.ready.push((
                        event.token().0 as u64,
                        event.is_readable() || event.is_read_closed(),
                        event.is_writable() || event.is_write_closed(),
                    ));
                }
            }
            for pos in 0..self.ready.len() {
                let (id, readable, writable) = self.ready[pos];
                let Some(live) = self.live.get_mut(&index(id)) else {
                    continue;
                };
                if live.registration.id != id {
                    continue;
                }
                let was_connected = live.connected_reply;
                let step = match &mut live.endpoint {
                    Endpoint::Tcp(endpoint) => match endpoint.on_event(
                        self.poll.registry(),
                        readable,
                        writable,
                        Instant::now(),
                    ) {
                        TcpStep::Retry | TcpStep::Ready => EventStep::Local,
                        TcpStep::Connecting => EventStep::Wait,
                        TcpStep::Failed(error) => EventStep::Failed(error),
                    },
                    Endpoint::InitialTls(key) => {
                        let lane = self
                            .tls_lane
                            .lock()
                            .expect("game TLS lane mutex is not poisoned")
                            .clone();
                        if let Some(lane) = lane {
                            if readable {
                                lane.cell().publish_ready(*key, ReadyDirection::Read);
                            }
                            if writable {
                                lane.cell().publish_ready(*key, ReadyDirection::Write);
                            }
                        }
                        EventStep::Wait
                    }
                    Endpoint::Wss(endpoint) => {
                        let step = endpoint.on_event(readable, writable, Instant::now());
                        match endpoint.apply_interest(self.poll.registry()) {
                            Err(error) => EventStep::Failed(error),
                            Ok(()) => match step {
                                WssStep::Ready | WssStep::Progress => EventStep::Local,
                                WssStep::Wait => EventStep::Wait,
                                WssStep::Failed(error) => EventStep::Failed(error),
                            },
                        }
                    }
                };
                match step {
                    EventStep::Local => self.enqueue(id),
                    EventStep::Wait => {}
                    EventStep::Failed(error) => {
                        let failure = (!was_connected).then(|| {
                            (
                                Arc::clone(&live.registration.shared),
                                live.endpoint.binding(),
                                error.kind(),
                                error.raw_os_error(),
                            )
                        });
                        self.close(id, None);
                        if let Some((shared, binding, kind, errno)) = failure {
                            shared.control.publish_primary(PrimaryReply::ConnectFailed {
                                binding,
                                kind,
                                errno,
                            });
                        }
                    }
                }
            }
            self.events.clear();
        }
    }

    fn process_commands(&mut self) {
        for _ in 0..MAX_CONNECTIONS {
            let Some(id) = self.bus.pop_id() else {
                break;
            };
            let Some(registration) = self
                .registry
                .lock()
                .expect("connection registry mutex is not poisoned")
                .get(id)
            else {
                continue;
            };
            let control = &registration.shared.control;
            self.bus.mark_taken(control);
            if let Some(close) = control.take_close() {
                if self.handle_close(id, close) {
                    if let Some(command) = control.take_primary() {
                        Self::reject_command(
                            &registration.shared,
                            command,
                            RejectedCommand::Closed,
                        );
                    }
                    continue;
                }
            }
            if let Some(command) = control.take_primary() {
                self.handle_primary(Arc::clone(&registration), command);
            }
            let resume = control.take_resume_read();
            let output = control.take_output_kick();
            let mut schedule = false;
            if let Some(live) = self
                .live
                .get_mut(&index(id))
                .filter(|live| live.registration.id == id)
            {
                if resume == live.endpoint.binding().binding && live.graceful_by.is_none() {
                    live.endpoint.resume_read();
                    schedule = true;
                }
                if output == live.endpoint.binding().binding {
                    live.endpoint.kick_output();
                    schedule = true;
                }
            }
            if schedule {
                let interest_failed = self
                    .live
                    .get_mut(&index(id))
                    .filter(|live| live.registration.id == id)
                    .and_then(|live| match &mut live.endpoint {
                        Endpoint::Wss(endpoint) => {
                            Some(endpoint.apply_interest(self.poll.registry()).is_err())
                        }
                        _ => None,
                    })
                    .unwrap_or(false);
                if interest_failed {
                    self.close(id, None);
                } else {
                    self.enqueue(id);
                }
            }
        }
    }

    fn handle_primary(&mut self, registration: Arc<Registration>, command: PrimaryCommand) {
        let id = registration.id;
        match command {
            PrimaryCommand::Connect(connect) => {
                if self.live.contains_key(&index(id)) || connect.binding.connection != id {
                    Self::reject_command(
                        &registration.shared,
                        PrimaryCommand::Connect(connect),
                        RejectedCommand::StaleIdentity,
                    );
                    return;
                }
                let ConnectCommand {
                    binding,
                    transport,
                    host,
                    port,
                    addresses,
                    cancel,
                    deadline,
                    retire,
                    queue_release,
                    tls_ticket,
                } = connect;
                let tls = match (transport, deadline, queue_release, tls_ticket) {
                    (Transport::Tcp, _, None, None) => None,
                    (Transport::Wss, Some(deadline), Some(proof), Some(ticket)) => {
                        Some(TlsFlight {
                            host,
                            port,
                            token: cancel.clone(),
                            deadline,
                            proof,
                            ticket: Some(ticket),
                        })
                    }
                    _ => {
                        retire.ack();
                        registration.release_charge();
                        registration.shared.publish_closed();
                        registration
                            .shared
                            .control
                            .publish_primary(PrimaryReply::Rejected {
                                binding,
                                reason: RejectedCommand::StaleIdentity,
                            });
                        let _ = self
                            .registry
                            .lock()
                            .expect("connection registry mutex is not poisoned")
                            .remove(id);
                        return;
                    }
                };
                match TcpEndpoint::new(
                    binding,
                    addresses,
                    Arc::clone(&registration.shared),
                    cancel,
                    deadline,
                    Arc::clone(&retire),
                ) {
                    Ok(endpoint) => {
                        self.active.push(id);
                        self.live.insert(
                            index(id),
                            Live {
                                registration,
                                endpoint: Endpoint::Tcp(endpoint),
                                connected_reply: false,
                                queued: false,
                                rebind: None,
                                graceful_by: None,
                                close_ack: None,
                                tls,
                            },
                        );
                        self.enqueue(id);
                    }
                    Err(error) => {
                        retire.ack();
                        registration.release_charge();
                        registration.shared.publish_closed();
                        registration
                            .shared
                            .control
                            .publish_primary(PrimaryReply::ConnectFailed {
                                binding,
                                kind: error.kind(),
                                errno: error.raw_os_error(),
                            });
                        let _ = self
                            .registry
                            .lock()
                            .expect("connection registry mutex is not poisoned")
                            .remove(id);
                    }
                }
            }
            PrimaryCommand::Seal { binding } => {
                let Some(live) = self
                    .live
                    .get_mut(&index(id))
                    .filter(|live| live.registration.id == id)
                else {
                    registration
                        .shared
                        .control
                        .publish_primary(PrimaryReply::Rejected {
                            binding,
                            reason: RejectedCommand::Closed,
                        });
                    return;
                };
                let reply = match live.endpoint.seal(binding, Instant::now()) {
                    Ok(confirmed_at) => PrimaryReply::Sealed {
                        binding,
                        confirmed_at,
                    },
                    Err(reason) => PrimaryReply::Rejected { binding, reason },
                };
                registration.shared.control.publish_primary(reply);
            }
            PrimaryCommand::Rebind {
                expected,
                next,
                deadline,
                cancel,
                retire,
            } => {
                let Some(live) = self
                    .live
                    .get_mut(&index(id))
                    .filter(|live| live.registration.id == id)
                else {
                    retire.ack();
                    registration
                        .shared
                        .control
                        .publish_primary(PrimaryReply::Rejected {
                            binding: next,
                            reason: RejectedCommand::Closed,
                        });
                    return;
                };
                if matches!(&live.endpoint, Endpoint::InitialTls(_)) {
                    retire.ack();
                    registration
                        .shared
                        .control
                        .publish_primary(PrimaryReply::Rejected {
                            binding: next,
                            reason: RejectedCommand::WorkerOwned,
                        });
                    return;
                }
                if !live.connected_reply {
                    retire.ack();
                    registration
                        .shared
                        .control
                        .publish_primary(PrimaryReply::Rejected {
                            binding: next,
                            reason: RejectedCommand::StaleIdentity,
                        });
                    return;
                }
                if expected != live.endpoint.binding() || live.rebind.is_some() {
                    retire.ack();
                    registration
                        .shared
                        .control
                        .publish_primary(PrimaryReply::Rejected {
                            binding: next,
                            reason: RejectedCommand::StaleIdentity,
                        });
                    return;
                }
                live.rebind = Some(RebindPending {
                    expected,
                    next,
                    deadline,
                    token: cancel,
                    retire,
                });
                self.enqueue(id);
            }
        }
    }

    fn handle_close(&mut self, id: u64, command: CloseCommand) -> bool {
        let Some(registration) = self
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned")
            .get(id)
        else {
            return false;
        };
        let current = self
            .live
            .get(&index(id))
            .filter(|live| live.registration.id == id)
            .is_some_and(|live| live.endpoint.binding() == command.binding);
        if !current && self.live.contains_key(&index(id)) {
            return false;
        }
        if let CloseReason::GracefulSession { by } = command.reason {
            if current {
                if let Some(live) = self.live.get_mut(&index(id)) {
                    live.graceful_by = Some(by);
                    live.close_ack = Some(command.binding);
                    registration.shared.disable_for_rebind();
                    live.endpoint.kick_output();
                    self.enqueue(id);
                    return true;
                }
            }
        }
        self.close(id, Some(command.binding));
        true
    }

    fn close(&mut self, id: u64, acknowledged_close: Option<BindingId>) {
        let mut close_ack = acknowledged_close;
        if let Some(pos) = self.active.iter().position(|&active| active == id) {
            self.active.swap_remove(pos);
        }
        let slot = index(id);
        let live = if self
            .live
            .get(&slot)
            .is_some_and(|live| live.registration.id == id)
        {
            self.live.remove(&slot)
        } else {
            None
        };
        if let Some(mut live) = live {
            if live.queued {
                self.local.retain(|queued| *queued != id);
                self.ledger.dequeue_local_work();
            }
            close_ack = close_ack.or(live.close_ack);
            if let Some(rebind) = live.rebind.take() {
                rebind.retire.ack();
                live.registration
                    .shared
                    .control
                    .publish_primary(PrimaryReply::Rejected {
                        binding: rebind.next,
                        reason: RejectedCommand::Closed,
                    });
            }
            let reason = if self.shutdown.load(Ordering::Acquire) {
                RevocationReason::Shutdown
            } else if live
                .tls
                .as_ref()
                .is_some_and(|flight| flight.token.is_cancelled())
            {
                RevocationReason::Cancelled
            } else if live
                .tls
                .as_ref()
                .is_some_and(|flight| Instant::now() >= flight.deadline)
            {
                RevocationReason::Deadline
            } else {
                RevocationReason::Cancelled
            };
            let lane = self
                .tls_lane
                .lock()
                .expect("game TLS lane mutex is not poisoned")
                .clone();
            live.endpoint
                .close(self.poll.registry(), lane.as_deref(), reason);
        } else {
            let registration = self
                .registry
                .lock()
                .expect("connection registry mutex is not poisoned")
                .get(id);
            if let Some(registration) = registration {
                registration.shared.publish_closed();
            }
        }
        let registration = self
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned")
            .remove(id);
        if let Some(registration) = registration {
            if let Some(command) = registration.shared.control.take_primary() {
                Self::reject_command(&registration.shared, command, RejectedCommand::Closed);
            }
            registration.release_charge();
            if let Some(binding) = close_ack {
                registration.shared.control.publish_closed(binding);
            }
        }
    }

    fn scan_deadlines(&mut self, now: Instant) {
        let mut pos = 0;
        while pos < self.active.len() {
            let id = self.active[pos];
            let live = self
                .live
                .get(&index(id))
                .filter(|live| live.registration.id == id)
                .expect("active ID has a connection");
            let attempt_failure = live.endpoint.attempt_failure(now).or_else(|| {
                live.tls.as_ref().and_then(|flight| {
                    if flight.token.is_cancelled() {
                        Some(io::ErrorKind::Interrupted)
                    } else if now >= flight.deadline {
                        Some(io::ErrorKind::TimedOut)
                    } else {
                        None
                    }
                })
            });
            if attempt_failure.is_none() && live.graceful_by.is_some_and(|by| now >= by) {
                self.health.record_graceful_close_timeout();
            }
            if attempt_failure.is_some() || live.graceful_by.is_some_and(|by| now >= by) {
                let failure = attempt_failure
                    .filter(|_| !live.connected_reply)
                    .map(|kind| {
                        (
                            Arc::clone(&live.registration.shared),
                            live.endpoint.binding(),
                            kind,
                        )
                    });
                self.close(id, None);
                if let Some((shared, binding, kind)) = failure {
                    shared.control.publish_primary(PrimaryReply::ConnectFailed {
                        binding,
                        kind,
                        errno: None,
                    });
                }
            } else {
                if live
                    .rebind
                    .as_ref()
                    .is_some_and(|rebind| rebind.token.is_cancelled() || now >= rebind.deadline)
                {
                    self.enqueue(id);
                }
                pos += 1;
            }
        }
    }

    fn drive_local(&mut self) {
        let round = self.local.len();
        for _ in 0..round {
            let Some(id) = self.local.pop_front() else {
                break;
            };
            self.ledger.dequeue_local_work();
            let Some(live) = self
                .live
                .get_mut(&index(id))
                .filter(|live| live.registration.id == id)
            else {
                continue;
            };
            live.queued = false;
            let mut failed_connect = None;
            let outcome = if !live.connected_reply {
                let lane = self
                    .tls_lane
                    .lock()
                    .expect("game TLS lane mutex is not poisoned")
                    .clone();
                match Self::advance_connect(live, self.poll.registry(), lane.as_deref()) {
                    ConnectProgress::Connected => {
                        live.connected_reply = true;
                        live.tls = None;
                        live.registration
                            .shared
                            .accept_binding(live.endpoint.binding());
                        live.registration
                            .shared
                            .control
                            .publish_primary(PrimaryReply::Connected {
                                binding: live.endpoint.binding(),
                            });
                        TurnOutcome::Local
                    }
                    ConnectProgress::Local => TurnOutcome::Local,
                    ConnectProgress::Wait => TurnOutcome::Wait,
                    ConnectProgress::Failed(kind, errno) => {
                        failed_connect = Some((
                            Arc::clone(&live.registration.shared),
                            live.endpoint.binding(),
                            kind,
                            errno,
                        ));
                        TurnOutcome::Close
                    }
                }
            } else if let Some(by) = live.graceful_by {
                if Instant::now() >= by {
                    self.health.record_graceful_close_timeout();
                    TurnOutcome::Close
                } else {
                    let was_draining = live.endpoint.draining_for_close();
                    let result = live.endpoint.flush_for_close();
                    if !was_draining && live.endpoint.draining_for_close() {
                        live.graceful_by = Some(Instant::now() + GRACEFUL_PEER_DRAIN);
                    }
                    match result {
                        Ok((true, _)) | Err(_) => TurnOutcome::Close,
                        Ok((false, true)) => TurnOutcome::Local,
                        Ok((false, false)) => TurnOutcome::Wait,
                    }
                }
            } else if live.rebind.is_some() {
                Self::drive_rebind(live)
            } else {
                match live.endpoint.drive() {
                    Ok(turn) if turn.stop == TcpStop::PeerClosed => TurnOutcome::Close,
                    Ok(turn) if turn.stop == TcpStop::LocalWork => TurnOutcome::Local,
                    Ok(_) => TurnOutcome::Wait,
                    Err(error) => {
                        // A real socket failure (a peer reset, a broken pipe)
                        // keeps its errno for the slot: published before the
                        // close wakes it, so login and session report the
                        // reset, not a bare end-of-stream.
                        if let Some(errno) = error.raw_os_error() {
                            live.registration.shared.publish_os_failure(errno);
                        }
                        TurnOutcome::Close
                    }
                }
            };
            let outcome = if let Endpoint::Wss(endpoint) = &mut live.endpoint {
                if endpoint.apply_interest(self.poll.registry()).is_err() {
                    TurnOutcome::Close
                } else {
                    outcome
                }
            } else {
                outcome
            };
            match outcome {
                TurnOutcome::Local => self.enqueue(id),
                TurnOutcome::Wait => {}
                TurnOutcome::Close => {
                    self.close(id, None);
                    if let Some((shared, binding, kind, errno)) = failed_connect {
                        shared.control.publish_primary(PrimaryReply::ConnectFailed {
                            binding,
                            kind,
                            errno,
                        });
                    }
                }
            }
            self.health.progress(Instant::now());
        }
    }

    fn drive_rebind(live: &mut Live) -> TurnOutcome {
        let pending = live.rebind.as_ref().expect("rebind command retained");
        let now = Instant::now();
        let failure = if pending.token.is_cancelled() {
            Some(RejectedCommand::Revoked)
        } else if now >= pending.deadline {
            Some(RejectedCommand::Deadline)
        } else {
            None
        };
        if let Some(reason) = failure {
            let pending = live.rebind.take().expect("rebind command retained");
            pending.retire.ack();
            live.registration
                .shared
                .control
                .publish_primary(PrimaryReply::Rejected {
                    binding: pending.next,
                    reason,
                });
            return TurnOutcome::Wait;
        }
        match live.endpoint.drain_for_rebind() {
            Ok(RebindBoundary::Pending) => TurnOutcome::Local,
            Ok(RebindBoundary::WaitWritable) => TurnOutcome::Wait,
            Ok(RebindBoundary::Clean) => {
                let pending = live.rebind.take().expect("rebind command retained");
                let reply = match live.endpoint.rebind(
                    pending.expected,
                    pending.next,
                    pending.deadline,
                    Instant::now(),
                    pending.token,
                    Arc::clone(&pending.retire),
                ) {
                    Ok(()) => PrimaryReply::Rebound {
                        binding: pending.next,
                    },
                    Err(reason) => {
                        pending.retire.ack();
                        PrimaryReply::Rejected {
                            binding: pending.next,
                            reason,
                        }
                    }
                };
                live.registration.shared.control.publish_primary(reply);
                TurnOutcome::Wait
            }
            Ok(RebindBoundary::PeerClosed) | Err(_) => TurnOutcome::Close,
        }
    }

    fn enqueue(&mut self, id: u64) {
        let Some(live) = self
            .live
            .get_mut(&index(id))
            .filter(|live| live.registration.id == id)
        else {
            return;
        };
        if live.queued {
            return;
        }
        if self.ledger.enqueue_local_work().is_err() {
            self.health.latch(ReactorLoss::ProgressLost);
            return;
        }
        live.queued = true;
        self.local.push_back(id);
    }

    fn reject_command(shared: &SharedConnection, command: PrimaryCommand, reason: RejectedCommand) {
        let binding = match command {
            PrimaryCommand::Connect(connect) => {
                connect.retire.ack();
                connect.binding
            }
            PrimaryCommand::Seal { binding } => binding,
            PrimaryCommand::Rebind { next, retire, .. } => {
                retire.ack();
                next
            }
        };
        shared
            .control
            .publish_primary(PrimaryReply::Rejected { binding, reason });
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        // A panicking reactor still owns its live sockets until this drop.
        // Return physical custody before publishing failure and releasing
        // charges; the owner health turn latches Panicked after unwind.
        for (slot, mut live) in self.live.drain() {
            let binding = live.endpoint.binding();
            let shared = Arc::clone(&live.registration.shared);
            let pending_connect = !live.connected_reply;
            let rebind = live.rebind.take();
            let close_ack = live.close_ack;
            let lane = self
                .tls_lane
                .lock()
                .expect("game TLS lane mutex is not poisoned")
                .clone();
            live.endpoint.close(
                self.poll.registry(),
                lane.as_deref(),
                RevocationReason::Shutdown,
            );
            let registration = {
                self.registry
                    .lock()
                    .expect("connection registry mutex is not poisoned")
                    .entries
                    .remove(&slot)
            };
            if let Some(registration) = registration {
                if let Some(command) = shared.control.take_primary() {
                    Self::reject_command(&shared, command, RejectedCommand::Unavailable);
                }
                registration.release_charge();
            }
            if let Some(rebind) = rebind {
                rebind.retire.ack();
                shared.control.publish_primary(PrimaryReply::Rejected {
                    binding: rebind.next,
                    reason: RejectedCommand::Unavailable,
                });
            } else if pending_connect && !shared.control.has_primary_reply() {
                shared.control.publish_primary(PrimaryReply::Rejected {
                    binding,
                    reason: RejectedCommand::Unavailable,
                });
            }
            let close_ack = close_ack.or_else(|| {
                shared
                    .control
                    .take_close()
                    .filter(|close| close.binding == binding)
                    .map(|close| close.binding)
            });
            if let Some(close_ack) = close_ack {
                if !shared.control.has_close_reply() {
                    shared.control.publish_closed(close_ack);
                }
            }
        }
        while self.local.pop_front().is_some() {
            self.ledger.dequeue_local_work();
        }
        let registrations = {
            let mut registry = self
                .registry
                .lock()
                .expect("connection registry mutex is not poisoned");
            std::mem::take(&mut registry.entries)
        };
        for registration in registrations.into_values() {
            if let Some(command) = registration.shared.control.take_primary() {
                Self::reject_command(&registration.shared, command, RejectedCommand::Unavailable);
            }
            registration.release_charge();
            registration.shared.publish_closed();
            if let Some(close) = registration.shared.control.take_close() {
                if close.binding == registration.initial
                    && !registration.shared.control.has_close_reply()
                {
                    registration.shared.control.publish_closed(close.binding);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::cancel::CancelSource;
    use super::super::park::ParkOutcome;
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::net::TcpListener;

    // macOS arm64 measured N=0 at 66,631 bytes, including the fixed 24 KiB
    // ready/local buffers. Windows measured 83,055 bytes: Mio's two event
    // buffers add 16,400 bytes over kqueue, plus 24 bytes of bootstrap state.
    // Account for that fixed difference, then allow 8 KiB bootstrap headroom.
    const WINDOWS_STARTUP_EXTRA_BYTES: usize = if cfg!(windows) { 16_424 } else { 0 };
    const STARTUP_ALLOCATION_LIMIT_BYTES: usize = 66_631 + WINDOWS_STARTUP_EXTRA_BYTES + 8 * 1024;
    // macOS arm64 measured N=4 at 222,179 bytes including those fixed buffers.
    // Windows has the same fixed startup increment. The 8 KiB headroom covers
    // startup variation, not per-bot growth; the admission delta is separate.
    const FOUR_ADMISSION_ALLOCATION_LIMIT_BYTES: usize =
        222_179 + WINDOWS_STARTUP_EXTRA_BYTES + 8 * 1024;
    // Measured admission delta is 38,887 bytes per connection. 40 KiB allows
    // 2,073 bytes of headroom, but rejects a new tens-of-KiB payload per bot.
    const PER_ADMISSION_ALLOCATION_LIMIT_BYTES: usize = 40 * 1024;
    const FIXED_HOT_PATH_STORAGE_BYTES: usize =
        MAX_CONNECTIONS * (std::mem::size_of::<(u64, bool, bool)>() + std::mem::size_of::<u64>());
    // 4 KiB leaves inline-layout headroom while rejecting a capacity-sized field embedded in Reactor.
    const INLINE_REACTOR_SIZE_LIMIT_BYTES: usize = 4 * 1024;
    // 16 KiB of sparse storage headroom, in addition to the fixed hot-path queues.
    const FOUR_CONNECTION_STORAGE_LIMIT_BYTES: usize = FIXED_HOT_PATH_STORAGE_BYTES + 16 * 1024;
    // Allow twice the four-entry occupancy for standard-container capacity rounding.
    const FOUR_CONNECTION_ENTRY_CAPACITY_LIMIT: usize = 8;

    struct ReactorTestAllocator;

    #[global_allocator]
    static REACTOR_TEST_ALLOCATOR: ReactorTestAllocator = ReactorTestAllocator;

    // Reactor construction and reservation allocate their owner state synchronously on this test thread.
    thread_local! {
        static TRACKED_ALLOCATIONS: Cell<(bool, usize)> = const { Cell::new((false, 0)) };
    }

    fn record_requested_allocation(bytes: usize) {
        let _ = TRACKED_ALLOCATIONS.try_with(|tracked| {
            let (enabled, total) = tracked.get();
            if enabled {
                tracked.set((enabled, total.saturating_add(bytes)));
            }
        });
    }

    // SAFETY: every operation forwards the caller's exact layout and pointer to System.
    unsafe impl GlobalAlloc for ReactorTestAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: `layout` is the allocator request supplied by the caller.
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null() {
                record_requested_allocation(layout.size());
            }
            pointer
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: `layout` is the allocator request supplied by the caller.
            let pointer = unsafe { System.alloc_zeroed(layout) };
            if !pointer.is_null() {
                record_requested_allocation(layout.size());
            }
            pointer
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: the caller supplied the allocation and original layout for this reallocation.
            let replacement = unsafe { System.realloc(pointer, layout, new_size) };
            if !replacement.is_null() {
                record_requested_allocation(new_size);
            }
            replacement
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: the caller supplied the allocation and matching layout.
            unsafe { System.dealloc(pointer, layout) };
        }
    }

    struct AllocationTracker;

    impl AllocationTracker {
        fn start() -> Self {
            TRACKED_ALLOCATIONS.with(|tracked| tracked.set((true, 0)));
            Self
        }

        fn requested_bytes(&self) -> usize {
            TRACKED_ALLOCATIONS.with(|tracked| tracked.get().1)
        }
    }

    impl Drop for AllocationTracker {
        fn drop(&mut self) {
            TRACKED_ALLOCATIONS.with(|tracked| tracked.set((false, 0)));
        }
    }

    fn estimated_reactor_storage_bytes(service: &TransportService, reactor: &Reactor) -> usize {
        let registration_capacity = service
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned")
            .entries
            .capacity();
        registration_capacity * std::mem::size_of::<(usize, Arc<Registration>)>()
            + reactor.live.capacity() * std::mem::size_of::<(usize, Live)>()
            + reactor.ready.capacity() * std::mem::size_of::<(u64, bool, bool)>()
            + reactor.active.capacity() * std::mem::size_of::<u64>()
            + reactor.local.capacity() * std::mem::size_of::<u64>()
    }

    fn submit_unconnected_tcp(
        service: &TransportService,
        park: Arc<Park>,
    ) -> (BindingId, Arc<SharedConnection>, Arc<AttemptRetire>) {
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let (binding, shared) = service.reserve(park, token.generation()).unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let mut addresses = super::super::resolver::ResolvedAddresses::new();
        assert!(addresses.push("127.0.0.1:9".parse().unwrap()));
        service
            .submit_connect(ConnectCommand {
                binding,
                transport: Transport::Tcp,
                host: String::new(),
                port: 9,
                addresses,
                deadline: Some(Instant::now() + Duration::from_secs(10)),
                cancel: token,
                retire: Arc::clone(&retire),
                queue_release: None,
                tls_ticket: None,
            })
            .unwrap();
        (binding, shared, retire)
    }

    #[test]
    fn startup_and_four_admissions_stay_within_allocation_tripwires() {
        let parks: [Arc<Park>; 4] = std::array::from_fn(|_| Arc::new(Park::new()));
        let tracker = AllocationTracker::start();
        let service = TransportService::start().unwrap();
        let startup_bytes = tracker.requested_bytes();
        assert!(
            startup_bytes <= STARTUP_ALLOCATION_LIMIT_BYTES,
            "empty transport startup requested {startup_bytes} bytes"
        );

        for park in parks {
            service.reserve(park, 1).unwrap();
        }
        let four_admissions_bytes = tracker.requested_bytes();
        assert!(
            four_admissions_bytes <= FOUR_ADMISSION_ALLOCATION_LIMIT_BYTES,
            "startup plus four admitted connections requested {four_admissions_bytes} bytes"
        );
        let admission_bytes = four_admissions_bytes - startup_bytes;
        assert!(
            admission_bytes <= 4 * PER_ADMISSION_ALLOCATION_LIMIT_BYTES,
            "four admissions requested {admission_bytes} bytes beyond startup"
        );
        let registry = service
            .registry
            .lock()
            .expect("connection registry mutex is not poisoned");
        assert_eq!(registry.entries.len(), 4);
        assert!(
            registry.entries.capacity() <= FOUR_CONNECTION_ENTRY_CAPACITY_LIMIT,
            "four registrations should reserve at most twice their occupied entries"
        );
        drop(registry);
        drop(tracker);
        println!(
            "REACTOR_ALLOCATION_RECEIPT n0={startup_bytes} n4={four_admissions_bytes} admission_total={admission_bytes}"
        );
        service.shutdown();
    }

    #[test]
    fn sparse_reactor_storage_handles_high_reused_ids_and_cleans_up() {
        let poll = Poll::new().unwrap();
        let registry = Arc::new(Mutex::new(Registered::new()));
        let bus = Arc::new(CommandBus::new(Arc::new(
            Waker::new(poll.registry(), WAKE_TOKEN).unwrap(),
        )));
        let ledger = ResourceLedger::new();
        let health = Arc::new(ReactorHealth::new(Instant::now()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let tls_lane = Arc::new(Mutex::new(None));
        let mut reactor = Reactor::new(
            poll,
            Arc::clone(&registry),
            Arc::clone(&bus),
            Arc::clone(&ledger),
            Arc::clone(&health),
            Arc::clone(&shutdown),
            Arc::clone(&tls_lane),
        );
        let service = TransportService {
            incarnation: 81,
            registry,
            bus,
            ledger,
            health,
            loss_notified: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            shutdown,
            tls_lane,
            worker: Mutex::new(None),
        };

        assert!(
            std::mem::size_of::<Reactor>() <= INLINE_REACTOR_SIZE_LIMIT_BYTES,
            "the reactor owner must remain small and must not embed capacity-sized slots"
        );
        assert_eq!(
            estimated_reactor_storage_bytes(&service, &reactor),
            FIXED_HOT_PATH_STORAGE_BYTES
        );
        assert_eq!(reactor.live.capacity(), 0);
        assert_eq!(reactor.ready.capacity(), MAX_CONNECTIONS);
        assert_eq!(reactor.active.capacity(), 0);
        assert_eq!(reactor.local.capacity(), MAX_CONNECTIONS);

        let high_id = (1_u64 << (usize::BITS - 2)) + 1;
        service.next_id.store(high_id, Ordering::Relaxed);
        let admitted: Vec<_> = (0..4)
            .map(|_| submit_unconnected_tcp(&service, Arc::new(Park::new())))
            .collect();
        reactor.process_commands();
        assert_eq!(reactor.live.len(), 4);
        assert_eq!(reactor.active.len(), 4);
        assert_eq!(reactor.local.len(), 4);
        assert!(reactor.ready.is_empty());
        assert!(
            reactor.live.capacity() <= FOUR_CONNECTION_ENTRY_CAPACITY_LIMIT,
            "four live endpoints should reserve at most twice their occupied entries"
        );
        assert!(
            reactor.active.capacity() <= FOUR_CONNECTION_ENTRY_CAPACITY_LIMIT,
            "active IDs should reserve at most twice their occupied entries"
        );
        assert!(
            estimated_reactor_storage_bytes(&service, &reactor)
                <= FOUR_CONNECTION_STORAGE_LIMIT_BYTES,
            "four admitted connections exceeded the estimated sparse reactor storage bound"
        );
        for (position, (binding, _, _)) in admitted.iter().enumerate() {
            assert_eq!(binding.connection, high_id + position as u64);
            assert!(reactor
                .live
                .get(&index(binding.connection))
                .is_some_and(|live| live.registration.id == binding.connection));
        }

        // Exercise the poll-loop buffers and actual enqueue path with growing
        // peaks after admission. Neither path may request an allocation.
        let tracker = AllocationTracker::start();
        for peak in [1, 2, 4] {
            reactor.ready.clear();
            while let Some(id) = reactor.local.pop_front() {
                reactor.ledger.dequeue_local_work();
                reactor.live.get_mut(&index(id)).unwrap().queued = false;
            }
            for (binding, _, _) in admitted.iter().take(peak) {
                reactor.ready.push((binding.connection, true, true));
                reactor.enqueue(binding.connection);
            }
            assert_eq!(reactor.ready.len(), peak);
            assert_eq!(reactor.local.len(), peak);
        }
        assert_eq!(tracker.requested_bytes(), 0);
        drop(tracker);

        for (binding, shared, retire) in admitted {
            reactor.close(binding.connection, None);
            assert!(shared.is_closed());
            assert_eq!(retire.ack_count(), 1);
        }
        assert!(reactor.live.is_empty());
        assert!(reactor.active.is_empty());
        assert!(reactor.local.is_empty());
        assert_eq!(service.ledger().snapshot().connections, 0);

        let reused_id = high_id + MAX_CONNECTIONS as u64;
        service.next_id.store(reused_id, Ordering::Relaxed);
        let (binding, shared, retire) = submit_unconnected_tcp(&service, Arc::new(Park::new()));
        assert_eq!(binding.connection, reused_id);
        assert_eq!(index(binding.connection), index(high_id));
        reactor.process_commands();
        assert_eq!(reactor.live.len(), 1);
        assert!(reactor
            .live
            .get(&index(reused_id))
            .is_some_and(|live| live.registration.id == reused_id));
        let close_by = Instant::now() + GRACEFUL_PEER_DRAIN;
        reactor.live.get_mut(&index(reused_id)).unwrap().graceful_by = Some(close_by);
        let just_before = close_by
            .checked_sub(Duration::from_nanos(1))
            .expect("deadline has an earlier instant");
        reactor.scan_deadlines(just_before);
        assert!(!shared.is_closed(), "peer-drain deadline has not passed");
        reactor.scan_deadlines(close_by);
        assert_eq!(service.health().graceful_close_timeouts(), 1);
        assert!(shared.is_closed());
        assert_eq!(retire.ack_count(), 1);
        assert!(reactor.live.is_empty());
        assert_eq!(service.ledger().snapshot().connections, 0);
    }

    #[test]
    fn unused_reservation_releases_connection_charge_without_a_socket() {
        let service = TransportService::start().unwrap();
        let (binding, shared) = service.reserve(Arc::new(Park::new()), 1).unwrap();
        assert_eq!(service.ledger().snapshot().connections, 1);
        assert!(service.abandon_unused(binding));
        assert!(shared.is_closed());
        assert!(!service.abandon_unused(binding));
        assert_eq!(service.ledger().snapshot().connections, 0);
    }

    #[test]
    fn close_before_connect_dispatch_rejects_and_retires_the_accepted_command() {
        let poll = Poll::new().unwrap();
        let registry = Arc::new(Mutex::new(Registered::new()));
        let bus = Arc::new(CommandBus::new(Arc::new(
            Waker::new(poll.registry(), WAKE_TOKEN).unwrap(),
        )));
        let ledger = ResourceLedger::new();
        let health = Arc::new(ReactorHealth::new(Instant::now()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let tls_lane = Arc::new(Mutex::new(None));
        let mut reactor = Reactor::new(
            poll,
            Arc::clone(&registry),
            Arc::clone(&bus),
            Arc::clone(&ledger),
            Arc::clone(&health),
            Arc::clone(&shutdown),
            Arc::clone(&tls_lane),
        );
        let service = TransportService {
            incarnation: 77,
            registry,
            bus,
            ledger,
            health,
            loss_notified: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            shutdown,
            tls_lane,
            worker: Mutex::new(None),
        };
        let park = Arc::new(Park::new());
        let token = CancelSource::new(Arc::clone(&park)).child().unwrap();
        let (binding, shared) = service.reserve(park, token.generation()).unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        shared
            .control
            .post_close(
                CloseCommand {
                    binding,
                    reason: CloseReason::Cancelled,
                },
                &service.bus(),
            )
            .unwrap();
        let mut addresses = super::super::resolver::ResolvedAddresses::new();
        assert!(addresses.push("127.0.0.1:9".parse().unwrap()));
        service
            .submit_connect(ConnectCommand {
                binding,
                transport: Transport::Tcp,
                host: String::new(),
                port: 9,
                addresses,
                deadline: Some(Instant::now() + Duration::from_secs(2)),
                cancel: token,
                retire: Arc::clone(&retire),
                queue_release: None,
                tls_ticket: None,
            })
            .unwrap();
        reactor.process_commands();
        assert_eq!(
            shared.control.take_primary_reply(),
            Some(PrimaryReply::Rejected {
                binding,
                reason: RejectedCommand::Closed,
            })
        );
        assert_eq!(shared.control.take_closed(), Some(binding));
        assert_eq!(retire.ack_count(), 1);
        assert_eq!(service.ledger().snapshot().connections, 0);
    }

    #[test]
    fn reactor_exit_releases_unconsumed_connect_and_its_charge() {
        let poll = Poll::new().unwrap();
        let registry = Arc::new(Mutex::new(Registered::new()));
        let bus = Arc::new(CommandBus::new(Arc::new(
            Waker::new(poll.registry(), WAKE_TOKEN).unwrap(),
        )));
        let ledger = ResourceLedger::new();
        let health = Arc::new(ReactorHealth::new(Instant::now()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let tls_lane = Arc::new(Mutex::new(None));
        let reactor = Reactor::new(
            poll,
            Arc::clone(&registry),
            Arc::clone(&bus),
            Arc::clone(&ledger),
            Arc::clone(&health),
            Arc::clone(&shutdown),
            Arc::clone(&tls_lane),
        );
        let service = TransportService {
            incarnation: 79,
            registry,
            bus,
            ledger,
            health,
            loss_notified: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            shutdown,
            tls_lane,
            worker: Mutex::new(None),
        };
        let park = Arc::new(Park::new());
        let token = CancelSource::new(Arc::clone(&park)).child().unwrap();
        let (binding, shared) = service.reserve(park, token.generation()).unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let mut addresses = super::super::resolver::ResolvedAddresses::new();
        assert!(addresses.push("127.0.0.1:9".parse().unwrap()));
        service
            .submit_connect(ConnectCommand {
                binding,
                transport: Transport::Tcp,
                host: String::new(),
                port: 9,
                addresses,
                deadline: Some(Instant::now() + Duration::from_secs(2)),
                cancel: token,
                retire: Arc::clone(&retire),
                queue_release: None,
                tls_ticket: None,
            })
            .unwrap();
        drop(reactor);
        assert_eq!(
            shared.control.take_primary_reply(),
            Some(PrimaryReply::Rejected {
                binding,
                reason: RejectedCommand::Unavailable,
            })
        );
        assert_eq!(retire.ack_count(), 1);
        assert!(shared.is_closed());
        assert_eq!(service.ledger().snapshot().connections, 0);
    }

    #[test]
    fn revoked_connect_retires_once_without_opening_a_socket() {
        let poll = Poll::new().unwrap();
        let registry = Arc::new(Mutex::new(Registered::new()));
        let bus = Arc::new(CommandBus::new(Arc::new(
            Waker::new(poll.registry(), WAKE_TOKEN).unwrap(),
        )));
        let ledger = ResourceLedger::new();
        let health = Arc::new(ReactorHealth::new(Instant::now()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let tls_lane = Arc::new(Mutex::new(None));
        let mut reactor = Reactor::new(
            poll,
            Arc::clone(&registry),
            Arc::clone(&bus),
            Arc::clone(&ledger),
            Arc::clone(&health),
            Arc::clone(&shutdown),
            Arc::clone(&tls_lane),
        );
        let service = TransportService {
            incarnation: 78,
            registry,
            bus,
            ledger,
            health,
            loss_notified: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            shutdown,
            tls_lane,
            worker: Mutex::new(None),
        };
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let (binding, shared) = service.reserve(park, token.generation()).unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let mut addresses = super::super::resolver::ResolvedAddresses::new();
        assert!(addresses.push("127.0.0.1:9".parse().unwrap()));
        service
            .submit_connect(ConnectCommand {
                binding,
                transport: Transport::Tcp,
                host: String::new(),
                port: 9,
                addresses,
                deadline: Some(Instant::now() + Duration::from_secs(2)),
                cancel: token,
                retire: Arc::clone(&retire),
                queue_release: None,
                tls_ticket: None,
            })
            .unwrap();
        source.cancel();
        reactor.process_commands();
        reactor.drive_local();
        assert_eq!(
            shared.control.take_primary_reply(),
            Some(PrimaryReply::ConnectFailed {
                binding,
                kind: io::ErrorKind::ConnectionAborted,
                errno: None,
            })
        );
        assert!(shared.is_closed());
        assert_eq!(retire.ack_count(), 1);
        assert_eq!(service.ledger().snapshot().connections, 0);
    }

    #[test]
    fn accepted_connect_and_close_retire_exactly_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let service = TransportService::start().unwrap();
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let (binding, shared) = service
            .reserve(Arc::clone(&park), token.generation())
            .unwrap();
        let retire = Arc::new(AttemptRetire::new(token.generation()));
        let mut addresses = super::super::resolver::ResolvedAddresses::new();
        assert!(addresses.push(listener.local_addr().unwrap()));
        let deadline = Instant::now() + Duration::from_secs(2);
        service
            .submit_connect(ConnectCommand {
                binding,
                transport: Transport::Tcp,
                host: String::new(),
                port: listener.local_addr().unwrap().port(),
                addresses,
                deadline: Some(deadline),
                cancel: token,
                retire: Arc::clone(&retire),
                queue_release: None,
                tls_ticket: None,
            })
            .unwrap();
        let connected = park.wait(
            deadline,
            Instant::now,
            || service.health().loss().is_some(),
            || false,
            || false,
            || shared.control.has_primary_reply().then_some(()),
            false,
            None,
        );
        assert_eq!(connected, ParkOutcome::Ready(()));
        assert_eq!(
            shared.control.take_primary_reply(),
            Some(PrimaryReply::Connected { binding })
        );
        assert!(!retire.acknowledged());
        shared
            .control
            .post_close(
                CloseCommand {
                    binding,
                    reason: CloseReason::Abort,
                },
                &service.bus(),
            )
            .unwrap();
        let closed = park.wait(
            deadline,
            Instant::now,
            || service.health().loss().is_some(),
            || false,
            || false,
            || shared.control.has_close_reply().then_some(()),
            false,
            None,
        );
        assert_eq!(closed, ParkOutcome::Ready(()));
        assert_eq!(shared.control.take_closed(), Some(binding));
        assert!(shared.is_closed());
        assert_eq!(retire.ack_count(), 1);
        assert_eq!(service.ledger().snapshot().connections, 0);
    }
}

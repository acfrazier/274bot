use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mio::net::TcpStream;
use mio::Waker;
use rustls::client::ClientConfig;
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, Error as TlsError};
use rustls_platform_verifier::BuilderVerifierExt;

use super::cancel::CancelToken;
use super::handoff::{HandoffCell, NativeCall, WorkerFailure, WorkerWork, WorkerYield};
use super::health::{ReactorHealth, ReactorLoss};
use super::park::Park;

const WORKER_NAME: &str = "net-tls-handshake";
const WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENCRYPTED_INPUT_PER_TURN: usize = 16 * 1024;
const MAX_ENCRYPTED_OUTPUT_PER_TURN: usize = 16 * 1024;
const MAX_OPERATIONS_PER_TURN: usize = 32;
const RUSTLS_BUFFER_LIMIT: usize = 64 * 1024;
const MAX_GAME_TLS_WAITERS: usize = 1024;
pub const MIN_WSS_START_REMAINING: Duration = Duration::from_secs(5);

const WAITING: u8 = 0;
const GRANTED: u8 = 1;
const EXPIRED: u8 = 2;
const CANCELLED: u8 = 3;
const INELIGIBLE: u8 = 4;

struct CapacityWaiter {
    id: u64,
    park: Arc<Park>,
    token: CancelToken,
    deadline: Instant,
    state: AtomicU8,
}

#[derive(Default)]
struct CapacityAdmission {
    next_id: u64,
    pending: VecDeque<Arc<CapacityWaiter>>,
    granted: Option<Arc<CapacityWaiter>>,
    closed: bool,
}

/// One exact WSS demand outside the login queue and outside socket custody.
/// A Ready ticket reserves the singleton lane until reactor submission or
/// explicit unused-grant release. Dropping it wakes the next FIFO waiter.
pub struct GameTlsTicket {
    lane: Arc<GameTlsLane>,
    waiter: Arc<CapacityWaiter>,
}

impl std::fmt::Debug for GameTlsTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameTlsTicket")
            .field("id", &self.waiter.id)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameTlsWait {
    Waiting,
    Ready,
    Deadline,
    Cancelled,
    InsufficientRemainingTime {
        remaining: Duration,
        minimum: Duration,
    },
}

impl GameTlsTicket {
    pub fn deadline(&self) -> Instant {
        self.waiter.deadline
    }

    pub fn poll(&self, now: Instant) -> GameTlsWait {
        if self.waiter.token.is_cancelled()
            || self.waiter.state.load(Ordering::Acquire) == CANCELLED
        {
            return GameTlsWait::Cancelled;
        }
        let state = self.waiter.state.load(Ordering::Acquire);
        if now >= self.waiter.deadline || state == EXPIRED {
            return GameTlsWait::Deadline;
        }
        if state == GRANTED {
            let remaining = self.waiter.deadline.duration_since(now);
            return if remaining < MIN_WSS_START_REMAINING {
                GameTlsWait::InsufficientRemainingTime {
                    remaining,
                    minimum: MIN_WSS_START_REMAINING,
                }
            } else {
                GameTlsWait::Ready
            };
        }
        if state == INELIGIBLE {
            return GameTlsWait::InsufficientRemainingTime {
                remaining: self.waiter.deadline.duration_since(now),
                minimum: MIN_WSS_START_REMAINING,
            };
        }
        GameTlsWait::Waiting
    }

    /// Called outside K2 after `poll` reports an ineligible grant. Keep this
    /// demand's original deadline but release exclusive lane eligibility, so
    /// later WSS demand can progress. `poll` remains an atomic-only predicate.
    pub fn release_for_insufficient_time(&self) {
        let mut admission = self.lane.admission.lock().unwrap();
        if admission
            .granted
            .as_ref()
            .is_some_and(|granted| Arc::ptr_eq(granted, &self.waiter))
        {
            admission.granted = None;
            self.waiter.state.store(INELIGIBLE, Ordering::Release);
        }
        drop(admission);
        self.lane.notify_capacity();
    }

    /// Post-K1 submission must not reapply the five-second *admission*
    /// floor: it only verifies that this exact grant remains owned.
    pub fn reserved(&self) -> bool {
        self.waiter.state.load(Ordering::Acquire) == GRANTED && !self.waiter.token.is_cancelled()
    }

    pub fn matches_token(&self, token: &CancelToken) -> bool {
        self.waiter.token.same_attempt(token)
    }
}

impl Drop for GameTlsTicket {
    fn drop(&mut self) {
        let mut admission = self.lane.admission.lock().unwrap();
        if admission
            .granted
            .as_ref()
            .is_some_and(|granted| Arc::ptr_eq(granted, &self.waiter))
        {
            admission.granted = None;
        } else {
            admission
                .pending
                .retain(|pending| !Arc::ptr_eq(pending, &self.waiter));
        }
        drop(admission);
        self.lane.notify_capacity();
    }
}

/// The original registered socket, its initial TLS state and its physical
/// capacity reservation move together while the handshake worker owns them.
///
/// Keep the socket first: automatic field destruction closes/deregisters it
/// before dropping the charge, so an opaque verifier can never release
/// physical capacity while the OS socket remains open.
pub struct GameTlsResource {
    pub socket: TcpStream,
    pub connection: Option<ClientConnection>,
    pub host: String,
    pub charge: Option<crate::ledger::ConnectionCharge>,
}

impl GameTlsResource {
    pub fn new(
        socket: TcpStream,
        host: String,
        charge: Option<crate::ledger::ConnectionCharge>,
    ) -> Self {
        Self {
            socket,
            connection: None,
            host,
            charge,
        }
    }
}

struct StartupConfig {
    result: Mutex<Option<Result<Arc<ClientConfig>, ()>>>,
    ready: Condvar,
}

impl StartupConfig {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn publish(&self, result: Result<Arc<ClientConfig>, ()>) {
        let mut current = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.is_none() {
            *current = Some(result);
            drop(current);
            self.ready.notify_all();
        }
    }

    fn wait(&self) -> Option<Arc<ClientConfig>> {
        let mut current = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while current.is_none() {
            current = self
                .ready
                .wait(current)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        current
            .as_ref()
            .expect("startup result is published")
            .as_ref()
            .ok()
            .cloned()
    }
}

/// One fixed worker lane for initial game TLS handshakes.
///
/// Admission has a separate bounded FIFO of slot-owned demand without
/// sockets; `HandoffCell` remains the sole authority for resource custody,
/// immutable transfer identity, readiness, cancellation and result publication.
pub struct GameTlsLane {
    cell: Arc<HandoffCell<GameTlsResource>>,
    startup: Arc<StartupConfig>,
    admission: Mutex<CapacityAdmission>,
}

impl GameTlsLane {
    /// Start the one bounded-stack handshake worker. The worker builds the
    /// platform-verifier client configuration before publishing the first
    /// startup result; callers can keep admitting plain TCP if that fails.
    pub fn start(reactor_waker: Arc<Waker>, health: Arc<ReactorHealth>) -> io::Result<Arc<Self>> {
        Self::start_with_builder(reactor_waker, health, build_platform_client_config)
    }

    /// Install a caller-created verifier configuration for isolated tests.
    #[cfg(feature = "test-support")]
    pub fn start_with_test_config(
        reactor_waker: Arc<Waker>,
        health: Arc<ReactorHealth>,
        config: Arc<ClientConfig>,
    ) -> io::Result<Arc<Self>> {
        Self::start_with_builder(reactor_waker, health, move || {
            let mut config = (*config).clone();
            config.resumption = rustls::client::Resumption::disabled();
            Ok(Arc::new(config))
        })
    }

    /// Construct a fresh platform-verifier generation on the actual worker
    /// with fixture roots, rather than prebuilding it on the test owner.
    #[cfg(feature = "test-support")]
    pub fn start_with_test_roots(
        reactor_waker: Arc<Waker>,
        health: Arc<ReactorHealth>,
        roots: Vec<rustls::pki_types::CertificateDer<'static>>,
    ) -> io::Result<Arc<Self>> {
        Self::start_with_builder(reactor_waker, health, move || {
            build_test_client_config_with_roots(&roots)
        })
    }

    fn start_with_builder<F>(
        reactor_waker: Arc<Waker>,
        health: Arc<ReactorHealth>,
        build_config: F,
    ) -> io::Result<Arc<Self>>
    where
        F: FnOnce() -> Result<Arc<ClientConfig>, TlsError> + Send + 'static,
    {
        let cell = Arc::new(HandoffCell::new(reactor_waker));
        let startup = Arc::new(StartupConfig::new());
        let worker_cell = Arc::clone(&cell);
        let worker_startup = Arc::clone(&startup);
        let panic_cell = Arc::clone(&worker_cell);
        let panic_startup = Arc::clone(&worker_startup);
        let worker = thread::Builder::new()
            .name(WORKER_NAME.to_owned())
            .stack_size(WORKER_STACK_BYTES)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let config = build_config().ok();
                    worker_startup.publish(config.as_ref().map(Arc::clone).ok_or(()));
                    handshake_worker(worker_cell, config);
                }));
                if result.is_err() {
                    health.latch(ReactorLoss::Panicked);
                    panic_startup.publish(Err(()));
                    let _ = panic_cell.stop_and_revoke_current();
                }
            })?;
        drop(worker);

        Ok(Arc::new(Self {
            cell,
            startup,
            admission: Mutex::new(CapacityAdmission {
                next_id: 1,
                ..CapacityAdmission::default()
            }),
        }))
    }

    /// Wait for the worker's first configuration result and clone the product
    /// configuration when available. A configuration failure is TLS-local.
    pub fn config(&self) -> Option<Arc<ClientConfig>> {
        self.startup.wait()
    }

    pub fn cell(&self) -> &HandoffCell<GameTlsResource> {
        &self.cell
    }

    /// Register one socket-free WSS demand. The ticket cannot skip an older
    /// eligible WSS waiter, and it retains the grant through the actual
    /// reactor-to-worker transfer. TLS setup failure is local to WSS.
    pub fn waiter(
        self: &Arc<Self>,
        park: Arc<Park>,
        deadline: Instant,
        token: CancelToken,
    ) -> io::Result<GameTlsTicket> {
        if self.config().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "game TLS verifier unavailable",
            ));
        }
        if !Arc::ptr_eq(&park, token.park()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WSS waiter park mismatch",
            ));
        }
        let waiter = {
            let mut admission = self.admission.lock().unwrap();
            if admission.closed {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "game TLS lane shut down",
                ));
            }
            if admission.pending.len() + usize::from(admission.granted.is_some())
                >= MAX_GAME_TLS_WAITERS
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "game TLS waiter capacity exhausted",
                ));
            }
            let id = admission.next_id;
            admission.next_id = id
                .checked_add(1)
                .ok_or_else(|| io::Error::other("game TLS waiter identity exhausted"))?;
            let waiter = Arc::new(CapacityWaiter {
                id,
                park,
                token,
                deadline,
                state: AtomicU8::new(WAITING),
            });
            admission.pending.push_back(Arc::clone(&waiter));
            waiter
        };
        self.notify_capacity();
        Ok(GameTlsTicket {
            lane: Arc::clone(self),
            waiter,
        })
    }

    /// Publish a wake after handoff acceptance or actual worker-owned close.
    /// Never hold the admission mutex while taking the handoff mutex or K2.
    pub fn notify_capacity(&self) {
        if self.admission.lock().unwrap().pending.is_empty() {
            return;
        }
        let available = self.cell.can_admit();
        let now = Instant::now();
        let mut wake = Vec::new();
        {
            let mut admission = self.admission.lock().unwrap();
            if admission.closed {
                return;
            }
            while admission.granted.is_none() {
                let Some(front) = admission.pending.front() else {
                    break;
                };
                let state = if front.token.is_cancelled() {
                    CANCELLED
                } else if now >= front.deadline {
                    EXPIRED
                } else if available {
                    GRANTED
                } else {
                    break;
                };
                let waiter = admission.pending.pop_front().expect("FIFO front");
                waiter.state.store(state, Ordering::Release);
                wake.push(Arc::clone(&waiter.park));
                if state == GRANTED {
                    admission.granted = Some(waiter);
                    break;
                }
            }
        }
        for park in wake {
            park.publish();
        }
    }

    pub fn owns_ticket(&self, ticket: &GameTlsTicket) -> bool {
        std::ptr::eq(self, Arc::as_ptr(&ticket.lane))
    }

    /// Stop new claims and revoke the exact active preaccept attempt. This
    /// publishes queue proof before waking either side and never joins the
    /// worker, which may be inside an opaque platform verifier.
    pub fn shutdown(&self) -> io::Result<()> {
        let revoked = self.cell.stop_and_revoke_current();
        let wake = {
            let mut admission = self.admission.lock().unwrap();
            admission.closed = true;
            let mut wake = Vec::with_capacity(
                admission.pending.len() + usize::from(admission.granted.is_some()),
            );
            for waiter in admission.pending.drain(..) {
                waiter.state.store(CANCELLED, Ordering::Release);
                wake.push(Arc::clone(&waiter.park));
            }
            if let Some(waiter) = admission.granted.take() {
                waiter.state.store(CANCELLED, Ordering::Release);
                wake.push(Arc::clone(&waiter.park));
            }
            wake
        };
        for park in wake {
            park.publish();
        }
        revoked.map(|_| ())
    }
}

impl Drop for GameTlsLane {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn build_platform_client_config() -> Result<Arc<ClientConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let versions = [&rustls::version::TLS13, &rustls::version::TLS12];
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&versions)?
        .with_platform_verifier()?
        .with_no_client_auth();
    Ok(Arc::new(apply_client_config_policy(config)))
}

#[cfg(feature = "test-support")]
pub fn build_test_client_config_with_verifier(
    verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
) -> Result<Arc<ClientConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let versions = [&rustls::version::TLS13, &rustls::version::TLS12];
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&versions)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(apply_client_config_policy(config)))
}

/// Augment (not replace) the OS verifier with test-only CA roots.
#[cfg(feature = "test-support")]
pub fn build_test_client_config_with_roots(
    roots: &[rustls::pki_types::CertificateDer<'static>],
) -> Result<Arc<ClientConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let versions = [&rustls::version::TLS13, &rustls::version::TLS12];
    let verifier = rustls_platform_verifier::Verifier::new_with_extra_roots(
        roots.iter().cloned(),
        Arc::clone(&provider),
    )?;
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&versions)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(apply_client_config_policy(config)))
}

fn apply_client_config_policy(mut config: ClientConfig) -> ClientConfig {
    config.enable_sni = true;
    config.resumption = rustls::client::Resumption::disabled();
    config
}

enum DriveResult {
    Complete,
    Failure(WorkerFailure),
    Requeued(Option<WorkerFailure>),
    Abandoned,
    Continue,
}

fn handshake_worker(cell: Arc<HandoffCell<GameTlsResource>>, config: Option<Arc<ClientConfig>>) {
    let mut deferred_failure = None;
    while let Some(mut work) = cell.worker_claim() {
        if let Some(failure) = deferred_failure.take() {
            let _ = cell.worker_failure(work, failure);
            continue;
        }

        let Some(config) = config.as_ref() else {
            let _ = cell.worker_failure(work, WorkerFailure::Verification);
            continue;
        };
        if let Err(failure) = initialize_connection(work.resource_mut(), config) {
            let _ = cell.worker_failure(work, failure);
            continue;
        }

        match drive_handshake(&cell, &mut work) {
            DriveResult::Complete => {
                let _ = cell.worker_success(work);
            }
            DriveResult::Failure(failure) => {
                let _ = cell.worker_failure(work, failure);
            }
            DriveResult::Requeued(failure) => {
                deferred_failure = failure;
                drop(work);
            }
            DriveResult::Abandoned => drop(work),
            DriveResult::Continue => unreachable!("drive loop always returns a terminal step"),
        }
    }
}

fn initialize_connection(
    resource: &mut GameTlsResource,
    config: &Arc<ClientConfig>,
) -> Result<(), WorkerFailure> {
    if resource.connection.is_some() {
        return Ok(());
    }

    let server_name =
        ServerName::try_from(resource.host.clone()).map_err(|_| WorkerFailure::Verification)?;
    let mut connection = ClientConnection::new(Arc::clone(config), server_name)
        .map_err(|error| classify_tls_error(&error))?;
    connection.set_buffer_limit(Some(RUSTLS_BUFFER_LIMIT));
    connection
        .writer()
        .write(&[])
        .map_err(|_| WorkerFailure::Handshake)?;
    resource.connection = Some(connection);
    Ok(())
}

fn drive_handshake(
    cell: &HandoffCell<GameTlsResource>,
    work: &mut WorkerWork<'_, GameTlsResource>,
) -> DriveResult {
    let mut input_bytes = 0;
    let mut output_bytes = 0;
    let mut operations = 0;

    loop {
        let (handshaking, wants_write) = {
            let connection = work
                .resource_mut()
                .connection
                .as_ref()
                .expect("worker initializes the TLS connection before driving it");
            (connection.is_handshaking(), connection.wants_write())
        };
        if !handshaking && !wants_write {
            // The Client Finished and every earlier TLS record have left the
            // rustls queue; no application-data read or write follows here.
            return DriveResult::Complete;
        }

        let next_operation_cost = if wants_write { 1 } else { 2 };
        if input_bytes >= MAX_ENCRYPTED_INPUT_PER_TURN
            || output_bytes >= MAX_ENCRYPTED_OUTPUT_PER_TURN
            || operations + next_operation_cost > MAX_OPERATIONS_PER_TURN
        {
            return from_worker_yield(cell.worker_yield(work, NativeCall::SyntheticBudget), None);
        }

        if wants_write {
            let remaining = MAX_ENCRYPTED_OUTPUT_PER_TURN - output_bytes;
            let result = {
                let resource = work.resource_mut();
                let connection = resource
                    .connection
                    .as_mut()
                    .expect("initialized TLS connection");
                let mut writer = BudgetWriter {
                    socket: &mut resource.socket,
                    remaining,
                };
                connection.write_tls(&mut writer)
            };
            operations += 1;
            if let Ok(bytes) = &result {
                output_bytes += *bytes;
            }
            let call = match &result {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => NativeCall::WouldBlock,
                _ => NativeCall::Progress,
            };
            let failure = match &result {
                Err(error)
                    if error.kind() != io::ErrorKind::WouldBlock
                        && error.kind() != io::ErrorKind::Interrupted =>
                {
                    Some(WorkerFailure::Io)
                }
                _ => None,
            };
            let yielded = from_worker_yield(cell.worker_yield(work, call), failure);
            match yielded {
                DriveResult::Continue => continue,
                step => return step,
            }
        }

        let remaining = MAX_ENCRYPTED_INPUT_PER_TURN - input_bytes;
        let (result, packet_failure, packet_operations) = {
            let resource = work.resource_mut();
            let connection = resource
                .connection
                .as_mut()
                .expect("initialized TLS connection");
            let mut reader = BudgetReader {
                socket: &mut resource.socket,
                remaining,
            };
            let result = connection.read_tls(&mut reader);
            let mut packet_failure = None;
            let mut packet_operations = 0;
            if matches!(&result, Ok(bytes) if *bytes > 0) {
                packet_operations += 1;
                if let Err(error) = connection.process_new_packets() {
                    packet_failure = Some(classify_tls_error(&error));
                }
            }
            (result, packet_failure, packet_operations)
        };
        operations += 1 + packet_operations;
        if let Ok(bytes) = &result {
            input_bytes += *bytes;
        }
        let call = match &result {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => NativeCall::WouldBlock,
            _ => NativeCall::Progress,
        };
        let mut failure = packet_failure;
        if failure.is_none() {
            failure = match &result {
                Ok(0) => Some(WorkerFailure::Handshake),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => None,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => None,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    Some(WorkerFailure::Handshake)
                }
                Err(_) => Some(WorkerFailure::Io),
                Ok(_) => None,
            };
        }
        let yielded = from_worker_yield(cell.worker_yield(work, call), failure);
        match yielded {
            DriveResult::Continue => {}
            step => return step,
        }
    }
}

fn from_worker_yield(result: WorkerYield, failure: Option<WorkerFailure>) -> DriveResult {
    match result {
        WorkerYield::Continue => failure.map_or(DriveResult::Continue, DriveResult::Failure),
        WorkerYield::Ready | WorkerYield::SyntheticBudget => DriveResult::Requeued(failure),
        WorkerYield::Revoked | WorkerYield::Stopped | WorkerYield::Stale => DriveResult::Abandoned,
    }
}

fn classify_tls_error(error: &TlsError) -> WorkerFailure {
    match error {
        TlsError::InvalidCertificate(_) => WorkerFailure::Verification,
        _ => WorkerFailure::Handshake,
    }
}

struct BudgetReader<'a> {
    socket: &'a mut TcpStream,
    remaining: usize,
}

impl Read for BudgetReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let limit = buffer.len().min(self.remaining);
        let bytes = self.socket.read(&mut buffer[..limit])?;
        self.remaining -= bytes;
        Ok(bytes)
    }
}

struct BudgetWriter<'a> {
    socket: &'a mut TcpStream,
    remaining: usize,
}

impl Write for BudgetWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 || buffer.is_empty() {
            return Ok(0);
        }
        let limit = buffer.len().min(self.remaining);
        let bytes = self.socket.write(&buffer[..limit])?;
        self.remaining -= bytes;
        Ok(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    use crate::cancel::CancelSource;
    use mio::{Poll, Token};

    fn lane_without_worker() -> Arc<GameTlsLane> {
        let poll = Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), Token(0)).unwrap());
        let startup = Arc::new(StartupConfig::new());
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
        startup.publish(Ok(Arc::new(config)));
        Arc::new(GameTlsLane {
            cell: Arc::new(HandoffCell::new(waker)),
            startup,
            admission: Mutex::new(CapacityAdmission {
                next_id: 1,
                ..CapacityAdmission::default()
            }),
        })
    }

    fn waiter(lane: &Arc<GameTlsLane>, deadline: Instant) -> (GameTlsTicket, CancelToken) {
        let park = Arc::new(Park::new());
        let token = CancelSource::new(Arc::clone(&park)).child().unwrap();
        let ticket = lane.waiter(park, deadline, token.clone()).unwrap();
        (ticket, token)
    }

    #[test]
    fn fifo_grant_is_exclusive_and_an_ineligible_waiter_releases_it() {
        let lane = lane_without_worker();
        let deadline = Instant::now() + Duration::from_secs(30);
        let (first, _) = waiter(&lane, deadline);
        let (second, _) = waiter(&lane, deadline);
        let (third, _) = waiter(&lane, deadline);
        assert_eq!(first.poll(Instant::now()), GameTlsWait::Ready);
        assert_eq!(second.poll(Instant::now()), GameTlsWait::Waiting);
        assert_eq!(third.poll(Instant::now()), GameTlsWait::Waiting);
        drop(first);
        assert_eq!(second.poll(Instant::now()), GameTlsWait::Ready);
        assert_eq!(third.poll(Instant::now()), GameTlsWait::Waiting);
        assert!(matches!(
            second.poll(deadline - Duration::from_secs(4)),
            GameTlsWait::InsufficientRemainingTime { .. },
        ));
        second.release_for_insufficient_time();
        assert_eq!(third.poll(Instant::now()), GameTlsWait::Ready);
        assert!(matches!(
            second.poll(deadline - Duration::from_secs(3)),
            GameTlsWait::InsufficientRemainingTime { .. },
        ));
        assert_eq!(second.poll(deadline), GameTlsWait::Deadline);
    }

    #[test]
    fn cancelled_front_cannot_hide_the_next_eligible_waiter() {
        let lane = lane_without_worker();
        let deadline = Instant::now() + Duration::from_secs(30);
        let (first, _) = waiter(&lane, deadline);
        let (cancelled, token) = waiter(&lane, deadline);
        let (third, _) = waiter(&lane, deadline);
        token.cancel();
        drop(first);
        assert_eq!(cancelled.poll(Instant::now()), GameTlsWait::Cancelled);
        assert_eq!(third.poll(Instant::now()), GameTlsWait::Ready);
    }
    #[test]
    fn uncaught_worker_panic_latches_process_health_before_startup_unblocks() {
        let poll = Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), Token(1)).unwrap());
        let health = Arc::new(ReactorHealth::new(Instant::now()));
        let lane = GameTlsLane::start_with_builder(waker, Arc::clone(&health), || {
            panic!("injected worker panic")
        })
        .unwrap();
        assert!(lane.config().is_none());
        assert_eq!(health.loss(), Some(ReactorLoss::Panicked));
    }
}

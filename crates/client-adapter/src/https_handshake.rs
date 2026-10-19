//! One process-scoped worker for HTTPS TCP connect and initial TLS handshake.
//!
//! The worker owns a socket only through initial certificate verification and
//! Finished flush. It publishes the established socket and rustls connection
//! into a never-reused fetch result cell. The fetch owner then owns all HTTP I/O.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection};
use client::io::client_stream::LoginCancel;
use crate::health::{ReactorHealth, ReactorLoss};

const WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;
const MAX_WAITING_FETCHES: usize = 1024;
const MAX_RESULT_CELLS: usize = 1024;
const MAX_ADDRESSES: usize = 64;
const MAX_HOST_BYTES: usize = 253;
const CONNECT_TRANSITION_RECHECK: Duration = Duration::from_millis(8);

#[derive(Clone, Copy)]
pub(crate) enum FetchClass {
    Stepped,
    BlockingLoginKey,
    Blocking,
}

#[derive(Clone, Copy)]
struct Limits {
    queue: Duration,
    connect: Duration,
    handshake: Duration,
}

impl FetchClass {
    fn limits(self) -> Limits {
        let phase = match self {
            Self::Stepped | Self::Blocking => Duration::from_secs(8),
            Self::BlockingLoginKey => Duration::from_secs(2),
        };
        Limits {
            queue: phase,
            connect: phase,
            handshake: phase,
        }
    }
}

/// The fixed process HTTPS handshake worker and its bounded fetch lane.
pub struct HttpsLane {
    inner: Arc<Inner>,
}

struct Inner {
    config: Option<Arc<ClientConfig>>,
    state: Mutex<State>,
    changed: Condvar,
}

struct State {
    accepting: bool,
    next_identity: u64,
    active: Option<u64>,
    queue: VecDeque<u64>,
    cells: HashMap<u64, Cell>,
}

struct Cell {
    owner_alive: bool,
    phase: Phase,
    phase_started: Instant,
    phase_deadline: Instant,
    queue_deadline: Instant,
    limits: Limits,
    address_count: usize,
    outcome: Option<Outcome>,
    job: Option<Job>,
}

#[derive(Clone, Copy)]
enum Phase {
    Queued,
    Connecting { address_index: usize },
    Handshaking,
}

enum Outcome {
    Success(EstablishedTls),
    Failure,
    Expired,
    Revoked,
}

struct Job {
    host: String,
    addresses: Vec<SocketAddr>,
}

/// The connection and socket published by the handshake worker.
pub(crate) struct EstablishedTls {
    pub(crate) socket: TcpStream,
    pub(crate) connection: ClientConnection,
}

/// A single owner's access to one never-reused HTTPS result cell.
pub(crate) struct HttpsTicket {
    inner: Arc<Inner>,
    identity: u64,
    armed: bool,
}

impl HttpsLane {
    /// Start the fixed `net-https-handshake` worker with a 2 MiB stack.
    pub fn start(
        config: Option<Arc<ClientConfig>>, health: Arc<ReactorHealth>,
    ) -> io::Result<Arc<Self>> {
        let inner = Arc::new(Inner {
            config,
            state: Mutex::new(State {
                accepting: true,
                next_identity: 1,
                active: None,
                queue: VecDeque::new(),
                cells: HashMap::new(),
            }),
            changed: Condvar::new(),
        });
        let worker_inner = Arc::clone(&inner);
        let panic_inner = Arc::clone(&inner);
        thread::Builder::new()
            .name("net-https-handshake".to_owned())
            .stack_size(WORKER_STACK_BYTES)
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                    worker_loop(worker_inner)
                )).is_err() {
                    health.latch(ReactorLoss::Panicked);
                    HttpsLane { inner: panic_inner }.shutdown();
                }
            })?;
        Ok(Arc::new(Self { inner }))
    }

    /// Close admission, revoke pending results, and wake every fetch owner.
    ///
    /// This does not join the worker. An OS verifier can remain blocked without
    /// holding up the caller of shutdown.
    pub fn shutdown(&self) {
        let mut close_after_unlock = Vec::new();
        let changed = {
            let mut state = lock_state(&self.inner.state);
            if !state.accepting {
                false
            } else {
                state.accepting = false;
                state.queue.clear();
                close_after_unlock.reserve(state.cells.len());
                for cell in state.cells.values_mut() {
                    drop(cell.job.take());
                    match cell.outcome.take() {
                        Some(Outcome::Success(established)) => {
                            close_after_unlock.push(established);
                            cell.outcome = Some(Outcome::Failure);
                        }
                        Some(outcome) => cell.outcome = Some(outcome),
                        None => cell.outcome = Some(Outcome::Failure),
                    }
                }
                true
            }
        };
        for established in close_after_unlock {
            close_established(established);
        }
        if changed {
            self.inner.changed.notify_all();
        }
    }

    pub(crate) fn begin_fetch(
        &self,
        host: &str,
        addresses: &[SocketAddr],
        class: FetchClass,
    ) -> Result<HttpsTicket, ()> {
        self.begin_fetch_with_limits(host, addresses, class.limits())
    }

    fn begin_fetch_with_limits(
        &self,
        host: &str,
        addresses: &[SocketAddr],
        limits: Limits,
    ) -> Result<HttpsTicket, ()> {
        if host.is_empty()
            || host.len() > MAX_HOST_BYTES
            || addresses.is_empty()
            || addresses.len() > MAX_ADDRESSES
        {
            return Err(());
        }

        let mut owned_host = String::new();
        owned_host.try_reserve_exact(host.len()).map_err(|_| ())?;
        owned_host.push_str(host);
        let mut owned_addresses = Vec::new();
        owned_addresses
            .try_reserve_exact(addresses.len())
            .map_err(|_| ())?;
        owned_addresses.extend_from_slice(addresses);

        let now = Instant::now();
        let queue_deadline = now.checked_add(limits.queue).ok_or(())?;
        let mut state = lock_state(&self.inner.state);
        if !state.accepting
            || state.queue.len() >= MAX_WAITING_FETCHES
            || state.cells.len() >= MAX_RESULT_CELLS
        {
            return Err(());
        }
        state.queue.try_reserve(1).map_err(|_| ())?;
        state.cells.try_reserve(1).map_err(|_| ())?;
        let identity = state.next_identity;
        state.next_identity = identity.checked_add(1).ok_or(())?;

        let has_config = self.inner.config.is_some();
        let mut cell = Cell {
            owner_alive: true,
            phase: Phase::Queued,
            phase_started: now,
            phase_deadline: queue_deadline,
            queue_deadline,
            limits,
            address_count: owned_addresses.len(),
            outcome: None,
            job: Some(Job {
                host: owned_host,
                addresses: owned_addresses,
            }),
        };
        if has_config {
            state.queue.push_back(identity);
        } else {
            drop(cell.job.take());
            cell.outcome = Some(Outcome::Failure);
        }
        drop(state.cells.insert(identity, cell));
        drop(state);
        self.inner.changed.notify_all();

        Ok(HttpsTicket {
            inner: Arc::clone(&self.inner),
            identity,
            armed: true,
        })
    }
}

impl Drop for HttpsLane {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl HttpsTicket {
    /// Register a one-shot owner-stop callback after releasing the lane lock.
    pub(crate) fn register_cancel_callback(&self, cancel: &LoginCancel) {
        let inner = Arc::downgrade(&self.inner);
        let identity = self.identity;
        cancel.register_cancel_callback(move || {
            if let Some(inner) = inner.upgrade() {
                cancel_identity(&inner, identity);
            }
        });
    }

    /// Return a published connection, or observe that this fetch is still live.
    pub(crate) fn try_take(&mut self) -> Result<Option<EstablishedTls>, ()> {
        let now = Instant::now();
        let mut notify = false;
        let result = {
            let mut state = lock_state(&self.inner.state);
            let Some(cell) = state.cells.get(&self.identity) else {
                self.armed = false;
                return Err(());
            };

            let should_expire = cell.outcome.is_none() && phase_expired(cell, now);
            if should_expire {
                if let Some(cell) = state.cells.get_mut(&self.identity) {
                    cell.outcome = Some(Outcome::Expired);
                    cell.owner_alive = false;
                    drop(cell.job.take());
                }
                state.queue.retain(|queued| *queued != self.identity);
                if state.active != Some(self.identity) {
                    drop(state.cells.remove(&self.identity));
                }
                notify = true;
                self.armed = false;
                Err(())
            } else {
                let active = state.active == Some(self.identity);
                let Some(cell) = state.cells.get_mut(&self.identity) else {
                    self.armed = false;
                    return Err(());
                };
                match cell.outcome.take() {
                    Some(Outcome::Success(established)) => {
                        cell.owner_alive = false;
                        drop(state.cells.remove(&self.identity));
                        self.armed = false;
                        Ok(Some(established))
                    }
                    Some(Outcome::Failure | Outcome::Expired | Outcome::Revoked) => {
                        cell.owner_alive = false;
                        if !active {
                            drop(state.cells.remove(&self.identity));
                        }
                        self.armed = false;
                        Err(())
                    }
                    None => Ok(None),
                }
            }
        };
        if notify {
            self.inner.changed.notify_all();
        }
        result
    }

    /// Wait for a result or phase update until `until`.
    ///
    /// A terminal result published before its phase deadline remains valid even
    /// when this owner observes it after `until`.
    pub(crate) fn wait_step(
        &mut self,
        until: Instant,
    ) -> Result<Option<EstablishedTls>, ()> {
        loop {
            if let Some(established) = self.try_take()? {
                return Ok(Some(established));
            }
            let now = Instant::now();
            if now >= until {
                return Ok(None);
            }
            let state = lock_state(&self.inner.state);
            let Some(cell) = state.cells.get(&self.identity) else {
                self.armed = false;
                return Err(());
            };
            if cell.outcome.is_some() || phase_expired(cell, now) {
                drop(state);
                continue;
            }
            let phase_wait = cell.phase_deadline.saturating_duration_since(now);
            let phase_wait = if phase_wait.is_zero()
                && is_nonfinal_connect(cell)
            {
                CONNECT_TRANSITION_RECHECK
            } else {
                phase_wait
            };
            let wait = phase_wait.min(until.saturating_duration_since(now));
            if wait.is_zero() {
                drop(state);
                continue;
            }
            let (state, _) = self
                .inner
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(state);
        }
    }

    /// Wait for a result, with an optional caller-owned absolute cap.
    pub(crate) fn wait_for_result(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<EstablishedTls, ()> {
        loop {
            if let Some(established) = self.try_take()? {
                return Ok(established);
            }
            let now = Instant::now();
            if deadline.is_some_and(|limit| now >= limit) {
                return Err(());
            }
            let state = lock_state(&self.inner.state);
            let Some(cell) = state.cells.get(&self.identity) else {
                self.armed = false;
                return Err(());
            };
            if cell.outcome.is_some() || phase_expired(cell, now) {
                drop(state);
                continue;
            }
            let phase_wait = cell.phase_deadline.saturating_duration_since(now);
            let phase_wait = if phase_wait.is_zero() && is_nonfinal_connect(cell) {
                CONNECT_TRANSITION_RECHECK
            } else {
                phase_wait
            };
            let wait = deadline.map_or(phase_wait, |limit| {
                phase_wait.min(limit.saturating_duration_since(now))
            });
            if wait.is_zero() {
                drop(state);
                continue;
            }
            let (state, _) = self
                .inner
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(state);
        }
    }
}

impl Drop for HttpsTicket {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut close_after_unlock = None;
        {
            let mut state = lock_state(&self.inner.state);
            let active = state.active == Some(self.identity);
            if let Some(cell) = state.cells.get_mut(&self.identity) {
                cell.owner_alive = false;
                drop(cell.job.take());
                if active {
                    if cell.outcome.is_none() {
                        cell.outcome = Some(Outcome::Revoked);
                    }
                } else {
                    if let Some(Outcome::Success(established)) = cell.outcome.take() {
                        close_after_unlock = Some(established);
                    }
                    state.queue.retain(|queued| *queued != self.identity);
                    drop(state.cells.remove(&self.identity));
                }
            }
        }
        if let Some(established) = close_after_unlock {
            close_established(established);
        }
        self.inner.changed.notify_all();
    }
}

fn cancel_identity(inner: &Inner, identity: u64) {
    let mut close_after_unlock = None;
    {
        let mut state = lock_state(&inner.state);
        let active = state.active == Some(identity);
        if let Some(cell) = state.cells.get_mut(&identity) {
            if cell.owner_alive {
                drop(cell.job.take());
                match cell.outcome.take() {
                    Some(Outcome::Success(established)) => {
                        close_after_unlock = Some(established);
                        cell.outcome = Some(Outcome::Failure);
                    }
                    Some(outcome) => cell.outcome = Some(outcome),
                    None => cell.outcome = Some(Outcome::Failure),
                }
            }
        }
        state.queue.retain(|queued| *queued != identity);
        if !active {
            let remove = state
                .cells
                .get(&identity)
                .is_some_and(|cell| !cell.owner_alive);
            if remove {
                drop(state.cells.remove(&identity));
            }
        }
        inner.changed.notify_all();
    }
    if let Some(established) = close_after_unlock {
        close_established(established);
    }
}

fn phase_expired(cell: &Cell, now: Instant) -> bool {
    if now < cell.phase_started || now < cell.phase_deadline {
        return false;
    }
    match cell.phase {
        Phase::Connecting { address_index } => address_index + 1 >= cell.address_count,
        Phase::Queued | Phase::Handshaking => true,
    }
}

fn is_nonfinal_connect(cell: &Cell) -> bool {
    matches!(
        cell.phase,
        Phase::Connecting { address_index } if address_index + 1 < cell.address_count
    )
}

fn worker_loop(inner: Arc<Inner>) {
    while let Some(work) = next_work(&inner) {
        process_job(&inner, work.identity, work.job, work.connect_deadline);
    }
}

struct Work {
    identity: u64,
    job: Job,
    connect_deadline: Instant,
}

fn next_work(inner: &Inner) -> Option<Work> {
    loop {
        let mut state = lock_state(&inner.state);
        if !state.accepting {
            return None;
        }

        let mut notify = false;
        let mut selected = None;
        while let Some(identity) = state.queue.pop_front() {
            let now = Instant::now();
            let mut remove = false;
            let mut expired = false;
            let mut job = None;
            let mut connect_deadline = None;
            if let Some(cell) = state.cells.get_mut(&identity) {
                if !cell.owner_alive || cell.outcome.is_some() {
                    remove = !cell.owner_alive;
                } else if now >= cell.queue_deadline {
                    cell.outcome = Some(Outcome::Expired);
                    drop(cell.job.take());
                    expired = true;
                    notify = true;
                } else if let Some(queued_job) = cell.job.take() {
                    cell.phase = Phase::Connecting { address_index: 0 };
                    cell.phase_started = now;
                    let deadline = now.checked_add(cell.limits.connect).unwrap_or(now);
                    cell.phase_deadline = deadline;
                    connect_deadline = Some(deadline);
                    job = Some(queued_job);
                } else {
                    remove = true;
                }
            } else {
                remove = true;
            }
            if remove {
                drop(state.cells.remove(&identity));
                continue;
            }
            if expired {
                continue;
            }
            if let (Some(job), Some(connect_deadline)) = (job, connect_deadline) {
                state.active = Some(identity);
                selected = Some(Work {
                    identity,
                    job,
                    connect_deadline,
                });
                break;
            }
        }

        if let Some(work) = selected {
            drop(state);
            inner.changed.notify_all();
            return Some(work);
        }
        if notify {
            drop(state);
            inner.changed.notify_all();
            continue;
        }

        let next_deadline = state
            .queue
            .front()
            .and_then(|identity| state.cells.get(identity))
            .map(|cell| cell.queue_deadline);
        if let Some(deadline) = next_deadline {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                drop(state);
                continue;
            }
            let (state, _) = inner
                .changed
                .wait_timeout(state, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(state);
        } else {
            let state = inner
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            drop(state);
        }
    }
}

fn process_job(inner: &Inner, identity: u64, job: Job, first_deadline: Instant) {
    let config = match &inner.config {
        Some(config) => Arc::clone(config),
        None => {
            finish_active(inner, identity, false);
            return;
        }
    };
    let server_name = match ServerName::try_from(job.host.clone()) {
        Ok(name) => name,
        Err(_) => {
            finish_active(inner, identity, false);
            return;
        }
    };

    let mut connect_deadline = first_deadline;
    for (address_index, address) in job.addresses.iter().copied().enumerate() {
        if address_index > 0 {
            let Some(deadline) = begin_connect_phase(inner, identity, address_index) else {
                finish_active(inner, identity, false);
                return;
            };
            connect_deadline = deadline;
        }
        let remaining = connect_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if address_index + 1 == job.addresses.len() {
                finish_active(inner, identity, false);
                return;
            }
            continue;
        }
        let socket = match TcpStream::connect_timeout(&address, remaining) {
            Ok(socket) => socket,
            Err(_) => {
                if address_index + 1 == job.addresses.len() {
                    finish_active(inner, identity, false);
                    return;
                }
                continue;
            }
        };
        let connected_at = Instant::now();
        if connected_at >= connect_deadline {
            close_socket(socket);
            if address_index + 1 == job.addresses.len() {
                finish_active(inner, identity, false);
                return;
            }
            continue;
        }
        let handshake_deadline = match begin_handshake_phase(inner, identity, connected_at) {
            Some(deadline) => deadline,
            None => {
                close_socket(socket);
                finish_active(inner, identity, false);
                return;
            }
        };
        let mut connection = match ClientConnection::new(Arc::clone(&config), server_name.clone()) {
            Ok(connection) => connection,
            Err(_) => {
                close_socket(socket);
                finish_active(inner, identity, false);
                return;
            }
        };
        connection.set_buffer_limit(Some(64 * 1024));
        match perform_handshake(
            inner,
            identity,
            &socket,
            &mut connection,
            connected_at,
            handshake_deadline,
        ) {
            HandshakeResult::Complete => {
                publish_success(
                    inner,
                    identity,
                    EstablishedTls { socket, connection },
                    handshake_deadline,
                );
            }
            HandshakeResult::Failed | HandshakeResult::Expired | HandshakeResult::Revoked => {
                drop(connection);
                close_socket(socket);
                finish_active(inner, identity, false);
            }
        }
        return;
    }

    finish_active(inner, identity, false);
}

fn begin_connect_phase(inner: &Inner, identity: u64, address_index: usize) -> Option<Instant> {
    let now = Instant::now();
    let deadline = {
        let mut state = lock_state(&inner.state);
        if state.active != Some(identity) || !state.accepting {
            return None;
        }
        let cell = state.cells.get_mut(&identity)?;
        if !cell.owner_alive || cell.outcome.is_some() {
            return None;
        }
        let deadline = now.checked_add(cell.limits.connect).unwrap_or(now);
        cell.phase = Phase::Connecting { address_index };
        cell.phase_started = now;
        cell.phase_deadline = deadline;
        deadline
    };
    inner.changed.notify_all();
    Some(deadline)
}

fn begin_handshake_phase(inner: &Inner, identity: u64, started: Instant) -> Option<Instant> {
    let mut state = lock_state(&inner.state);
    if state.active != Some(identity) || !state.accepting {
        return None;
    }
    let cell = state.cells.get_mut(&identity)?;
    if !cell.owner_alive || cell.outcome.is_some() {
        return None;
    }
    let deadline = started.checked_add(cell.limits.handshake).unwrap_or(started);
    cell.phase = Phase::Handshaking;
    cell.phase_started = started;
    cell.phase_deadline = deadline;
    drop(state);
    inner.changed.notify_all();
    Some(deadline)
}

enum HandshakeResult {
    Complete,
    Failed,
    Expired,
    Revoked,
}

fn perform_handshake(
    inner: &Inner,
    identity: u64,
    socket: &TcpStream,
    connection: &mut ClientConnection,
    started: Instant,
    deadline: Instant,
) -> HandshakeResult {
    let quiet_limit = {
        let state = lock_state(&inner.state);
        let Some(cell) = state.cells.get(&identity) else {
            return HandshakeResult::Revoked;
        };
        cell.limits.handshake
    };
    let mut quiet_deadline = started.checked_add(quiet_limit).unwrap_or(started);

    loop {
        if !worker_still_owns(inner, identity) {
            return HandshakeResult::Revoked;
        }
        let now = Instant::now();
        if now >= deadline || now >= quiet_deadline {
            return HandshakeResult::Expired;
        }
        let operation_deadline = deadline.min(quiet_deadline);
        let timeout = operation_deadline.saturating_duration_since(now);
        if timeout.is_zero() {
            return HandshakeResult::Expired;
        }

        if connection.wants_write() {
            if socket.set_write_timeout(Some(timeout)).is_err() {
                return HandshakeResult::Failed;
            }
            match connection.write_tls(&mut &*socket) {
                Ok(0) => return HandshakeResult::Failed,
                Ok(_) => {
                    let progressed = Instant::now();
                    quiet_deadline = progressed.checked_add(quiet_limit).unwrap_or(progressed);
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if Instant::now() >= operation_deadline {
                        return HandshakeResult::Expired;
                    }
                    continue;
                }
                Err(_) => return HandshakeResult::Failed,
            }
        }

        if !connection.is_handshaking() {
            return HandshakeResult::Complete;
        }
        if !connection.wants_read() {
            return HandshakeResult::Failed;
        }
        if socket.set_read_timeout(Some(timeout)).is_err() {
            return HandshakeResult::Failed;
        }
        match connection.read_tls(&mut &*socket) {
            Ok(0) => return HandshakeResult::Failed,
            Ok(_) => {
                let progressed = Instant::now();
                quiet_deadline = progressed.checked_add(quiet_limit).unwrap_or(progressed);
                if !worker_still_owns(inner, identity) {
                    return HandshakeResult::Revoked;
                }
                if connection.process_new_packets().is_err() {
                    return HandshakeResult::Failed;
                }
                if Instant::now() >= deadline {
                    return HandshakeResult::Expired;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= operation_deadline {
                    return HandshakeResult::Expired;
                }
            }
            Err(_) => return HandshakeResult::Failed,
        }
    }
}

fn worker_still_owns(inner: &Inner, identity: u64) -> bool {
    let state = lock_state(&inner.state);
    state.accepting
        && state.active == Some(identity)
        && state
            .cells
            .get(&identity)
            .is_some_and(|cell| cell.owner_alive && cell.outcome.is_none())
}

fn publish_success(
    inner: &Inner,
    identity: u64,
    established: EstablishedTls,
    handshake_deadline: Instant,
) {
    let mut established = Some(established);
    let published = {
        let now = Instant::now();
        let mut state = lock_state(&inner.state);
        let publish = if state.accepting && state.active == Some(identity) {
            let mut publish = false;
            if let Some(cell) = state.cells.get_mut(&identity) {
                if cell.owner_alive
                    && cell.outcome.is_none()
                    && now < handshake_deadline
                    && now < cell.phase_deadline
                {
                    if let Some(established) = established.take() {
                        cell.outcome = Some(Outcome::Success(established));
                        publish = true;
                    }
                }
            }
            publish
        } else {
            false
        };
        if publish {
            state.active = None;
        }
        publish
    };
    if published {
        inner.changed.notify_all();
    } else {
        if let Some(established) = established {
            close_established(established);
        }
        finish_active(inner, identity, false);
    }
}

fn finish_active(inner: &Inner, identity: u64, success: bool) {
    let notify = {
        let now = Instant::now();
        let mut state = lock_state(&inner.state);
        if state.active != Some(identity) {
            return;
        }
        if let Some(cell) = state.cells.get_mut(&identity) {
            if cell.outcome.is_none() {
                cell.outcome = Some(if success {
                    Outcome::Failure
                } else if !cell.owner_alive {
                    Outcome::Revoked
                } else if now >= cell.phase_deadline {
                    Outcome::Expired
                } else {
                    Outcome::Failure
                });
            }
            if !cell.owner_alive {
                drop(state.cells.remove(&identity));
            }
        }
        state.active = None;
        true
    };
    if notify {
        inner.changed.notify_all();
    }
}

fn close_established(established: EstablishedTls) {
    drop(established.connection);
    close_socket(established.socket);
}

fn close_socket(socket: TcpStream) {
    let _ = socket.shutdown(Shutdown::Both);
    drop(socket);
}

fn lock_state(mutex: &Mutex<State>) -> MutexGuard<'_, State> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
pub(crate) fn test_client_config() -> Arc<ClientConfig> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};

    let root = CertificateDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/ca.pem"
    ))
    .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

#[cfg(test)]
fn test_server_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

    let certificate = CertificateDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.pem"
    ))
    .unwrap();
    let key = PrivateKeyDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.key"
    ))
    .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    Arc::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    fn server_handshake(socket: &mut TcpStream, config: Arc<rustls::ServerConfig>) {
        let mut connection = rustls::ServerConnection::new(config).unwrap();
        while connection.is_handshaking() {
            connection.complete_io(socket).unwrap();
        }
    }

    fn wait_for_publication(ticket: &HttpsTicket) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = lock_state(&ticket.inner.state);
        loop {
            let cell = state
                .cells
                .get(&ticket.identity)
                .expect("ticket cell remains until owner takes result");
            if matches!(cell.outcome.as_ref(), Some(Outcome::Success(_))) {
                return;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "TLS result was not published");
            let (next_state, wait) = ticket
                .inner
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(!wait.timed_out(), "TLS result was not published");
            state = next_state;
        }
    }

    fn test_limits(queue: Duration, connect: Duration, handshake: Duration) -> Limits {
        Limits {
            queue,
            connect,
            handshake,
        }
    }

    #[test]
    fn published_result_frees_lane_before_owner_take() {
        let lane = HttpsLane::start(Some(test_client_config()), Arc::new(ReactorHealth::new(Instant::now()))).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = test_server_config();
        let (ready_tx, ready_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                server_handshake(&mut socket, Arc::clone(&server_config));
                ready_tx.send(()).unwrap();
            }
        });

        let mut first = lane
            .begin_fetch("localhost", &[address], FetchClass::Stepped)
            .unwrap();
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut second = lane
            .begin_fetch("localhost", &[address], FetchClass::Stepped)
            .unwrap();
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(first.try_take().unwrap().is_some());
        assert!(second.try_take().unwrap().is_some());
        server.join().unwrap();
        lane.shutdown();
    }

    #[test]
    fn owner_drop_closes_a_published_socket() {
        let lane = HttpsLane::start(Some(test_client_config()), Arc::new(ReactorHealth::new(Instant::now()))).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = test_server_config();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            server_handshake(&mut socket, server_config);
            let mut byte = [0u8; 1];
            let result = socket.read(&mut byte);
            assert!(matches!(result, Ok(0) | Err(_)));
        });

        let ticket = lane
            .begin_fetch("localhost", &[address], FetchClass::Stepped)
            .unwrap();
        wait_for_publication(&ticket);
        drop(ticket);
        server.join().unwrap();
        lane.shutdown();
    }

    #[test]
    fn queue_deadline_expires_without_waiting_for_active_handshake() {
        let lane = HttpsLane::start(Some(test_client_config()), Arc::new(ReactorHealth::new(Instant::now()))).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            accepted_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
            let mut byte = [0u8; 1];
            let _ = socket.read(&mut byte);
        });

        let active = lane
            .begin_fetch_with_limits(
                "localhost",
                &[address],
                test_limits(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    Duration::from_millis(150),
                ),
            )
            .unwrap();
        accepted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut queued = lane
            .begin_fetch_with_limits(
                "localhost",
                &[address],
                test_limits(
                    Duration::from_millis(40),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
            )
            .unwrap();
        assert!(queued
            .wait_step(Instant::now() + Duration::from_millis(200))
            .is_err());
        drop(active);
        drop(queued);
        release_tx.send(()).unwrap();
        server.join().unwrap();
        lane.shutdown();
    }

    #[test]
    fn missing_config_is_a_local_failure_and_does_not_start_a_plain_request() {
        let lane = HttpsLane::start(None, Arc::new(ReactorHealth::new(Instant::now()))).unwrap();
        let address = SocketAddr::from(([127, 0, 0, 1], 9));
        let mut ticket = lane
            .begin_fetch("localhost", &[address], FetchClass::Stepped)
            .unwrap();
        assert!(ticket.try_take().is_err());
        lane.shutdown();
    }
}

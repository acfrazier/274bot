use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Deadline, Event, Scope, SessionKey, SlotKey, SlotKeyMinter};
use api::lifecycle::{
    AbortedAttempt, HandshakeKind, HoldReason, Intent, IntentCommand, IntentMailbox, LifecycleFact,
    LoginIntent, LoginLatch, MemoryMode, ParkReason, PostReceipt, QueueBlocker, SessionEndCause,
    SessionEventOut, SessionGate, TransportFailure, WorldChoice as IntentWorldChoice,
};
use client::client::{Client, LoginError, LoginStage, LoginTransportError};
use client::config::IfType;
use client::io::client_stream::LoginCancel;
use client::io::host_network::{HostAttemptIdentity, SealReceipt};
use client_adapter::cancel::{AttemptRetire, CancelSource, CancelToken};
#[cfg(test)]
use client_adapter::control::BindingId;
use client_adapter::handoff::QueueReleaseProof;
use client_adapter::health::ReactorHealth;
use client_adapter::host_network::{with_prepared_game_attempt, PreparedGameAttempt};
use client_adapter::park::{Park, ParkOutcome};
use client_adapter::tls_handshake::{GameTlsTicket, GameTlsWait, MIN_WSS_START_REMAINING};
use host::login_queue::{
    AdmitBlocker, LoginBackoff, LoginQueue, Permit, QueueAttemptId, QueuePos, HANDSHAKE_HOLD,
};
use parking_lot::Mutex as QueueMutex;
use vault::{Profile, RandomEventPolicy};

use super::{clear_startup_progress, lock_statuses, public_worlds, Play, SlotStatus, StartupPhase};

pub(super) type SharedLoginQueue = Arc<QueueMutex<LoginQueue>>;
static DIRECT_LOGIN_QUEUE: LazyLock<SharedLoginQueue> =
    LazyLock::new(|| Arc::new(QueueMutex::new(LoginQueue::default())));

fn direct_login_queue() -> SharedLoginQueue {
    Arc::clone(&*DIRECT_LOGIN_QUEUE)
}

struct DirectPrequeueCancelGuard<'a> {
    arm: &'a SlotArm,
    token: CancelToken,
}

impl Drop for DirectPrequeueCancelGuard<'_> {
    fn drop(&mut self) {
        self.arm.clear_prequeue_cancel(&self.token);
    }
}

/// Persistent host-login authority for one caller-owned [`Client`].
///
/// Construct one context per client and retain it across relogs. Each call to
/// [`Self::login`] makes at most one client login attempt and returns server
/// refusals unchanged; callers own any retry policy. The composition must
/// start and maintain [`client_adapter::service::NetworkService`] first.
///
/// Invoke `login` only after the client's previous session has disconnected.
/// The method consumes any pending client session-exit fact before admission,
/// then applies the same clean-release pacing as a production slot. Dropping
/// this context removes its FIFO owner but does not log out or close the
/// caller-owned client.
pub struct DirectClientLogin {
    arm: Arc<SlotArm>,
    queue: SharedLoginQueue,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    backoff: LoginBackoff,
}

impl DirectClientLogin {
    /// Create the persistent queue/session context for one client.
    pub fn new(client: &Client) -> Self {
        Self {
            arm: SlotArm::new(client.login_uid, true),
            queue: direct_login_queue(),
            statuses: Arc::new(Mutex::new(Vec::new())),
            backoff: LoginBackoff::new(),
        }
    }

    /// Perform one host-admitted client login.
    ///
    /// The returned [`LoginError`] is the original typed client/server error
    /// when the handshake reached the game server. This method never retries
    /// a server refusal; a later explicit call uses the same slot identity,
    /// queue budgets, clean-release state, and backoff.
    pub fn login(
        &mut self,
        client: &mut Client,
        username: &str,
        password: &str,
        reconnect: bool,
    ) -> Result<(), LoginError> {
        if client.ingame {
            return Err(direct_login_error(
                LoginStage::Connect,
                io::ErrorKind::InvalidInput,
                "client is still in game; disconnect its session before login",
            ));
        }

        observe_client_session_exit(client, &self.arm);
        self.arm.sync_login_identity(client.login_uid, password);
        let Some((command, world_generation)) =
            capture_login_authorization(&self.arm, client.ingame)
        else {
            return Err(direct_login_error(
                LoginStage::Connect,
                io::ErrorKind::Interrupted,
                "login is not authorized by the current slot intent",
            ));
        };
        let uid = self.arm.uid.load(Ordering::Acquire);
        let token = self.arm.new_cancel_token().ok_or_else(|| {
            direct_login_error(
                LoginStage::Connect,
                io::ErrorKind::Other,
                "login cancellation generation exhausted",
            )
        })?;
        if !self
            .arm
            .publish_prequeue_cancel(token.clone(), command, world_generation)
        {
            return Err(direct_login_error(
                LoginStage::Connect,
                io::ErrorKind::Interrupted,
                "login was withdrawn before transport preparation",
            ));
        }
        let _prequeue_cancel = DirectPrequeueCancelGuard {
            arm: &self.arm,
            token: token.clone(),
        };
        let _queue_place = QueuePlaceRetirement {
            queue: &self.queue,
            statuses: &self.statuses,
            username,
            arm: &self.arm,
        };

        let prepared = super::play_slots::prepare_game_transport(client, &self.arm, &token)
            .map_err(|error| {
                direct_login_error(
                    LoginStage::Connect,
                    io::ErrorKind::Other,
                    format!("game transport preparation failed: {error}"),
                )
            })?;
        if token.is_cancelled() || !self.arm.login_attempt_is_current(command, world_generation) {
            return Err(direct_login_error(
                LoginStage::Connect,
                io::ErrorKind::Interrupted,
                "login was cancelled during transport preparation",
            ));
        }
        self.arm.prepared();

        let mut tls_deadline = None;
        let mut wss_health = None;
        let wait = if prepared.needs_initial_tls() {
            let network_health = prepared.network_health();
            let deadline = Instant::now() + HANDSHAKE_HOLD;
            tls_deadline = Some(deadline);
            wss_health = Some(Arc::clone(&network_health));
            let ticket = prepared
                .game_tls_waiter(self.arm.park(), deadline)
                .map_err(|error| {
                    direct_login_error(
                        LoginStage::Connect,
                        error.kind(),
                        format!("game TLS capacity: {error}"),
                    )
                })?;
            let (host, port) = prepared.endpoint();
            wait_for_wss_permit(
                &self.queue,
                &self.statuses,
                username,
                uid,
                &self.arm,
                command,
                world_generation,
                token.clone(),
                ticket,
                network_health,
                || {
                    self.arm
                        .clean_release_admission_at(host, port, Instant::now())
                },
                &mut || Duration::from_millis(20),
            )
        } else {
            enqueue_queue_place(&self.queue, &self.statuses, username, uid, &self.arm);
            if let Some(until) = self.arm.clean_release_admission_at(
                &client.config.host,
                client.config.port,
                Instant::now(),
            ) {
                self.queue
                    .lock()
                    .defer_owner_until(self.arm.queue_owner, until);
            }
            wait_for_permit_with_cancel(
                &self.queue,
                &self.statuses,
                username,
                uid,
                &self.arm,
                &token,
                &mut || Duration::from_millis(20),
            )
        };
        let mut permit = match wait {
            PermitWait::Granted(permit) => permit,
            PermitWait::Cancelled => {
                return Err(direct_login_error(
                    LoginStage::Connect,
                    io::ErrorKind::Interrupted,
                    "login was cancelled while waiting for admission",
                ));
            }
            PermitWait::Deadline => {
                self.arm.publish_handshake_aborted(
                    AbortedAttempt::Deadline,
                    command,
                    world_generation,
                    token.generation(),
                );
                return Err(direct_login_error(
                    LoginStage::Connect,
                    io::ErrorKind::TimedOut,
                    "initial game TLS capacity deadline expired",
                ));
            }
            PermitWait::Unavailable => {
                self.arm.publish_handshake_aborted(
                    AbortedAttempt::Transport(TransportFailure::Closed),
                    command,
                    world_generation,
                    token.generation(),
                );
                return Err(direct_login_error(
                    LoginStage::Connect,
                    io::ErrorKind::NotConnected,
                    "host game transport is unavailable",
                ));
            }
        };

        let generation = prepared.token().generation();
        let result = {
            let mut slot_login = SlotLogin {
                client,
                arm: &self.arm,
                command,
                world_generation,
                username,
                password,
                reconnect,
                backoff: &self.backoff,
            };
            slot_login.client.set_external_reconnect_owner(true);
            let hold = Cell::new(None);
            let Some(outcome) = permit.attempt(
                prepared,
                || {
                    self.arm.uid.load(Ordering::Acquire) != uid
                        || !self.arm.login_attempt_is_current(command, world_generation)
                },
                |cancel, generation| {
                    self.arm.begin_attempt_with_generation(
                        command,
                        world_generation,
                        cancel,
                        generation,
                    )
                },
                |deadline, cancel| {
                    let (result, server_hold) = slot_login.run(deadline, cancel, |_| {});
                    hold.set(server_hold);
                    result
                },
                |_| hold.get(),
            ) else {
                self.arm.clear_unused_attempt(generation);
                let error = if wss_health
                    .as_ref()
                    .is_some_and(|health| health.loss().is_some())
                {
                    self.arm.publish_handshake_aborted(
                        AbortedAttempt::Transport(TransportFailure::Closed),
                        command,
                        world_generation,
                        generation,
                    );
                    direct_login_error(
                        LoginStage::Connect,
                        io::ErrorKind::NotConnected,
                        "host game transport became unavailable before login",
                    )
                } else if tls_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    self.arm.publish_handshake_aborted(
                        AbortedAttempt::Deadline,
                        command,
                        world_generation,
                        generation,
                    );
                    direct_login_error(
                        LoginStage::Connect,
                        io::ErrorKind::TimedOut,
                        "initial game TLS capacity deadline expired",
                    )
                } else {
                    direct_login_error(
                        LoginStage::Connect,
                        io::ErrorKind::Interrupted,
                        "login was cancelled before the handshake started",
                    )
                };
                return Err(error);
            };

            let result = slot_login.finish_after_q(outcome);
            if let Err(error) = &result {
                let _ = self.arm.defer_clean_release_at(
                    error,
                    command,
                    &slot_login.client.config.host,
                    slot_login.client.config.port,
                    Instant::now(),
                );
            }
            result
        };

        match result {
            Ok(SlotLoginOutcome::Promoted) => {
                self.backoff.reset();
                Ok(())
            }
            Ok(SlotLoginOutcome::Aborted(reason)) => Err(direct_login_error(
                LoginStage::Connect,
                if reason == AbortedAttempt::Deadline {
                    io::ErrorKind::TimedOut
                } else {
                    io::ErrorKind::Interrupted
                },
                format!("login attempt aborted: {reason:?}"),
            )),
            Ok(SlotLoginOutcome::CommittedSessionExit(cause)) => Err(direct_login_error(
                LoginStage::Connect,
                io::ErrorKind::ConnectionAborted,
                format!("host login commit ended the session: {cause:?}"),
            )),
            Err(error) => {
                if error.code == 16 {
                    let _ = self.backoff.throttle_delay();
                }
                Err(error)
            }
        }
    }
}

impl Drop for DirectClientLogin {
    fn drop(&mut self) {
        self.queue.lock().leave_owner(self.arm.queue_owner);
    }
}

fn direct_login_error(
    stage: LoginStage,
    kind: io::ErrorKind,
    detail: impl Into<String>,
) -> LoginError {
    let error = io::Error::new(kind, detail.into());
    LoginError {
        code: -1,
        mes1: String::new(),
        mes2: error.to_string(),
        retry_after: None,
        transport: Some(LoginTransportError::from_io(stage, &error)),
    }
}

/// Title frames a slot owner runs while it waits outside its client pump
/// (queue, retry backoff, transfer countdown). Each call runs the frame that
/// is due, if any, and returns how long the waiter may block before the
/// next one.
pub(super) type WaitFrames<'a> = dyn FnMut() -> Duration + 'a;

static NEXT_SLOT_KEY: SlotKeyMinter = SlotKeyMinter::new();

/// Three independent unexpected exits inside ten minutes are unlikely to be
/// a transient network blip, but still bound an accidental reconnect storm.
/// The operator can always clear the guard with an explicit Log in.
pub const UNEXPECTED_LOGOUT_THRESHOLD: usize = 3;
pub const UNEXPECTED_LOGOUT_WINDOW: Duration = Duration::from_secs(10 * 60);

// World.ts:795-803 retries an unacknowledged player_logout every 15 s;
// LoginServer.ts:415-455 saves before releasing ownership. The 30 s host
// budget covers the initial post plus one 15 s retry, not two retries.
// WorldConfig.ts:108-109 and World.ts:177,2202-2212 reject the fifth device
// attempt with a refreshed 15 s idle TTL. The 5 s host probe cadence avoids
// bursts; normal queue admission still enforces four grants then 15 s idle.
pub(super) const CLEAN_RELEASE_BUDGET: Duration = Duration::from_secs(30);
pub(super) const CLEAN_RELEASE_CADENCE: Duration = Duration::from_secs(5);

struct CleanExit {
    observed: Instant,
    not_before: Instant,
    host: String,
    port: u16,
    uid: i32,
    world_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginLatchReason {
    OperatorLogout,
    PersistedLogout,
    RepeatedUnexpectedLogouts { count: u8, window_seconds: u64 },
}

impl std::fmt::Display for LoginLatchReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OperatorLogout => f.write_str("operator logout"),
            Self::PersistedLogout => f.write_str("saved logged-out state"),
            Self::RepeatedUnexpectedLogouts {
                count,
                window_seconds,
            } => write!(
                f,
                "repeat guard: {count} unexpected logouts in {window_seconds}s"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UnexpectedLogoutRecord {
    pub(super) count: usize,
    pub(super) latched: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelogParkReason {
    NoIntent,
    OperatorLogout,
    PersistedLogout,
    RepeatGuard,
    WorldError,
}

impl std::fmt::Display for RelogParkReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoIntent => "no_intent",
            Self::OperatorLogout => "operator_logout",
            Self::PersistedLogout => "persisted_logout",
            Self::RepeatGuard => "repeat_guard",
            Self::WorldError => "world_error",
        })
    }
}

/// Maximum unconsumed session boundaries per slot. Overflow drops the oldest
/// boundary and reports its count with the next consuming read.
const SESSION_BOUNDARY_LIMIT: usize = 32;

/// The frozen mailbox, the one currently running handshake, and owner-clock
/// evidence share one lock. Posting an invalidating row takes its cancel
/// handle under this lock and fires it only after dropping the lock.
struct SlotLifecycle {
    mailbox: IntentMailbox,
    session: SessionGate,
    last_session: Option<SessionKey>,
    scene_ready: bool,
    /// The public world the slot's client is configured for: the world
    /// the next handshake logs in to.
    configured_world: Option<u16>,
    /// The public world the current session logged in to (copied from
    /// `configured_world` at its handshake), bound to that session.
    session_world: Option<u16>,
    latest_event: Option<Event<SessionEventOut>>,
    /// Bounded, chronological unconsumed session boundaries. Overflow is
    /// counted separately rather than silently replacing an unread boundary.
    pending_session_ends: VecDeque<Event<SessionEventOut>>,
    missed_session_ends: u64,
    last_queue: Option<SessionEventOut>,
    prequeue_cancel: Option<CancelToken>,
    attempt: Option<LoginCancel>,
    promotion_pending: Option<u64>,
    attempt_generation: Option<u64>,
    prequeue_abort: Option<(u64, AbortedAttempt)>,
    unexpected_logout_times: [Option<Instant>; UNEXPECTED_LOGOUT_THRESHOLD],
    /// Logout/turnover policy captured before SessionExited clears its command;
    /// current script activity is sampled when the boundary is consumed.
    boundary_work_allowed: Option<bool>,
    clean_exit: Option<CleanExit>,
}

impl SlotLifecycle {
    fn slot_event(&mut self, payload: SessionEventOut) {
        self.latest_event = Some(Event::new(Scope::Slot(self.session.slot()), None, payload));
    }

    fn session_event(&mut self, payload: SessionEventOut) {
        if let Some(key) = self.session.current() {
            self.session
                .observe(key, &payload)
                .expect("owner stamped current session");
            let event = Event::new(Scope::Session(key), None, payload);
            if matches!(payload, SessionEventOut::SessionEnded { .. }) {
                if self.pending_session_ends.len() == SESSION_BOUNDARY_LIMIT {
                    self.pending_session_ends.pop_front();
                    self.missed_session_ends = self.missed_session_ends.saturating_add(1);
                }
                self.pending_session_ends.push_back(event.clone());
            }
            self.latest_event = Some(event);
        }
    }
}

impl SlotLifecycle {
    fn login_succeeded(&mut self, command: IntentCommand, kind: HandshakeKind) {
        self.mailbox
            .observe(LifecycleFact::LoginSucceeded)
            .expect("slot success generation exhausted");
        self.mailbox.acknowledge_login(command);
        self.clean_exit = None;
        let next = match self.last_session {
            Some(previous) => previous.next().expect("session epoch exhausted"),
            None => SessionKey::first(self.session.slot()),
        };
        self.session
            .begin(next)
            .expect("owner minted a new session");
        self.last_session = Some(next);
        self.scene_ready = false;
        self.session_world = self.configured_world;
        self.last_queue = None;
        self.slot_event(SessionEventOut::HandshakeOk { kind });
    }

    fn observe_exit(&mut self, cause: SessionEndCause, resumable: bool) {
        if !self.mailbox.session_online() {
            return;
        }
        let snapshot = self.mailbox.snapshot();
        self.boundary_work_allowed = Some(
            !snapshot.logout_wanted()
                && !snapshot.turnover_pending()
                && !matches!(
                    snapshot.latch(),
                    Some(LoginLatch::OperatorLogout | LoginLatch::PersistedLogout)
                ),
        );
        self.session_event(SessionEventOut::SessionEnded { cause, resumable });
        self.scene_ready = false;
        self.mailbox
            .observe(LifecycleFact::SessionExited {
                clean: cause.is_clean(),
                resumable,
            })
            .expect("slot session exit generation exhausted");
    }

    fn record_unexpected_logout_at(&mut self, now: Instant) -> Option<UnexpectedLogoutRecord> {
        let snapshot = self.mailbox.snapshot();
        if snapshot.latch().is_some() || snapshot.logout_wanted() || snapshot.turnover_pending() {
            return None;
        }
        let mut retained = [None; UNEXPECTED_LOGOUT_THRESHOLD];
        let mut count = 0;
        for timestamp in self.unexpected_logout_times.iter().flatten().copied() {
            if now.saturating_duration_since(timestamp) <= UNEXPECTED_LOGOUT_WINDOW {
                retained[count] = Some(timestamp);
                count += 1;
            }
        }
        if count < UNEXPECTED_LOGOUT_THRESHOLD {
            retained[count] = Some(now);
            count += 1;
        }
        self.unexpected_logout_times = retained;
        let latched = count >= UNEXPECTED_LOGOUT_THRESHOLD;
        if latched {
            self.mailbox
                .observe(LifecycleFact::RepeatGuardTripped {
                    count: count as u8,
                    window_seconds: UNEXPECTED_LOGOUT_WINDOW.as_secs(),
                })
                .expect("slot repeat-guard generation exhausted");
            self.clean_exit = None;
        }
        Some(UnexpectedLogoutRecord { count, latched })
    }
}

#[cfg(test)]
type WorkerStartGate = (
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
);
#[cfg(test)]
type RetryRaceGate = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);
#[cfg(test)]
type WorkerQueuePanicGate = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

/// Watches an explicitly armed test's token from K4 entry until unlock.
/// Drop catches even a second, direct `cancel()` inserted inside the lock.
#[cfg(test)]
struct K4CancelGuard<'a> {
    lifecycle: parking_lot::MutexGuard<'a, SlotLifecycle>,
    pending: Option<LoginCancel>,
    pending_prequeue: Option<CancelToken>,
}

#[cfg(test)]
impl<'a> K4CancelGuard<'a> {
    fn new(lifecycle: parking_lot::MutexGuard<'a, SlotLifecycle>, enabled: bool) -> Self {
        let pending = enabled
            .then(|| lifecycle.attempt.as_ref())
            .flatten()
            .filter(|cancel| !cancel.is_cancelled())
            .cloned();
        let pending_prequeue = enabled
            .then(|| lifecycle.prequeue_cancel.as_ref())
            .flatten()
            .filter(|cancel| !cancel.is_cancelled())
            .cloned();
        Self {
            lifecycle,
            pending,
            pending_prequeue,
        }
    }
}

#[cfg(test)]
impl std::ops::Deref for K4CancelGuard<'_> {
    type Target = SlotLifecycle;

    fn deref(&self) -> &Self::Target {
        &self.lifecycle
    }
}

#[cfg(test)]
impl std::ops::DerefMut for K4CancelGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.lifecycle
    }
}

#[cfg(test)]
impl Drop for K4CancelGuard<'_> {
    fn drop(&mut self) {
        if let Some(cancel) = &self.pending {
            assert!(
                !cancel.is_cancelled(),
                "login token fired while K4 was held"
            );
        }
        if let Some(cancel) = &self.pending_prequeue {
            assert!(
                !cancel.is_cancelled(),
                "prequeue token fired while K4 was held"
            );
        }
    }
}
/// Watches an armed test's queue cancellation effects until K1 unlocks.
#[cfg(test)]
struct K1CancelGuard<'a> {
    queue: parking_lot::MutexGuard<'a, LoginQueue>,
    pending: Vec<CancelToken>,
    enabled: bool,
}

#[cfg(test)]
impl<'a> K1CancelGuard<'a> {
    fn new(queue: parking_lot::MutexGuard<'a, LoginQueue>, enabled: bool) -> Self {
        Self {
            queue,
            pending: Vec::new(),
            enabled,
        }
    }

    fn watch(&mut self, cancels: &[CancelToken]) {
        if self.enabled {
            self.pending.extend(
                cancels
                    .iter()
                    .filter(|cancel| !cancel.is_cancelled())
                    .cloned(),
            );
        }
    }
}

#[cfg(test)]
impl std::ops::Deref for K1CancelGuard<'_> {
    type Target = LoginQueue;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

#[cfg(test)]
impl std::ops::DerefMut for K1CancelGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.queue
    }
}

#[cfg(test)]
impl Drop for K1CancelGuard<'_> {
    fn drop(&mut self) {
        for cancel in &self.pending {
            assert!(
                !cancel.is_cancelled(),
                "login token fired while K1 was held"
            );
        }
    }
}

/// Per-slot control arm. The panel flips these to make a slot sit on the
/// title screen (no handshake) until login is armed, request a clean IF
/// logout, or stop the thread. A `None` arm at spawn means CLI/e2e: the
/// slot logs in immediately.
pub struct SlotArm {
    /// The profile uid this arm controls. Device UIDs are throttle keys, not
    /// queue identities; each worker gets a process-unique owner below.
    pub uid: AtomicI32,
    /// Process-unique FIFO identity. Device UIDs are not unique account
    /// identities and are used only for server throttle accounting.
    pub(super) queue_owner: u64,
    /// Shared readiness/revocation wake channel for the adapter's slot waits.
    park: Arc<Park>,
    /// Monotonic cancellation generations for this slot's attempts.
    cancel_source: CancelSource,
    /// Intent mailbox, session gate and in-flight handshake under one K4 lock.
    lifecycle: parking_lot::Mutex<SlotLifecycle>,
    key_fetch_cancel: parking_lot::Mutex<Option<LoginCancel>>,
    /// Only isolated lock-order tests watch token state; another thread may
    /// legitimately fire a previously collected token while this arm is locked.
    #[cfg(test)]
    assert_cancel_order: AtomicBool,
    /// Pump shutdown signal, derived from the mailbox's terminal command.
    /// The legacy client startup and render loops read this atomic directly.
    pub stop: Arc<AtomicBool>,
    /// Live guardian toggle (`ProfileSettings.random_events`). Mirrored
    /// from the vault on spawn and by panel/TUI settings writes so a
    /// toggle-off never acts/holds without a respawn.
    pub random_events: Arc<AtomicBool>,
    /// Live lamp auto-use toggle (`ProfileSettings.lamp_auto`).
    pub lamp_auto: Arc<AtomicBool>,
    /// Live lamp skill choice (`ProfileSettings.lamp_skill`).
    pub lamp_skill: Arc<Mutex<String>>,
    /// Live per-event Guardian policies, mirrored from profile saves.
    pub random_event_policies: Arc<Mutex<BTreeMap<String, RandomEventPolicy>>>,
    /// Live Cure poison policy as its mirrored byte
    /// ([`script::survival::CurePoison::to_byte`]): 0 is the default, cure
    /// when needed. Mirrored from the vault on spawn and by settings
    /// writes, and read once per frame into the posted script bundle, so
    /// a settings change reaches a running slot within one frame. The
    /// `Arc` is one 24-byte allocation per slot, made at slot creation.
    pub cure_poison: Arc<AtomicU8>,
    /// The password the next handshake sends. Set from the profile at spawn
    /// and by [`crate::Play::remember_profile`], so a saved password change
    /// reaches a running worker's next login without a respawn. Private: it
    /// never appears in Debug output or logs.
    password: parking_lot::Mutex<Arc<str>>,
    retry_wake: parking_lot::Condvar,
    /// Test seam for worker lifecycle cases whose subject starts at the
    /// login queue, after unrelated asset initialization.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) bypass_asset_startup: AtomicBool,
    /// Deterministic worker-entry gate for lifecycle race regressions.
    #[cfg(test)]
    worker_start_gate: parking_lot::Mutex<Option<WorkerStartGate>>,
    /// One-shot gates around the retry-wait race boundary.
    #[cfg(test)]
    retry_wait_gate: parking_lot::Mutex<Option<RetryRaceGate>>,
    #[cfg(test)]
    retry_wait_timeout: parking_lot::Mutex<Option<Duration>>,
    #[cfg(test)]
    retry_notify_gate: parking_lot::Mutex<Option<RetryRaceGate>>,
    #[cfg(test)]
    stop_cleanup_signal: parking_lot::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// Deterministic panic after FIFO publication, used to prove the
    /// worker-scope retirement guard.
    #[cfg(test)]
    worker_queue_panic_gate: parking_lot::Mutex<Option<WorkerQueuePanicGate>>,
}

impl SlotArm {
    pub fn new(uid: i32, want_login: bool) -> Arc<Self> {
        let park = Arc::new(Park::new());
        let slot = NEXT_SLOT_KEY.mint().expect("slot identity exhausted");
        Arc::new(Self {
            uid: AtomicI32::new(uid),
            park: Arc::clone(&park),
            cancel_source: CancelSource::new(park),
            queue_owner: slot.incarnation().get(),
            lifecycle: parking_lot::Mutex::new(SlotLifecycle {
                mailbox: IntentMailbox::new(want_login, IntentWorldChoice::Automatic),
                session: SessionGate::new(slot),
                last_session: None,
                scene_ready: false,
                configured_world: None,
                session_world: None,
                latest_event: None,
                pending_session_ends: VecDeque::new(),
                missed_session_ends: 0,
                last_queue: None,
                prequeue_cancel: None,
                attempt: None,
                promotion_pending: None,
                attempt_generation: None,
                unexpected_logout_times: [None; UNEXPECTED_LOGOUT_THRESHOLD],
                prequeue_abort: None,
                clean_exit: None,
                boundary_work_allowed: None,
            }),
            key_fetch_cancel: parking_lot::Mutex::new(None),
            #[cfg(test)]
            assert_cancel_order: AtomicBool::new(false),
            stop: Arc::new(AtomicBool::new(false)),
            random_events: Arc::new(AtomicBool::new(true)),
            lamp_auto: Arc::new(AtomicBool::new(true)),
            lamp_skill: Arc::new(Mutex::new("strength".to_string())),
            random_event_policies: Arc::new(Mutex::new(BTreeMap::new())),
            cure_poison: Arc::new(AtomicU8::new(
                script::survival::CurePoison::WhenNeeded.to_byte(),
            )),
            password: parking_lot::Mutex::new(Arc::from("")),
            retry_wake: parking_lot::Condvar::new(),
            #[cfg(any(test, feature = "test-support"))]
            bypass_asset_startup: AtomicBool::new(false),
            #[cfg(test)]
            worker_start_gate: parking_lot::Mutex::new(None),
            #[cfg(test)]
            retry_wait_gate: parking_lot::Mutex::new(None),
            #[cfg(test)]
            retry_wait_timeout: parking_lot::Mutex::new(None),
            #[cfg(test)]
            retry_notify_gate: parking_lot::Mutex::new(None),
            #[cfg(test)]
            stop_cleanup_signal: parking_lot::Mutex::new(None),
            #[cfg(test)]
            worker_queue_panic_gate: parking_lot::Mutex::new(None),
        })
    }

    /// Publish the active HTTPS login-key fetch before it can block. K4
    /// serializes this registration with invalidating intents; the cancel is
    /// always fired only after K4 has been released.
    fn publish_key_fetch_cancel(&self, cancel: &LoginCancel) -> bool {
        let lifecycle = self.lifecycle.lock();
        let mut active = self.key_fetch_cancel.lock();
        assert!(
            active.is_none(),
            "one slot owns at most one login-key fetch"
        );
        *active = Some(cancel.clone());
        drop(active);
        drop(lifecycle);
        if self.stop.load(Ordering::Acquire) {
            cancel.cancel();
            return false;
        }
        true
    }

    fn clear_key_fetch_cancel(&self) {
        self.key_fetch_cancel.lock().take();
    }
    /// Enter the spawned worker at the queue/login seam. Production slots
    /// always run the complete asset startup.
    #[cfg(any(test, feature = "test-support"))]
    pub fn bypass_asset_startup_for_test(&self) {
        self.bypass_asset_startup.store(true, Ordering::Relaxed);
    }

    /// Pause a spawned worker before it may publish lifetime-owned entries.
    /// Returns entry/publication receivers and the release sender.
    #[cfg(test)]
    pub(crate) fn hold_worker_start_for_test(
        &self,
    ) -> (
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    ) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (published_tx, published_rx) = std::sync::mpsc::channel();
        *self.worker_start_gate.lock() = Some((entered_tx, release_rx, published_tx));
        (entered_rx, release_tx, published_rx)
    }

    #[cfg(test)]
    pub(crate) fn wait_worker_start_for_test(&self) -> Option<std::sync::mpsc::Sender<()>> {
        let (entered, release, published) = self.worker_start_gate.lock().take()?;
        entered.send(()).unwrap();
        release.recv().unwrap();
        Some(published)
    }

    #[cfg(test)]
    pub(crate) fn hold_worker_queue_panic_for_test(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.worker_queue_panic_gate.lock() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    #[cfg(test)]
    pub(crate) fn panic_after_queue_for_test(&self) {
        if let Some((entered, release)) = self.worker_queue_panic_gate.lock().take() {
            entered.send(()).unwrap();
            release.recv().unwrap();
            panic!("synthetic queued worker panic");
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_stop_cleanup_for_test(&self) -> std::sync::mpsc::Receiver<()> {
        let (sent, received) = std::sync::mpsc::channel();
        *self.stop_cleanup_signal.lock() = Some(sent);
        received
    }

    #[cfg(test)]
    pub(crate) fn signal_stop_cleanup_for_test(&self) {
        if let Some(signal) = self.stop_cleanup_signal.lock().take() {
            signal.send(()).unwrap();
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_retry_wait_before_park_for_test(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.retry_wait_gate.lock() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    #[cfg(test)]
    pub(crate) fn retry_wait_timeout_for_test(&self) -> Option<Duration> {
        *self.retry_wait_timeout.lock()
    }

    #[cfg(test)]
    pub(crate) fn hold_retry_notify_before_lock_for_test(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.retry_notify_gate.lock() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    #[cfg(test)]
    fn wait_at_retry_race_gate(gate: &parking_lot::Mutex<Option<RetryRaceGate>>) {
        if let Some((entered, release)) = gate.lock().take() {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
    }

    /// Apply one frozen row and fire cancellation effects outside K4.
    pub fn post_intent(&self, row: Intent) -> Option<PostReceipt> {
        let stop_fetch = matches!(row, Intent::RemoveSlot | Intent::ProcessShutdown);
        let (receipt, cancel, prequeue_cancel, key_fetch_cancel) = {
            #[cfg(test)]
            let mut lifecycle = K4CancelGuard::new(
                self.lifecycle.lock(),
                self.assert_cancel_order.load(Ordering::Relaxed),
            );
            #[cfg(not(test))]
            let mut lifecycle = self.lifecycle.lock();
            let receipt = match lifecycle.mailbox.post(row) {
                Ok(receipt) => receipt,
                Err(api::lifecycle::MailboxError::Closed) => return None,
                Err(error) => panic!("slot intent generation exhausted: {error}"),
            };
            let invalidated = receipt.invalidated_attempt();
            if invalidated {
                lifecycle.clean_exit = None;
                lifecycle.last_queue = None;
            }
            if row == Intent::Login {
                lifecycle.unexpected_logout_times = [None; UNEXPECTED_LOGOUT_THRESHOLD];
            }
            if stop_fetch {
                self.stop.store(true, Ordering::Release);
            }
            lifecycle.slot_event(SessionEventOut::IntentApplied {
                generation: receipt.generation(),
            });
            let cancel = invalidated.then(|| lifecycle.attempt.clone()).flatten();
            let prequeue_cancel = (invalidated || stop_fetch)
                .then(|| lifecycle.prequeue_cancel.take())
                .flatten();
            let key_fetch_cancel = (invalidated || stop_fetch)
                .then(|| self.key_fetch_cancel.lock().clone())
                .flatten();
            (receipt, cancel, prequeue_cancel, key_fetch_cancel)
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        if let Some(cancel) = prequeue_cancel {
            cancel.cancel();
        }
        if let Some(cancel) = key_fetch_cancel {
            cancel.cancel();
        }
        self.retry_wake.notify_all();
        Some(receipt)
    }

    pub fn arm_explicit_login(&self) {
        self.post_intent(Intent::Login);
    }

    pub fn request_logout(&self) {
        self.post_intent(Intent::Logout);
    }

    pub fn request_relog(&self) {
        self.post_intent(Intent::RelogNow);
    }

    pub fn switch_world(&self, number: u16) {
        self.post_intent(Intent::SwitchWorld(number));
    }

    pub fn hold_logged_out(&self) {
        self.post_intent(Intent::HoldLoggedOut);
    }

    pub fn wants_login(&self) -> bool {
        self.lifecycle.lock().mailbox.snapshot().login() != LoginIntent::Withdrawn
    }

    pub fn wants_logout(&self) -> bool {
        self.lifecycle.lock().mailbox.snapshot().logout_wanted()
    }

    pub fn login_latched(&self) -> bool {
        self.lifecycle.lock().mailbox.snapshot().latch().is_some()
    }

    pub fn login_latch_reason(&self) -> Option<LoginLatchReason> {
        self.lifecycle
            .lock()
            .mailbox
            .snapshot()
            .latch()
            .map(|reason| match reason {
                LoginLatch::OperatorLogout => LoginLatchReason::OperatorLogout,
                LoginLatch::PersistedLogout => LoginLatchReason::PersistedLogout,
                LoginLatch::RepeatGuard {
                    count,
                    window_seconds,
                } => LoginLatchReason::RepeatedUnexpectedLogouts {
                    count,
                    window_seconds,
                },
            })
    }

    pub fn set_lowmem_handshake(&self, lowmem: bool) {
        self.post_intent(Intent::MemoryMode(if lowmem {
            MemoryMode::Low
        } else {
            MemoryMode::High
        }));
    }

    pub fn lowmem_handshake(&self) -> Option<bool> {
        self.lifecycle
            .lock()
            .mailbox
            .snapshot()
            .memory()
            .map(|mode| mode == MemoryMode::Low)
    }

    pub fn login_applies_memory(&self) -> bool {
        self.lifecycle.lock().mailbox.login_applies_memory()
    }

    pub fn reconnects(&self) -> bool {
        self.lifecycle.lock().mailbox.reconnects()
    }

    pub fn auto_login(&self) -> bool {
        self.lifecycle.lock().mailbox.snapshot().auto_login()
    }

    pub(super) fn take_logout_work_reset(&self) -> bool {
        self.lifecycle.lock().mailbox.take_work_reset()
    }

    /// The owner has exactly one active session boundary at a time.
    #[cfg(test)]
    pub(super) fn session_online(&self) -> bool {
        self.lifecycle.lock().mailbox.session_online()
    }

    /// The owner-authorized session; absent once it ends.
    pub fn session_key(&self) -> Option<SessionKey> {
        self.lifecycle.lock().session.current()
    }

    /// The current session once its scene is ready, with the public world
    /// it logged in to (`None` on a local server): read under one lock, so
    /// the key, the readiness and the world describe the same session. A
    /// session that has not reached its scene yet, or none, is `None`.
    pub fn ready_session(&self) -> Option<(SessionKey, Option<u16>)> {
        let lifecycle = self.lifecycle.lock();
        let key = lifecycle.session.current()?;
        lifecycle
            .scene_ready
            .then_some((key, lifecycle.session_world))
    }

    /// Record the public world the slot's client is configured for; the
    /// next session to log in is bound to it.
    pub(super) fn note_configured_world(&self, world: u16) {
        self.lifecycle.lock().configured_world = Some(world);
    }

    /// The slot identity minted when this arm was created; every lifecycle
    /// event of this arm is scoped to it.
    pub fn slot(&self) -> SlotKey {
        self.lifecycle.lock().session.slot()
    }

    /// Borrow the actual session gate while its owner lock remains held.
    pub(super) fn session_gate(&self) -> parking_lot::MappedMutexGuard<'_, SessionGate> {
        parking_lot::MutexGuard::map(self.lifecycle.lock(), |lifecycle| &mut lifecycle.session)
    }

    /// The most recently published event, including a newer session's exit.
    /// Consumers requiring ordered boundaries and overflow accounting use
    /// `take_lifecycle_event`.
    pub fn latest_lifecycle_event(&self) -> Option<Event<SessionEventOut>> {
        self.lifecycle.lock().latest_event.clone()
    }

    /// Consume the oldest retained boundary before the latest replaceable
    /// status event. The second value counts boundaries dropped since the
    /// previous read. At most `SESSION_BOUNDARY_LIMIT` boundaries and one
    /// status are stored; callers must inspect the count even when no event
    /// is returned. `latest_lifecycle_event` is only a replaceable view.
    pub fn take_lifecycle_event(&self) -> (Option<Event<SessionEventOut>>, u64) {
        let mut lifecycle = self.lifecycle.lock();
        let missed = std::mem::take(&mut lifecycle.missed_session_ends);
        if let Some(event) = lifecycle.pending_session_ends.pop_front() {
            if lifecycle.latest_event.as_ref() == Some(&event) {
                lifecycle.latest_event = None;
            }
            return (Some(event), missed);
        }
        (lifecycle.latest_event.take(), missed)
    }

    pub(super) fn prepared(&self) {
        let mut lifecycle = self.lifecycle.lock();
        lifecycle.last_queue = None;
        lifecycle.slot_event(SessionEventOut::Prepared);
    }

    pub(super) fn publish_queued(&self, event: SessionEventOut) {
        debug_assert!(matches!(event, SessionEventOut::Queued { .. }));
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.last_queue != Some(event) {
            lifecycle.last_queue = Some(event);
            lifecycle.slot_event(event);
        }
    }

    /// Publish a scene observation and each hold as an independently
    /// stamped session event. A stale session can never authorize a tick.
    pub(super) fn update_session_readiness(
        &self,
        ready: bool,
        welcome_hold: bool,
        guardian_hold: bool,
        boundary_hold: bool,
    ) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.session.current().is_none() {
            return false;
        }
        if ready && !lifecycle.scene_ready {
            lifecycle.session_event(SessionEventOut::SceneReady);
            lifecycle.scene_ready = true;
        }
        for (why, held) in [
            (HoldReason::WelcomeModal, welcome_hold),
            (HoldReason::Guardian, guardian_hold),
            (HoldReason::SessionBoundary, boundary_hold),
        ] {
            if lifecycle.session.held_by(why) != held {
                lifecycle.session_event(if held {
                    SessionEventOut::Held { why }
                } else {
                    SessionEventOut::Released { why }
                });
            }
        }
        ready && lifecycle.session.ready().is_some()
    }

    /// Why the current session is held, for the live seat cells.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn session_hold_reasons(&self) -> Vec<HoldReason> {
        let lifecycle = self.lifecycle.lock();
        HoldReason::ALL
            .into_iter()
            .filter(|why| lifecycle.session.held_by(*why))
            .collect()
    }

    pub(super) fn observe_session_exit(&self, cause: SessionEndCause, resumable: bool) {
        self.lifecycle.lock().observe_exit(cause, resumable);
    }

    pub(super) fn take_session_boundary(&self) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        if let Some(allowed) = lifecycle.boundary_work_allowed.take() {
            return allowed && lifecycle.mailbox.snapshot().script_needs_session();
        }
        let keep_work = lifecycle.mailbox.keeps_run_work_after_exit();
        if lifecycle.mailbox.session_online() {
            lifecycle.session_event(SessionEventOut::SessionEnded {
                cause: if self.stop.load(Ordering::Acquire) {
                    SessionEndCause::Removed
                } else {
                    SessionEndCause::ConnectionLost
                },
                resumable: false,
            });
            lifecycle.scene_ready = false;
            lifecycle
                .mailbox
                .observe(LifecycleFact::SessionExited {
                    clean: false,
                    resumable: false,
                })
                .expect("slot fallback exit generation exhausted");
        }
        keep_work
    }

    pub(super) fn session_boundary_pending(&self) -> bool {
        let lifecycle = self.lifecycle.lock();
        lifecycle.boundary_work_allowed.is_some() || lifecycle.mailbox.session_online()
    }

    pub(super) fn relog_park_reason(&self) -> RelogParkReason {
        match self.lifecycle.lock().mailbox.snapshot().park_reason() {
            Some(ParkReason::OperatorLogout) => RelogParkReason::OperatorLogout,
            Some(ParkReason::PersistedLogout) => RelogParkReason::PersistedLogout,
            Some(ParkReason::RepeatGuard) => RelogParkReason::RepeatGuard,
            Some(ParkReason::WorldError) => RelogParkReason::WorldError,
            Some(ParkReason::NoIntent) | None => RelogParkReason::NoIntent,
        }
    }

    pub fn lifetime_id(&self) -> u64 {
        self.queue_owner
    }

    pub fn login_wanted(&self) -> bool {
        self.lifecycle.lock().mailbox.snapshot().login_wanted()
            && !self.stop.load(Ordering::Acquire)
    }

    pub fn login_held_by_error(&self) -> bool {
        self.lifecycle.lock().mailbox.snapshot().error_hold()
    }

    pub fn set_auto_login(&self, enabled: bool) {
        self.post_intent(Intent::AutoLogin(enabled));
    }

    /// Mirror a durable Cure poison write onto this arm. The next frame
    /// the slot observes posts the new policy to its script.
    pub fn set_cure_poison(&self, policy: vault::CurePoison) {
        // The vault-to-policy conversion lives here, in the crate that
        // owns both the profile row and the mirror; the script enum
        // itself carries only the byte encoding.
        let policy = match policy {
            vault::CurePoison::WhenNeeded => script::survival::CurePoison::WhenNeeded,
            vault::CurePoison::Always => script::survival::CurePoison::Always,
            vault::CurePoison::Never => script::survival::CurePoison::Never,
        };
        self.cure_poison.store(policy.to_byte(), Ordering::Relaxed);
    }

    /// The Cure poison policy this arm currently mirrors.
    pub fn cure_poison(&self) -> script::survival::CurePoison {
        script::survival::CurePoison::from_byte(self.cure_poison.load(Ordering::Relaxed))
    }

    pub fn withdraw_login(&self) {
        self.post_intent(Intent::WithdrawLogin);
    }

    pub(super) fn hold_login_on_error(&self) {
        let cancel = {
            #[cfg(test)]
            let mut lifecycle = K4CancelGuard::new(
                self.lifecycle.lock(),
                self.assert_cancel_order.load(Ordering::Relaxed),
            );
            #[cfg(not(test))]
            let mut lifecycle = self.lifecycle.lock();
            let receipt = lifecycle
                .mailbox
                .observe(LifecycleFact::WorldPreferenceFailed)
                .expect("slot world-failure generation exhausted");
            lifecycle.clean_exit = None;
            receipt
                .invalidated_attempt()
                .then(|| lifecycle.attempt.clone())
                .flatten()
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        self.retry_wake.notify_all();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_login_on_error_for_test(&self) {
        self.hold_login_on_error();
    }

    pub(super) fn set_script_active(&self, active: bool) {
        if self
            .lifecycle
            .lock()
            .mailbox
            .snapshot()
            .script_needs_session()
            != active
        {
            self.post_intent(Intent::ScriptNeedsSession(active));
        }
    }

    pub(super) fn relogs_after_drop(&self) -> bool {
        self.lifecycle.lock().mailbox.relogs_after_drop()
    }

    pub(super) fn keeps_script_work_after_exit(&self) -> bool {
        self.lifecycle.lock().mailbox.keeps_run_work_after_exit()
    }

    pub(super) fn relog_decision_context(&self) -> (bool, bool, bool) {
        let snapshot = self.lifecycle.lock().mailbox.snapshot();
        (
            snapshot.script_needs_session(),
            snapshot.error_hold(),
            snapshot.login() == LoginIntent::Auto,
        )
    }

    /// Lock the same K4 predicate as the waiter before notifying it.
    #[cfg(test)]
    pub(super) fn notify_retry_wait(&self) {
        Self::wait_at_retry_race_gate(&self.retry_notify_gate);
        let _lifecycle = self.lifecycle.lock();
        self.retry_wake.notify_all();
    }

    pub(super) fn wait_for_retry(
        &self,
        timeout: Duration,
        attempt: Option<(IntentCommand, u64)>,
        frames: &mut WaitFrames<'_>,
    ) -> bool {
        #[cfg(test)]
        {
            *self.retry_wait_timeout.lock() = Some(timeout);
        }
        self.wait_for_retry_inner(timeout, true, attempt, frames)
    }

    pub(super) fn wait_for_transfer(&self, timeout: Duration, frames: &mut WaitFrames<'_>) -> bool {
        self.wait_for_retry_inner(timeout, false, None, frames)
    }

    fn wait_for_retry_inner(
        &self,
        timeout: Duration,
        interrupt_on_world_change: bool,
        attempt: Option<(IntentCommand, u64)>,
        frames: &mut WaitFrames<'_>,
    ) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        let initial = lifecycle.mailbox.snapshot();
        let world_generation = attempt.map_or(initial.world_generation(), |(_, world)| world);
        let cancellation = attempt.map_or(initial.cancellation(), |(command, _)| {
            command.cancellation()
        });
        let deadline = Instant::now() + timeout;
        loop {
            let next_frame = parking_lot::MutexGuard::unlocked(&mut lifecycle, &mut *frames);
            let snapshot = lifecycle.mailbox.snapshot();
            if self.stop.load(Ordering::Acquire) || !snapshot.login_wanted() {
                return false;
            }
            if interrupt_on_world_change
                && (snapshot.world_generation() != world_generation
                    || snapshot.cancellation() != cancellation)
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            #[cfg(test)]
            Self::wait_at_retry_race_gate(&self.retry_wait_gate);
            self.retry_wake
                .wait_for(&mut lifecycle, left.min(next_frame));
        }
    }

    pub(super) fn login_command(&self, ingame: bool) -> Option<IntentCommand> {
        self.lifecycle
            .lock()
            .mailbox
            .login_command(ingame)
            .filter(|_| !self.stop.load(Ordering::Acquire))
    }

    pub(super) fn logout_command(&self, ingame: bool) -> Option<IntentCommand> {
        self.lifecycle.lock().mailbox.logout_command(ingame)
    }

    pub(super) fn acknowledge_logout(&self, command: IntentCommand) {
        self.lifecycle.lock().mailbox.acknowledge_logout(command);
    }

    #[cfg(test)]
    pub(super) fn record_unexpected_logout_at(
        &self,
        now: Instant,
    ) -> Option<UnexpectedLogoutRecord> {
        self.lifecycle.lock().record_unexpected_logout_at(now)
    }

    #[cfg(test)]
    pub(super) fn unexpected_logout_count_at(&self, now: Instant) -> usize {
        self.lifecycle
            .lock()
            .unexpected_logout_times
            .iter()
            .flatten()
            .filter(|timestamp| {
                now.saturating_duration_since(**timestamp) <= UNEXPECTED_LOGOUT_WINDOW
            })
            .count()
    }
}

impl SlotArm {
    pub(super) fn login_attempt_is_current(
        &self,
        command: IntentCommand,
        world_generation: u64,
    ) -> bool {
        self.lifecycle
            .lock()
            .mailbox
            .attempt_is_current(command, world_generation)
            && !self.stop.load(Ordering::Acquire)
    }

    pub fn world_choice(&self) -> Option<u16> {
        match self.lifecycle.lock().mailbox.snapshot().world() {
            IntentWorldChoice::Automatic => None,
            IntentWorldChoice::World(number) => Some(number),
        }
    }

    pub(super) fn world_generation(&self) -> u64 {
        self.lifecycle.lock().mailbox.snapshot().world_generation()
    }

    pub(super) fn has_clean_release(&self) -> bool {
        self.lifecycle.lock().clean_exit.is_some()
    }

    pub(super) fn park(&self) -> Arc<Park> {
        Arc::clone(&self.park)
    }

    pub(super) fn new_cancel_token(&self) -> Option<CancelToken> {
        self.cancel_source.child()
    }

    /// Publish one exact prequeue cancellation token under its authorization.
    pub(super) fn publish_prequeue_cancel(
        &self,
        cancel: CancelToken,
        command: IntentCommand,
        world_generation: u64,
    ) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        if self.stop.load(Ordering::Acquire)
            || cancel.is_cancelled()
            || !lifecycle
                .mailbox
                .attempt_is_current(command, world_generation)
            || lifecycle.prequeue_cancel.is_some()
        {
            return false;
        }
        lifecycle.prequeue_cancel = Some(cancel);
        true
    }

    /// Clear only this prequeue attempt's cancellation token.
    pub(super) fn clear_prequeue_cancel(&self, cancel: &CancelToken) {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle
            .prequeue_cancel
            .as_ref()
            .is_some_and(|pending| pending.same_attempt(cancel))
        {
            lifecycle.prequeue_cancel = None;
        }
    }

    /// Publish the client cancellation handle and exact adapter generation
    /// together under K4, before the queue starts this attempt.
    pub(super) fn begin_attempt_with_generation(
        &self,
        command: IntentCommand,
        world_generation: u64,
        cancel: LoginCancel,
        generation: u64,
    ) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        let current = !self.stop.load(Ordering::Acquire)
            && lifecycle
                .mailbox
                .attempt_is_current(command, world_generation);
        if !current {
            if lifecycle
                .prequeue_cancel
                .as_ref()
                .is_some_and(|pending| pending.generation() == generation)
            {
                lifecycle.prequeue_cancel = None;
            }
            return false;
        }
        if lifecycle
            .prequeue_cancel
            .as_ref()
            .is_some_and(|pending| pending.generation() == generation)
        {
            lifecycle.prequeue_cancel = None;
        }
        lifecycle.attempt = Some(cancel);
        lifecycle.attempt_generation = Some(generation);
        lifecycle.promotion_pending = None;
        lifecycle.last_queue = None;
        let kind = if lifecycle.mailbox.reconnects() {
            HandshakeKind::Reconnect
        } else {
            HandshakeKind::Fresh
        };
        let world = match lifecycle.mailbox.snapshot().world() {
            IntentWorldChoice::Automatic => None,
            IntentWorldChoice::World(number) => Some(number),
        };
        lifecycle.slot_event(SessionEventOut::AttemptStarted { kind, world });
        true
    }

    #[cfg(test)]
    pub(super) fn begin_attempt(
        &self,
        command: IntentCommand,
        world_generation: u64,
        cancel: LoginCancel,
    ) -> bool {
        self.begin_attempt_with_generation(command, world_generation, cancel, 0)
    }

    /// A refused unused grant has no network or queue settlement to retire.
    pub(super) fn clear_unused_attempt(&self, generation: u64) {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.attempt_generation == Some(generation) {
            lifecycle.attempt = None;
            lifecycle.attempt_generation = None;
            lifecycle.promotion_pending = None;
            lifecycle.last_queue = None;
        }
    }
    /// Publish a pre-start WSS terminal once for the exact registered demand.
    pub(super) fn publish_handshake_aborted(
        &self,
        reason: AbortedAttempt,
        command: IntentCommand,
        world_generation: u64,
        generation: u64,
    ) {
        let mut lifecycle = self.lifecycle.lock();
        if self.stop.load(Ordering::Acquire)
            || !lifecycle
                .mailbox
                .attempt_is_current(command, world_generation)
        {
            return;
        }
        let prequeue_matches = lifecycle
            .prequeue_cancel
            .as_ref()
            .is_some_and(|token| token.generation() == generation && !token.is_cancelled());
        let attempt_matches = lifecycle.attempt_generation == Some(generation)
            && lifecycle
                .attempt
                .as_ref()
                .is_some_and(|cancel| !cancel.is_cancelled());
        if !prequeue_matches && !attempt_matches {
            return;
        }
        let current = (generation, reason);
        if lifecycle.prequeue_abort == Some(current) {
            return;
        }
        lifecycle.prequeue_abort = Some(current);
        lifecycle.attempt = None;
        lifecycle.attempt_generation = None;
        lifecycle.promotion_pending = None;
        lifecycle.last_queue = None;
        lifecycle.slot_event(SessionEventOut::HandshakeAborted { reason });
    }

    /// Phase R preflight under K4. The exact token remains published while
    /// the reactor seals this generation outside K4.
    pub(super) fn may_seal_after_q(
        &self,
        command: IntentCommand,
        world_generation: u64,
        deadline: Instant,
        cancel: &LoginCancel,
        generation: u64,
        queue_accepted: bool,
    ) -> bool {
        if !queue_accepted || Instant::now() >= deadline || cancel.is_cancelled() {
            return false;
        }
        let mut lifecycle = self.lifecycle.lock();
        let current = lifecycle.attempt_generation == Some(generation)
            && !self.stop.load(Ordering::Acquire)
            && lifecycle
                .mailbox
                .attempt_is_current(command, world_generation);
        if current {
            lifecycle.promotion_pending = Some(generation);
        }
        current
    }

    /// Phase C publishes session authority under K4. The receipt is committed
    /// by the reactor only after this lock has been released.
    #[expect(
        clippy::too_many_arguments,
        reason = "Phase Q/R/C retains explicit command identity, cancellation, and receipt"
    )]
    fn finish_attempt_after_q(
        &self,
        command: IntentCommand,
        world_generation: u64,
        deadline: Instant,
        cancel: &LoginCancel,
        generation: u64,
        queue_accepted: bool,
        result: &Result<(), LoginError>,
        seal: Option<Result<SealReceipt, std::io::Error>>,
    ) -> AttemptDisposition {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.attempt_generation != Some(generation) {
            return AttemptDisposition::Aborted(AbortedAttempt::Cancelled);
        }
        lifecycle.attempt = None;
        lifecycle.attempt_generation = None;
        lifecycle.last_queue = None;
        let promotion_pending = lifecycle.promotion_pending.take() == Some(generation);
        let cancelled = cancel.is_cancelled();
        let owned = !self.stop.load(Ordering::Acquire)
            && lifecycle
                .mailbox
                .attempt_is_current(command, world_generation);
        let has_receipt = matches!(seal.as_ref(), Some(Ok(_)));
        // A positive receipt certifies the reactor detached before D. Do not
        // invalidate it because the slot woke after D.
        let expired = !has_receipt && Instant::now() >= deadline;

        if result.is_err() {
            if cancelled || !owned {
                lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                    reason: AbortedAttempt::Cancelled,
                });
                return AttemptDisposition::Aborted(AbortedAttempt::Cancelled);
            }
            if !queue_accepted || expired {
                let reason = if expired {
                    AbortedAttempt::Deadline
                } else {
                    AbortedAttempt::Cancelled
                };
                lifecycle.slot_event(SessionEventOut::HandshakeAborted { reason });
                return AttemptDisposition::Aborted(reason);
            }
            if let Err(error) = result {
                if let Some(transport) = error.transport.as_ref() {
                    lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                        reason: AbortedAttempt::Transport(TransportFailure::classify(transport)),
                    });
                } else if let Ok(code) = u8::try_from(error.code) {
                    lifecycle.slot_event(SessionEventOut::HandshakeRefused {
                        code,
                        retry_after: error.retry_after,
                    });
                }
            }
            return AttemptDisposition::CurrentFailure;
        }

        if !queue_accepted {
            lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                reason: AbortedAttempt::LateSuccess,
            });
            return AttemptDisposition::Aborted(AbortedAttempt::LateSuccess);
        }
        if cancelled || !owned {
            lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                reason: AbortedAttempt::Cancelled,
            });
            return AttemptDisposition::Aborted(AbortedAttempt::Cancelled);
        }
        if expired {
            lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                reason: AbortedAttempt::LateSuccess,
            });
            return AttemptDisposition::Aborted(AbortedAttempt::LateSuccess);
        }
        if !promotion_pending {
            lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                reason: AbortedAttempt::Cancelled,
            });
            return AttemptDisposition::Aborted(AbortedAttempt::Cancelled);
        }

        let receipt = match seal {
            Some(Ok(receipt)) => receipt,
            Some(Err(_)) | None => {
                lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                    reason: AbortedAttempt::Transport(TransportFailure::Closed),
                });
                return AttemptDisposition::Aborted(AbortedAttempt::Transport(
                    TransportFailure::Closed,
                ));
            }
        };
        let kind = if lifecycle.mailbox.reconnects() {
            HandshakeKind::Reconnect
        } else {
            HandshakeKind::Fresh
        };
        lifecycle.login_succeeded(command, kind);
        AttemptDisposition::Promoted(receipt)
    }

    /// Unit-level lifecycle reducer for non-transport intent regressions.
    #[cfg(test)]
    pub(super) fn finish_attempt(
        &self,
        command: IntentCommand,
        world_generation: u64,
        deadline: Instant,
        cancel: &LoginCancel,
        result: &Result<(), LoginError>,
    ) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        lifecycle.attempt = None;
        lifecycle.attempt_generation = None;
        lifecycle.promotion_pending = None;
        lifecycle.last_queue = None;
        let cancelled = cancel.is_cancelled();
        let expired = Instant::now() >= deadline;
        let owned = !self.stop.load(Ordering::Acquire)
            && lifecycle
                .mailbox
                .attempt_is_current(command, world_generation);
        let current = owned && !cancelled && !expired;
        if !current {
            let reason = if cancelled || !owned {
                AbortedAttempt::Cancelled
            } else if result.is_ok() && expired {
                AbortedAttempt::LateSuccess
            } else {
                AbortedAttempt::Deadline
            };
            lifecycle.slot_event(SessionEventOut::HandshakeAborted { reason });
        } else if result.is_ok() {
            let kind = if lifecycle.mailbox.reconnects() {
                HandshakeKind::Reconnect
            } else {
                HandshakeKind::Fresh
            };
            lifecycle.login_succeeded(command, kind);
        } else if let Err(error) = result {
            if let Some(transport) = error.transport.as_ref() {
                lifecycle.slot_event(SessionEventOut::HandshakeAborted {
                    reason: AbortedAttempt::Transport(TransportFailure::classify(transport)),
                });
            } else if let Ok(code) = u8::try_from(error.code) {
                lifecycle.slot_event(SessionEventOut::HandshakeRefused {
                    code,
                    retry_after: error.retry_after,
                });
            }
        }
        current || (owned && !cancelled && result.is_err())
    }

    /// Classify and settle one client exit using the same K4 snapshot as
    /// concurrent Logout/Stop intent dispatch.
    fn observe_client_exit(
        &self,
        cause: SessionEndCause,
        resumable: bool,
        host: &str,
        port: u16,
        now: Instant,
    ) -> (
        bool,
        Option<LoginLatchReason>,
        Option<UnexpectedLogoutRecord>,
    ) {
        let mut lifecycle = self.lifecycle.lock();
        let stopping = self.stop.load(Ordering::Acquire);
        let snapshot = lifecycle.mailbox.snapshot();
        let latch = snapshot.latch().map(|reason| match reason {
            LoginLatch::OperatorLogout => LoginLatchReason::OperatorLogout,
            LoginLatch::PersistedLogout => LoginLatchReason::PersistedLogout,
            LoginLatch::RepeatGuard {
                count,
                window_seconds,
            } => LoginLatchReason::RepeatedUnexpectedLogouts {
                count,
                window_seconds,
            },
        });
        if !lifecycle.mailbox.session_online() {
            // An offline client cannot mint another session boundary, but a
            // later dirty transport observation invalidates clean-release
            // evidence retained from its last boundary.
            if !cause.is_clean() {
                lifecycle.clean_exit = None;
            }
            return (stopping, latch, None);
        }
        lifecycle.clean_exit = cause.is_clean().then(|| CleanExit {
            observed: now,
            not_before: now + host::login_queue::LOGIN_START_SPACING,
            host: host.to_owned(),
            port,
            uid: self.uid.load(Ordering::Relaxed),
            world_generation: snapshot.world_generation(),
        });
        let repeat = if !stopping && cause.counts_toward_repeat_guard() {
            lifecycle.record_unexpected_logout_at(now)
        } else {
            None
        };
        lifecycle.observe_exit(cause, resumable);
        (stopping, latch, repeat)
    }

    #[cfg(test)]
    fn record_session_exit_at(&self, clean: bool, host: &str, port: u16, now: Instant) {
        let mut lifecycle = self.lifecycle.lock();
        lifecycle.clean_exit = clean.then(|| CleanExit {
            observed: now,
            not_before: now + host::login_queue::LOGIN_START_SPACING,
            host: host.to_owned(),
            port,
            uid: self.uid.load(Ordering::Relaxed),
            world_generation: lifecycle.mailbox.snapshot().world_generation(),
        });
    }

    pub(super) fn clean_release_admission_at(
        &self,
        host: &str,
        port: u16,
        now: Instant,
    ) -> Option<Instant> {
        let mut lifecycle = self.lifecycle.lock();
        let snapshot = lifecycle.mailbox.snapshot();
        let exit = lifecycle.clean_exit.as_mut()?;
        if !snapshot.login_wanted()
            || self.stop.load(Ordering::Acquire)
            || exit.host != host
            || exit.port != port
            || exit.uid != self.uid.load(Ordering::Relaxed)
            || exit.world_generation != snapshot.world_generation()
            || now.saturating_duration_since(exit.observed) >= CLEAN_RELEASE_BUDGET
        {
            lifecycle.clean_exit = None;
            return None;
        }
        Some(exit.not_before)
    }

    pub(super) fn defer_clean_release_at(
        &self,
        error: &LoginError,
        command: IntentCommand,
        host: &str,
        port: u16,
        now: Instant,
    ) -> Option<Duration> {
        if error.code != 5 {
            return None;
        }
        self.clean_release_admission_at(host, port, now)?;
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.mailbox.snapshot().cancellation() != command.cancellation() {
            return None;
        }
        let exit = lifecycle.clean_exit.as_mut()?;
        let next = now + CLEAN_RELEASE_CADENCE;
        if next >= exit.observed + CLEAN_RELEASE_BUDGET {
            lifecycle.clean_exit = None;
            return None;
        }
        exit.not_before = next;
        Some(CLEAN_RELEASE_CADENCE)
    }

    /// The credential value and its invalidation share K4 with publication.
    /// A start cannot authorize old credentials against a newly saved value.
    fn sync_login_identity(&self, uid: i32, password: &str) {
        let (cancel, prequeue_cancel) = {
            #[cfg(test)]
            let mut lifecycle = K4CancelGuard::new(
                self.lifecycle.lock(),
                self.assert_cancel_order.load(Ordering::Relaxed),
            );
            #[cfg(not(test))]
            let mut lifecycle = self.lifecycle.lock();
            let uid_changed = self.uid.swap(uid, Ordering::Relaxed) != uid;
            let password_changed = {
                let mut current = self.password.lock();
                if **current != *password {
                    *current = Arc::from(password);
                    true
                } else {
                    false
                }
            };
            if !uid_changed && !password_changed {
                return;
            }
            let receipt = lifecycle
                .mailbox
                .post(Intent::LoginIdentityChanged)
                .expect("slot identity change generation exhausted");
            if uid_changed {
                lifecycle.clean_exit = None;
            }
            lifecycle.last_queue = None;
            lifecycle.slot_event(SessionEventOut::IntentApplied {
                generation: receipt.generation(),
            });
            (lifecycle.attempt.clone(), lifecycle.prequeue_cancel.take())
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        if let Some(cancel) = prequeue_cancel {
            cancel.cancel();
        }
        self.retry_wake.notify_all();
    }

    pub(super) fn login_password(&self) -> Arc<str> {
        Arc::clone(&self.password.lock())
    }
}

pub(super) fn sync_profile_arm(arm: &SlotArm, profile: &Profile) {
    api::hostlog::register_secret(&profile.password);
    arm.sync_login_identity(profile.uid, &profile.password);
    arm.random_events
        .store(profile.settings.random_events, Ordering::Relaxed);
    arm.lamp_auto
        .store(profile.settings.lamp_auto, Ordering::Relaxed);
    *arm.lamp_skill.lock().unwrap() = profile.settings.lamp_skill.clone();
    *arm.random_event_policies.lock().unwrap() = profile.settings.random_event_policies.clone();
    arm.set_cure_poison(profile.settings.survival.cure_poison);
    arm.post_intent(Intent::WorldPreference(match profile.settings.world {
        Some(number) => IntentWorldChoice::World(number),
        None => IntentWorldChoice::Automatic,
    }));
}

/// Whether the slot may start a login handshake: on the title (not ingame)
/// and the arm wants a login that is not latched by an intentional logout.
pub(super) fn should_handshake(arm: &SlotArm, ingame: bool) -> bool {
    arm.login_command(ingame).is_some()
}

/// Capture the command and world fence together before resolution/queue
/// admission. A subsequent Logout→Login cannot authorize this old attempt.
pub(super) fn capture_login_authorization(
    arm: &SlotArm,
    ingame: bool,
) -> Option<(IntentCommand, u64)> {
    let lifecycle = arm.lifecycle.lock();
    lifecycle
        .mailbox
        .login_command(ingame)
        .filter(|_| !arm.stop.load(Ordering::Acquire))
        .map(|command| (command, lifecycle.mailbox.snapshot().world_generation()))
}

/// A profile edit can land while this slot waits in the FIFO. `false`: the
/// caller drops its unused grant so the next pass can configure the new
/// endpoint before the socket handshake starts.
pub(super) fn granted_permit_world_is_current(
    round: Option<&public_worlds::WorldRound>,
    arm: &SlotArm,
) -> bool {
    round.is_none_or(|round| round.preference() == arm.world_choice())
}

/// One exact adapter identity and its queue-owned cancellation, retirement,
/// and optional initial-TLS proof cells.
#[derive(Debug, Clone)]
pub(crate) struct QueueAttemptCells {
    identity: QueueAttemptId,
    cancel: CancelToken,
    retirement: Arc<AttemptRetire>,
    release_proof: Option<Arc<QueueReleaseProof>>,
}

/// Frozen transport constraints used at the queue-start boundary.
struct AttemptStartPolicy<F> {
    tls_deadline: Option<Instant>,
    hook_identity: Option<HostAttemptIdentity>,
    selected_intent: F,
}

#[cfg(test)]
pub(crate) fn synthetic_queue_attempt(owner: u64, binding_number: u64) -> QueueAttemptCells {
    let park = Arc::new(Park::new());
    let source = CancelSource::new(Arc::clone(&park));
    let cancel = source.child().expect("test cancellation generation");
    let retirement = Arc::new(AttemptRetire::new(cancel.generation()));
    let binding = BindingId {
        incarnation: 1,
        connection: binding_number,
        binding: binding_number,
        attempt: cancel.generation(),
    };
    QueueAttemptCells {
        identity: QueueAttemptId::new(owner, binding, cancel.generation(), None),
        cancel,
        retirement,
        release_proof: None,
    }
}

impl QueueAttemptCells {
    #[cfg(test)]
    pub(crate) fn identity(&self) -> QueueAttemptId {
        self.identity
    }

    #[cfg(test)]
    pub(crate) fn cancel(&self) -> CancelToken {
        self.cancel.clone()
    }

    #[cfg(test)]
    pub(crate) fn start(&self, queue: &mut LoginQueue, now: Instant) -> Option<Instant> {
        queue.start_attempt(
            self.identity,
            now,
            self.cancel.clone(),
            Arc::clone(&self.retirement),
            self.release_proof.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn mark_physical_release(&self) {
        assert!(self.retirement.ack(), "test physical release ack is unique");
    }

    #[cfg(test)]
    pub(crate) fn acknowledge_after_physical_release(
        &self,
        queue: &mut LoginQueue,
        now: Instant,
    ) -> bool {
        assert!(
            self.retirement.acknowledged(),
            "physical release must be acknowledged before acquiring K1"
        );
        queue.acknowledge_retirement(self.identity, &self.retirement, now)
    }
}

#[cfg(test)]
impl LoginAttemptReturn<(), LoginError> {
    pub(crate) fn result(&self) -> &Result<(), LoginError> {
        &self.result
    }
}

#[cfg(test)]
impl<T, E> LoginAttemptReturn<T, E> {
    pub(crate) fn acknowledge_after_physical_release(&self, now: Instant) -> bool {
        assert!(self.retirement.ack(), "test physical release ack is unique");
        self.queue
            .lock()
            .acknowledge_retirement(self.identity, &self.retirement, now)
    }
    pub(crate) fn test_result(&self) -> &Result<T, E> {
        &self.result
    }
    pub(crate) fn into_test_result(self) -> Result<T, E> {
        self.result
    }
}

/// Return facts retained across the Q→R→C handoff. Returning a live queue
/// lease is not physical retirement.
pub(super) struct LoginAttemptReturn<T, E> {
    result: Result<T, E>,
    accepted: bool,
    deadline: Instant,
    cancel: LoginCancel,
    identity: QueueAttemptId,
    retirement: Arc<AttemptRetire>,
    queue: SharedLoginQueue,
}

enum AttemptDisposition {
    Promoted(SealReceipt),
    CurrentFailure,
    Aborted(AbortedAttempt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlotLoginOutcome {
    Promoted,
    Aborted(AbortedAttempt),
    CommittedSessionExit(SessionEndCause),
}

/// Owns one granted login reservation from the granting poll on (it is
/// built under the queue lock that granted it). Dropped unused, it gives the
/// grant back; after start, it returns only the exact flight and never
/// acknowledges physical retirement on behalf of the transport.
pub(super) struct GrantedReservation {
    queue: SharedLoginQueue,
    owner: u64,
    state: ReservationState,
    tls_ticket: Option<GameTlsTicket>,
    tls_park: Option<Arc<Park>>,
    network_health: Option<Arc<ReactorHealth>>,
}

#[derive(Debug)]
enum ReservationState {
    Unused,
    Attempted {
        identity: QueueAttemptId,
        retirement: Arc<AttemptRetire>,
    },
    Resolved,
}

impl std::fmt::Debug for GrantedReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrantedReservation")
            .field("owner", &self.owner)
            .field("state", &self.state)
            .finish()
    }
}

impl GrantedReservation {
    pub(super) fn new(queue: &SharedLoginQueue, owner: u64) -> Self {
        Self {
            queue: Arc::clone(queue),
            owner,
            state: ReservationState::Unused,
            tls_ticket: None,
            tls_park: None,
            network_health: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_owner(&self) -> u64 {
        self.owner
    }
    fn wait_for_ineligible_tls_ticket(
        &self,
        ticket: &GameTlsTicket,
        selected_intent: &impl Fn() -> bool,
    ) {
        ticket.release_for_insufficient_time();
        let park = self
            .tls_park
            .as_ref()
            .expect("WSS reservation carries its slot Park");
        let health = self
            .network_health
            .as_ref()
            .expect("WSS reservation carries reactor health");
        let _ = wss_wait_for_deadline_or_cancel(ticket, park, selected_intent, health, None);
    }

    /// Publish K4 authority, start this exact prepared transport attempt
    /// under K1, run one scoped Client::login, and return Phase Q facts.
    pub(super) fn attempt<T, E>(
        &mut self,
        mut prepared: PreparedGameAttempt,
        selected_intent: impl Fn() -> bool,
        publish: impl FnOnce(LoginCancel, u64) -> bool,
        login: impl FnOnce(Instant, LoginCancel) -> Result<T, E>,
        server_hold: impl FnOnce(&Result<T, E>) -> Option<Duration>,
    ) -> Option<LoginAttemptReturn<T, E>> {
        let mut tls_deadline = None;
        if let Some(ticket) = self.tls_ticket.take() {
            let deadline = ticket.deadline();
            match ticket.poll(Instant::now()) {
                GameTlsWait::Ready => {}
                GameTlsWait::InsufficientRemainingTime { .. } => {
                    self.state = ReservationState::Resolved;
                    let _ = self.queue.lock().abandon_permit(self.owner);
                    self.wait_for_ineligible_tls_ticket(&ticket, &selected_intent);
                    return None;
                }
                GameTlsWait::Deadline | GameTlsWait::Cancelled | GameTlsWait::Waiting => {
                    self.state = ReservationState::Resolved;
                    let _ = self.queue.lock().abandon_permit(self.owner);
                    return None;
                }
            }
            if prepared.set_tls_ticket(ticket).is_err() {
                self.state = ReservationState::Resolved;
                let _ = self.queue.lock().abandon_permit(self.owner);
                return None;
            }
            tls_deadline = Some(deadline);
        }
        if prepared.refresh_wss_transfer().is_err() {
            self.state = ReservationState::Resolved;
            let _ = self.queue.lock().abandon_permit(self.owner);
            return None;
        }

        let cancel = prepared.token();
        let retirement = prepared.retirement();
        let release_proof = prepared.queue_release_proof();
        let transfer_seq = release_proof.as_ref().map(|proof| proof.transfer_seq());
        let identity = QueueAttemptId::new(
            self.owner,
            prepared.binding(),
            cancel.generation(),
            transfer_seq,
        );
        let hook_identity = prepared.hook_identity(self.owner, transfer_seq);
        let cells = QueueAttemptCells {
            identity,
            cancel: cancel.clone(),
            retirement: Arc::clone(&retirement),
            release_proof,
        };
        self.attempt_with_cells(
            cells,
            Some(prepared),
            AttemptStartPolicy {
                tls_deadline,
                hook_identity: Some(hook_identity),
                selected_intent,
            },
            publish,
            move |prepared, deadline, cancel| {
                let prepared = prepared.expect("prepared game attempt is retained until start");
                with_prepared_game_attempt(prepared, || login(deadline, cancel))
            },
            server_hold,
        )
    }

    #[cfg(test)]
    pub(crate) fn attempt_with_test_cells<T, E>(
        &mut self,
        cells: QueueAttemptCells,
        publish: impl FnOnce(LoginCancel, u64) -> bool,
        login: impl FnOnce(Instant, LoginCancel) -> Result<T, E>,
        server_hold: impl FnOnce(&Result<T, E>) -> Option<Duration>,
    ) -> Option<LoginAttemptReturn<T, E>> {
        self.attempt_with_cells(
            cells,
            None,
            AttemptStartPolicy {
                tls_deadline: None,
                hook_identity: None,
                selected_intent: || false,
            },
            publish,
            move |_, deadline, cancel| login(deadline, cancel),
            server_hold,
        )
    }

    fn attempt_with_cells<T, E, F>(
        &mut self,
        cells: QueueAttemptCells,
        mut prepared: Option<PreparedGameAttempt>,
        start_policy: AttemptStartPolicy<F>,
        publish: impl FnOnce(LoginCancel, u64) -> bool,
        login: impl FnOnce(Option<PreparedGameAttempt>, Instant, LoginCancel) -> Result<T, E>,
        server_hold: impl FnOnce(&Result<T, E>) -> Option<Duration>,
    ) -> Option<LoginAttemptReturn<T, E>>
    where
        F: Fn() -> bool,
    {
        let AttemptStartPolicy {
            tls_deadline,
            hook_identity,
            selected_intent,
        } = start_policy;
        let login_cancel = LoginCancel::new();
        if let Some(identity) = hook_identity {
            login_cancel.set_host_attempt_identity(identity);
        }
        if !publish(login_cancel.clone(), cells.identity.generation()) {
            self.state = ReservationState::Resolved;
            let _ = self.queue.lock().abandon_permit(self.owner);
            return None;
        }
        let adapter_cancel = cells.cancel.clone();
        login_cancel.register_cancel_callback(move || adapter_cancel.cancel());
        if selected_intent() {
            self.state = ReservationState::Resolved;
            let _ = self.queue.lock().abandon_permit(self.owner);
            return None;
        }

        let (deadline, capacity_refusal, unavailable, cancelled) = {
            let mut q = self.queue.lock();
            let now = Instant::now();
            let capacity_refusal = tls_deadline.and_then(|deadline| {
                let remaining = deadline.saturating_duration_since(now);
                (remaining < MIN_WSS_START_REMAINING).then_some(
                    GameTlsWait::InsufficientRemainingTime {
                        remaining,
                        minimum: MIN_WSS_START_REMAINING,
                    },
                )
            });
            if cells.cancel.is_cancelled() {
                let _ = q.abandon_permit(self.owner);
                (None, None, false, true)
            } else if self
                .network_health
                .as_ref()
                .is_some_and(|health| health.loss().is_some())
            {
                let _ = q.abandon_permit(self.owner);
                (None, None, true, false)
            } else if capacity_refusal.is_some() {
                let _ = q.abandon_permit(self.owner);
                (None, capacity_refusal, false, false)
            } else {
                let deadline = q
                    .start_attempt(
                        cells.identity,
                        now,
                        cells.cancel.clone(),
                        Arc::clone(&cells.retirement),
                        cells.release_proof.clone(),
                    )
                    .map(|ordinary_deadline| {
                        tls_deadline.map_or(ordinary_deadline, |capacity_deadline| {
                            ordinary_deadline.min(capacity_deadline)
                        })
                    });
                (deadline, None, false, false)
            }
        };
        if cancelled {
            self.state = ReservationState::Resolved;
            return None;
        }
        if unavailable {
            self.state = ReservationState::Resolved;
            return None;
        }
        if matches!(
            capacity_refusal,
            Some(GameTlsWait::InsufficientRemainingTime { .. })
        ) {
            self.state = ReservationState::Resolved;
            if let Some(prepared) = prepared.as_mut() {
                if let Some(ticket) = prepared.take_tls_ticket() {
                    self.wait_for_ineligible_tls_ticket(&ticket, &selected_intent);
                }
            }
            return None;
        }
        let Some(deadline) = deadline else {
            self.state = ReservationState::Resolved;
            let _ = self.queue.lock().abandon_permit(self.owner);
            if cells.cancel.is_cancelled() {
                return None;
            }
            host_log!(
                Category::Login,
                Level::Warn,
                "login grant or transport identity was refused before start"
            );
            return None;
        };

        self.state = ReservationState::Attempted {
            identity: cells.identity,
            retirement: Arc::clone(&cells.retirement),
        };
        let result = login(prepared.take(), deadline, login_cancel.clone());
        let hold = server_hold(&result);
        let now = Instant::now();
        let returned = self.queue.lock().return_attempt(cells.identity, now, hold);
        self.state = ReservationState::Resolved;
        for effect in returned.cancels {
            effect.cancel();
        }
        if cells.retirement.acknowledged() {
            let _ =
                self.queue
                    .lock()
                    .acknowledge_retirement(cells.identity, &cells.retirement, now);
        }
        if !returned.accepted {
            host_log!(
                Category::Login,
                Level::Warn,
                "login returned after its in-flight lease ran out"
            );
        }
        Some(LoginAttemptReturn {
            result,
            accepted: returned.accepted,
            deadline,
            cancel: login_cancel,
            identity: cells.identity,
            retirement: cells.retirement,
            queue: Arc::clone(&self.queue),
        })
    }
}

impl Drop for GrantedReservation {
    fn drop(&mut self) {
        match &self.state {
            ReservationState::Unused => {
                let _ = self.queue.lock().abandon_permit(self.owner);
            }
            ReservationState::Attempted {
                identity,
                retirement,
            } => {
                let now = Instant::now();
                let returned = self.queue.lock().return_attempt(*identity, now, None);
                for effect in returned.cancels {
                    effect.cancel();
                }
                if retirement.acknowledged() {
                    let _ = self
                        .queue
                        .lock()
                        .acknowledge_retirement(*identity, retirement, now);
                }
            }
            ReservationState::Resolved => {}
        }
    }
}

/// The shared hold a response 16 (too many login attempts) puts on every
/// sibling's next attempt: this slot's next throttle delay, which its own
/// retry wait then takes ([`login_retry_wait`]).
pub(super) fn server_hold<T>(
    result: &Result<T, LoginError>,
    backoff: &LoginBackoff,
) -> Option<Duration> {
    match result {
        Err(error) if error.code == 16 => Some(backoff.next_throttle_delay()),
        _ => None,
    }
}

/// The one login call before queue return. Success remains a candidate until
/// `finish_after_q` performs R and publishes C under K4; adapter commit follows unlocked.
pub(super) struct SlotLogin<'a> {
    pub client: &'a mut Client,
    pub arm: &'a SlotArm,
    pub command: IntentCommand,
    pub world_generation: u64,
    pub username: &'a str,
    pub password: &'a str,
    pub reconnect: bool,
    pub backoff: &'a LoginBackoff,
}

impl SlotLogin<'_> {
    pub(super) fn run(
        &mut self,
        deadline: Instant,
        cancel: LoginCancel,
        after_return: impl FnOnce(&Client),
    ) -> (Result<(), LoginError>, Option<Duration>) {
        if cancel.is_cancelled()
            || !self
                .arm
                .login_attempt_is_current(self.command, self.world_generation)
        {
            return (Ok(()), None);
        }
        self.client.set_login_deadline(deadline);
        self.client.set_login_cancel(cancel);
        let result = self
            .client
            .login(self.username, self.password, self.reconnect);
        let hold = server_hold(&result, self.backoff);
        after_return(self.client);
        (result, hold)
    }

    pub(super) fn finish_after_q(
        &mut self,
        outcome: LoginAttemptReturn<(), LoginError>,
    ) -> Result<SlotLoginOutcome, LoginError> {
        let generation = outcome.identity.generation();
        let seal = if outcome.accepted
            && outcome.result.is_ok()
            && self.arm.may_seal_after_q(
                self.command,
                self.world_generation,
                outcome.deadline,
                &outcome.cancel,
                generation,
                outcome.accepted,
            ) {
            Some(self.client.seal_host_login())
        } else {
            None
        };
        let disposition = self.arm.finish_attempt_after_q(
            self.command,
            self.world_generation,
            outcome.deadline,
            &outcome.cancel,
            generation,
            outcome.accepted,
            &outcome.result,
            seal,
        );
        match disposition {
            AttemptDisposition::Promoted(receipt) => {
                let committed = self.client.commit_host_login(receipt).is_ok();
                if committed {
                    acknowledge_committed_session_retirement(&outcome);
                    Ok(SlotLoginOutcome::Promoted)
                } else {
                    self.client.close_failed_host_commit_as_session();
                    self.arm
                        .observe_session_exit(SessionEndCause::ConnectionLost, false);
                    acknowledge_retired_attempt(&outcome);
                    Ok(SlotLoginOutcome::CommittedSessionExit(
                        SessionEndCause::ConnectionLost,
                    ))
                }
            }
            AttemptDisposition::CurrentFailure => {
                abort_login_transport(self.client);
                acknowledge_retired_attempt(&outcome);
                Err(outcome
                    .result
                    .expect_err("current failure has a login error"))
            }
            AttemptDisposition::Aborted(reason) => {
                abort_login_transport(self.client);
                acknowledge_retired_attempt(&outcome);
                Ok(SlotLoginOutcome::Aborted(reason))
            }
        }
    }
}
fn acknowledge_committed_session_retirement<T, E>(outcome: &LoginAttemptReturn<T, E>) {
    let _ = outcome.retirement.ack();
    acknowledge_retired_attempt(outcome);
}

fn acknowledge_retired_attempt<T, E>(outcome: &LoginAttemptReturn<T, E>) {
    if outcome.retirement.acknowledged() {
        let _ = outcome.queue.lock().acknowledge_retirement(
            outcome.identity,
            &outcome.retirement,
            Instant::now(),
        );
    }
}

fn abort_login_transport(client: &mut Client) {
    client.abort_host_login();
}

#[cfg(any(test, feature = "test-support"))]
impl SlotArm {
    /// Configure the next login's public world, as the slot does before its
    /// handshake.
    pub fn configure_world_for_test(&self, world: u16) {
        self.note_configured_world(world);
    }

    /// The current session's scene is built, as the slot's frame reports.
    pub fn scene_ready_for_test(&self) {
        self.update_session_readiness(true, false, false, false);
    }

    pub fn complete_login_for_test(&self) {
        if self.session_key().is_some() {
            self.observe_session_exit(SessionEndCause::ServerLogout, false);
        }
        let command = self.login_command(false).expect("test login wanted");
        let mut lifecycle = self.lifecycle.lock();
        let kind = if lifecycle.mailbox.reconnects() {
            HandshakeKind::Reconnect
        } else {
            HandshakeKind::Fresh
        };
        lifecycle.login_succeeded(command, kind);
    }

    /// The session drops and the repeat guard latches automatic logins off,
    /// as the slot reports after repeated unexpected logouts: no command is
    /// posted.
    pub fn exit_and_trip_repeat_guard_for_test(&self) {
        self.observe_session_exit(SessionEndCause::ConnectionLost, false);
        self.lifecycle
            .lock()
            .mailbox
            .observe(LifecycleFact::RepeatGuardTripped {
                count: UNEXPECTED_LOGOUT_THRESHOLD as u8,
                window_seconds: UNEXPECTED_LOGOUT_WINDOW.as_secs(),
            })
            .expect("test repeat-guard generation exhausted");
    }
}

#[cfg(test)]
pub(super) fn on_login_success(arm: &SlotArm, command: IntentCommand) {
    let mut lifecycle = arm.lifecycle.lock();
    let kind = if lifecycle.mailbox.reconnects() {
        HandshakeKind::Reconnect
    } else {
        HandshakeKind::Fresh
    };
    lifecycle.login_succeeded(command, kind);
}

/// Reduce the client's exit reason before any slot boundary retires its
/// session. The observer and the per-frame flag probe may both see the exit,
/// but `take_session_exit_reason` gives exactly one of them the fact.
pub(super) fn observe_client_session_exit(client: &mut Client, arm: &SlotArm) {
    if let Some(reason) = client.take_session_exit_reason() {
        let cause = SessionEndCause::classify(&reason);
        let (stopping, latch, repeat) = arm.observe_client_exit(
            cause,
            client.session_resumable(),
            &client.config.host,
            client.config.port,
            Instant::now(),
        );
        match (stopping, latch) {
            // `signal_slot_stop` logged the removal transition when it armed
            // Stop. A coincident transport observation is not a second exit.
            (true, _) => {}
            (false, Some(LoginLatchReason::OperatorLogout)) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Info,
                    "session exit reason=operator_logout transport={reason}"
                );
            }
            (false, _) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "session exit reason={reason}"
                );
                if let Some(record) = repeat {
                    if record.latched {
                        host_log!(
                            Category::Login,
                            Level::Warn,
                            "relog decision repeat_guard count={} window_s={}",
                            record.count,
                            UNEXPECTED_LOGOUT_WINDOW.as_secs()
                        );
                    } else {
                        host_log!(
                            Category::Login,
                            Level::Info,
                            "unexpected logout count={} window_s={}",
                            record.count,
                            UNEXPECTED_LOGOUT_WINDOW.as_secs()
                        );
                    }
                }
            }
        }
    }
}

/// Per-frame arm handling in the 20 ms body: press the CC_LOGOUT iface when
/// the panel armed a logout on an ingame slot, then report whether the
/// thread must stop (rail ✕). Probe order: logout press returns `false`
/// (keep running until `!ingame`); only then may `stop` end the body. The
/// press is the only place a clean logout can go out while the slot is
/// inside [`Host::run_client`].
pub(super) fn tick_flags(
    client: &mut Client,
    ifaces: &[Option<Box<IfType>>],
    arm: &SlotArm,
) -> bool {
    observe_client_session_exit(client, arm);
    if let Some(command) = arm.logout_command(client.ingame) {
        if !api::interact::logout(client, ifaces) {
            // Missing/refused IF is still pending. A removal deadline may set
            // Stop, in which case fall back to a dirty disconnect rather than
            // reporting a logout that was never sent.
            return arm.stop.load(Ordering::Relaxed);
        }
        arm.acknowledge_logout(command);
        // Do not honor `stop` on the same probe as the logout press — the
        // body must keep running until the client leaves the game.
        return false;
    }
    arm.stop.load(Ordering::Relaxed)
}

impl Play {
    /// CLI-only preference for accounts whose stored world is auto.
    pub fn set_auto_world(&mut self, number: u16) -> Result<(), String> {
        let worlds = self
            .connection
            .profile()
            .and_then(|p| p.public_worlds())
            .ok_or("--world requires the rs2b2t profile")?;
        if worlds.by_number(number).is_none() {
            return Err(format!(
                "world {number} is not in the configured public worlds"
            ));
        }
        self.auto_world = Some(number);
        Ok(())
    }
}

pub(super) fn transfer_countdown_message(remaining: u64) -> String {
    format!("Your profile will be transferred in: {remaining} seconds")
}

fn publish_transfer_countdown(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, remaining: u64) {
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_phase = StartupPhase::Connecting;
        s.startup_phase_started = Instant::now();
        s.error = None;
        s.startup_progress_percent = None;
        s.startup_progress_message = transfer_countdown_message(remaining);
    }
}

/// Handle a typed response-21 cooldown before generic world/error policy.
/// `Some` means the response was consumed and the caller must retry the same
/// endpoint; `None` leaves non-21 or malformed errors to normal handling.
pub(super) fn wait_for_transfer_response(
    error: &LoginError,
    arm: &SlotArm,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    countdown: &Cell<Option<u64>>,
    frames: &mut WaitFrames<'_>,
) -> Option<bool> {
    if error.code != 21 {
        return None;
    }
    let delay = error.retry_after?;
    let mut remaining = delay.as_secs();
    loop {
        countdown.set(Some(remaining));
        publish_transfer_countdown(statuses, name, remaining);
        if !arm.wait_for_transfer(Duration::from_secs(1), frames) {
            clear_startup_progress(statuses, name);
            return Some(false);
        }
        if remaining == 0 {
            break;
        }
        remaining -= 1;
    }
    clear_startup_progress(statuses, name);
    Some(true)
}

pub(super) fn refresh_slot_world_preference(
    round: &mut public_worlds::WorldRound,
    worlds: &public_worlds::PublicWorlds,
    arm: &SlotArm,
) -> Result<bool, String> {
    round.reselect_if_changed(worlds, arm.world_choice())
}

pub(super) fn configure_slot_world(
    client: &mut Client,
    world: &public_worlds::PublicWorld,
    refresh: bool,
    arm: &SlotArm,
) -> Result<(), String> {
    let wait_cancel = LoginCancel::new();
    if !arm.publish_key_fetch_cancel(&wait_cancel) {
        return Ok(());
    }
    let fetch_cancel = LoginCancel::new();
    let modulus = public_worlds::modulus_for(world, refresh, &wait_cancel, |host, port| {
        Client::fetch_login_modulus_for_cancellable(
            client::Transport::Wss,
            host,
            port,
            &fetch_cancel,
        )
    });
    arm.clear_key_fetch_cancel();
    if wait_cancel.is_cancelled() || arm.stop.load(Ordering::Acquire) {
        return Ok(());
    }
    client.set_public_world(&world.host, world.port, world.node_id, &modulus)
}

/// Delay before the next attempt after a failed login. A failed connection
/// and a full world back off with the slot's escalating, jittered delay
/// (`entropy` picks the jitter); response 16 escalates on its own schedule
/// and waits out the same delay [`server_hold`] published to its siblings.
pub(super) fn login_retry_wait(
    backoff: &mut LoginBackoff,
    error: &LoginError,
    entropy: u32,
) -> Duration {
    match error.code {
        1 => Duration::from_secs(2),
        16 => backoff.throttle_delay(),
        5 => Duration::from_secs(60),
        7 => backoff.retry_delay(entropy),
        _ if error.transport.is_some() => backoff.retry_delay(entropy),
        _ => Duration::from_secs(5),
    }
}

/// Whether the refused world was the operator's pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorldChoice {
    /// The profile's world is Auto: the slot tries the other public worlds.
    Auto,
    /// The profile names a world: the slot keeps it.
    Pinned,
    /// A local server profile, which has no world list.
    Local,
}

/// A world number (public worlds only) and its game endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorldEndpoint<'a> {
    pub(super) world: Option<u16>,
    pub(super) host: &'a str,
    pub(super) port: u16,
}

impl std::fmt::Display for WorldEndpoint<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(world) = self.world {
            write!(f, "w{world} ")?;
        }
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// One line explaining a world-full refusal (response 7) and what the slot
/// does next. In the engine, 7 means the world has no room: more than
/// `maxConnected` players (`World.ts:2227-2229`) or no free player slot
/// (`World.ts:883-888`); the login server's own refusals map to 11 instead
/// (`World.ts:1873-1877`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorldFullReceipt<'a> {
    pub(super) refused: WorldEndpoint<'a>,
    pub(super) choice: WorldChoice,
    /// Auto only: every world refused this round, so the slot waits.
    pub(super) round_full: bool,
    pub(super) next: WorldEndpoint<'a>,
    /// `None`: the next world is tried at once (through the login queue).
    pub(super) retry_in: Option<Duration>,
}

impl std::fmt::Display for WorldFullReceipt<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "world full (code 7) on {} (", self.refused)?;
        f.write_str(match self.choice {
            WorldChoice::Auto if self.round_full => "auto world; every world was full this round",
            WorldChoice::Auto => "auto world",
            WorldChoice::Pinned => "pinned world",
            WorldChoice::Local => "local server",
        })?;
        write!(f, "); next attempt {}", self.next)?;
        match self.retry_in {
            Some(delay) => write!(f, " in {:.1} s", delay.as_secs_f64()),
            None => f.write_str(" now, through the login queue"),
        }
    }
}

/// What a slot does after a failed login (response 21 is consumed by the
/// transfer countdown before this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RetryPlan<'a> {
    /// Configure the round's next world before the next attempt.
    pub(super) switch_world: bool,
    /// Wait before re-entering the login queue; `None` re-enters at once.
    pub(super) wait: Option<Duration>,
    /// Present for response 7.
    pub(super) world_full: Option<WorldFullReceipt<'a>>,
}

/// Plan the retry after `error`. `round` and `worlds` are the slot's public
/// world rotation (absent for a local profile, whose endpoint is `local`).
/// An Auto profile moves on to the next world at once and waits only after a
/// full round; a pinned or local profile keeps its endpoint and waits.
pub(super) fn plan_login_retry<'a>(
    error: &LoginError,
    public: Option<(
        &mut public_worlds::WorldRound,
        &'a public_worlds::PublicWorlds,
    )>,
    local: (&'a str, u16),
    backoff: &mut LoginBackoff,
    entropy: u32,
) -> RetryPlan<'a> {
    let endpoint = |world: &'a public_worlds::PublicWorld| WorldEndpoint {
        world: Some(world.number),
        host: &world.host,
        port: world.port,
    };
    let (step, choice, refused, next) = match public {
        Some((round, worlds)) => {
            let refused = endpoint(&worlds.worlds[round.index]);
            let choice = if round.preference().is_some() {
                WorldChoice::Pinned
            } else {
                WorldChoice::Auto
            };
            let step = round.on_login_error(error.code, worlds.worlds.len());
            (step, choice, refused, endpoint(&worlds.worlds[round.index]))
        }
        None => {
            let local = WorldEndpoint {
                world: None,
                host: local.0,
                port: local.1,
            };
            (
                public_worlds::WorldErrorStep::Stay,
                WorldChoice::Local,
                local,
                local,
            )
        }
    };
    let switch_world = step != public_worlds::WorldErrorStep::Stay;
    let wait = (step != public_worlds::WorldErrorStep::SwitchNow)
        .then(|| login_retry_wait(backoff, error, entropy));
    RetryPlan {
        switch_world,
        wait,
        world_full: (error.code == 7).then_some(WorldFullReceipt {
            refused,
            choice,
            round_full: step == public_worlds::WorldErrorStep::SwitchAfterWait,
            next,
            retry_in: wait,
        }),
    }
}

/// Publish the current login latch and its reason onto the slot's status row.
/// A latch supersedes queue and retry activity until explicit Log in.
pub(super) fn publish_login_latched(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    reason: Option<LoginLatchReason>,
) {
    if let Some(s) = lock_statuses(statuses)
        .iter_mut()
        .find(|s| s.username == name)
    {
        s.login_latched = reason.is_some();
        s.login_latch_reason = reason;
        if reason.is_some() {
            s.error = None;
            s.login_started = None;
            s.queue_position = -1;
            s.queue_total = -1;
            s.queue_blocker = None;
            s.startup_progress_percent = None;
            s.startup_progress_message.clear();
        }
    }
}

pub(super) fn publish_login_latched_from_arm(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    arm: &SlotArm,
) {
    publish_login_latched(statuses, name, arm.login_latch_reason());
}

/// Publish a FIFO place (`None` clears it). The head's admission blocker is
/// cleared too; [`publish_queue_place`] republishes it from the queue.
pub(super) fn apply_queue_wait(rows: &mut [SlotStatus], name: &str, pos: Option<QueuePos>) {
    let (position, total) =
        match pos.filter(|p| p.position >= 1 && p.total >= 1 && p.position <= p.total) {
            Some(p) => (p.position as i32, p.total as i32),
            None => (-1, -1),
        };
    for s in rows.iter_mut().filter(|s| s.username == name) {
        s.queue_position = position;
        s.queue_total = total;
        s.queue_blocker = None;
    }
}

/// Publish `owner`'s place from the locked queue and return the same facts
/// as a typed event. The caller stamps it only after dropping the queue lock.
pub(super) fn publish_queue_place(
    q: &LoginQueue,
    rows: &mut [SlotStatus],
    name: &str,
    owner: u64,
) -> Option<SessionEventOut> {
    let pos = q.status_owner(owner);
    let blocker = pos.and_then(|_| q.head_blocker(owner, Instant::now()));
    apply_queue_wait(rows, name, pos);
    for s in rows.iter_mut().filter(|s| s.username == name) {
        s.queue_blocker = blocker;
    }
    pos.map(|pos| SessionEventOut::Queued {
        place: u16::try_from(pos.position).expect("queue place exceeds lifecycle vocabulary"),
        total: u16::try_from(pos.total).expect("queue size exceeds lifecycle vocabulary"),
        blocker: blocker.map(|blocker| match blocker {
            AdmitBlocker::Handshake => QueueBlocker::Handshake,
            AdmitBlocker::Retiring => QueueBlocker::Retiring,
            AdmitBlocker::Spacing { until } => QueueBlocker::Spacing {
                until: Deadline::at(until),
            },
            AdmitBlocker::AddressBudget { attempts, until } => QueueBlocker::AddressBudget {
                attempts,
                until: until.map(Deadline::at),
            },
            AdmitBlocker::DeviceBudget { until } => QueueBlocker::DeviceBudget {
                until: until.map(Deadline::at),
            },
            AdmitBlocker::ServerHold { until } => QueueBlocker::ServerHold {
                until: Deadline::at(until),
            },
        }),
    })
}

/// Refresh cadence for a waiting slot's published place. A window-blocked
/// head can sleep a whole 60 s deadline in one wait, so the card would
/// otherwise miss members that queue behind it. Read-only: the refresh
/// re-reads `status(uid)` and never requests a permit.
const QUEUE_PUBLISH: Duration = Duration::from_millis(200);

/// Outcome of a login queue admission. A WSS `Deadline` means the one initial
/// TLS demand reached its original D without an eligible queue start.
#[derive(Debug)]
pub(super) enum PermitWait {
    Granted(GrantedReservation),
    Cancelled,
    Deadline,
    Unavailable,
}

/// Enter this slot owner once and publish its authoritative place while the
/// queue lock prevents a concurrent grant/leave from overtaking the row
/// update.
pub(super) fn enqueue_queue_place(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
) {
    enqueue_queue_place_with_defer(queue, statuses, username, uid, arm, None);
}

fn enqueue_queue_place_with_defer(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
    not_before: Option<Instant>,
) {
    let event = {
        let mut q = queue.lock();
        q.enqueue_owner(arm.queue_owner, uid);
        if let Some(until) = not_before {
            q.defer_owner_until(arm.queue_owner, until);
        }
        publish_queue_place(&q, &mut lock_statuses(statuses), username, arm.queue_owner)
    };
    if let Some(event) = event {
        arm.publish_queued(event);
    }
}

/// Drop one slot owner's login-FIFO place and always clear this slot's
/// published `k of n`.
pub(super) fn drop_queue_place(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    owner: u64,
) {
    let mut q = queue.lock();
    q.leave_owner(owner);
    apply_queue_wait(&mut lock_statuses(statuses), username, None);
}

/// Retire a slot owner's queue eligibility and published place. This is
/// idempotent so explicit Stop cleanup and the worker exit guard may race.
pub(super) fn retire_queue_place(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    arm: &SlotArm,
) {
    queue.lock().leave_owner(arm.queue_owner);
    let mut rows = lock_statuses(statuses);
    apply_queue_wait(&mut rows, username, None);
}

/// Worker-scope guard: every return and unwind retires this worker's FIFO
/// owner, including panics in preparation, maininit, and permit waiting.
pub(super) struct QueuePlaceRetirement<'a> {
    pub(super) queue: &'a SharedLoginQueue,
    pub(super) statuses: &'a Arc<Mutex<Vec<SlotStatus>>>,
    pub(super) username: &'a str,
    pub(super) arm: &'a SlotArm,
}

impl Drop for QueuePlaceRetirement<'_> {
    fn drop(&mut self) {
        retire_queue_place(self.queue, self.statuses, self.username, self.arm);
    }
}

/// A withdrawn login or stopped slot never keeps a FIFO place. The mailbox,
/// rather than an auto/script policy calculation, owns the login intent.
pub(super) fn permit_wait_cancelled(arm: &SlotArm) -> bool {
    !arm.login_wanted() || arm.stop.load(Ordering::Acquire)
}

/// One fresh WSS demand before its ticket has moved into a granted reservation.
struct WssPermitDemand {
    ticket: Option<GameTlsTicket>,
    park: Arc<Park>,
    network_health: Arc<ReactorHealth>,
    identity: i32,
    command: IntentCommand,
    world_generation: u64,
}

fn wss_authorization_is_current(
    arm: &SlotArm,
    identity: i32,
    command: IntentCommand,
    world_generation: u64,
) -> bool {
    arm.uid.load(Ordering::Relaxed) == identity
        && !permit_wait_cancelled(arm)
        && arm.login_attempt_is_current(command, world_generation)
}

fn wss_wait_for_deadline_or_cancel(
    ticket: &GameTlsTicket,
    park: &Park,
    selected_intent: impl Fn() -> bool,
    network_health: &ReactorHealth,
    mut frames: Option<&mut WaitFrames<'_>>,
) -> PermitWait {
    loop {
        if selected_intent() {
            return PermitWait::Cancelled;
        }
        if network_health.loss().is_some() {
            return PermitWait::Unavailable;
        }
        match ticket.poll(Instant::now()) {
            GameTlsWait::Deadline => return PermitWait::Deadline,
            GameTlsWait::Cancelled => return PermitWait::Cancelled,
            _ => {}
        }
        let next_frame = frames
            .as_deref_mut()
            .map(|frames| frames())
            .unwrap_or(Duration::from_millis(100));
        let wait_slice = next_frame
            .max(Duration::from_millis(1))
            .min(Duration::from_millis(100));
        let now = Instant::now();
        let wake_at = ticket.deadline().min(now + wait_slice);
        match park.wait(
            wake_at,
            Instant::now,
            || network_health.loss().is_some(),
            || matches!(ticket.poll(Instant::now()), GameTlsWait::Cancelled),
            || matches!(ticket.poll(Instant::now()), GameTlsWait::Cancelled),
            || match ticket.poll(Instant::now()) {
                GameTlsWait::Deadline => Some(GameTlsWait::Deadline),
                GameTlsWait::Cancelled => Some(GameTlsWait::Cancelled),
                _ => None,
            },
            false,
            None,
        ) {
            ParkOutcome::Ready(GameTlsWait::Deadline) => return PermitWait::Deadline,
            ParkOutcome::Ready(GameTlsWait::Cancelled)
            | ParkOutcome::Revoked
            | ParkOutcome::SelectedIntent => return PermitWait::Cancelled,
            ParkOutcome::Unavailable => return PermitWait::Unavailable,
            ParkOutcome::Deadline => {}
            ParkOutcome::Ready(
                GameTlsWait::Ready
                | GameTlsWait::Waiting
                | GameTlsWait::InsufficientRemainingTime { .. },
            ) => {}
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Park, slot authorization, and frame observation must stay borrowed"
)]
fn wss_wait_for_ready(
    ticket: &GameTlsTicket,
    park: &Park,
    arm: &SlotArm,
    identity: i32,
    command: IntentCommand,
    world_generation: u64,
    network_health: &ReactorHealth,
    frames: &mut WaitFrames<'_>,
) -> Result<(), PermitWait> {
    loop {
        if !wss_authorization_is_current(arm, identity, command, world_generation) {
            return Err(PermitWait::Cancelled);
        }
        if network_health.loss().is_some() {
            return Err(PermitWait::Unavailable);
        }
        match ticket.poll(Instant::now()) {
            GameTlsWait::Ready => return Ok(()),
            GameTlsWait::Deadline => return Err(PermitWait::Deadline),
            GameTlsWait::Cancelled => return Err(PermitWait::Cancelled),
            GameTlsWait::InsufficientRemainingTime { .. } => {
                ticket.release_for_insufficient_time();
                return Err(wss_wait_for_deadline_or_cancel(
                    ticket,
                    park,
                    || !wss_authorization_is_current(arm, identity, command, world_generation),
                    network_health,
                    Some(frames),
                ));
            }
            GameTlsWait::Waiting => {}
        }
        let next_frame = frames()
            .max(Duration::from_millis(1))
            .min(Duration::from_millis(100));
        let now = Instant::now();
        let wake_at = ticket.deadline().min(now + next_frame);
        match park.wait(
            wake_at,
            Instant::now,
            || network_health.loss().is_some(),
            || {
                arm.stop.load(Ordering::Acquire)
                    || matches!(ticket.poll(Instant::now()), GameTlsWait::Cancelled)
            },
            || matches!(ticket.poll(Instant::now()), GameTlsWait::Cancelled),
            || match ticket.poll(Instant::now()) {
                GameTlsWait::Waiting => None,
                result => Some(result),
            },
            false,
            None,
        ) {
            ParkOutcome::Ready(GameTlsWait::Ready) => {
                if !wss_authorization_is_current(arm, identity, command, world_generation) {
                    return Err(PermitWait::Cancelled);
                }
                if network_health.loss().is_some() {
                    return Err(PermitWait::Unavailable);
                }
                return Ok(());
            }
            ParkOutcome::Ready(GameTlsWait::InsufficientRemainingTime { .. }) => {
                ticket.release_for_insufficient_time();
                return Err(wss_wait_for_deadline_or_cancel(
                    ticket,
                    park,
                    || !wss_authorization_is_current(arm, identity, command, world_generation),
                    network_health,
                    Some(frames),
                ));
            }
            ParkOutcome::Ready(GameTlsWait::Deadline) => return Err(PermitWait::Deadline),
            ParkOutcome::Ready(GameTlsWait::Cancelled)
            | ParkOutcome::Revoked
            | ParkOutcome::SelectedIntent => return Err(PermitWait::Cancelled),
            ParkOutcome::Unavailable => return Err(PermitWait::Unavailable),
            ParkOutcome::Ready(GameTlsWait::Waiting) | ParkOutcome::Deadline => {
                if Instant::now() >= ticket.deadline() {
                    return Err(PermitWait::Deadline);
                }
            }
        }
    }
}

/// Wait for initial TLS capacity before entering the login queue. The
/// caller keeps its K4-published prequeue token through this reservation.
#[expect(
    clippy::too_many_arguments,
    reason = "The WSS permit checks independent queue, identity, and cancellation owners"
)]
pub(super) fn wait_for_wss_permit(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
    command: IntentCommand,
    world_generation: u64,
    token: CancelToken,
    ticket: GameTlsTicket,
    network_health: Arc<ReactorHealth>,
    not_before: impl FnOnce() -> Option<Instant>,
    frames: &mut WaitFrames<'_>,
) -> PermitWait {
    let identity = arm.uid.load(Ordering::Relaxed);
    if uid != identity
        || !wss_authorization_is_current(arm, identity, command, world_generation)
        || !ticket.matches_token(&token)
    {
        return PermitWait::Cancelled;
    }
    if network_health.loss().is_some() {
        return PermitWait::Unavailable;
    }
    let park = arm.park();
    if let Err(wait) = wss_wait_for_ready(
        &ticket,
        &park,
        arm,
        identity,
        command,
        world_generation,
        &network_health,
        frames,
    ) {
        if !wss_authorization_is_current(arm, identity, command, world_generation) {
            return PermitWait::Cancelled;
        }
        if network_health.loss().is_some() {
            return PermitWait::Unavailable;
        }
        return wait;
    }
    if !wss_authorization_is_current(arm, identity, command, world_generation) {
        return PermitWait::Cancelled;
    }
    if network_health.loss().is_some() {
        return PermitWait::Unavailable;
    }
    let deferred_until = not_before();
    enqueue_queue_place_with_defer(queue, statuses, username, uid, arm, deferred_until);
    #[cfg(test)]
    arm.panic_after_queue_for_test();
    let mut demand = WssPermitDemand {
        ticket: Some(ticket),
        park,
        network_health,
        identity,
        command,
        world_generation,
    };
    wait_for_permit_inner(
        queue,
        statuses,
        username,
        uid,
        arm,
        frames,
        None,
        Some(&mut demand),
    )
}

/// Block until the already-enqueued slot owner receives a handshake permit,
/// mirroring the queue position onto the slot's status row while it waits
/// and keeping the logged-out title live through `frames`.
/// Withdrawal is observed before every poll, so a dropped place is never
/// recreated by this waiter.
#[cfg(test)]
pub(super) fn wait_for_permit(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
    frames: &mut WaitFrames<'_>,
) -> PermitWait {
    wait_for_permit_inner(queue, statuses, username, uid, arm, frames, None, None)
}

pub(super) fn wait_for_permit_with_cancel(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
    cancel: &CancelToken,
    frames: &mut WaitFrames<'_>,
) -> PermitWait {
    wait_for_permit_inner(
        queue,
        statuses,
        username,
        uid,
        arm,
        frames,
        Some(cancel),
        None,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Permit wait rechecks independent queue, identity, and cancellation owners"
)]
fn wait_for_permit_inner(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
    frames: &mut WaitFrames<'_>,
    cancel: Option<&CancelToken>,
    mut wss: Option<&mut WssPermitDemand>,
) -> PermitWait {
    let withdraw = || {
        drop_queue_place(queue, statuses, username, arm.queue_owner);
        host_log!(
            Category::Login,
            Level::Info,
            slot = username,
            "permit wait withdrawn"
        );
    };
    let identity = wss
        .as_deref()
        .map_or_else(|| arm.uid.load(Ordering::Relaxed), |demand| demand.identity);
    let world_generation = wss
        .as_deref()
        .map_or_else(|| arm.world_generation(), |demand| demand.world_generation);
    let replaced = || {
        arm.uid.load(Ordering::Relaxed) != identity || arm.world_generation() != world_generation
    };
    loop {
        if let Some(demand) = wss.as_deref() {
            if !wss_authorization_is_current(
                arm,
                demand.identity,
                demand.command,
                demand.world_generation,
            ) {
                drop_queue_place(queue, statuses, username, arm.queue_owner);
                return PermitWait::Cancelled;
            }
            if demand.network_health.loss().is_some() {
                drop_queue_place(queue, statuses, username, arm.queue_owner);
                return PermitWait::Unavailable;
            }
            let ticket = demand
                .ticket
                .as_ref()
                .expect("WSS demand retains its ticket");
            match ticket.poll(Instant::now()) {
                GameTlsWait::Ready => {}
                GameTlsWait::InsufficientRemainingTime { .. } => {
                    drop_queue_place(queue, statuses, username, arm.queue_owner);
                    ticket.release_for_insufficient_time();
                    return wss_wait_for_deadline_or_cancel(
                        ticket,
                        &demand.park,
                        || {
                            !wss_authorization_is_current(
                                arm,
                                demand.identity,
                                demand.command,
                                demand.world_generation,
                            )
                        },
                        &demand.network_health,
                        Some(frames),
                    );
                }
                GameTlsWait::Deadline => {
                    drop_queue_place(queue, statuses, username, arm.queue_owner);
                    return PermitWait::Deadline;
                }
                GameTlsWait::Cancelled => {
                    drop_queue_place(queue, statuses, username, arm.queue_owner);
                    return PermitWait::Cancelled;
                }
                GameTlsWait::Waiting => {
                    drop_queue_place(queue, statuses, username, arm.queue_owner);
                    return PermitWait::Cancelled;
                }
            }
        }
        // Retire the old FIFO wait without withdrawing its replacement intent.
        if cancel.is_some_and(CancelToken::is_cancelled) || replaced() {
            drop_queue_place(queue, statuses, username, arm.queue_owner);
            return PermitWait::Cancelled;
        }
        if permit_wait_cancelled(arm) {
            withdraw();
            return PermitWait::Cancelled;
        }
        // A cancelled release observation drops only its pacing, not the
        // slot's FIFO place. Reaffirming login/password edits keep both.
        let release_wait = arm.has_clean_release();
        if !release_wait {
            queue.lock().clear_owner_defer(arm.queue_owner);
        }
        let (polled, event) = {
            #[cfg(test)]
            let mut q = K1CancelGuard::new(
                queue.lock(),
                arm.assert_cancel_order.load(Ordering::Relaxed),
            );
            #[cfg(not(test))]
            let mut q = queue.lock();
            let polled = q.poll_owner(arm.queue_owner, uid, Instant::now());
            #[cfg(test)]
            q.watch(&polled.cancels);
            let event = if matches!(polled.permit, Permit::Wait(_)) {
                publish_queue_place(&q, &mut lock_statuses(statuses), username, arm.queue_owner)
            } else {
                None
            };
            (polled, event)
        };
        if let Some(event) = event {
            arm.publish_queued(event);
        }
        // LoginCancel can close a socket and must never run under the queue
        // mutex; the retiring flight keeps admission occupied until return.
        for cancel in polled.cancels {
            cancel.cancel();
        }
        let wait = match polled.permit {
            Permit::Grant => {
                let mut permit = GrantedReservation::new(queue, arm.queue_owner);
                if cancel.is_some_and(CancelToken::is_cancelled)
                    || permit_wait_cancelled(arm)
                    || replaced()
                {
                    drop(permit);
                    drop_queue_place(queue, statuses, username, arm.queue_owner);
                    return PermitWait::Cancelled;
                }
                apply_queue_wait(&mut lock_statuses(statuses), username, None);
                if let Some(demand) = wss.as_deref_mut() {
                    if !wss_authorization_is_current(
                        arm,
                        demand.identity,
                        demand.command,
                        demand.world_generation,
                    ) {
                        drop(permit);
                        drop_queue_place(queue, statuses, username, arm.queue_owner);
                        return PermitWait::Cancelled;
                    }
                    if demand.network_health.loss().is_some() {
                        drop(permit);
                        drop_queue_place(queue, statuses, username, arm.queue_owner);
                        return PermitWait::Unavailable;
                    }
                    let ticket = demand
                        .ticket
                        .as_ref()
                        .expect("WSS demand retains its ticket");
                    match ticket.poll(Instant::now()) {
                        GameTlsWait::Ready => {
                            permit.tls_ticket = demand.ticket.take();
                            permit.tls_park = Some(Arc::clone(&demand.park));
                            permit.network_health = Some(Arc::clone(&demand.network_health));
                            return PermitWait::Granted(permit);
                        }
                        GameTlsWait::InsufficientRemainingTime { .. } => {
                            drop(permit);
                            drop_queue_place(queue, statuses, username, arm.queue_owner);
                            ticket.release_for_insufficient_time();
                            return wss_wait_for_deadline_or_cancel(
                                ticket,
                                &demand.park,
                                || {
                                    !wss_authorization_is_current(
                                        arm,
                                        demand.identity,
                                        demand.command,
                                        demand.world_generation,
                                    )
                                },
                                &demand.network_health,
                                Some(frames),
                            );
                        }
                        GameTlsWait::Deadline => {
                            drop(permit);
                            drop_queue_place(queue, statuses, username, arm.queue_owner);
                            return PermitWait::Deadline;
                        }
                        GameTlsWait::Cancelled | GameTlsWait::Waiting => {
                            drop(permit);
                            drop_queue_place(queue, statuses, username, arm.queue_owner);
                            return PermitWait::Cancelled;
                        }
                    }
                }
                return PermitWait::Granted(permit);
            }
            Permit::Wait(wait) => wait,
        };
        // A window-blocked head can sleep a whole deadline. Wake in short
        // intervals for cancellation and refresh its published place.
        let deadline = Instant::now() + wait;
        let mut next_publish = Instant::now() + QUEUE_PUBLISH;
        while Instant::now() < deadline
            && !permit_wait_cancelled(arm)
            && !replaced()
            && (!release_wait || arm.has_clean_release())
        {
            let now = Instant::now();
            if now >= next_publish {
                let event = {
                    let q = queue.lock();
                    publish_queue_place(&q, &mut lock_statuses(statuses), username, arm.queue_owner)
                };
                if let Some(event) = event {
                    arm.publish_queued(event);
                }
                next_publish = now + QUEUE_PUBLISH;
            }
            let next_frame = frames();
            let left = deadline.saturating_duration_since(Instant::now());
            thread::sleep(left.min(Duration::from_millis(20)).min(next_frame));
        }
    }
}

#[cfg(test)]
#[path = "play_login_tests.rs"]
mod release_tests;

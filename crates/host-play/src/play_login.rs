use std::cell::Cell;
use std::collections::BTreeMap;

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{Deadline, Event, Scope, SessionKey, SlotKeyMinter};
use api::lifecycle::{
    AbortedAttempt, HandshakeKind, HoldReason, Intent, IntentCommand, IntentMailbox, LifecycleFact,
    LoginIntent, LoginLatch, MemoryMode, ParkReason, PostReceipt, QueueBlocker, SessionEndCause,
    SessionEventOut, SessionGate, TransportFailure, WorldChoice as IntentWorldChoice,
};
use client::client::{Client, LoginError};
use client::config::IfType;
use client::io::client_stream::LoginCancel;
use host::login_queue::{AdmitBlocker, LoginBackoff, LoginQueue, Permit, QueuePos};
use parking_lot::Mutex as QueueMutex;
use vault::{Profile, RandomEventPolicy};

use super::{clear_startup_progress, lock_statuses, public_worlds, Play, SlotStatus, StartupPhase};

pub(super) type SharedLoginQueue = Arc<QueueMutex<LoginQueue>>;

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

/// The frozen mailbox, the one currently running handshake, and owner-clock
/// evidence share one lock. Posting an invalidating row takes its cancel
/// handle under this lock and fires it only after dropping the lock.
struct SlotLifecycle {
    mailbox: IntentMailbox,
    session: SessionGate,
    last_session: Option<SessionKey>,
    scene_ready: bool,
    latest_event: Option<Event<SessionEventOut>>,
    /// A boundary event remains available until a consumer takes it, even
    /// when a later slot event changes the latest status.
    pending_session_end: Option<Event<SessionEventOut>>,
    last_queue: Option<SessionEventOut>,
    attempt: Option<LoginCancel>,
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
            if matches!(payload, SessionEventOut::SessionEnded { .. })
                && self.pending_session_end.is_none()
            {
                // The consumer takes the oldest unread boundary. The latest
                // view still reports the current session's event.
                self.pending_session_end = Some(event.clone());
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
    lifecycle: parking_lot::Mutex<SlotLifecycle>,
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
        let slot = NEXT_SLOT_KEY.mint().expect("slot identity exhausted");
        Arc::new(Self {
            uid: AtomicI32::new(uid),
            queue_owner: slot.incarnation().get(),
            lifecycle: parking_lot::Mutex::new(SlotLifecycle {
                mailbox: IntentMailbox::new(want_login, IntentWorldChoice::Automatic),
                session: SessionGate::new(slot),
                last_session: None,
                scene_ready: false,
                latest_event: None,
                pending_session_end: None,
                last_queue: None,
                attempt: None,
                unexpected_logout_times: [None; UNEXPECTED_LOGOUT_THRESHOLD],
                clean_exit: None,
                boundary_work_allowed: None,
            }),
            stop: Arc::new(AtomicBool::new(false)),
            random_events: Arc::new(AtomicBool::new(true)),
            lamp_auto: Arc::new(AtomicBool::new(true)),
            lamp_skill: Arc::new(Mutex::new("strength".to_string())),
            random_event_policies: Arc::new(Mutex::new(BTreeMap::new())),
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

    /// Apply one frozen row and fire its cancellation effect outside K4.
    pub fn post_intent(&self, row: Intent) -> Option<PostReceipt> {
        let (receipt, cancel) = {
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
            if matches!(row, Intent::RemoveSlot | Intent::ProcessShutdown) {
                self.stop.store(true, Ordering::Release);
            }
            lifecycle.slot_event(SessionEventOut::IntentApplied {
                generation: receipt.generation(),
            });
            let cancel = invalidated.then(|| lifecycle.attempt.clone()).flatten();
            (receipt, cancel)
        };
        if let Some(cancel) = cancel {
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

    /// The most recently published event, including a newer session's exit.
    /// Consumers that must not miss a boundary use `take_lifecycle_event`.
    pub fn latest_lifecycle_event(&self) -> Option<Event<SessionEventOut>> {
        self.lifecycle.lock().latest_event.clone()
    }

    /// Consume the oldest unread session boundary before the latest
    /// replaceable status event. At most one of each is retained.
    pub fn take_lifecycle_event(&self) -> Option<Event<SessionEventOut>> {
        let mut lifecycle = self.lifecycle.lock();
        if let Some(event) = lifecycle.pending_session_end.take() {
            if lifecycle.latest_event.as_ref() == Some(&event) {
                lifecycle.latest_event = None;
            }
            return Some(event);
        }
        lifecycle.latest_event.take()
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

    #[cfg(any(test, feature = "test-support"))]
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

    pub fn withdraw_login(&self) {
        self.post_intent(Intent::WithdrawLogin);
    }

    pub(super) fn hold_login_on_error(&self) {
        let cancel = {
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

    /// Start only if the command and endpoint are still current after the
    /// FIFO grant. The queue's token becomes the mailbox's cancellation
    /// effect before any socket connect starts.
    pub(super) fn begin_attempt(
        &self,
        command: IntentCommand,
        world_generation: u64,
        cancel: LoginCancel,
    ) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        if self.stop.load(Ordering::Acquire)
            || !lifecycle
                .mailbox
                .attempt_is_current(command, world_generation)
        {
            return false;
        }
        lifecycle.attempt = Some(cancel);
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

    /// An expired unused grant has no network or queue settlement to retire.
    pub(super) fn clear_unused_attempt(&self) {
        self.lifecycle.lock().attempt = None;
    }

    /// The sole post-login handoff, before any Ready publication. A late,
    /// cancelled or superseded success cannot be promoted to a session; an
    /// error still reaches the owner's status/retry path when its deadline
    /// passed, so a timed-out handshake names its actual failure.
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
        lifecycle.last_queue = None;
        let cancelled = cancel.is_cancelled();
        let expired = Instant::now() >= deadline;
        let owned = !self.stop.load(Ordering::Acquire)
            && lifecycle
                .mailbox
                .attempt_is_current(command, world_generation);
        let current = owned && !cancelled && !expired;
        if !current {
            let reason = if result.is_ok() && expired {
                AbortedAttempt::LateSuccess
            } else if expired {
                AbortedAttempt::Deadline
            } else {
                AbortedAttempt::Cancelled
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
        current || (owned && result.is_err())
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
        let cancel = {
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
            lifecycle.clean_exit = None;
            lifecycle.last_queue = None;
            lifecycle.slot_event(SessionEventOut::IntentApplied {
                generation: receipt.generation(),
            });
            lifecycle.attempt.clone()
        };
        if let Some(cancel) = cancel {
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

/// Owns one granted login reservation from the granting poll on (it is
/// built under the queue lock that granted it). Dropped unused, it gives the
/// grant back; dropped after its attempt started, it acknowledges the
/// attempt. So no return, panic or unwind between the grant and the end of
/// `client.login` leaks the queue's only in-flight slot.
pub(super) struct GrantedReservation {
    queue: SharedLoginQueue,
    owner: u64,
    state: ReservationState,
}

#[derive(Debug, Clone, Copy)]
enum ReservationState {
    Unused,
    Attempted,
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
        }
    }

    /// Publish the attempt token before K1 charges a start. A refused
    /// publication drops the unused reservation without spacing or UID/IP
    /// settlement. Once started, the queue owns its actual return. `login`
    /// closes refused/cancelled transport before this method acknowledges
    /// that return. The hold callback runs before the acknowledgement too.
    pub(super) fn attempt<T, E>(
        &mut self,
        publish: impl FnOnce(LoginCancel) -> bool,
        login: impl FnOnce(Instant, LoginCancel) -> Result<T, E>,
        server_hold: impl FnOnce(&Result<T, E>) -> Option<Duration>,
    ) -> Option<Result<T, E>> {
        let cancel = LoginCancel::new();
        if !publish(cancel.clone()) {
            return None;
        }
        let started = self
            .queue
            .lock()
            .start_attempt(self.owner, Instant::now(), cancel.clone());
        let Some(deadline) = started else {
            self.state = ReservationState::Resolved;
            host_log!(
                Category::Login,
                Level::Warn,
                "login grant expired before the attempt started; queueing again"
            );
            return None;
        };
        self.state = ReservationState::Attempted;
        let result = login(deadline, cancel);
        let hold = server_hold(&result);
        let acknowledged = {
            let mut queue = self.queue.lock();
            let now = Instant::now();
            if let Some(hold) = hold {
                queue.hold_for(now, hold);
            }
            queue.acknowledge_login_return(self.owner, now)
        };
        self.state = ReservationState::Resolved;
        if !acknowledged {
            host_log!(
                Category::Login,
                Level::Warn,
                "login returned after its in-flight lease ran out"
            );
        }
        Some(result)
    }
}

impl Drop for GrantedReservation {
    fn drop(&mut self) {
        match self.state {
            ReservationState::Unused => {
                let _ = self.queue.lock().abandon_permit(self.owner);
            }
            ReservationState::Attempted => {
                let _ = self
                    .queue
                    .lock()
                    .acknowledge_login_return(self.owner, Instant::now());
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

/// Release today's ClientStream before the queue acknowledges retirement.
/// A successful-but-stale login has installed an in-game session; a refused
/// login may also have retained a stream despite never owning a session.
pub(super) fn close_login_transport(client: &mut Client) {
    if client.ingame {
        client.logout();
    } else if let Some(mut stream) = client.stream.take() {
        stream.close();
    }
}

/// The single current-transport login return path. It retains the raw server
/// response for the shared hold and releases non-session sockets before the
/// caller acknowledges the queue's retiring flight.
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
        self,
        deadline: Instant,
        cancel: LoginCancel,
        after_return: impl FnOnce(&Client),
    ) -> (Result<bool, LoginError>, Option<Duration>) {
        if cancel.is_cancelled()
            || !self
                .arm
                .login_attempt_is_current(self.command, self.world_generation)
        {
            self.arm.finish_attempt(
                self.command,
                self.world_generation,
                deadline,
                &cancel,
                &Ok(()),
            );
            return (Ok(false), None);
        }
        self.client.set_login_deadline(deadline);
        self.client.set_login_cancel(cancel.clone());
        let result = self
            .client
            .login(self.username, self.password, self.reconnect);
        let hold = server_hold(&result, self.backoff);
        after_return(self.client);
        let current = self.arm.finish_attempt(
            self.command,
            self.world_generation,
            deadline,
            &cancel,
            &result,
        );
        if !current || result.is_err() {
            close_login_transport(self.client);
        }
        if !current {
            (Ok(false), hold)
        } else {
            (result.map(|()| true), hold)
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl SlotArm {
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
    stop: &AtomicBool,
) -> Result<(), String> {
    let modulus = public_worlds::modulus_for(world, refresh, |host, port| {
        Client::fetch_login_modulus_for(client::Transport::Wss, host, port)
    });
    if stop.load(Ordering::Relaxed) {
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

/// Outcome of [`wait_for_permit`]. `Granted` carries the reservation, owned
/// from the granting poll on: the caller passes it into `client.login`
/// ([`GrantedReservation::attempt`]) or drops it to give the grant back.
/// `Cancelled` owns no reservation: the request was withdrawn before a grant
/// and neither its FIFO place nor its published `k of n` survives.
#[derive(Debug)]
pub(super) enum PermitWait {
    Granted(GrantedReservation),
    Cancelled,
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
    let event = {
        let mut q = queue.lock();
        q.enqueue_owner(arm.queue_owner, uid);
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

/// A withdrawn intent never keeps a FIFO place. The mailbox, rather than a
/// second auto/script policy calculation in the queue waiter, owns the want.
pub(super) fn permit_wait_cancelled(arm: &SlotArm) -> bool {
    !arm.login_wanted()
}

/// Block until the already-enqueued slot owner receives a handshake permit,
/// mirroring the queue position onto the slot's status row while it waits
/// and keeping the logged-out title live through `frames`.
/// Withdrawal is observed before every poll, so a dropped place is never
/// recreated by this waiter.
pub(super) fn wait_for_permit(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
    frames: &mut WaitFrames<'_>,
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
    let identity = arm.uid.load(Ordering::Relaxed);
    let world_generation = arm.world_generation();
    let replaced = || {
        arm.uid.load(Ordering::Relaxed) != identity || arm.world_generation() != world_generation
    };
    loop {
        // Retire the old FIFO wait without withdrawing its replacement intent.
        if replaced() {
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
            let mut q = queue.lock();
            let polled = q.poll_owner(arm.queue_owner, uid, Instant::now());
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
                let permit = GrantedReservation::new(queue, arm.queue_owner);
                apply_queue_wait(&mut lock_statuses(statuses), username, None);
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

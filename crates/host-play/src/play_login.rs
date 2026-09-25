use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use client::client::client::SessionExitObservation;
use client::client::{Client, LoginError};
use client::config::IfType;
use client::BotTarget;
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use parking_lot::Mutex as QueueMutex;
use vault::Profile;

use super::{clear_startup_progress, debug_enabled, public_worlds, Play, SlotStatus, StartupPhase};

pub(super) type SharedLoginQueue = Arc<QueueMutex<LoginQueue>>;

static NEXT_QUEUE_OWNER: AtomicU64 = AtomicU64::new(1);

/// Login/logout controller state. Every operator command advances one
/// generation under this lock. A worker acknowledgement from an older
/// generation applies only when the current intent still requests the same
/// outcome; agreeing refreshes do not create another one-shot command. The
/// intent lock is never held with queue, status, or script locks.
struct SlotIntent {
    generation: u64,
    want_login: bool,
    want_logout: bool,
    login_latched: bool,
    auto_intent: bool,
}

#[derive(Clone, Copy)]
pub(super) struct IntentCommand {
    generation: u64,
}

#[cfg(test)]
type WorkerStartGate = (
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
);
#[cfg(test)]
type RetryRaceGate = (
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);


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
    intent: parking_lot::Mutex<SlotIntent>,
    pub stop: Arc<AtomicBool>,
    /// The spawn-time auto-login intent (CLI `new(uid, true)` stays armed
    /// so an unexpected DC re-handshakes; a panel one-shot arm disarms
    /// after the handshake unless the profile's auto_login was on).
    pub auto_login: Arc<AtomicBool>,
    /// Live guardian toggle (`ProfileSettings.random_events`). Mirrored
    /// from the vault on spawn and by panel/TUI settings writes so a
    /// toggle-off never acts/holds without a respawn.
    pub random_events: Arc<AtomicBool>,
    /// Live lamp auto-use toggle (`ProfileSettings.lamp_auto`).
    pub lamp_auto: Arc<AtomicBool>,
    /// Live lamp skill choice (`ProfileSettings.lamp_skill`).
    pub lamp_skill: Arc<Mutex<String>>,
    /// Operator-selected world for the next login handshake. `None` keeps
    /// automatic fallback; panel profile saves update this shared value
    /// without disturbing an online connection.
    pub world: Arc<parking_lot::Mutex<Option<u16>>>,
    pub(super) world_generation: AtomicU64,
    /// Next handshake is opcode 18 (lost_con reconnect). First-ever online
    /// is 16; after a grant this is true.
    pub reconnect: Arc<AtomicBool>,
    retry_wake: parking_lot::Condvar,
    /// Test seam for worker lifecycle cases whose subject starts at the
    /// login queue, after unrelated asset initialization.
    #[cfg(test)]
    pub(crate) bypass_asset_startup: AtomicBool,
    /// Deterministic worker-entry gate for lifecycle race regressions.
    #[cfg(test)]
    worker_start_gate: parking_lot::Mutex<Option<WorkerStartGate>>,
    /// One-shot gates around the retry-wait race boundary.
    #[cfg(test)]
    retry_wait_gate: parking_lot::Mutex<Option<RetryRaceGate>>,
    #[cfg(test)]
    retry_notify_gate: parking_lot::Mutex<Option<RetryRaceGate>>,
    #[cfg(test)]
    stop_cleanup_signal: parking_lot::Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl SlotArm {
    pub fn new(uid: i32, want_login: bool) -> Arc<Self> {
        Arc::new(Self {
            uid: AtomicI32::new(uid),
            queue_owner: NEXT_QUEUE_OWNER.fetch_add(1, Ordering::Relaxed),
            intent: parking_lot::Mutex::new(SlotIntent {
                generation: 0,
                want_login,
                want_logout: false,
                login_latched: false,
                auto_intent: want_login,
            }),
            stop: Arc::new(AtomicBool::new(false)),
            auto_login: Arc::new(AtomicBool::new(want_login)),
            random_events: Arc::new(AtomicBool::new(true)),
            lamp_auto: Arc::new(AtomicBool::new(true)),
            lamp_skill: Arc::new(Mutex::new("strength".to_string())),
            world: Arc::new(parking_lot::Mutex::new(None)),
            world_generation: AtomicU64::new(0),
            reconnect: Arc::new(AtomicBool::new(false)),
            retry_wake: parking_lot::Condvar::new(),
            #[cfg(test)]
            bypass_asset_startup: AtomicBool::new(false),
            #[cfg(test)]
            worker_start_gate: parking_lot::Mutex::new(None),
            #[cfg(test)]
            retry_wait_gate: parking_lot::Mutex::new(None),
            #[cfg(test)]
            retry_notify_gate: parking_lot::Mutex::new(None),
            #[cfg(test)]
            stop_cleanup_signal: parking_lot::Mutex::new(None),
        })
    }
    /// Enter the spawned worker at the queue/login seam. Production slots
    /// always run the complete asset startup.
    #[cfg(test)]
    pub(crate) fn bypass_asset_startup_for_test(&self) {
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
    ) -> (
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.retry_wait_gate.lock() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    #[cfg(test)]
    pub(crate) fn hold_retry_notify_before_lock_for_test(
        &self,
    ) -> (
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
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

    /// Arm an operator-requested one-shot login independently of auto-login.
    pub fn arm_explicit_login(&self) {
        let mut intent = self.intent.lock();
        intent.generation = intent.generation.wrapping_add(1);
        intent.login_latched = false;
        intent.want_login = true;
        intent.auto_intent = false;
        intent.want_logout = false;
        drop(intent);
        self.retry_wake.notify_all();
    }

    /// Arm an operator-requested clean logout and withdraw any login intent.
    pub fn request_logout(&self) {
        let mut intent = self.intent.lock();
        intent.generation = intent.generation.wrapping_add(1);
        intent.login_latched = true;
        intent.want_logout = true;
        intent.want_login = false;
        intent.auto_intent = false;
        drop(intent);
        self.retry_wake.notify_all();
    }

    /// Restore a persisted logged-out hold without requesting an IF logout
    /// from a fresh title-screen client.
    pub fn hold_logged_out(&self) {
        let mut intent = self.intent.lock();
        intent.generation = intent.generation.wrapping_add(1);
        intent.login_latched = true;
        intent.want_login = false;
        intent.want_logout = false;
        intent.auto_intent = false;
        drop(intent);
        self.retry_wake.notify_all();
    }

    pub fn wants_login(&self) -> bool {
        self.intent.lock().want_login
    }

    pub fn wants_logout(&self) -> bool {
        self.intent.lock().want_logout
    }

    pub fn login_latched(&self) -> bool {
        self.intent.lock().login_latched
    }

    /// Apply the live auto-login policy. Disabling it withdraws only an
    /// auto-derived intent; enabling it arms an unlatched parked slot.
    pub fn set_auto_login(&self, enabled: bool) {
        self.auto_login.store(enabled, Ordering::Relaxed);
        let mut intent = self.intent.lock();
        intent.generation = intent.generation.wrapping_add(1);
        if enabled {
            if !intent.login_latched && !intent.want_login {
                intent.want_login = true;
                intent.auto_intent = true;
            }
        } else if intent.auto_intent {
            intent.auto_intent = false;
            intent.want_login = false;
        }
        drop(intent);
        self.retry_wake.notify_all();
    }

    /// Withdraw the active login intent without changing the saved
    /// auto-login policy.
    pub fn withdraw_login(&self) {
        let mut intent = self.intent.lock();
        intent.generation = intent.generation.wrapping_add(1);
        intent.want_login = false;
        intent.auto_intent = false;
        drop(intent);
        self.retry_wake.notify_all();
    }

    /// Wake a retry/backoff wait after non-intent control changes. Taking the
    /// predicate lock closes the gap between a waiter's final check and its
    /// atomic unlock-and-park in `Condvar::wait_for`.
    pub(super) fn notify_retry_wait(&self) {
        #[cfg(test)]
        Self::wait_at_retry_race_gate(&self.retry_notify_gate);
        let _intent = self.intent.lock();
        self.retry_wake.notify_all();
    }

    /// Wait to a retry deadline. Notifications only re-check Stop,
    /// withdrawal, latch, and optionally world selection; generic UI wakes
    /// use a separate channel and cannot spend another login attempt.
    pub(super) fn wait_for_retry(&self, timeout: Duration) -> bool {
        self.wait_for_retry_inner(timeout, true)
    }

    /// Response 21 is tied to the same server-selected world. A profile world
    /// edit must not turn the transfer cooldown into an early retry/switch.
    pub(super) fn wait_for_transfer(&self, timeout: Duration) -> bool {
        self.wait_for_retry_inner(timeout, false)
    }

    fn wait_for_retry_inner(&self, timeout: Duration, interrupt_on_world_change: bool) -> bool {
        let mut intent = self.intent.lock();
        let world_generation = self.world_generation.load(Ordering::Relaxed);
        let deadline = Instant::now() + timeout;
        loop {
            if self.stop.load(Ordering::Relaxed) || !intent.want_login || intent.login_latched {
                return false;
            }
            if interrupt_on_world_change
                && self.world_generation.load(Ordering::Relaxed) != world_generation
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            #[cfg(test)]
            Self::wait_at_retry_race_gate(&self.retry_wait_gate);
            self.retry_wake.wait_for(&mut intent, left);
        }
    }

    pub(super) fn login_command(&self, ingame: bool) -> Option<IntentCommand> {
        let intent = self.intent.lock();
        (!ingame && intent.want_login && !intent.login_latched).then_some(IntentCommand {
            generation: intent.generation,
        })
    }

    pub(super) fn logout_command(&self, ingame: bool) -> Option<IntentCommand> {
        let intent = self.intent.lock();
        (ingame && intent.want_logout).then_some(IntentCommand {
            generation: intent.generation,
        })
    }

    pub(super) fn acknowledge_logout(&self, command: IntentCommand) {
        let mut intent = self.intent.lock();
        if intent.generation != command.generation && !intent.want_logout {
            return;
        }
        intent.want_logout = false;
        intent.login_latched = true;
        intent.want_login = false;
        intent.auto_intent = false;
    }

    fn acknowledge_observed_idle_logout(&self) {
        let mut intent = self.intent.lock();
        // An explicit Login issued after the idle request is the newest
        // command and must survive the delayed server acknowledgement.
        if intent.want_login && !intent.auto_intent && !intent.want_logout {
            return;
        }
        intent.login_latched = true;
        intent.want_login = false;
        intent.auto_intent = false;
    }

    fn acknowledge_login(&self, command: IntentCommand) {
        let mut intent = self.intent.lock();
        if intent.generation != command.generation
            && (!intent.want_login || intent.login_latched || intent.want_logout)
        {
            return;
        }
        let keep = self.auto_login.load(Ordering::Relaxed) && !intent.login_latched;
        intent.want_login = keep;
        intent.auto_intent = keep;
    }
}

pub(super) fn sync_profile_arm(arm: &SlotArm, profile: &Profile) {
    arm.uid.store(profile.uid, Ordering::Relaxed);
    arm.random_events
        .store(profile.settings.random_events, Ordering::Relaxed);
    arm.lamp_auto
        .store(profile.settings.lamp_auto, Ordering::Relaxed);
    *arm.lamp_skill.lock().unwrap() = profile.settings.lamp_skill.clone();
    let world_changed = {
        let mut world = arm.world.lock();
        let changed = *world != profile.settings.world;
        *world = profile.settings.world;
        changed
    };
    if world_changed {
        arm.world_generation.fetch_add(1, Ordering::Relaxed);
        arm.notify_retry_wait();
    }
}

/// Whether the slot may start a login handshake: on the title (not ingame)
/// and the arm wants a login that is not latched by an intentional logout.
pub(super) fn should_handshake(arm: &SlotArm, ingame: bool) -> bool {
    arm.login_command(ingame).is_some()
}

/// Claim the boundary between a granted reservation and `client.login`.
/// Cancellation or stop observed here abandons only this unused permit; a
/// returned generation must be acknowledged after the login call.
pub(super) fn granted_permit_may_start_login(
    queue: &SharedLoginQueue,
    uid: i32,
    arm: &SlotArm,
    ingame: bool,
) -> Option<IntentCommand> {
    let command = (!arm.stop.load(Ordering::Relaxed))
        .then(|| arm.login_command(ingame))
        .flatten();
    if command.is_some() {
        return command;
    }
    let abandoned = queue.lock().abandon_permit(uid);
    debug_assert!(abandoned, "granted permit must be abandoned exactly once");
    None
}

/// A profile edit can land while this slot waits in the FIFO. Abandon that
/// unused grant so the next pass can configure the new endpoint before the
/// socket handshake starts.
pub(super) fn granted_permit_world_is_current(
    queue: &SharedLoginQueue,
    uid: i32,
    round: Option<&public_worlds::WorldRound>,
    arm: &SlotArm,
) -> bool {
    if round.is_none_or(|round| round.preference() == *arm.world.lock()) {
        return true;
    }
    let abandoned = queue.lock().abandon_permit(uid);
    debug_assert!(abandoned, "granted permit must be abandoned exactly once");
    false
}

/// Unwind-safe ownership for one granted reservation. Until the socket call
/// begins, Drop abandons it; once attempted, Drop conservatively acknowledges
/// it. Thus neither preparation panics nor `client.login` panics leak capacity.
pub(super) struct GrantedReservation<'a> {
    queue: &'a QueueMutex<LoginQueue>,
    uid: i32,
    state: ReservationState,
}

#[derive(Clone, Copy)]
enum ReservationState {
    Unused,
    Attempted,
    Resolved,
}

impl<'a> GrantedReservation<'a> {
    pub(super) fn new(queue: &'a SharedLoginQueue, uid: i32) -> Self {
        Self {
            queue,
            uid,
            state: ReservationState::Unused,
        }
    }

    fn attempt<T, E>(&mut self, login: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        self.state = ReservationState::Attempted;
        let result = login();
        let acknowledged = self
            .queue
            .lock()
            .acknowledge_login_return(self.uid, Instant::now());
        debug_assert!(
            acknowledged,
            "each client.login return acknowledges one granted permit"
        );
        self.state = ReservationState::Resolved;
        result
    }
}

impl Drop for GrantedReservation<'_> {
    fn drop(&mut self) {
        match self.state {
            ReservationState::Unused => {
                let _ = self.queue.lock().abandon_permit(self.uid);
            }
            ReservationState::Attempted => {
                let _ = self
                    .queue
                    .lock()
                    .acknowledge_login_return(self.uid, Instant::now());
            }
            ReservationState::Resolved => {}
        }
    }
}

pub(super) fn login_and_acknowledge_permit<T, E>(
    permit: &mut GrantedReservation<'_>,
    login: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    permit.attempt(login)
}

/// After a successful handshake, keep an unlatched slot armed exactly when
/// its saved auto-login policy is enabled. A newer command generation wins.
pub(super) fn on_login_success(arm: &SlotArm, command: IntentCommand) {
    arm.acknowledge_login(command);
    // A later DC / tune / park is opcode 18, not a cold 16.
    arm.reconnect.store(true, Ordering::Relaxed);
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
    if let Some(SessionExitObservation::ServerLogoutAfterLocalIdleRequest) =
        client.take_session_exit_observation()
    {
        arm.acknowledge_observed_idle_logout();
    }
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
            .ok_or("--world requires public-289")?;
        if worlds.by_number(number).is_none() {
            return Err(format!(
                "world {number} is not in the configured public worlds"
            ));
        }
        self.auto_world = Some(number);
        Ok(())
    }
}

fn publish_transfer_countdown(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, remaining: u64) {
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_phase = StartupPhase::Connecting;
        s.startup_phase_started = Instant::now();
        s.error = None;
        s.startup_progress_percent = None;
        s.startup_progress_message =
            format!("Your profile will be transferred in: {remaining} seconds");
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
) -> Option<bool> {
    if error.code != 21 {
        return None;
    }
    let delay = error.retry_after?;
    let mut remaining = delay.as_secs();
    loop {
        publish_transfer_countdown(statuses, name, remaining);
        if !arm.wait_for_transfer(Duration::from_secs(1)) {
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
    let choice = *arm.world.lock();
    round.reselect_if_changed(worlds, choice)
}

pub(super) fn configure_slot_world(
    client: &mut Client,
    world: &public_worlds::PublicWorld,
    refresh: bool,
    stop: &AtomicBool,
) -> Result<(), String> {
    let modulus = public_worlds::modulus_for(world, refresh, |host, port| {
        Client::fetch_login_modulus_for(BotTarget::Prod, host, port)
    });
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    client.set_public_world(&world.host, world.port, world.node_id, &modulus)
}

pub(super) fn login_retry_wait(backoff: &mut LoginBackoff, code: i32) -> Duration {
    match code {
        1 => Duration::from_secs(2),
        16 => backoff.delay(),
        5 => Duration::from_secs(60),
        _ => Duration::from_secs(5),
    }
}

/// Copy a login-queue snapshot onto every `SlotStatus` row named `name`;
/// `None` (granted or not queued) clears both fields back to -1.
pub(super) fn publish_login_latched(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    latched: bool,
) {
    if let Some(s) = statuses
        .lock()
        .unwrap()
        .iter_mut()
        .find(|s| s.username == name)
    {
        s.login_latched = latched;
        if latched {
            s.error = None;
            s.login_started = None;
            s.queue_position = -1;
            s.queue_total = -1;
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
    publish_login_latched(statuses, name, arm.login_latched());
}

pub(super) fn apply_queue_wait(rows: &mut [SlotStatus], name: &str, pos: Option<QueuePos>) {
    let (position, total) =
        match pos.filter(|p| p.position >= 1 && p.total >= 1 && p.position <= p.total) {
            Some(p) => (p.position as i32, p.total as i32),
            None => (-1, -1),
        };
    for s in rows.iter_mut().filter(|s| s.username == name) {
        s.queue_position = position;
        s.queue_total = total;
    }
}

/// Refresh cadence for a waiting slot's published place. A window-blocked
/// head can sleep a whole 60 s deadline in one wait, so the card would
/// otherwise miss members that queue behind it. Read-only: the refresh
/// re-reads `status(uid)` and never requests a permit.
const QUEUE_PUBLISH: Duration = Duration::from_millis(200);

/// Outcome of [`wait_for_permit`]. `Granted` owns a pending reservation that
/// the caller must either pass into `client.login` and acknowledge on return,
/// or abandon if its final intent check fails. `Cancelled` owns no reservation:
/// the request was withdrawn before a grant and neither its FIFO place nor its
/// published `k of n` survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PermitWait {
    Granted,
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
    let mut q = queue.lock();
    q.enqueue_owner(arm.queue_owner, uid);
    let pos = q.status_owner(arm.queue_owner);
    apply_queue_wait(&mut statuses.lock().unwrap(), username, pos);
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
    apply_queue_wait(&mut statuses.lock().unwrap(), username, None);
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
    let mut rows = statuses
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

/// Whether a pending permit wait must be withdrawn before any handshake.
/// Intent provenance lives on the arm, so turning auto-login off is still
/// observed even when it happens before the first queue poll.
pub(super) fn permit_wait_cancelled(arm: &SlotArm) -> bool {
    let intent = arm.intent.lock();
    arm.stop.load(Ordering::Relaxed)
        || !intent.want_login
        || intent.login_latched
        || (intent.auto_intent && !arm.auto_login.load(Ordering::Relaxed))
}

/// Block until the already-enqueued slot owner receives a handshake permit,
/// mirroring the queue position onto the slot's status row while it waits.
/// Withdrawal is observed before every poll, so a dropped place is never
/// recreated by this waiter.
pub(super) fn wait_for_permit(
    queue: &SharedLoginQueue,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
) -> PermitWait {
    let withdraw = || {
        arm.withdraw_login();
        drop_queue_place(queue, statuses, username, arm.queue_owner);
        if debug_enabled() {
            eprintln!("[host-play] slot {username}: permit wait withdrawn");
        }
    };
    loop {
        if permit_wait_cancelled(arm) {
            withdraw();
            return PermitWait::Cancelled;
        }
        let wait = {
            let mut q = queue.lock();
            match q.poll_owner(arm.queue_owner, uid, Instant::now()) {
                Permit::Grant => {
                    apply_queue_wait(&mut statuses.lock().unwrap(), username, None);
                    return PermitWait::Granted;
                }
                Permit::Wait(wait) => {
                    let pos = q.status_owner(arm.queue_owner);
                    apply_queue_wait(&mut statuses.lock().unwrap(), username, pos);
                    wait
                }
            }
        };
        // A window-blocked head can sleep a whole deadline. Wake in short
        // intervals for cancellation and refresh its published place.
        let deadline = Instant::now() + wait;
        let mut next_publish = Instant::now() + QUEUE_PUBLISH;
        while Instant::now() < deadline && !permit_wait_cancelled(arm) {
            let now = Instant::now();
            if now >= next_publish {
                let pos = queue.lock().status_owner(arm.queue_owner);
                apply_queue_wait(&mut statuses.lock().unwrap(), username, pos);
                next_publish = now + QUEUE_PUBLISH;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            thread::sleep(left.min(Duration::from_millis(20)));
        }
    }
}

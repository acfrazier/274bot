use std::cell::Cell;

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::SlotKeyMinter;
use api::lifecycle::{Intent, IntentCommand};
use api::snapshot::GameSnapshot;
use client::client::{Client, LoginError};
use client::config::{Cache, IfType, IfTypeMut};
use host::login_queue::LoginBackoff;
use host::{
    prepare_client, should_emit_tick, wake_channel, DetectedRandom, FrameBuf, Host, Pump,
    ScriptRunPolicy, SlotInput, SlotPark, SlotRender, SlotWake, TitleWait,
};
use nav_planner::world::NavWorld;
use vault::Profile;

#[cfg(feature = "memory-profile")]
use crate::memory;
use crate::play_bootstrap::{bot_client_config, PlayConnection};
use crate::play_login::{
    capture_login_authorization, configure_slot_world, drop_queue_place, enqueue_queue_place,
    granted_permit_world_is_current, observe_client_session_exit, plan_login_retry,
    publish_login_latched_from_arm, refresh_slot_world_preference, should_handshake,
    sync_profile_arm, tick_flags, transfer_countdown_message, wait_for_permit,
    wait_for_transfer_response, PermitWait, QueuePlaceRetirement, RelogParkReason,
    SharedLoginQueue, SlotArm, SlotLogin,
};
use crate::play_status::{
    apply_startup_phase, clear_startup_progress, copy_stream_bytes, lock_statuses,
    mark_login_started, publish_login_lowmem, publish_session_boundary_status,
    publish_slot_disconnected, publish_startup_phase, publish_startup_progress,
    publish_worker_terminal, record_login_error, set_startup_phase, SlotStatus, StartupPhase,
    WorkerTerminal,
};
use crate::play_wires::{dispatch_wires, WireCmd};
#[cfg(feature = "js")]
use crate::script_runtime::{
    deliver_channel_events, project_npc_boxes_for_isolate_snapshot, projected_npc_boxes,
};
use crate::script_runtime::{
    hold_script_nav, nav_world_state_for_observe, observe_script_inv, publish_script_paint,
    publish_script_survival, reset_script_nav, script_active, script_frame,
    script_observe_cached_with_channels, script_paint_of, script_slot, script_slot_or_insert,
    script_survival_of, slot_arrival_reach, step_nav_bot, take_manual_walk_ownership, NavBot,
    ScriptSlot, ScriptWall,
};
#[cfg(feature = "test-support")]
use crate::tick_engine_test_support::TickEngineTestHook;
use crate::{
    catalog_core, login_readiness, paired_core, public_worlds, Play, RandomClaim, RandomStatus,
};

/// The process's slot worker lifetimes: Play assigns one `SlotKey` per
/// spawned worker. Process-wide, so two `Play`s in one process never share
/// a key; never reused, refused at exhaustion.
static SLOT_KEYS: SlotKeyMinter = SlotKeyMinter::new();

/// The Fight service of a slot (design V11D 4.1): the slot's arm, where the
/// Guardian and the observation share their one frame's port, and the
/// selected content the child prices its escape from.
pub(crate) struct FightService {
    arm: Arc<SlotArm>,
    game_data: Arc<api::game_data::SelectedGameData>,
}

impl FightService {
    pub(crate) fn new(arm: Arc<SlotArm>, game_data: Arc<api::game_data::SelectedGameData>) -> Self {
        Self { arm, game_data }
    }
}

/// The script callback is edge-triggered. Load's cached ignore membership
/// can change independently while the Guardian keeps the same solver. The
/// same adapter carries the Guardian's Fight to the slot's child.
pub(crate) struct ScriptRandomClaims {
    scripts: ScriptWall,
    name: String,
    fight: Option<FightService>,
}

impl ScriptRandomClaims {
    pub(crate) fn new(scripts: ScriptWall, name: String) -> Self {
        Self {
            scripts,
            name,
            fight: None,
        }
    }

    /// The adapter with a Fight service. Without one every offer is refused,
    /// and the Guardian flees as it always did.
    pub(crate) fn with_fight(mut self, fight: FightService) -> Self {
        self.fight = Some(fight);
        self
    }
}

impl host::RandomClaimSource for ScriptRandomClaims {
    fn on_random(&mut self, event: &DetectedRandom) -> RandomClaim {
        let Some(slot) = script_slot(&self.scripts, &self.name) else {
            return RandomClaim::Host;
        };
        let Ok(mut slot) = slot.lock() else {
            return RandomClaim::Host;
        };
        slot.on_random(event)
    }

    fn current_claim(&mut self, event: &DetectedRandom) -> Option<RandomClaim> {
        let slot = script_slot(&self.scripts, &self.name)?;
        let slot = slot.lock().ok()?;
        slot.current_random_claim(event)
    }

    /// A missing slot is no runtime: a claim made under one returns to Host.
    fn runtime(&mut self) -> Option<host::ClaimRuntime> {
        let slot = script_slot(&self.scripts, &self.name)?;
        let slot = slot.lock().ok()?;
        let (incarnation, run) = slot.random_claim_runtime();
        Some(host::ClaimRuntime::new(incarnation, run))
    }

    /// The child lives in the slot, whatever card it holds: a slot that never
    /// loaded one gets its cell here.
    fn fight_offer(
        &mut self,
        offer: &api::random::FightOffer,
    ) -> Result<(), api::random::FightReason> {
        let service = self
            .fight
            .as_ref()
            .ok_or(api::random::FightReason::NoService)?;
        let slot = script_slot_or_insert(&self.scripts, &self.name);
        let mut slot = slot
            .lock()
            .map_err(|_| api::random::FightReason::NoService)?;
        slot.guardian_offer(*offer, &service.game_data)
    }

    fn fight_revoke(&mut self, token: api::random::FightToken, cancel: api::random::FightCancel) {
        let Some(slot) = script_slot(&self.scripts, &self.name) else {
            return;
        };
        let Ok(mut slot) = slot.lock() else {
            return;
        };
        slot.guardian_revoke_token(token, cancel);
    }

    fn fight_read(&mut self, token: api::random::FightToken) -> Option<api::random::FightRead> {
        let slot = script_slot(&self.scripts, &self.name)?;
        let slot = slot.lock().ok()?;
        slot.guardian_read(token)
    }

    fn fight_release(&mut self, token: api::random::FightToken) {
        if let Some(service) = &self.fight {
            service.arm.fight.release(token);
        }
        let Some(slot) = script_slot(&self.scripts, &self.name) else {
            return;
        };
        let Ok(mut slot) = slot.lock() else {
            return;
        };
        slot.guardian_release(token);
    }

    fn fight_stage_escape(&mut self, intent: host::EscapeIntent) {
        if let Some(service) = &self.fight {
            service.arm.fight.stage(intent);
        }
    }

    fn fight_take_sent(&mut self, token: api::random::FightToken) -> Option<host::SentEscape> {
        self.fight.as_ref()?.arm.fight.take_sent(token)
    }
}

/// Snapshot families whose new evidence can settle a parked machine.
pub(crate) fn script_evidence_dirty(dirty: host::DirtyFamilies) -> bool {
    dirty.iface || dirty.inv || dirty.stat || dirty.chat
}

/// Facts delivered before frontend follow and script observation/dispatch.
/// `hold` retains the guardian/readiness gate; it does not suppress intent.
/// Each queued manual step counts even if the client later refuses its send.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlotFrameInput {
    pub hold: bool,
    #[cfg(feature = "live-probe")]
    pub guardian_fight_outcome: Option<api::random::FightOutcome>,
    #[cfg(feature = "live-probe")]
    pub guardian_no_progress_ticks: Option<u16>,
    /// Native/script or escape ownership freezes ordinary frontend movement.
    pub host_move_owned: bool,
    pub manual_move_intent: Option<host::ManualMoveIntent>,
    pub manual_steps: usize,
}

impl SlotFrameInput {
    /// Number of qualifying gestures in this frame, independent of route ownership.
    pub fn manual_move_count(self) -> usize {
        usize::from(self.manual_move_intent.is_some()) + self.manual_steps
    }
}
/// Project script and native-navigation ownership into the frame value read
/// by the panel and TUI follow gates.
pub(super) fn project_slot_frame_host_move_owned(
    script_state: Option<script::RunState>,
    nav_owned: bool,
) -> bool {
    crate::script_runtime::script_movement_owned(script_state) || nav_owned
}

/// Starting frames can dispatch the first native walk before the outer
/// pump's Running flag flips; those walks still need snapshot stats
/// (Fishing Guild door is Fishing 68).
pub(crate) fn script_state_builds_nav_facts(script_state: Option<script::RunState>) -> bool {
    matches!(
        script_state,
        Some(script::RunState::Starting | script::RunState::Running)
    )
}

/// Queue one host-owned client frame under the slot lock, then wait for its
/// isolate receipt after releasing that lock.
pub(crate) fn dispatch_client_frame(slot: &Arc<Mutex<script::SlotScript>>, _name: &str) -> bool {
    #[cfg(feature = "memory-profile")]
    memory::frame_dispatch_begin(_name);
    let dispatch = {
        let mut slot = slot.lock().unwrap();
        slot.begin_client_frame()
    };
    let delivered = dispatch.is_some_and(|dispatch| dispatch.wait_for_ack());
    #[cfg(feature = "memory-profile")]
    memory::frame_dispatch_end(_name, delivered);
    delivered
}

pub(super) type SlotFrame = Arc<dyn Fn(&mut Client, &str, SlotFrameInput) + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
pub(super) type PostFrameObserver = Arc<dyn Fn(&Client, &str) + Send + Sync>;

pub(super) fn take_slot_frame_input(
    input: &SlotInput,
    name: &str,
    wire_queues: &Mutex<HashMap<String, VecDeque<WireCmd>>>,
    hold: bool,
    random_status: &RandomStatus,
) -> (SlotFrameInput, VecDeque<WireCmd>) {
    let _ = random_status;
    let wires = wire_queues
        .lock()
        .unwrap()
        .get_mut(name)
        .map(std::mem::take)
        .unwrap_or_default();
    let frame = SlotFrameInput {
        hold,
        #[cfg(feature = "live-probe")]
        guardian_fight_outcome: random_status.fight_outcome,
        #[cfg(feature = "live-probe")]
        guardian_no_progress_ticks: random_status.no_progress_ticks,
        host_move_owned: false,
        manual_move_intent: input.take_manual_move_intent(),
        manual_steps: wires
            .iter()
            .filter(|cmd| matches!(cmd, WireCmd::Walk { .. }))
            .count(),
    };
    (frame, wires)
}
/// Slot thread stack: 1 MiB (the Java client thread default).
const THREAD_STACK: usize = 1024 * 1024;
/// Per-slot nav latch key: the `(player gen, here)` pair the pump last
/// stepped. Follow advances at most once per key. Hold or a dispatched
/// exclusive batch suppresses that key's step but still advances the latch,
/// so follow resumes on the next player key. An unarmed check does not consume
/// the key: an asynchronously published route can still step on that key.
pub(crate) type NavStepKey = (u64, Option<(i32, i32, i32)>);

pub(crate) fn nav_step_due<F>(
    last: &mut Option<NavStepKey>,
    key: NavStepKey,
    hold: bool,
    exclusive: bool,
    route_armed: F,
) -> bool
where
    F: FnOnce() -> bool,
{
    if *last == Some(key) {
        return false;
    }
    if hold || exclusive {
        *last = Some(key);
        return false;
    }
    if key.1.is_none() || !route_armed() {
        return false;
    }
    *last = Some(key);
    true
}
/// Retire one failed lifetime's script state without replaying its poisoned
/// mutex on the UI thread. `stop` is still attempted to tear down a usable
/// isolate; a second cleanup panic is contained because the slot has already
/// been removed from the wall and cannot be observed again.
fn stop_retired_script(slot: ScriptSlot) {
    let stopped = catch_unwind(AssertUnwindSafe(|| {
        let mut script = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        script.stop_removed();
    }))
    .is_ok();
    if stopped {
        slot.clear_poison();
    }
}

impl Play {
    /// Publish Stop before any join. The queue lock is taken only after the
    /// arm control lock has been released by `notify_retry_wait`; no shared
    /// status/script lock is held here.
    pub(super) fn signal_slot_stop(&self, name: &str) {
        if let Some(arm) = self.arms.get(name) {
            let connected = self.slot_connected(name);
            arm.post_intent(Intent::RemoveSlot);
            self.queue.lock().leave_owner(arm.queue_owner);
            if connected {
                host_log!(
                    Category::Lifecycle,
                    Level::Info,
                    slot = name,
                    "session exit reason=removal"
                );
            } else {
                host_log!(
                    Category::Lifecycle,
                    Level::Info,
                    slot = name,
                    "slot stop reason=removal"
                );
            }
        }
    }

    /// Retire all name-owned registries and return the worker handle without
    /// joining it. Registry guards are always released before stopping a
    /// removed script, so cleanup adds no script-wall -> script-slot edge.
    /// The unloaded run ends as a Stop's does: the script stops first, then
    /// [`SlotStatus::retire_script_run`] retires its row once, and only then
    /// is the row removed. Lock order is the Stop paths' (script, then
    /// statuses), never nested: each guard ends before the next is taken.
    fn take_slot_for_stop(&mut self, name: &str) -> Option<thread::JoinHandle<()>> {
        #[cfg(test)]
        let arm = self.arms.get(name).cloned();
        self.signal_slot_stop(name);
        self.spawned.remove(name);
        self.arms.remove(name);
        let removed = self.scripts.lock().unwrap().remove(name);
        if let Some(slot) = removed {
            stop_retired_script(slot);
        }
        lock_statuses(&self.statuses).retain_mut(|status| {
            if status.username != name {
                return true;
            }
            status.retire_script_run();
            false
        });
        self.cheats.lock().unwrap().remove(name);
        self.wires.lock().unwrap().remove(name);
        #[cfg(test)]
        if let Some(arm) = arm.as_ref() {
            arm.signal_stop_cleanup_for_test();
        }
        if self.focused.as_deref() == Some(name) {
            self.focused = None;
            self.queue.lock().set_preferred_owner(None);
        }
        let handle = self.handles.remove(name);
        if handle.is_some() {
            self.wake(name);
        }
        self.wakes.remove(name);
        handle
    }

    /// Retire workers that ended without Stop while preserving their terminal
    /// status row for the operator. Joins are non-blocking because every
    /// selected handle has already finished. Registry guards are never nested;
    /// a removed script is stopped only after releasing the script wall.
    pub fn reap_finished_workers(&mut self) -> Vec<String> {
        let finished: Vec<String> = self
            .handles
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(name, _)| name.clone())
            .collect();
        for name in &finished {
            let arm = self.arms.remove(name);
            if let Some(arm) = arm.as_ref() {
                self.queue.lock().leave_owner(arm.queue_owner);
            }
            self.spawned.remove(name);
            let removed = self.scripts.lock().unwrap().remove(name);
            if let Some(slot) = removed {
                stop_retired_script(slot);
            }
            self.cheats.lock().unwrap().remove(name);
            self.wires.lock().unwrap().remove(name);
            self.navs.lock().unwrap().remove(name);
            self.wakes.remove(name);
            if self.focused.as_deref() == Some(name.as_str()) {
                self.focused = None;
                self.queue.lock().set_preferred_owner(None);
            }
            if let Some(handle) = self.handles.remove(name) {
                let _ = handle.join();
            }
        }
        finished
    }

    /// Join workers whose threads have already finished, including
    /// asynchronous Stop. Frontends call this once per UI pump before
    /// sampling live arms.
    pub fn pump_worker_reaps(&mut self) {
        self.reap_finished_workers();
        self.reap_stopped_slots();
    }

    /// Stop and retire a slot without joining its worker on the caller.
    /// [`Play::reap_stopped_slots`] joins only handles already known finished.
    pub fn begin_stop_slot(&mut self, name: &str) {
        self.pump_worker_reaps();
        if self.retiring.contains_key(name) {
            return;
        }
        if let Some(handle) = self.take_slot_for_stop(name) {
            self.retiring.insert(name.to_string(), handle);
        }
    }

    /// Join completed asynchronous stops. `is_finished` makes every join in
    /// this method non-blocking.
    pub fn reap_stopped_slots(&mut self) {
        let finished: Vec<String> = self
            .retiring
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(name, _)| name.clone())
            .collect();
        for name in finished {
            if let Some(handle) = self.retiring.remove(&name) {
                let _ = handle.join();
            }
        }
    }

    /// Copy every status row into `out`, reusing its rows' buffers (see
    /// [`SlotStatus`]'s `clone_from`): no allocation when nothing grew.
    pub fn statuses_into(&self, out: &mut Vec<SlotStatus>) {
        out.clone_from(&lock_statuses(&self.statuses));
    }

    /// Whether `name`'s row reports a connected session, read in place.
    pub fn slot_connected(&self, name: &str) -> bool {
        lock_statuses(&self.statuses)
            .iter()
            .any(|status| status.username == name && status.connected)
    }

    /// Whether `name`'s stopped worker ([`Play::begin_stop_slot`]) has not
    /// exited yet. A respawn of the same name is refused until it has.
    pub fn slot_stopping(&self, name: &str) -> bool {
        self.retiring
            .get(name)
            .is_some_and(|handle| !handle.is_finished())
    }

    /// Synchronous stop for CLI/tests and final teardown. UI removal uses
    /// [`Play::begin_stop_slot`] instead.
    pub fn stop_slot(&mut self, name: &str) {
        if let Some(handle) = self.retiring.remove(name) {
            let _ = handle.join();
            return;
        }
        if let Some(handle) = self.take_slot_for_stop(name) {
            let _ = handle.join();
        }
    }

    /// Register a control arm without spawning a slot thread (panel unit
    /// tests that drive login/logout flags through [`Play::arm`]).
    pub fn attach_arm(&mut self, name: &str, arm: Arc<SlotArm>) {
        self.arms.insert(name.to_string(), arm);
    }

    /// Arm plus an already-finished join handle. UI pumps must reap this
    /// before treating the name as a live background bot.
    #[cfg(feature = "test-support")]
    pub fn attach_finished_worker_for_test(&mut self, name: &str, arm: Arc<SlotArm>) {
        self.attach_arm(name, arm);
        self.spawned.insert(name.to_string());
        let (exited, observe_exit) = std::sync::mpsc::channel();
        self.handles.insert(
            name.to_string(),
            thread::spawn(move || {
                let _ = exited.send(());
            }),
        );
        observe_exit.recv().unwrap();
        while !self.handles[name].is_finished() {
            thread::yield_now();
        }
    }

    /// Arm plus a worker that runs until the returned sender is used or
    /// dropped, so a Stop leaves it [`Play::slot_stopping`] for as long as
    /// the caller holds it (a slow worker exit, deterministically).
    #[cfg(feature = "test-support")]
    pub fn attach_blocked_worker_for_test(
        &mut self,
        name: &str,
        arm: Arc<SlotArm>,
    ) -> std::sync::mpsc::Sender<()> {
        self.attach_arm(name, arm);
        self.spawned.insert(name.to_string());
        let (release, released) = std::sync::mpsc::channel::<()>();
        self.handles.insert(
            name.to_string(),
            thread::spawn(move || {
                let _ = released.recv();
            }),
        );
        release
    }

    /// Attach a worker that records its exit after the Play signals shutdown.
    #[cfg(feature = "test-support")]
    pub fn attach_stop_observed_worker_for_test(
        &mut self,
        name: &str,
        arm: Arc<SlotArm>,
    ) -> std::sync::mpsc::Receiver<()> {
        let stop = Arc::clone(&arm.stop);
        self.attach_arm(name, arm);
        self.spawned.insert(name.to_string());
        let (exited, observe_exit) = std::sync::mpsc::channel();
        self.handles.insert(
            name.to_string(),
            thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                let _ = exited.send(());
            }),
        );
        observe_exit
    }

    /// Spawn one more slot on this play's FIFO. No-op if `username` is
    /// already in the status list (already running). `None` arm behaves as
    /// [`SlotArm::new(profile.uid, true)`] — the slot logs in immediately
    /// (CLI/e2e); the panel passes a real arm so it can sit on the title.
    pub fn spawn_slot(
        &mut self,
        profile: Profile,
        input: Option<Arc<SlotInput>>,
        mailbox: Option<Arc<FrameBuf>>,
        arm: Option<Arc<SlotArm>>,
    ) {
        let username = profile.username.clone();
        if let Err(error) = self.try_spawn_slot(profile, input, mailbox, arm) {
            host_log!(stderr; Category::Lifecycle, Level::Error, slot = &username, "{error}");
        }
    }

    pub fn try_spawn_slot(
        &mut self,
        profile: Profile,
        input: Option<Arc<SlotInput>>,
        mailbox: Option<Arc<FrameBuf>>,
        arm: Option<Arc<SlotArm>>,
    ) -> Result<(), String> {
        self.connection.require_bot_operation()?;
        self.reap_finished_workers();
        if self
            .retiring
            .get(&profile.username)
            .is_some_and(|handle| !handle.is_finished())
        {
            return Err(format!("slot {} is still stopping", profile.username));
        }
        if let Some(handle) = self.retiring.remove(&profile.username) {
            let _ = handle.join();
        }
        let world_round = self
            .connection
            .profile()
            .and_then(|p| p.public_worlds().map(|worlds| (p, worlds)))
            .map(|(bound, worlds)| {
                let default = self.auto_world.or_else(|| {
                    worlds
                        .by_endpoint(bound.client().game_host(), bound.client().game_port())
                        .map(|world| world.number)
                });
                public_worlds::WorldRound::new(worlds, profile.settings.world, default)
            })
            .transpose()?;
        // Keep the vault credentials on the wall for later spawns and
        // publish a changed world preference to an existing slot before
        // the already-spawned early return.
        self.remember_profile(profile.clone());
        if self.spawned.contains(&profile.username) {
            return Ok(());
        }
        // One `SlotKey` per worker lifetime: every render head the worker
        // attaches carries it, so a respawned profile is a new slot even when
        // its name is reused. Minted before any spawn state changes, so a
        // refused mint leaves nothing behind.
        let slot_key = SLOT_KEYS.mint().map_err(|error| error.to_string())?;
        let slot_render = mailbox.map(|mailbox| SlotRender::new(slot_key, mailbox));
        self.spawned.insert(profile.username.clone());
        let arm = arm.unwrap_or_else(|| SlotArm::new(profile.uid, true));
        // Store through the shared inner fields so a caller's own clone
        // cannot retain stale profile settings.
        sync_profile_arm(&arm, &profile);
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        // The control wake: `Play::wake` kicks the parked slot thread on
        // focus/draw/stop/spawn changes; the slot thread polls the park end.
        let (wake, park) = wake_channel();
        let worker_wake = wake.clone();
        self.wakes.insert(profile.username.clone(), wake);
        // A slot lifetime owns exactly one row/script/command set. Publish
        // every entry synchronously before the worker can run; each lock is
        // released before the next is acquired, so this adds no lock-order edge.
        let username = profile.username.clone();
        let slot_input = input.unwrap_or_else(SlotInput::new);
        let slot_script = script_slot_or_insert(&self.scripts, &username);
        slot_script
            .lock()
            .unwrap()
            .bind_native_input(slot_input.authority());
        slot_script
            .lock()
            .unwrap()
            .bind_world_members(self.connection.world_members());
        slot_script
            .lock()
            .unwrap()
            .bind_quest_pairs(self.quest_pairs.seat(
                &username,
                self.connection.game_host(),
                self.connection.game_port(),
            ));
        // The slot's local-state capability (T5.2): the process service
        // plus the account identity, resolved here on the spawning thread
        // from the composition's explicit root and namespace. A name the
        // server would refuse at login gets none, with one warning line.
        let slot_state = self
            .local_state
            .as_ref()
            .and_then(|state| state.slot_state(&username));
        if let Some(state) = &slot_state {
            slot_script
                .lock()
                .unwrap()
                .bind_local_state(Arc::clone(state));
        }
        self.cheats
            .lock()
            .unwrap()
            .entry(username.clone())
            .or_default();
        self.wires
            .lock()
            .unwrap()
            .entry(username.clone())
            .or_default();
        let work_scope = slot_script
            .lock()
            .expect("slot script mutex is not poisoned")
            .work_scope();
        {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(username.clone()).or_default();
            bot.work_scope = work_scope;
            bot.walk_globals = Some(Arc::clone(&self.walk_globals));
            bot.walk_globals_store = self.walk_globals_store.clone();
        }
        // The account's bank memory, keyed by the login identity every
        // spelling of the name shares (its hint identity), and its hint
        // port on the process state service, both resolved here on the
        // spawning thread (design-bank-snapshot §1.2, §1.4): the slot
        // thread never resolves a root.
        let bank_memory = Arc::clone(
            self.bank_memories
                .entry(super::slot_bank_memory::memory_key(&username).into_owned())
                .or_default(),
        );
        let bank_hint = match (&self.local_state, &slot_state) {
            (Some(composition), Some(state)) => Some(super::slot_bank_memory::SlotHintPort::new(
                Arc::clone(composition.service()),
                state.identity(api::local_state::LocalStateDomain::BankHints),
            )),
            (Some(_), None) => {
                host_log!(
                    Category::BankOp,
                    Level::Warn,
                    slot = &username,
                    "bank hints: the memory lives for this process only"
                );
                None
            }
            (None, _) => None,
        };
        let slot_bank = super::slot_bank_memory::SlotBankMemory::new(bank_memory, bank_hint);
        let world = world_round.as_ref().and_then(|round| {
            self.connection
                .profile()
                .and_then(|profile| profile.public_worlds())
                .map(|worlds| worlds.worlds[round.index].number)
        });
        let mut statuses = lock_statuses(&self.statuses);
        statuses.retain(|status| status.username != username);
        statuses.push(SlotStatus {
            username,
            world,
            ..SlotStatus::default()
        });
        drop(statuses);
        #[cfg(feature = "js")]
        let slot_channels = self.channels.slot(&profile.username);
        spawn_slot_thread(
            SlotThreadArgs {
                connection: self.connection.clone(),
                profile,
                world_round,
                slot_input,
                slot_render,
                // The park end survives re-login rounds (`run_client` is entered
                // once per ingame stretch), so wrap it once here.
                park: Some(Arc::new(park)),
                slot_wake: worker_wake,
                arm,
                slot_cache: Arc::clone(&self.cache),
                ifaces_template: self.ifaces.clone(),
                ifaces_mut_template: self.ifaces_mut_template.clone(),
                slot_queue: Arc::clone(&self.queue),
                slot_statuses: Arc::clone(&self.statuses),
                slot_scripts: Arc::clone(&self.scripts),
                #[cfg(feature = "js")]
                slot_channels,
                slot_cheats: Arc::clone(&self.cheats),
                slot_wires: Arc::clone(&self.wires),
                slot_navs: Arc::clone(&self.navs),
                slot_bank,
                slot_manual_abort_pause: Arc::clone(&self.pause_script_on_manual_walk_abort),
                slot_world: self.world.clone(),
                slot_obj_names: Arc::clone(&self.obj_names),
                slot_catalog_core: self.catalog_core.clone(),
                slot_paired_core: self.paired_core.clone(),
                slot_frame: Arc::clone(&self.per_frame),
                #[cfg(any(test, feature = "test-support"))]
                post_native_player_info: self.post_native_player_info.clone(),
                #[cfg(feature = "test-support")]
                post_host_frame: self.post_host_frame.clone(),
                slot_capacity: Arc::clone(&self.capacity),
            },
            &mut self.handles,
        );
        Ok(())
    }
}
/// End one connected session without carrying deferred game actions into the
/// next login. Operator intent survives (`on_is_up(false)` pauses a started
/// script); packets, isolate interactions, route follows and scene caches
/// belong to the disconnected session and are discarded.
///
/// `keep_work`: the boundary came from an unexpected exit of a running or
/// paused script (see [`end_slot_session`]). A Load script's own work is then
/// held whole for either an automatic relog or a repeat-guard park, and its
/// armed walk resumes after the next login. Frozen AutoRelogin pauses the
/// script on disconnect and resumes it on the new session's scene 2
/// (`AutoRelogin.ts:180-190`, `159-163`). Deliberate logout or Stop ends it.
pub(super) fn reset_slot_session_work(
    name: &str,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    keep_work: bool,
) {
    // `Some(carry)`: the script held its work; `carry` names the run whose
    // armed walk the relogged session re-arms.
    let held = script_slot(scripts, name).and_then(|slot| {
        let Ok(mut slot) = slot.lock() else {
            return None;
        };
        if keep_work && slot.surface_active() {
            let carry = slot.reconnect_session_work();
            Some(carry.then(|| slot.runtime_generation()))
        } else {
            slot.reset_session_work();
            None
        }
    });
    if let Some(queue) = cheats.lock().unwrap().get_mut(name) {
        queue.clear();
    }
    if let Some(queue) = wires.lock().unwrap().get_mut(name) {
        queue.clear();
    }
    match held {
        Some(carry) => hold_script_nav(navs, name, carry),
        None => {
            // The relogged session's status rows reset with the login
            // (`reset_slot_observation`), including the walk destination.
            reset_script_nav(navs, name, None);
        }
    }
    // Prayer varps are temporary: relog starts all-off. Do not carry a toggle
    // from the old session into the new one, even if its route is held.
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        bot.walk_guard = None;
        bot.walk_guard_off = None;
        bot.combat_prayer_off = None;
    }
}

/// The per-slot effects retired together at a session boundary.
struct BoundaryWork<'a> {
    scripts: &'a ScriptWall,
    cheats: &'a Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &'a Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &'a Arc<Mutex<HashMap<String, NavBot>>>,
    #[cfg(feature = "js")]
    channels: &'a super::script_channels::SlotChannels,
}

/// Reset one session boundary of the slot thread. A drop relogs when a login
/// is wanted or a script is running or paused (frozen `wantLogin =
/// credentials && (autoLogin || scriptActive())`,
/// `AutoRelogin.ts:175-190`), unless the slot is stopping or a deliberate
/// operator/persisted logout or the repeat guard latched it off. The repeat
/// guard parks login without discarding a script's work, so explicit Log in
/// can resume it.
///
/// A BroadcastChannel membership belongs to the script run, not to the
/// connection: an isolate this boundary keeps holds its membership suspended,
/// and the next session re-admits it only under the same run generation and
/// world. A stopping slot thread observes no later session, so its member
/// leaves now.
fn reset_slot_session_boundary(
    name: &str,
    arm: &SlotArm,
    work: BoundaryWork<'_>,
    keep_for_exit: bool,
) -> bool {
    let BoundaryWork {
        scripts,
        cheats,
        wires,
        navs,
        #[cfg(feature = "js")]
        channels,
    } = work;
    let deliberate_logout = arm.take_logout_work_reset();
    let stopping = arm.stop.load(Ordering::Relaxed);
    let keep_work = !deliberate_logout && !stopping && keep_for_exit;
    reset_slot_session_work(name, scripts, cheats, wires, navs, keep_work);
    #[cfg(feature = "js")]
    if channels.tracks() {
        let (generation, kept) = script_slot(scripts, name)
            .and_then(|slot| {
                slot.lock()
                    .ok()
                    .map(|slot| (slot.runtime_generation(), slot.surface_active()))
            })
            .unwrap_or((0, false));
        let deliveries = if kept && !stopping {
            channels.suspend(generation)
        } else {
            channels.leave()
        };
        deliver_channel_events(scripts, deliveries);
    }
    keep_work
}

fn log_session_boundary(name: &str, arm: &SlotArm, keep_work: bool, kind: &str) {
    let stopping = arm.stop.load(Ordering::Relaxed);
    let relog = !stopping && arm.relogs_after_drop();
    let (script_active, error_hold, auto_intent) = arm.relog_decision_context();
    host_log!(
        Category::Lifecycle,
        Level::Info,
        slot = name,
        "session boundary kind={kind} keep_work={keep_work} relog={relog} stopping={stopping} script_active={script_active} error_hold={error_hold} auto_intent={auto_intent} auto_login={} want_login={} want_logout={} latch={:?}",
        arm.auto_login(),
        arm.wants_login(),
        arm.wants_logout(),
        arm.login_latch_reason()
    );
}

/// End a connected session and record its one boundary decision.
pub(super) fn end_slot_session(
    name: &str,
    arm: &SlotArm,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    #[cfg(feature = "js")] channels: &super::script_channels::SlotChannels,
) {
    // Reduce a fallback exit exactly once if the pump left without a client
    // exit reason; otherwise consume the decision recorded before that fact.
    arm.set_script_active(script_active(scripts, name));
    let keep_for_exit = arm.take_session_boundary();
    let keep_work = reset_slot_session_boundary(
        name,
        arm,
        BoundaryWork {
            scripts,
            cheats,
            wires,
            navs,
            #[cfg(feature = "js")]
            channels,
        },
        keep_for_exit,
    );
    log_session_boundary(name, arm, keep_work, "exit");
}

/// Apply an operator Logout that arrived after the connected-session boundary.
/// A guard-parked or queued script still owns held work, so the title loop
/// ends that work exactly once before it can honor a later explicit Log in.
pub(super) fn apply_offline_logout_reset(
    name: &str,
    arm: &SlotArm,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    #[cfg(feature = "js")] channels: &super::script_channels::SlotChannels,
) {
    if !arm.take_logout_work_reset() {
        return;
    }
    arm.set_script_active(script_active(scripts, name));
    let (script_active, _, _) = arm.relog_decision_context();
    if script_active {
        let keep_work = reset_slot_session_boundary(
            name,
            arm,
            BoundaryWork {
                scripts,
                cheats,
                wires,
                navs,
                #[cfg(feature = "js")]
                channels,
            },
            false,
        );
        log_session_boundary(name, arm, keep_work, "offline_logout");
    }
}

/// Publish the slot's script activity to its login want, as frozen
/// recomputes `autoLogin || scriptActive()` every frame
/// (`AutoRelogin.ts:175-198`): a script started during an outage logs the
/// slot back in, a stopped one stops the relog nothing else wants.
pub(super) fn sync_script_login(arm: &SlotArm, scripts: &ScriptWall, name: &str) {
    arm.set_script_active(script_active(scripts, name));
}

fn active_guardian_event_name(status: &RandomStatus) -> Option<&str> {
    status
        .name
        .as_deref()
        .filter(|_| status.ours || status.handling)
}

/// Compact prior-frame guardian fact for catalog proof. The observe hook
/// receives last frame's `client_frame` `RandomStatus`.
fn bounded_guardian_fact(status: &RandomStatus) -> catalog_core::BoundedGuardian {
    catalog_core::BoundedGuardian {
        kind: status.kind.map(|kind| {
            match kind {
                api::random::RandomKind::Dialog => "dialog",
                api::random::RandomKind::Pick => "pick",
                api::random::RandomKind::Evade => "evade",
                api::random::RandomKind::Maze => "maze",
                api::random::RandomKind::Mime => "mime",
                api::random::RandomKind::Box => "box",
                api::random::RandomKind::Lamp => "lamp",
                api::random::RandomKind::Hazard => "hazard",
                api::random::RandomKind::LostTool => "lost_tool",
                api::random::RandomKind::LostGear => "lost_gear",
            }
            .to_string()
        }),
        name: status.name.clone(),
        ours: status.ours,
        hold: status.hold,
    }
}

/// Catalog/paired observer hook shared by the slot pump. Lifecycle and guardian
/// facts are produced only while the catalog watch is configured; paired
/// snapshot conversion runs only while the pair watch is configured.
#[allow(clippy::too_many_arguments)]
pub(super) fn observe_slot_catalog_and_paired(
    catalog: &catalog_core::CoreWatch,
    paired: &paired_core::PairWatch,
    account: &str,
    snapshot: &GameSnapshot,
    names: &api::obj_names::ObjNames,
    session_boundary: bool,
    lifecycle_receipt: impl FnOnce() -> Option<script::ScriptLifecycleReceipt>,
    guardian_fact: impl FnOnce() -> catalog_core::BoundedGuardian,
    inspect: Option<catalog_core::RouteInspectPublished>,
    paint: Option<&script::paint::ScriptPaint>,
    acts: Option<catalog_core::ScriptActsPublished>,
) {
    if catalog.configured() {
        let script_lifecycle = lifecycle_receipt();
        let guardian = if session_boundary {
            catalog_core::BoundedGuardian::default()
        } else {
            guardian_fact()
        };
        catalog.observe_snapshot_with_lifecycle(
            account,
            snapshot,
            names,
            script_lifecycle,
            guardian,
            session_boundary,
            inspect,
            paint,
            acts,
        );
    }
    if paired.configured() {
        paired.observe_snapshot(account, snapshot, session_boundary);
    }
}

/// Whether this frame's bot counts toward the host capacity warning.
fn capacity_service_class(
    ingame: bool,
    latched: bool,
    frame: crate::script_runtime::ScriptFrame,
) -> host::ServiceClass {
    if !ingame || latched {
        host::ServiceClass::Parked
    } else if frame.state == Some(script::RunState::Paused) {
        host::ServiceClass::Paused
    } else if frame.frozen || frame.state == Some(script::RunState::Stopping) {
        host::ServiceClass::Suspended
    } else if frame.no_game_packets {
        host::ServiceClass::Quiet
    } else {
        host::ServiceClass::Active
    }
}

/// Handles and configuration moved into a spawned slot thread.
struct SlotThreadArgs {
    connection: PlayConnection,
    profile: Profile,
    world_round: Option<public_worlds::WorldRound>,
    slot_input: Arc<SlotInput>,
    slot_render: Option<SlotRender>,
    park: Option<Arc<SlotPark>>,
    slot_wake: SlotWake,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    slot_queue: SharedLoginQueue,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: ScriptWall,
    #[cfg(feature = "js")]
    slot_channels: super::script_channels::SlotChannels,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    slot_navs: Arc<Mutex<HashMap<String, NavBot>>>,
    /// Borrowed, not cloned: the slot thread's one handle on the account's
    /// bank memory and hint file.
    slot_bank: super::slot_bank_memory::SlotBankMemory,
    slot_manual_abort_pause: Arc<std::sync::atomic::AtomicBool>,
    slot_world: Option<Arc<NavWorld>>,
    slot_obj_names: Arc<api::obj_names::ObjNames>,
    slot_catalog_core: catalog_core::CoreWatch,
    slot_paired_core: paired_core::PairWatch,
    slot_frame: SlotFrame,
    #[cfg(any(test, feature = "test-support"))]
    post_native_player_info: Option<PostFrameObserver>,
    #[cfg(feature = "test-support")]
    post_host_frame: Option<PostFrameObserver>,
    slot_capacity: Arc<host::CapacityMonitor>,
}

/// Per-slot handshake backoff, world rotation, and title-wait state.
struct SlotLoginState {
    backoff: LoginBackoff,
    world_round: Option<public_worlds::WorldRound>,
    world_dirty: bool,
    refresh_key: bool,
    key_refreshed: bool,
    last_relog_park: Option<RelogParkReason>,
    title_wait: Option<TitleWait>,
}

/// Result of polling one title/login handshake step.
enum HandshakeOutcome {
    Proceed { left_pump_to_log_in: bool },
    Continue,
    Stop,
}

/// Per-slot references held across frames inside `run_client`.
struct SlotObserveCtx<'a> {
    args: &'a SlotThreadArgs,
    username: &'a str,
    slot_canlight: Option<Arc<[u64]>>,
    map_members: bool,
    script_tick: &'a mut u64,
    last_nav_step: Option<NavStepKey>,
    nav_world_state_cache: super::script_runtime::NavWorldStateCache,
    session_epoch: u64,
    debug_replies: super::debug_replies::DebugReplies,
}

/// Queued wire commands and checkpoint flags for one wire dispatch pass.
struct DispatchWiresInput {
    wires: VecDeque<WireCmd>,
    exclusive: bool,
    nav_facts: bool,
    tick_edge: bool,
    condition_dispatch: Option<api::interact::PacketCheckpoint>,
}

/// Borrowed snapshot reference matching test-support mutability rules.
#[cfg(feature = "test-support")]
type ObserveSnapshotRef<'a> = &'a mut GameSnapshot;
/// Borrowed snapshot reference matching release-build immutability rules.
#[cfg(not(feature = "test-support"))]
type ObserveSnapshotRef<'a> = &'a GameSnapshot;

/// Per-ingame-stretch state machine advancing frames, status, and script observe.
struct SlotFrameRunner<'a> {
    ctx: SlotObserveCtx<'a>,
    mainland: bool,
    #[cfg(feature = "js")]
    public_world_profile: bool,
    mainland_sent: bool,
    pump: Pump,
    nav_snapshot: GameSnapshot,
    welcome: login_readiness::LoginReadiness,
    title_park_reason: &'a mut Option<RelogParkReason>,
}

/// Frame-level inputs needed to publish one slot status row.
struct FrameStatusInput<'a> {
    ready: bool,
    run_sends: u32,
    status: &'a RandomStatus,
    session_boundary: bool,
    condition: api::condition::BotCondition,
    welcome_step: &'a login_readiness::WelcomeStep,
}

/// Snapshot metadata returned from publishing one frame status row.
struct PublishedFrameStatus {
    up: bool,
    here: Option<(i32, i32, i32)>,
    #[cfg(feature = "js")]
    active_world: Option<u16>,
}

/// Frame-level inputs passed into `script_observe_cached_with_channels`.
struct ObservePassInput<'a> {
    up: bool,
    here: Option<(i32, i32, i32)>,
    hold: bool,
    guardian_hold: bool,
    ours: bool,
    guardian_active_event: Option<&'a str>,
    inv: Option<&'a [(i32, i32)]>,
    #[cfg(feature = "js")]
    npc_boxes: Option<&'a [script::native::snapshot_source::NpcBoxInput]>,
    #[cfg(feature = "js")]
    broker_world: super::script_channels::BrokerWorld,
}

/// Every profile spawns one slot thread; shared handles are threaded through
/// via [`SlotThreadArgs`].
fn spawn_slot_thread(args: SlotThreadArgs, handles: &mut HashMap<String, thread::JoinHandle<()>>) {
    let username = args.profile.username.clone();
    handles.insert(
        username.clone(),
        thread::Builder::new()
            .name(username)
            .stack_size(THREAD_STACK)
            .spawn(move || run_slot_thread(args))
            .expect("failed to spawn slot thread"),
    );
}

fn run_slot_thread(mut args: SlotThreadArgs) {
    let username = args.profile.username.clone();
    let world_round = args.world_round.take();
    api::hostlog::bind_slot(&username);
    host::bind_capacity(&args.slot_capacity);
    let worker_outcome = catch_unwind(AssertUnwindSafe(|| {
        run_slot_worker(&args, world_round, &username)
    }));
    host::unbind_capacity();
    retire_slot_worker(&args, &username, worker_outcome);
}

fn retire_slot_worker(
    args: &SlotThreadArgs,
    username: &str,
    worker_outcome: std::thread::Result<()>,
) {
    // No later session observes this slot, however the worker ended (a Stop
    // while parked at the title never reaches the boundary below): its
    // channel member leaves the party now.
    #[cfg(feature = "js")]
    if args.slot_channels.tracks() {
        deliver_channel_events(&args.slot_scripts, args.slot_channels.leave());
    }
    if !args.arm.stop.load(Ordering::Relaxed) {
        match worker_outcome {
            Ok(()) => {
                publish_worker_terminal(&args.slot_statuses, username, WorkerTerminal::Failed, None)
            }
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic payload".to_string());
                publish_worker_terminal(
                    &args.slot_statuses,
                    username,
                    WorkerTerminal::Panicked,
                    Some(format!("slot worker panicked: {detail}")),
                );
            }
        }
    }
}

fn run_slot_worker(
    args: &SlotThreadArgs,
    world_round: Option<public_worlds::WorldRound>,
    username: &str,
) {
    #[cfg(test)]
    let startup_entries_published = args.arm.wait_worker_start_for_test();
    let _queue_retirement = QueuePlaceRetirement {
        queue: &args.slot_queue,
        statuses: &args.slot_statuses,
        username,
        arm: &args.arm,
    };
    #[cfg(feature = "memory-profile")]
    let _duel_retirement = memory::DuelReportRetirement { name: username };
    // Pending bank observations are saved however the worker leaves
    // (design-bank-snapshot §1.4: slot exit is a save point).
    let _bank_exit_save = super::slot_bank_memory::ExitSave(&args.slot_bank);
    #[cfg(test)]
    if let Some(published) = startup_entries_published {
        published.send(()).unwrap();
    }
    // Preparation has no determinate client percentage, but publish a phase
    // immediately so a slow cache fetch is visibly active.
    publish_startup_phase(&args.slot_statuses, username, "Preparing client");
    let park = args.park.clone();
    let Some(mut client) = prepare_slot_client(args, username) else {
        return;
    };
    let mut login = SlotLoginState {
        backoff: LoginBackoff::new(),
        world_dirty: world_round.is_some(),
        world_round,
        refresh_key: false,
        key_refreshed: false,
        last_relog_park: None,
        // Frames that keep the logged-out title live while a login waits
        // outside the pump; built by the first wait of a login series and
        // dropped before the pump takes over.
        title_wait: None,
    };
    // Slot-thread tick identity: one increment per observed drain containing
    // PLAYER_INFO. It survives relogs; dirty snapshots and host receipts reuse
    // it, never a wall-clock/frame-derived tick.
    let mut script_tick: u64 = 0;
    let mut run_policy = ScriptRunPolicy::default();
    loop {
        if args.arm.stop.load(Ordering::Relaxed) {
            return;
        }
        let left_pump_to_log_in =
            match login.step_offline_and_handshake(args, username, &mut client) {
                HandshakeOutcome::Proceed {
                    left_pump_to_log_in,
                } => left_pump_to_log_in,
                HandshakeOutcome::Continue => continue,
                HandshakeOutcome::Stop => return,
            };
        if left_pump_to_log_in && !client.ingame {
            // Back on the title after a login attempt: input made while no
            // title frame ran (the handshake, or the moment a wait was
            // cancelled) answers no screen the operator saw, so a stale Log In
            // must not undo a Logout.
            args.slot_input.discard_user(&mut client.shell);
            show_title_message(&mut client, "", "");
        }
        let pump_had_session = client.ingame;
        let slot_game_data = args
            .connection
            .profile()
            .and_then(|profile| profile.game_data());
        let mut knock =
            ScriptRandomClaims::new(Arc::clone(&args.slot_scripts), username.to_owned());
        if let Some(game_data) = slot_game_data.clone() {
            knock = knock.with_fight(FightService::new(Arc::clone(&args.arm), game_data));
        }
        let mut runner =
            SlotFrameRunner::new(args, username, &mut script_tick, &mut login.last_relog_park);
        let probe_arm = Arc::clone(&args.arm);
        let ifaces_template = Arc::clone(&args.ifaces_template);
        run_policy = Host::run_client_with_event_policies(
            &mut client,
            username,
            args.profile.settings.clone(),
            Arc::clone(&args.arm.random_events),
            Arc::clone(&args.arm.lamp_auto),
            Arc::clone(&args.arm.lamp_skill),
            Arc::clone(&args.arm.random_event_policies),
            slot_game_data,
            Some(Arc::clone(&args.slot_input)),
            args.slot_render.clone(),
            park.clone(),
            run_policy,
            move |c, _ignored, run_sends, status, run_policy| {
                runner.observe_frame(c, run_sends, status, run_policy)
            },
            move |c| slot_client_pump_should_exit(c, &ifaces_template, &probe_arm),
            knock,
        );
        if pump_had_session && args.arm.session_boundary_pending() {
            publish_slot_disconnected(&args.slot_statuses, username);
            end_slot_session(
                username,
                &args.arm,
                &args.slot_scripts,
                &args.slot_cheats,
                &args.slot_wires,
                &args.slot_navs,
                #[cfg(feature = "js")]
                &args.slot_channels,
            );
            args.slot_bank.session_ended();
        }
        if args.arm.stop.load(Ordering::Relaxed) {
            return;
        }
    }
}

fn prepare_slot_client(args: &SlotThreadArgs, username: &str) -> Option<Client> {
    let mut client = match &args.connection {
        PlayConnection::Direct(options) => prepare_client(
            bot_client_config(options, &args.profile),
            args.profile.uid,
            Arc::clone(&args.slot_cache),
            args.ifaces_template.clone(),
            args.ifaces_mut_template.clone(),
        ),
        PlayConnection::Bound { template, .. } => {
            match template.prepare_client(args.profile.uid, args.profile.settings.lowmem) {
                Ok(client) => client,
                Err(error) => {
                    if let Some(row) = lock_statuses(&args.slot_statuses)
                        .iter_mut()
                        .find(|s| s.username == username)
                    {
                        row.startup_phase = StartupPhase::Error;
                        row.startup_phase_started = Instant::now();
                        row.error = Some(error);
                    }
                    clear_startup_progress(&args.slot_statuses, username);
                    return None;
                }
            }
        }
    };
    if let PlayConnection::Direct(options) = &args.connection {
        client.set_transport(options.transport);
        client.set_cheat_admission(match args.connection.profile_class() {
            crate::ProfileClass::Local => client::CheatAdmission::Granted,
            crate::ProfileClass::Remote => client::CheatAdmission::Remote(options.transport),
        });
    }
    // The host owns every reconnect attempt so each fresh socket returns
    // through the shared FIFO and reservation accounting.
    client.set_external_reconnect_owner(true);
    host_log!(Category::Lifecycle, Level::Info, "thread up");

    // Jag/anim/model/map prefetch (mirrors client-play; the scene cannot
    // reach scene_state 2 until the loc models are in). `maininit` is
    // renderer-free now: progress recording lives on the Client, and no
    // `Renderer` is constructed for a headless slot.
    #[cfg(any(test, feature = "test-support"))]
    let bypass_asset_startup = args.arm.bypass_asset_startup.load(Ordering::Relaxed);
    #[cfg(not(any(test, feature = "test-support")))]
    let bypass_asset_startup = false;
    if !bypass_asset_startup {
        client.maininit_with_progress(Some(&mut |client, message, percent| {
            if args.arm.stop.load(Ordering::Relaxed) {
                client.shell.stop();
            } else {
                publish_startup_progress(&args.slot_statuses, username, message, percent);
            }
        }));
    }
    if client.error_loading && args.connection.profile().is_some() {
        if let Some(row) = lock_statuses(&args.slot_statuses)
            .iter_mut()
            .find(|s| s.username == username)
        {
            row.startup_phase = StartupPhase::Error;
            row.startup_phase_started = Instant::now();
            row.error = Some(format!(
                "profile asset initialization failed: {}",
                client.last_progress_message
            ));
        }
        clear_startup_progress(&args.slot_statuses, username);
        return None;
    }
    clear_startup_progress(&args.slot_statuses, username);
    set_startup_phase(&args.slot_statuses, username, StartupPhase::Queueing);
    if client.error_loading {
        host_log!(Category::Lifecycle, Level::Error, "maininit failed");
    }
    Some(client)
}

fn sync_offline_title_state(args: &SlotThreadArgs, username: &str) {
    sync_script_login(&args.arm, &args.slot_scripts, username);
    apply_offline_logout_reset(
        username,
        &args.arm,
        &args.slot_scripts,
        &args.slot_cheats,
        &args.slot_wires,
        &args.slot_navs,
        #[cfg(feature = "js")]
        &args.slot_channels,
    );
}

impl SlotLoginState {
    fn step_offline_and_handshake(
        &mut self,
        args: &SlotThreadArgs,
        username: &str,
        client: &mut Client,
    ) -> HandshakeOutcome {
        if !client.ingame {
            sync_offline_title_state(args, username);
            if !should_handshake(&args.arm, client.ingame) {
                let reason = args.arm.relog_park_reason();
                if self.last_relog_park != Some(reason) {
                    host_log!(
                        Category::Login,
                        Level::Info,
                        "relog decision park reason={reason}"
                    );
                    self.last_relog_park = Some(reason);
                }
                // No pending intent (title hold, latched logout, or a withdrawn
                // wait): the live title pump holds no FIFO place and publishes
                // no `k of n`.
                drop_queue_place(
                    &args.slot_queue,
                    &args.slot_statuses,
                    username,
                    args.arm.queue_owner,
                );
                publish_login_latched_from_arm(&args.slot_statuses, username, &args.arm);
            }
        }
        if !client.ingame && should_handshake(&args.arm, client.ingame) {
            return self.run_handshake(args, username, client);
        }
        // The pump's own frames take over the title from any wait.
        HandshakeOutcome::Proceed {
            left_pump_to_log_in: self.title_wait.take().is_some(),
        }
    }

    fn ensure_title_wait<'w>(
        slot: &'w mut Option<TitleWait>,
        args: &SlotThreadArgs,
    ) -> &'w mut TitleWait {
        slot.get_or_insert_with(|| {
            TitleWait::new(Some(Arc::clone(&args.slot_input)), args.slot_render.clone())
        })
    }

    fn run_handshake(
        &mut self,
        args: &SlotThreadArgs,
        username: &str,
        client: &mut Client,
    ) -> HandshakeOutcome {
        // A request raised after the observe armed this handshake must not
        // survive the ownership handoff and undo a later operator Logout.
        let _ = client.take_title_login_request();
        self.last_relog_park = None;
        Self::ensure_title_wait(&mut self.title_wait, args);
        // Leaving title park for handshake: refresh latch from the arm so an
        // explicit Log in cannot keep a stale TRUE from the last park publish
        // through queue/connect.
        publish_login_latched_from_arm(&args.slot_statuses, username, &args.arm);
        if let Some(round) = self.world_round.as_mut() {
            let worlds = args
                .connection
                .profile()
                .and_then(|p| p.public_worlds())
                .expect("public world round requires bound public worlds");
            match refresh_slot_world_preference(round, worlds, &args.arm) {
                Ok(true) => {
                    self.world_dirty = true;
                    self.refresh_key = false;
                    self.key_refreshed = false;
                }
                Ok(false) => {}
                Err(error) => {
                    if let Some(row) = lock_statuses(&args.slot_statuses)
                        .iter_mut()
                        .find(|s| s.username == username)
                    {
                        row.startup_phase = StartupPhase::Error;
                        row.startup_phase_started = Instant::now();
                        row.error = Some(format!("public world preference failed: {error}"));
                    }
                    args.arm.hold_login_on_error();
                    return HandshakeOutcome::Continue;
                }
            }
        }
        let Some((login_command, login_world_generation)) =
            capture_login_authorization(&args.arm, client.ingame)
        else {
            return HandshakeOutcome::Continue;
        };
        // The slot can outlive a profile uid edit. Prepare the next handshake
        // and its queue accounting from the live identity; K4's publication
        // fence rejects an edit that races this snapshot.
        let uid = args.arm.uid.load(Ordering::Acquire);
        client.login_uid = uid;
        if self.world_dirty && !self.configure_public_world(args, username, client) {
            return HandshakeOutcome::Stop;
        }
        // Publish only the command that selected this world.
        args.arm.prepared();
        enqueue_queue_place(
            &args.slot_queue,
            &args.slot_statuses,
            username,
            uid,
            &args.arm,
        );
        #[cfg(test)]
        args.arm.panic_after_queue_for_test();
        if let Some(until) = args.arm.clean_release_admission_at(
            &client.config.host,
            client.config.port,
            Instant::now(),
        ) {
            args.slot_queue
                .lock()
                .defer_owner_until(args.arm.queue_owner, until);
        }
        host_log!(Category::Login, Level::Info, "relog decision wait queue");
        show_title_message(client, "", "Waiting to log in...");
        let wait = wait_for_permit(
            &args.slot_queue,
            &args.slot_statuses,
            username,
            uid,
            &args.arm,
            &mut title_frames(
                Self::ensure_title_wait(&mut self.title_wait, args),
                client,
                username,
                &args.slot_frame,
                None,
            ),
        );
        // The reservation is owned from the granting poll on: every
        // `continue`/`return` (or unwind) below that drops it unused gives the
        // grant back.
        let PermitWait::Granted(mut permit) = wait else {
            return if args.arm.stop.load(Ordering::Relaxed) {
                HandshakeOutcome::Stop
            } else {
                HandshakeOutcome::Continue
            };
        };
        if !granted_permit_world_is_current(self.world_round.as_ref(), &args.arm) {
            drop(permit);
            // A returned grant must not immediately re-enter the queue while
            // the world preference stays mismatched.
            args.arm.wait_for_retry(
                Duration::from_millis(50),
                None,
                &mut title_frames(
                    Self::ensure_title_wait(&mut self.title_wait, args),
                    client,
                    username,
                    &args.slot_frame,
                    None,
                ),
            );
            return HandshakeOutcome::Continue;
        }
        // Script demand may change during the FIFO wait. Reduce it without
        // replacing the captured authorization.
        sync_script_login(&args.arm, &args.slot_scripts, username);
        let reconnect = args.arm.reconnects();
        host_log!(
            Category::Login,
            Level::Info,
            "relog decision attempt reconnect={reconnect}"
        );
        // Read at each handshake, not captured at spawn: a password saved
        // since then applies to this login.
        let password = args.arm.login_password();
        // An in-world reconnect is only a server-side socket swap: it retains
        // the old mode and tabs. Only a clean logout permits the queued mode
        // to apply at this handshake.
        if args.arm.login_applies_memory() {
            if let Some(lowmem) = args.arm.lowmem_handshake() {
                if !client.configure_login_memory(lowmem) {
                    host_log!(
                        Category::Login,
                        Level::Warn,
                        "login memory configuration refused"
                    );
                }
            }
        }
        let handshake_lowmem = client.config.lowmem;
        // Publication is still an unused permit. A Logout or Stop before it
        // must not charge start spacing or UID/IP TTLs.
        let hold = Cell::new(None);
        let Some(login) = permit.attempt(
            |cancel| {
                args.arm
                    .begin_attempt(login_command, login_world_generation, cancel)
            },
            |deadline, cancel| {
                mark_login_started(&args.slot_statuses, username);
                let (result, server_hold) = SlotLogin {
                    client,
                    arm: &args.arm,
                    command: login_command,
                    world_generation: login_world_generation,
                    username,
                    password: &password,
                    reconnect,
                    backoff: &self.backoff,
                }
                .run(deadline, cancel, |_| {});
                hold.set(server_hold);
                result
            },
            |_| hold.get(),
        ) else {
            args.arm.clear_unused_attempt();
            drop(permit);
            // Refusal returns the unused grant before a bounded park; a stable
            // stale capture cannot spin the queue.
            args.arm.wait_for_retry(
                Duration::from_millis(50),
                None,
                &mut title_frames(
                    Self::ensure_title_wait(&mut self.title_wait, args),
                    client,
                    username,
                    &args.slot_frame,
                    None,
                ),
            );
            return HandshakeOutcome::Continue;
        };
        match login {
            Ok(false) => HandshakeOutcome::Continue,
            Ok(true) => {
                show_title_message(client, "", "");
                self.backoff.reset();
                if let Some(round) = self.world_round.as_mut() {
                    round.reset();
                }
                self.key_refreshed = false;
                publish_login_lowmem(&args.slot_statuses, username, handshake_lowmem);
                set_startup_phase(&args.slot_statuses, username, StartupPhase::LoadingScene);
                host_log!(Category::Login, Level::Info, "handshake ok");
                HandshakeOutcome::Proceed {
                    left_pump_to_log_in: self.title_wait.take().is_some(),
                }
            }
            Err(e) => {
                self.handle_login_failure(
                    args,
                    username,
                    client,
                    &e,
                    (login_command, login_world_generation),
                );
                HandshakeOutcome::Continue
            }
        }
    }

    fn configure_public_world(
        &mut self,
        args: &SlotThreadArgs,
        username: &str,
        client: &mut Client,
    ) -> bool {
        let round = self.world_round.as_ref().expect("public world round");
        let worlds = args
            .connection
            .profile()
            .and_then(|p| p.public_worlds())
            .expect("bound public worlds");
        let world = &worlds.worlds[round.index];
        if let Err(error) = configure_slot_world(client, world, self.refresh_key, &args.arm.stop) {
            if args.arm.stop.load(Ordering::Relaxed) {
                return false;
            }
            if let Some(row) = lock_statuses(&args.slot_statuses)
                .iter_mut()
                .find(|s| s.username == username)
            {
                row.startup_phase = StartupPhase::Error;
                row.startup_phase_started = Instant::now();
                row.error = Some(format!("public world login configuration failed: {error}"));
            }
            clear_startup_progress(&args.slot_statuses, username);
            return false;
        }
        if let Some(slot) = script_slot(&args.slot_scripts, username) {
            slot.lock()
                .unwrap()
                .pair_world_changed(&client.config.host, client.config.port);
        }
        if args.arm.stop.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(row) = lock_statuses(&args.slot_statuses)
            .iter_mut()
            .find(|s| s.username == username)
        {
            row.world = Some(world.number);
        }
        // The next session to log in is bound to this world.
        args.arm.note_configured_world(world.number);
        host_log!(
            Category::Login,
            Level::Info,
            "world w{} {}:{} node {}",
            world.number,
            world.host,
            world.port,
            world.node_id
        );
        self.world_dirty = false;
        self.refresh_key = false;
        true
    }

    fn handle_login_failure(
        &mut self,
        args: &SlotThreadArgs,
        username: &str,
        client: &mut Client,
        error: &LoginError,
        (login_command, login_world_generation): (IntentCommand, u64),
    ) {
        let wait_frames = Self::ensure_title_wait(&mut self.title_wait, args);
        show_title_message(client, &error.mes1, &error.mes2);
        let transfer_countdown = Cell::new(None);
        if wait_for_transfer_response(
            error,
            &args.arm,
            &args.slot_statuses,
            username,
            &transfer_countdown,
            &mut title_frames(
                wait_frames,
                client,
                username,
                &args.slot_frame,
                Some((&transfer_countdown, &error.mes1)),
            ),
        )
        .is_some()
        {
            return;
        }
        if !args
            .arm
            .login_attempt_is_current(login_command, login_world_generation)
        {
            return;
        }
        record_login_error(&args.slot_statuses, username, error);
        if let Some(delay) = args.arm.defer_clean_release_at(
            error,
            login_command,
            &client.config.host,
            client.config.port,
            Instant::now(),
        ) {
            host_log!(
                Category::Login,
                Level::Info,
                "relog decision release_wait_ms={} code=5; rejoining login queue",
                delay.as_millis()
            );
            return;
        }
        let worlds = args.connection.profile().and_then(|p| p.public_worlds());
        let plan = plan_login_retry(
            error,
            self.world_round
                .as_mut()
                .map(|round| (round, &**worlds.expect("bound public worlds"))),
            (&client.config.host, client.config.port),
            &mut self.backoff,
            rand_core::RngCore::next_u32(&mut rand_core::OsRng),
        );
        if let Some(receipt) = plan.world_full {
            host_log!(Category::Login, Level::Warn, "{receipt}");
        }
        if plan.switch_world {
            self.world_dirty = true;
            self.key_refreshed = false;
        }
        // Auto world rotation: straight back into the paced queue for the next
        // world.
        let Some(retry) = plan.wait else {
            return;
        };
        if self.world_round.is_some()
            && public_worlds::refresh_after_login_error(error.code, &mut self.key_refreshed)
        {
            self.refresh_key = true;
            self.world_dirty = true;
        }
        host_log!(
            Category::Login,
            Level::Info,
            "relog decision wait backoff_ms={} code={}",
            retry.as_millis(),
            error.code
        );
        args.arm.wait_for_retry(
            retry,
            Some((login_command, login_world_generation)),
            &mut title_frames(wait_frames, client, username, &args.slot_frame, None),
        );
    }
}

impl<'a> SlotFrameRunner<'a> {
    fn new(
        args: &'a SlotThreadArgs,
        username: &'a str,
        script_tick: &'a mut u64,
        title_park_reason: &'a mut Option<RelogParkReason>,
    ) -> Self {
        let mainland = match &args.connection {
            PlayConnection::Direct(options) => options.mainland,
            PlayConnection::Bound { mainland, .. } => *mainland,
        };
        #[cfg(feature = "js")]
        let public_world_profile = args
            .connection
            .profile()
            .and_then(|profile| profile.public_worlds())
            .is_some();
        let slot_canlight = args.connection.profile().and_then(|p| p.canlight());
        let map_members = args
            .connection
            .profile()
            .map(|p| p.map_members())
            .unwrap_or(false);
        Self {
            ctx: SlotObserveCtx {
                args,
                username,
                slot_canlight,
                map_members,
                script_tick,
                // Last `(player gen, here)` the nav bot stepped: skip until
                // either changes so the hop budget counts server ticks, not 20 ms
                // frames (panel `tick_latch`).
                last_nav_step: None,
                nav_world_state_cache: super::script_runtime::NavWorldStateCache::default(),
                session_epoch: 0,
                debug_replies: super::debug_replies::DebugReplies::default(),
            },
            mainland,
            #[cfg(feature = "js")]
            public_world_profile,
            mainland_sent: false,
            pump: Pump::new(),
            // The slot's per-tick nav snapshot: rebuilt each observe when a
            // family's gen moved (the walk arm gates on its facts; the follow
            // surface reads the canonical base + route-head tile from it).
            nav_snapshot: GameSnapshot::new(),
            welcome: login_readiness::LoginReadiness::default(),
            title_park_reason,
        }
    }

    fn observe_frame(
        &mut self,
        c: &mut Client,
        run_sends: u32,
        status: &RandomStatus,
        run_policy: &mut ScriptRunPolicy,
    ) -> bool {
        let name = self.ctx.username;
        #[cfg(feature = "test-support")]
        if let Some(test_hook) = TickEngineTestHook::bind_current_for_account(name) {
            test_hook.bind_client(c, name, Arc::clone(&self.ctx.args.slot_input));
        }
        if !c.ingame {
            self.step_logged_out_title_frame(c);
        }
        let drain = self.pump.drain_client(c);
        let session_boundary = publish_session_boundary_status(
            &self.ctx.args.slot_statuses,
            name,
            drain.session_changed,
            c.ingame,
            self.nav_snapshot.ingame(),
        );
        if session_boundary {
            self.ctx.handle_session_boundary(c);
        }
        self.nav_snapshot
            .set_loc_projection_wanted(run_policy.loc_projection_wanted());
        host::publish_snapshot(&mut self.nav_snapshot, c, drain);
        if session_boundary && c.ingame {
            self.ctx
                .args
                .slot_navs
                .lock()
                .unwrap()
                .entry(name.to_owned())
                .or_default()
                .condition
                .session_started(
                    self.nav_snapshot.tick(),
                    c.chat_seq as i32,
                    self.ctx
                        .args
                        .slot_world
                        .as_deref()
                        .and_then(crate::admission::condition_tables),
                );
        }
        // The open bank's rows reach the account memory here
        // (design-bank-snapshot §1.3); a close saves.
        self.ctx.args.slot_bank.observe_frame(&self.nav_snapshot);
        let condition = {
            let mut navs = self.ctx.args.slot_navs.lock().unwrap();
            if let Some(bot) = navs.get_mut(name) {
                bot.condition
                    .observe_frame(&self.nav_snapshot, &drain, status.hold);
                bot.condition.published()
            } else {
                api::condition::BotCondition::default()
            }
        };
        let condition_dispatch = self
            .ctx
            .args
            .slot_navs
            .lock()
            .unwrap()
            .get(name)
            .and_then(|bot| bot.condition.dispatch_checkpoint(c));
        api::hostlog::set_tick(self.nav_snapshot.tick());
        self.ctx.debug_replies.observe(name, &self.nav_snapshot);
        self.publish_catalog_and_paired(status, session_boundary);
        let ready = c.ingame && c.scene_state == 2 && self.nav_snapshot.local_player().is_some();
        let welcome_obs = login_readiness::WelcomeObservation {
            session_epoch: self.ctx.session_epoch,
            tick: *self.ctx.script_tick,
            now: Instant::now(),
            ingame: c.ingame,
            scene_state: c.scene_state,
            welcome_interface_id: c.welcome_interface_id,
            main_modal_id: c.main_modal_id,
            allow_close: c.ingame && c.scene_state == 2 && !session_boundary,
        };
        let welcome_step = self.welcome.step(&welcome_obs, || {
            login_readiness::try_close_welcome(&self.nav_snapshot, c)
        });
        if let Some(line) = welcome_step.notice.as_deref() {
            host_log!(stderr; Category::Echo, Level::Info, "{line}");
        }
        #[cfg(test)]
        let injected_maze_hold = self.try_inject_maze_hold(ready, status, session_boundary);
        #[cfg(test)]
        let guardian_hold = status.hold || injected_maze_hold;
        #[cfg(not(test))]
        let guardian_hold = status.hold;
        let session_ready = self.ctx.args.arm.update_session_readiness(
            ready,
            welcome_step.hold,
            guardian_hold,
            session_boundary,
        );
        // This frame's random status is published before observe; its hold
        // freezes script tick and nav follow.
        let hold = !session_ready || !ready || session_boundary;
        // Collect intent before either follow pump or script dispatch; this is
        // the shared takeover ordering seam. Keep these commands for their
        // normal late send/hold gate.
        let (mut frame_input, wires) = take_slot_frame_input(
            &self.ctx.args.slot_input,
            name,
            &self.ctx.args.slot_wires,
            hold,
            status,
        );
        take_manual_walk_ownership(
            &self.ctx.args.slot_scripts,
            &self.ctx.args.slot_navs,
            name,
            frame_input,
            c.ingame && !session_boundary,
            *self.ctx.script_tick,
            self.ctx
                .args
                .slot_manual_abort_pause
                .load(Ordering::Relaxed),
        );
        // The Guardian's Fight child (design V11D 4.2, 4.4). A manual step is
        // a takeover: it revokes the child before anything dispatches, and the
        // Guardian's own escape answers the event after the manual lease.
        // Otherwise the child may act under the Guardian's hold and under no
        // other reason's.
        let manual = frame_input.manual_move_count() > 0;
        if let (true, Some(token)) = (manual, status.fight) {
            if let Some(slot) = script_slot(&self.ctx.args.slot_scripts, name) {
                if let Ok(mut slot) = slot.lock() {
                    slot.guardian_revoke_token(token, api::random::FightCancel::ManualTakeover);
                }
            }
        }
        self.ctx.args.arm.fight.publish(
            status.fight.filter(|_| !manual && !session_boundary),
            c.ingame
                && !session_boundary
                && !manual
                && self.ctx.args.arm.guardian_child_ready(ready),
        );
        let frame = script_frame(&self.ctx.args.slot_scripts, name);
        let script_state = frame.state;
        let running = script_state == Some(script::RunState::Running);
        host::set_service_class(capacity_service_class(
            c.ingame,
            self.ctx.args.arm.login_latched(),
            frame,
        ));
        let nav_facts = script_state_builds_nav_facts(script_state);
        let (nav_owned, nav_armed) =
            self.ctx
                .args
                .slot_navs
                .lock()
                .unwrap()
                .get(name)
                .map_or((false, false), |bot| {
                    (
                        bot.ordinary_movement_owned(self.nav_snapshot.tick() as u16),
                        bot.has_route() || bot.bank_fetch.is_some(),
                    )
                });
        frame_input.host_move_owned = project_slot_frame_host_move_owned(script_state, nav_owned);
        #[cfg(feature = "memory-profile")]
        memory::client_frame(c, name, hold, script_state);
        (self.ctx.args.slot_frame)(c, name, frame_input);
        // Operator pause parks ScriptRunner work, not BotHost client frames
        // (`BotHost.ts:48-55`).
        let mut frame_delivered = false;
        if matches!(
            script_state,
            Some(script::RunState::Starting | script::RunState::Running | script::RunState::Paused,)
        ) {
            if let Some(slot) = script_slot(&self.ctx.args.slot_scripts, name) {
                frame_delivered = dispatch_client_frame(&slot, name);
            }
        }
        if !self.mainland_sent && self.mainland && ready {
            api::interact::mainland_hop(c);
            self.mainland_sent = true;
            host_log!(Category::Lifecycle, Level::Info, "mainland hop queued");
        }
        let tick_edge = should_emit_tick(drain.player_info);
        if tick_edge {
            *self.ctx.script_tick = self.ctx.script_tick.wrapping_add(1);
        }
        #[cfg(feature = "test-support")]
        if let Some(test_hook) = TickEngineTestHook::current() {
            let tick_engine_suppressed_openchat = test_hook.before_snapshot_observe(
                name,
                &mut self.nav_snapshot,
                *self.ctx.script_tick as u32,
            );
            test_hook.before_frame(
                c,
                name,
                &self.nav_snapshot,
                *self.ctx.script_tick as u32,
                tick_edge,
                tick_engine_suppressed_openchat,
            );
        }
        let published = self.publish_frame_status(
            c,
            &FrameStatusInput {
                ready,
                run_sends,
                status,
                session_boundary,
                condition,
                welcome_step: &welcome_step,
            },
        );
        let here = published.here;
        #[cfg(feature = "js")]
        let broker_world = if self.public_world_profile {
            published.active_world.map_or(
                super::script_channels::BrokerWorld::Unavailable,
                super::script_channels::BrokerWorld::Public,
            )
        } else {
            super::script_channels::BrokerWorld::Local
        };
        let inv = observe_script_inv(running, tick_edge, &self.nav_snapshot);
        #[cfg(feature = "test-support")]
        let test_inv = inv.map(<[_]>::to_vec);
        #[cfg(feature = "test-support")]
        let inv = test_inv.as_deref();
        let nav_state = nav_world_state_for_observe(
            &mut self.ctx.nav_world_state_cache,
            here,
            &self.nav_snapshot,
            nav_facts,
            nav_armed,
            self.ctx.map_members,
            self.ctx.session_epoch,
        );
        #[cfg(feature = "js")]
        let npc_boxes = project_npc_boxes_for_isolate_snapshot(
            &self.ctx.args.slot_scripts,
            name,
            tick_edge,
            || projected_npc_boxes(c),
        );
        let pass_input = ObservePassInput {
            up: published.up,
            here,
            hold,
            guardian_hold,
            ours: status.ours,
            guardian_active_event: active_guardian_event_name(status),
            inv,
            #[cfg(feature = "js")]
            npc_boxes: npc_boxes.as_deref(),
            #[cfg(feature = "js")]
            broker_world,
        };
        #[cfg(feature = "test-support")]
        let nav_snapshot_ref = &mut self.nav_snapshot;
        #[cfg(not(feature = "test-support"))]
        let nav_snapshot_ref = &self.nav_snapshot;
        let crate::script_runtime::ScriptObservation {
            wrote: _wrote,
            journal_paint_hidden,
            exclusive,
        } = self.ctx.run_observe(
            c,
            nav_snapshot_ref,
            &pass_input,
            (tick_edge, script_evidence_dirty(drain.dirty)),
            nav_state,
            run_policy,
        );
        #[cfg(feature = "test-support")]
        if let Some(test_hook) = TickEngineTestHook::current() {
            let status = script_slot(&self.ctx.args.slot_scripts, name)
                .and_then(|slot| slot.lock().ok()?.native_status());
            test_hook.after_frame(name, tick_edge, status);
        }
        #[cfg(feature = "test-support")]
        if let Some(observer) = &self.ctx.args.post_host_frame {
            observer(c, name);
        }
        #[cfg(test)]
        if crate::combat_proof::capture_enabled(name) {
            crate::combat_proof::record_frame(name, *self.ctx.script_tick, &self.nav_snapshot);
            crate::combat_proof::record_observation(
                name,
                *self.ctx.script_tick,
                &self.nav_snapshot,
                exclusive,
                tick_edge,
            );
            if let Some(status) = script_slot(&self.ctx.args.slot_scripts, name)
                .and_then(|slot| slot.lock().ok()?.native_status())
            {
                crate::combat_proof::record_status(name, &status, Some(*self.ctx.script_tick));
            }
        }
        c.set_journal_paint_hidden(c.ingame && journal_paint_hidden);
        #[cfg(feature = "test-support")]
        let nav_snapshot_ref = &mut self.nav_snapshot;
        #[cfg(not(feature = "test-support"))]
        let nav_snapshot_ref = &self.nav_snapshot;
        self.ctx.dispatch_wires_and_nav(
            c,
            nav_snapshot_ref,
            &pass_input,
            run_policy,
            DispatchWiresInput {
                wires,
                exclusive,
                nav_facts,
                tick_edge,
                condition_dispatch,
            },
        );
        self.ctx.slot_requires_tick(running, frame_delivered)
    }

    fn step_logged_out_title_frame(&mut self, c: &mut Client) {
        // The live title pump owns offline script/latch synchronization. A
        // hosted title click is only operator intent: arm the ordinary
        // vault/FIFO path and never use the client's title fields.
        if c.take_title_login_request() {
            self.ctx.args.arm.arm_explicit_login();
            self.ctx.args.slot_wake.wake();
        }
        sync_offline_title_state(self.ctx.args, self.ctx.username);
        if should_handshake(&self.ctx.args.arm, c.ingame) {
            *self.title_park_reason = None;
        } else {
            let reason = self.ctx.args.arm.relog_park_reason();
            if *self.title_park_reason != Some(reason) {
                host_log!(
                    Category::Login,
                    Level::Info,
                    "relog decision park reason={reason}"
                );
                *self.title_park_reason = Some(reason);
            }
        }
    }

    fn publish_catalog_and_paired(&self, status: &RandomStatus, session_boundary: bool) {
        let name = self.ctx.username;
        // `script_observe_with_npc_boxes` below reaps the isolate and can
        // publish a Stop receipt. The next frame attaches that bounded value
        // here before status publication; pending panel logs are never consumed
        // by this read. `status` is this frame's client_frame publication,
        // copied onto the slot row below. The hunt cards also read the slot's
        // own record of the requests host-play dispatched for it.
        let copies_inspect = self.ctx.args.slot_catalog_core.copies_route_inspect();
        let copies_hunt = self.ctx.args.slot_catalog_core.copies_hunt();
        let (inspect, acts) = if copies_inspect || copies_hunt {
            let navs = self.ctx.args.slot_navs.lock().unwrap();
            let bot = navs.get(name);
            (
                bot.filter(|_| copies_inspect)
                    .map(|bot| bot.inspect.published_core_facts()),
                copies_hunt.then(|| bot.map(|bot| bot.acts.published()).unwrap_or_default()),
            )
        } else {
            (None, None)
        };
        let catalog_paint = if copies_hunt {
            script_paint_of(&self.ctx.args.slot_scripts, name)
        } else {
            None
        };
        observe_slot_catalog_and_paired(
            &self.ctx.args.slot_catalog_core,
            &self.ctx.args.slot_paired_core,
            name,
            &self.nav_snapshot,
            self.ctx.args.slot_obj_names.as_ref(),
            session_boundary,
            || {
                script_slot(&self.ctx.args.slot_scripts, name)
                    .and_then(|slot| slot.lock().ok().and_then(|slot| slot.lifecycle_receipt()))
            },
            || bounded_guardian_fact(status),
            inspect,
            catalog_paint.as_deref(),
            acts,
        );
    }

    #[cfg(test)]
    fn try_inject_maze_hold(
        &self,
        ready: bool,
        status: &RandomStatus,
        session_boundary: bool,
    ) -> bool {
        let name = self.ctx.username;
        if !(ready
            && !status.hold
            && !session_boundary
            && crate::combat_proof::maze_injection_pending(name))
        {
            return false;
        }
        if !crate::combat_proof::protect_from_melee_active(&self.nav_snapshot) {
            return false;
        }
        match script_slot(&self.ctx.args.slot_scripts, name) {
            Some(slot) => match slot.lock() {
                Ok(mut slot) => {
                    let event = DetectedRandom {
                        kind: host::RandomKind::Maze,
                        name: "combat-live-proof synthetic Maze hold".to_owned(),
                        ours: true,
                        npc_index: None,
                    };
                    let owner_live_before = crate::combat_proof::maze_attack_owner_live(name);
                    let claim = slot.on_random(&event);
                    let owner_live_after = crate::combat_proof::maze_attack_owner_live(name);
                    crate::combat_proof::record_maze_injection(
                        name,
                        Some(*self.ctx.script_tick),
                        &self.nav_snapshot,
                        claim,
                        owner_live_before,
                        owner_live_after,
                    );
                    true
                }
                Err(_) => {
                    crate::combat_proof::record_maze_injection_failure(
                        name,
                        "script slot mutex poisoned at Maze injection",
                    );
                    false
                }
            },
            None => {
                crate::combat_proof::record_maze_injection_failure(
                    name,
                    "script slot absent at Maze injection",
                );
                false
            }
        }
    }

    fn publish_frame_status(
        &self,
        c: &Client,
        input: &FrameStatusInput<'_>,
    ) -> PublishedFrameStatus {
        let name = self.ctx.username;
        // The slot's paint frame is read before the status lock
        // (`scripts -> statuses` is the only order the two mutexes may nest).
        let paint = script_paint_of(&self.ctx.args.slot_scripts, name);
        let survival =
            script_survival_of(&self.ctx.args.slot_scripts, &self.ctx.args.slot_navs, name);
        let login_latch_reason = self.ctx.args.arm.login_latch_reason();
        let mut all = lock_statuses(&self.ctx.args.slot_statuses);
        let mut up = false;
        let mut here = None;
        #[cfg(feature = "js")]
        let mut active_world = None;
        for s in all.iter_mut() {
            if s.username == name {
                // Keep the producer gate closed until a current player
                // observation can authorize game actions.
                s.ingame = input.ready;
                s.scene_state = self.nav_snapshot.scene_state();
                s.login_latched = login_latch_reason.is_some();
                s.login_latch_reason = login_latch_reason;
                apply_startup_phase(s, name, input.ready, c.ingame);
                s.runenergy = if input.ready { c.runenergy } else { 0 };
                s.run_sends = input.run_sends;
                s.main_modal_id = self.nav_snapshot.modals().main;
                s.welcome_hold = input.welcome_step.hold;
                s.welcome_failure = input.welcome_step.failure.clone();
                if input.welcome_step.notice.is_some() {
                    s.welcome_notice = input.welcome_step.notice.clone();
                }
                copy_stream_bytes(c, s);
                s.chat_head = if input.ready {
                    c.chat_text[0].clone()
                } else {
                    String::new()
                };
                // The snapshot fact, not the chat head: the box survives
                // chat-ring turnover and clears on a click that bumps no
                // family. Bool copy.
                s.tutorial_pending = self.nav_snapshot.tutorial_pending();
                s.random = if input.session_boundary {
                    RandomStatus::default()
                } else {
                    input.status.clone()
                };
                s.condition = input.condition;
                publish_script_survival(s, survival, input.session_boundary);
                publish_script_paint(s, paint.clone());
                here = self.nav_snapshot.tile();
                let (tx, tz, level) = here.unwrap_or((-1, -1, -1));
                s.tile_x = tx;
                s.tile_z = tz;
                s.tile_level = level;
                s.player = self
                    .nav_snapshot
                    .local_player()
                    .and_then(|p| p.player.actor.name.clone())
                    .unwrap_or_default();
                up = s.is_up();
                #[cfg(feature = "js")]
                {
                    active_world = s.world;
                }
            }
        }
        PublishedFrameStatus {
            up,
            here,
            #[cfg(feature = "js")]
            active_world,
        }
    }
}

impl<'a> SlotObserveCtx<'a> {
    fn handle_session_boundary(&mut self, c: &mut Client) {
        self.debug_replies = super::debug_replies::DebugReplies::default();
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.nav_world_state_cache.clear();
        if c.ingame {
            self.args
                .arm
                .set_script_active(script_active(&self.args.slot_scripts, self.username));
            reset_slot_session_boundary(
                self.username,
                &self.args.arm,
                BoundaryWork {
                    scripts: &self.args.slot_scripts,
                    cheats: &self.args.slot_cheats,
                    wires: &self.args.slot_wires,
                    navs: &self.args.slot_navs,
                    #[cfg(feature = "js")]
                    channels: &self.args.slot_channels,
                },
                self.args.arm.keeps_script_work_after_exit(),
            );
            // Before the first script tick of the session
            // (design-bank-snapshot §1.4).
            self.args.slot_bank.session_started();
        } else {
            observe_client_session_exit(c, &self.args.arm);
            end_slot_session(
                self.username,
                &self.args.arm,
                &self.args.slot_scripts,
                &self.args.slot_cheats,
                &self.args.slot_wires,
                &self.args.slot_navs,
                #[cfg(feature = "js")]
                &self.args.slot_channels,
            );
            self.args.slot_bank.session_ended();
            if let Some(bot) = self.args.slot_navs.lock().unwrap().get_mut(self.username) {
                bot.condition.session_ended();
            }
        }
        self.last_nav_step = None;
    }

    fn run_observe(
        &mut self,
        c: &mut Client,
        nav_snapshot: ObserveSnapshotRef<'_>,
        pass: &ObservePassInput<'_>,
        (tick_edge, evidence_dirty): (bool, bool),
        state: Option<Arc<nav_planner::WorldState>>,
        run_policy: &mut ScriptRunPolicy,
    ) -> crate::script_runtime::ScriptObservation {
        // The observer borrows the account's bank memory for the compiled tick
        // alone; the slot's own write for the frame already happened above.
        script_observe_cached_with_channels(
            c,
            self.username,
            pass.up,
            tick_edge,
            evidence_dirty,
            *self.script_tick,
            pass.here,
            pass.inv,
            state,
            #[cfg(feature = "test-support")]
            nav_snapshot,
            #[cfg(not(feature = "test-support"))]
            Some(nav_snapshot),
            Some(self.args.slot_bank.memory()),
            #[cfg(feature = "js")]
            pass.npc_boxes,
            Some(self.args.slot_obj_names.as_ref()),
            &self.args.slot_scripts,
            &self.args.slot_cheats,
            &self.args.slot_navs,
            &self.args.slot_world,
            pass.hold,
            pass.guardian_hold,
            pass.ours,
            pass.guardian_active_event,
            self.slot_canlight.as_deref(),
            Some(self.args.slot_input.as_ref()),
            Some(Arc::clone(&self.args.slot_cache)),
            Some(Arc::clone(&self.args.slot_obj_names)),
            #[cfg(feature = "js")]
            Some(&self.args.slot_channels),
            #[cfg(feature = "js")]
            pass.broker_world,
            Some(run_policy),
            Some(&mut self.debug_replies),
            Some(self.args.arm.as_ref()),
            Some(&self.args.slot_statuses),
        )
    }

    fn dispatch_wires_and_nav(
        &mut self,
        c: &mut Client,
        nav_snapshot: ObserveSnapshotRef<'_>,
        pass: &ObservePassInput<'_>,
        run_policy: &mut ScriptRunPolicy,
        input: DispatchWiresInput,
    ) {
        let DispatchWiresInput {
            wires,
            exclusive,
            nav_facts,
            tick_edge,
            condition_dispatch,
        } = input;
        let hold = pass.hold;
        let name = self.username;
        // TUI chat / WASD sends: run the queued wire commands through
        // `Interactions` on this slot's own Client, so Continue/Answer/Walk
        // respect the same preconditions as the guardian and the scenario
        // runner. The slot must not be frozen by the guardian's hold when it
        // presses a dialog the guardian is talking through, but a walk while
        // held is dropped (the hold freezes the follow too).
        #[cfg(test)]
        if crate::combat_proof::capture_enabled(name) {
            for wire in &wires {
                if !hold || !matches!(wire, WireCmd::Walk { .. }) {
                    crate::combat_proof::record_other_request(
                        name,
                        Some(*self.script_tick),
                        nav_snapshot,
                        "play-slot-wire",
                        &format!("{wire:?}"),
                    );
                }
            }
        }
        if !wires.is_empty() {
            dispatch_wires(c, nav_snapshot, wires.into(), hold);
        }
        // Poll work independently of the per-player follow latch.
        crate::script_runtime::pump_route_work(&self.args.slot_navs, name);
        // Per-uid nav step on the pump, gated on the player-gen/tile latch
        // like the panel's WalkTo hook so a hop is sent once per server tick,
        // not re-sent every 20 ms frame. The snapshot was already rebuilt
        // above. Guardian hold and a dispatched batch freeze follow for this
        // key; the armed route resumes at the next key.
        let nav_key = (c.gens.player, pass.here);
        if nav_step_due(&mut self.last_nav_step, nav_key, hold, exclusive, || {
            self.args
                .slot_navs
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|b| {
                    b.has_route()
                        || b.route_work_pending()
                        || b.bank_fetch.is_some()
                        || b.walk_guard_off.is_some()
                        || b.combat_prayer_off.is_some()
                })
        }) {
            let before_outcome = self
                .args
                .slot_navs
                .lock()
                .unwrap()
                .get(name)
                .map(|bot| bot.walk_outcome_seq);
            step_nav_bot(
                c,
                name,
                pass.here,
                nav_snapshot,
                &self.args.slot_navs,
                &self.args.slot_statuses,
                self.args.slot_world.as_ref(),
                hold,
                self.map_members,
                Some(self.args.slot_bank.memory()),
                || {
                    slot_arrival_reach(
                        &self.args.slot_scripts,
                        name,
                        nav_snapshot,
                        pass.here,
                        self.args.slot_world.as_deref(),
                        self.slot_canlight.as_deref(),
                    )
                },
            );
            let after_outcome = self
                .args
                .slot_navs
                .lock()
                .unwrap()
                .get(name)
                .map(|bot| bot.walk_outcome_seq);
            if before_outcome != after_outcome {
                // Follow can finish on this PLAYER_INFO edge after the first
                // observation. Deliver its receipt now, without another game
                // tick. This is still a partial PI-frame scene: later zone
                // packets may change a loc. Same-call consumers must not treat
                // arrival as a tick-end fence for their next scene
                // interaction.
                let completion_state = nav_world_state_for_observe(
                    &mut self.nav_world_state_cache,
                    pass.here,
                    nav_snapshot,
                    nav_facts,
                    true,
                    self.map_members,
                    self.session_epoch,
                );
                let completion = self.run_observe(
                    c,
                    nav_snapshot,
                    pass,
                    (false, false),
                    completion_state,
                    run_policy,
                );
                c.set_journal_paint_hidden(c.ingame && completion.journal_paint_hidden);
            }
        }
        #[cfg(any(test, feature = "test-support"))]
        if tick_edge {
            if let Some(observer) = &self.args.post_native_player_info {
                observer(c, name);
            }
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = tick_edge;
        // Inspect actual queued packets before mainloop's next flush. This
        // spans scripts, wires and Traveller. Guardian dispatch is already
        // covered by status.hold.
        if let Some(checkpoint) = condition_dispatch {
            if let Some(bot) = self.args.slot_navs.lock().unwrap().get_mut(name) {
                bot.condition.observe_dispatch(c, checkpoint);
            }
        }
    }

    fn slot_requires_tick(&self, running: bool, frame_delivered: bool) -> bool {
        let name = self.username;
        // Busy flag for the idle scheduler: a slot with a running script, a
        // frame listener (also while paused: one callback per 20 ms client
        // loop), queued cheats, or an armed nav bot must keep ticking (never
        // parked), so the observe hook reports it every frame.
        slot_busy_requires_tick(running, frame_delivered)
            || self
                .args
                .slot_cheats
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|q| !q.is_empty())
            || self
                .args
                .slot_wires
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|q| !q.is_empty())
            || self
                .args
                .slot_navs
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(|b| {
                    b.has_route()
                        || b.bank_fetch.is_some()
                        || b.walk_guard_off.is_some()
                        || b.combat_prayer_off.is_some()
                })
    }
}

/// Return the live client pump to its slot owner for Stop or for a newly
/// eligible host-owned handshake. Login itself remains outside the pump.
pub(super) fn slot_client_pump_should_exit(
    client: &mut Client,
    ifaces: &[Option<Box<IfType>>],
    arm: &SlotArm,
) -> bool {
    tick_flags(client, ifaces, arm) || (!client.ingame && should_handshake(arm, client.ingame))
}

/// One login wait's title frames: the pump's frame on this slot's client,
/// with the per-frame hook it runs on a logged-out title.
pub(super) fn title_frames<'a>(
    wait: &'a mut TitleWait,
    client: &'a mut Client,
    username: &'a str,
    slot_frame: &'a SlotFrame,
    countdown: Option<(&'a Cell<Option<u64>>, &'a str)>,
) -> impl FnMut() -> Duration + 'a {
    let mut displayed_countdown: Option<u64> = None;
    move || {
        if let Some((countdown, mes1)) = countdown {
            if let Some(remaining) = countdown.get() {
                update_transfer_title(client, mes1, remaining, &mut displayed_countdown);
            }
        }
        wait.frame(client, username, |c| {
            slot_frame(
                c,
                username,
                SlotFrameInput {
                    hold: true,
                    ..SlotFrameInput::default()
                },
            )
        })
    }
}

/// Say on the hosted title why the login waits, in the two Java message
/// lines its chrome draws; two empty lines restore the welcome text.
/// Unchanged lines are not rewritten.
fn show_title_message(client: &mut Client, mes1: &str, mes2: &str) {
    for (line, text) in [
        (&mut client.login_mes1, mes1),
        (&mut client.login_mes2, mes2),
    ] {
        if line != text {
            line.clear();
            line.push_str(text);
        }
    }
}

pub(super) fn update_transfer_title(
    client: &mut Client,
    mes1: &str,
    remaining: u64,
    displayed: &mut Option<u64>,
) -> bool {
    if *displayed == Some(remaining) {
        return false;
    }
    let mes2 = transfer_countdown_message(remaining);
    show_title_message(client, mes1, &mes2);
    *displayed = Some(remaining);
    true
}

fn slot_busy_requires_tick(running: bool, frame_delivered: bool) -> bool {
    running || frame_delivered
}

#[cfg(test)]
mod capacity_class_tests {
    use super::capacity_service_class;
    use crate::script_runtime::ScriptFrame;
    use host::ServiceClass;

    fn frame(state: Option<script::RunState>) -> ScriptFrame {
        ScriptFrame {
            state,
            ..ScriptFrame::default()
        }
    }

    #[test]
    fn only_an_in_game_unpaused_bot_hearing_the_server_counts() {
        let running = frame(Some(script::RunState::Running));
        assert_eq!(
            capacity_service_class(true, false, running),
            ServiceClass::Active
        );
        assert_eq!(
            capacity_service_class(true, false, frame(None)),
            ServiceClass::Active,
            "an in-game bot without a script counts"
        );
        assert_eq!(
            capacity_service_class(false, false, running),
            ServiceClass::Parked
        );
        assert_eq!(
            capacity_service_class(true, true, running),
            ServiceClass::Parked
        );
        assert_eq!(
            capacity_service_class(true, false, frame(Some(script::RunState::Paused))),
            ServiceClass::Paused
        );
        assert_eq!(
            capacity_service_class(true, false, frame(Some(script::RunState::Stopping))),
            ServiceClass::Suspended
        );
        let frozen = ScriptFrame {
            frozen: true,
            ..running
        };
        assert_eq!(
            capacity_service_class(true, false, frozen),
            ServiceClass::Suspended
        );
        let quiet = ScriptFrame {
            no_game_packets: true,
            ..running
        };
        assert_eq!(
            capacity_service_class(true, false, quiet),
            ServiceClass::Quiet
        );
    }
    #[test]
    fn delivered_frame_keeps_even_a_paused_slot_busy() {
        assert!(!super::slot_busy_requires_tick(false, false));
        assert!(super::slot_busy_requires_tick(false, true));
        assert!(super::slot_busy_requires_tick(true, false));
    }
}
#[cfg(test)]
mod active_guardian_event_tests {
    use super::*;

    #[test]
    fn only_owned_or_handled_events_reach_script_context() {
        let mut status = RandomStatus {
            name: Some("abyssal guardian".into()),
            ..RandomStatus::default()
        };
        assert_eq!(active_guardian_event_name(&status), None);

        status.ours = true;
        assert_eq!(
            active_guardian_event_name(&status),
            Some("abyssal guardian")
        );

        status.ours = false;
        status.handling = true;
        assert_eq!(
            active_guardian_event_name(&status),
            Some("abyssal guardian")
        );
    }
}
#[cfg(test)]
mod loc_projection_tests {
    use super::*;
    use client::client::ClientConfig;
    use client::io::ClientRevision;

    #[test]
    fn slot_without_js_surface_does_not_build_projection_locs() {
        let script = script::SlotScript::new();
        assert!(!script.loc_projection_wanted());
        let mut run_policy = ScriptRunPolicy::default();
        run_policy.set_loc_projection_wanted(script.loc_projection_wanted());
        assert!(!run_policy.loc_projection_wanted());

        let mut client = Client::from_shared_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                members: true,
                lowmem: true,
            },
            Arc::new(Cache::default()),
            Arc::new(Vec::new()),
            Vec::new(),
            ClientRevision::R289,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.gens.scene = 1;
        let mut snapshot = GameSnapshot::new();
        snapshot.set_loc_projection_wanted(run_policy.loc_projection_wanted());
        snapshot.rebuild_family(&client, api::snapshot::Family::Loc);
        assert!(
            snapshot.projection_scene().locs.is_empty(),
            "a slot without a JavaScript surface does not request loc projection data"
        );
    }
}

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::config::{Cache, IfType, IfTypeMut};
use host::login_queue::LoginBackoff;
use host::{
    prepare_client, should_emit_tick, wake_channel, DetectedRandom, FrameBuf, Host, Pump,
    ScriptRunPolicy, SlotInput, SlotPark, SlotWake, TitleWait,
};
use nav::world::NavWorld;
use vault::Profile;

#[cfg(feature = "memory-profile")]
use crate::memory;
use crate::play_bootstrap::{bot_client_config, PlayConnection};
use crate::play_login::{
    configure_slot_world, drop_queue_place, enqueue_queue_place, granted_permit_may_start_login,
    granted_permit_world_is_current, login_and_acknowledge_permit, login_retry_wait,
    on_login_success, publish_login_latched_from_arm, refresh_slot_world_preference,
    should_handshake, sync_profile_arm, tick_flags, wait_for_permit, wait_for_transfer_response,
    GrantedReservation, PermitWait, QueuePlaceRetirement, SharedLoginQueue, SlotArm,
};
use crate::play_status::{
    apply_startup_phase, clear_startup_progress, copy_stream_bytes, lock_statuses,
    mark_login_started, publish_login_lowmem, publish_session_boundary_status,
    publish_slot_disconnected, publish_startup_phase, publish_startup_progress,
    publish_worker_terminal, record_login_error, set_startup_phase, SlotStatus, StartupPhase,
    WorkerTerminal,
};
use crate::play_wires::{dispatch_wires, WireCmd};
use crate::script_runtime::{
    deliver_channel_events, hold_script_nav, nav_world_state_for_observe, observe_script_inv,
    project_npc_boxes_for_isolate_snapshot, projected_npc_boxes, publish_script_paint,
    reset_script_nav, script_active, script_observe_cached_with_channels, script_paint_of,
    script_running, script_slot, script_slot_or_insert, slot_arrival_reach, step_nav_bot,
    take_manual_walk_ownership, NavBot, ScriptSlot, ScriptWall,
};
use crate::{
    catalog_core, login_readiness, paired_core, public_worlds, Play, RandomClaim, RandomStatus,
};

/// Facts delivered before frontend follow and script observation/dispatch.
/// `hold` retains the guardian/readiness gate; it does not suppress intent.
/// Each queued manual step counts even if the client later refuses its send.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlotFrameInput {
    pub hold: bool,
    pub manual_move_intent: Option<host::ManualMoveIntent>,
    pub manual_steps: usize,
}

impl SlotFrameInput {
    /// Number of qualifying gestures in this frame, independent of route ownership.
    pub fn manual_move_count(self) -> usize {
        usize::from(self.manual_move_intent.is_some()) + self.manual_steps
    }
}

pub(super) type SlotFrame = Arc<dyn Fn(&mut Client, &str, SlotFrameInput) + Send + Sync>;

pub(super) fn take_slot_frame_input(
    input: &SlotInput,
    name: &str,
    wire_queues: &Mutex<HashMap<String, VecDeque<WireCmd>>>,
    hold: bool,
) -> (SlotFrameInput, VecDeque<WireCmd>) {
    let wires = wire_queues
        .lock()
        .unwrap()
        .get_mut(name)
        .map(std::mem::take)
        .unwrap_or_default();
    let frame = SlotFrameInput {
        hold,
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
/// so follow resumes on the next player key.
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
    *last = Some(key);
    key.1.is_some() && !hold && !exclusive && route_armed()
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
            arm.stop.store(true, Ordering::Relaxed);
            arm.notify_retry_wait();
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
    fn take_slot_for_stop(&mut self, name: &str) -> Option<thread::JoinHandle<()>> {
        #[cfg(test)]
        let arm = self.arms.get(name).cloned();
        self.signal_slot_stop(name);
        self.spawned.remove(name);
        lock_statuses(&self.statuses).retain(|status| status.username != name);
        self.arms.remove(name);
        let removed = self.scripts.lock().unwrap().remove(name);
        if let Some(slot) = removed {
            stop_retired_script(slot);
        }
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
        if !self.spawned.insert(profile.username.clone()) {
            return Ok(());
        }
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
        spawn_slot_thread(
            &self.connection,
            profile,
            world_round,
            slot_input,
            mailbox,
            Some(park),
            worker_wake,
            arm,
            Arc::clone(&self.cache),
            self.ifaces.clone(),
            self.ifaces_mut_template.clone(),
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.scripts),
            self.channels.clone(),
            Arc::clone(&self.cheats),
            Arc::clone(&self.wires),
            Arc::clone(&self.navs),
            Arc::clone(&self.pause_script_on_manual_walk_abort),
            self.world.clone(),
            Arc::clone(&self.obj_names),
            self.catalog_core.clone(),
            self.paired_core.clone(),
            Arc::clone(&self.per_frame),
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
        if keep_work && slot.load_active() {
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
        None => reset_script_nav(navs, name, None),
    }
    // Prayer varps are temporary: relog starts all-off. Do not carry a toggle
    // from the old session into the new one, even if its route is held.
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        bot.walk_guard = None;
        bot.walk_guard_off = None;
    }
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
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    channels: &super::script_channels::SlotChannels,
) -> bool {
    arm.set_script_active(script_active(scripts, name));
    let deliberate_logout = arm.take_logout_work_reset();
    let stopping = arm.stop.load(Ordering::Relaxed);
    let keep_work = !deliberate_logout && !stopping && arm.keeps_script_work_after_exit();
    reset_slot_session_work(name, scripts, cheats, wires, navs, keep_work);
    if channels.tracks() {
        let (generation, kept) = script_slot(scripts, name)
            .and_then(|slot| {
                slot.lock()
                    .ok()
                    .map(|slot| (slot.runtime_generation(), slot.load_active()))
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
        arm.auto_login.load(Ordering::Relaxed),
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
    channels: &super::script_channels::SlotChannels,
) {
    // Publish offline before resetting so a concurrent Logout becomes an
    // offline reset instead of falling through the just-ended session.
    arm.mark_session_offline();
    let keep_work = reset_slot_session_boundary(name, arm, scripts, cheats, wires, navs, channels);
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
    channels: &super::script_channels::SlotChannels,
) {
    if !arm.logout_work_reset_pending() {
        return;
    }
    arm.set_script_active(script_active(scripts, name));
    let (script_active, _, _) = arm.relog_decision_context();
    if script_active {
        let keep_work =
            reset_slot_session_boundary(name, arm, scripts, cheats, wires, navs, channels);
        log_session_boundary(name, arm, keep_work, "offline_logout");
    } else {
        let _ = arm.take_logout_work_reset();
    }
}

/// Publish the slot's script activity to its login want, as frozen
/// recomputes `autoLogin || scriptActive()` every frame
/// (`AutoRelogin.ts:175-198`): a script started during an outage logs the
/// slot back in, a stopped one stops the relog nothing else wants.
pub(super) fn sync_script_login(arm: &SlotArm, scripts: &ScriptWall, name: &str) {
    arm.set_script_active(script_active(scripts, name));
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
    paint: Option<&script::shim::ScriptPaint>,
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

/// Every profile spawns one slot thread; shared handles are threaded through
/// because the closure moves most of them (allowed: see `script_observe`).
// The worker body keeps its hand layout: rustfmt used to skip it (an
// overlong literal), and reformatting ~650 lines belongs in its own
// move-only change, not in a behavioural one.
#[rustfmt::skip]
#[allow(clippy::too_many_arguments)]
fn spawn_slot_thread(
    connection: &PlayConnection,
    profile: Profile,
    mut world_round: Option<public_worlds::WorldRound>,
    slot_input: Arc<SlotInput>,
    slot_mailbox: Option<Arc<FrameBuf>>,
    park: Option<SlotPark>,
    slot_wake: SlotWake,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    slot_queue: SharedLoginQueue,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: ScriptWall,
    slot_channels: super::script_channels::ChannelBroker,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    slot_navs: Arc<Mutex<HashMap<String, NavBot>>>,
    slot_manual_abort_pause: Arc<std::sync::atomic::AtomicBool>,
    slot_world: Option<Arc<NavWorld>>,
    slot_obj_names: Arc<api::obj_names::ObjNames>,
    slot_catalog_core: catalog_core::CoreWatch,
    slot_paired_core: paired_core::PairWatch,
    slot_frame: SlotFrame,
    handles: &mut HashMap<String, thread::JoinHandle<()>>,
) {
    let username = profile.username.clone();
    let uid = profile.uid;
    let connection = connection.clone();
    let public_world_profile = connection
        .profile()
        .and_then(|profile| profile.public_worlds())
        .is_some();
    let mainland = match &connection {
        PlayConnection::Direct(options) => options.mainland,
        PlayConnection::Bound { mainland, .. } => *mainland,
    };
    let slot_channels = slot_channels.slot(&username);

    handles.insert(
        username.clone(),
        thread::Builder::new()
            .name(username.clone())
            .stack_size(THREAD_STACK)
            .spawn(move || {
            api::hostlog::bind_slot(&username);
            let worker_outcome = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            let startup_entries_published = arm.wait_worker_start_for_test();
            let _queue_retirement = QueuePlaceRetirement {
                queue: &slot_queue,
                statuses: &slot_statuses,
                username: &username,
                arm: &arm,
            };
            #[cfg(feature = "memory-profile")]
            let _duel_retirement = memory::DuelReportRetirement { name: &username };
            #[cfg(test)]
            if let Some(published) = startup_entries_published {
                published.send(()).unwrap();
            }
            // Preparation has no determinate client percentage, but publish
            // a phase immediately so a slow cache fetch is visibly active.
            publish_startup_phase(&slot_statuses, &username, "Preparing client");
            // The park end survives re-login rounds (run_client is entered
            // once per ingame stretch), so wrap it once here.
            let park = park.map(Arc::new);
            let mut client = match &connection {
                PlayConnection::Direct(options) => prepare_client(
                    bot_client_config(options, &profile), uid, Arc::clone(&slot_cache),
                    ifaces_template.clone(), ifaces_mut_template.clone(),
                ),
                PlayConnection::Bound { template, .. } => match template.prepare_client(uid, profile.settings.lowmem) {
                    Ok(client) => client,
                    Err(error) => {
                        if let Some(row) = lock_statuses(&slot_statuses).iter_mut().find(|s| s.username == username) {
                            row.startup_phase = StartupPhase::Error;
                            row.startup_phase_started = Instant::now();
                            row.error = Some(error);
                        }
                        clear_startup_progress(&slot_statuses, &username);
                        return;
                    }
                },
            };
            if let PlayConnection::Direct(options) = &connection {
                client.set_transport(options.transport);
                client.set_cheat_admission(match connection.profile_class() {
                    crate::ProfileClass::Local => client::CheatAdmission::Granted,
                    crate::ProfileClass::Remote => client::CheatAdmission::Remote(options.transport),
                });
            }
            // The host owns every reconnect attempt so each fresh socket
            // returns through the shared FIFO and reservation accounting.
            client.set_external_reconnect_owner(true);
            host_log!(Category::Lifecycle, Level::Info, "thread up");

            // Jag/anim/model/map prefetch (mirrors client-play; the scene
            // cannot reach scene_state 2 until the loc models are in).
            // `maininit` is renderer-free now: progress recording lives on
            // the Client, and no `Renderer` is constructed for a headless
            // slot.
            #[cfg(any(test, feature = "test-support"))]
            let bypass_asset_startup = arm.bypass_asset_startup.load(Ordering::Relaxed);
            #[cfg(not(any(test, feature = "test-support")))]
            let bypass_asset_startup = false;
            if !bypass_asset_startup {
                client.maininit_with_progress(Some(&mut |client, message, percent| {
                    if arm.stop.load(Ordering::Relaxed) {
                        client.shell.stop();
                    } else {
                        publish_startup_progress(&slot_statuses, &username, message, percent);
                    }
                }));
            }
            if client.error_loading && connection.profile().is_some() {
                if let Some(row) = lock_statuses(&slot_statuses).iter_mut().find(|s| s.username == username) {
                    row.startup_phase = StartupPhase::Error;
                    row.startup_phase_started = Instant::now();
                    row.error = Some(format!("profile asset initialization failed: {}", client.last_progress_message));
                }
                clear_startup_progress(&slot_statuses, &username);
                return;
            }
            clear_startup_progress(&slot_statuses, &username);
            set_startup_phase(&slot_statuses, &username, StartupPhase::Queueing);
            if client.error_loading {
                host_log!(Category::Lifecycle, Level::Error, "maininit failed");
            }

            let mut backoff = LoginBackoff::new();
            let mut world_dirty = world_round.is_some();
            let mut refresh_key = false;
            let mut key_refreshed = false;
            let mut script_tick: u64 = 0;
            let mut run_policy = ScriptRunPolicy::default();
            let mut last_relog_park = None;
            // Frames that keep the logged-out title live while a login waits
            // outside the pump; built by the first wait of a login series and
            // dropped before the pump takes over.
            let mut title_wait: Option<TitleWait> = None;
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    return;
                }
                if !client.ingame {
                    sync_script_login(&arm, &slot_scripts, &username);
                    apply_offline_logout_reset(
                        &username,
                        &arm,
                        &slot_scripts,
                        &slot_cheats,
                        &slot_wires,
                        &slot_navs,
                        &slot_channels,
                    );
                    if !should_handshake(&arm, client.ingame) {
                        let reason = arm.relog_park_reason();
                        if last_relog_park != Some(reason) {
                            host_log!(
                                Category::Login,
                                Level::Info,
                                "relog decision park reason={reason}"
                            );
                            last_relog_park = Some(reason);
                        }
                        // No pending intent (title hold, latched logout, or a
                        // withdrawn wait): the live title pump holds no FIFO
                        // place and publishes no `k of n`.
                        drop_queue_place(
                            &slot_queue,
                            &slot_statuses,
                            &username,
                            arm.queue_owner,
                        );
                        publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                    }
                }
                if !client.ingame && should_handshake(&arm, client.ingame) {
                    // A request raised after the observe armed this handshake
                    // must not survive the ownership handoff and undo a later
                    // operator Logout.
                    let _ = client.take_title_login_request();
                    last_relog_park = None;
                    let wait_frames = title_wait.get_or_insert_with(|| {
                        TitleWait::new(Some(Arc::clone(&slot_input)), slot_mailbox.clone())
                    });
                    // Leaving title park for handshake: refresh latch from the
                    // arm so an explicit Log in cannot keep a stale TRUE from
                    // the last park publish through queue/connect.
                    publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                    enqueue_queue_place(&slot_queue, &slot_statuses, &username, uid, &arm);
                    #[cfg(test)]
                    arm.panic_after_queue_for_test();
                    if let Some(round) = world_round.as_mut() {
                        let worlds = connection
                            .profile()
                            .and_then(|p| p.public_worlds())
                            .expect("public world round requires bound public worlds");
                        match refresh_slot_world_preference(round, worlds, &arm) {
                            Ok(true) => {
                                world_dirty = true;
                                refresh_key = false;
                                key_refreshed = false;
                            }
                            Ok(false) => {}
                            Err(error) => {
                                if let Some(row) = lock_statuses(&slot_statuses)
                                    .iter_mut()
                                    .find(|s| s.username == username)
                                {
                                    row.startup_phase = StartupPhase::Error;
                                    row.startup_phase_started = Instant::now();
                                    row.error = Some(format!(
                                        "public world preference failed: {error}"
                                    ));
                                }
                                arm.hold_login_on_error();
                                continue;
                            }
                        }
                    }
                    if world_dirty {
                        let round = world_round.as_ref().expect("public world round");
                        let worlds = connection.profile().and_then(|p| p.public_worlds())
                            .expect("bound public worlds");
                        let world = &worlds.worlds[round.index];
                        if let Err(error) = configure_slot_world(&mut client, world, refresh_key, &arm.stop) {
                            if arm.stop.load(Ordering::Relaxed) {
                                return;
                            }
                            if let Some(row) = lock_statuses(&slot_statuses).iter_mut().find(|s| s.username == username) {
                                row.startup_phase = StartupPhase::Error;
                                row.startup_phase_started = Instant::now();
                                row.error = Some(format!("public world login configuration failed: {error}"));
                            }
                            clear_startup_progress(&slot_statuses, &username);
                            return;
                        }
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Some(row) = lock_statuses(&slot_statuses).iter_mut().find(|s| s.username == username) {
                            row.world = Some(world.number);
                        }
                        host_log!(
                            Category::Login,
                            Level::Info,
                            "world w{} {}:{} node {}",
                            world.number, world.host, world.port, world.node_id
                        );
                        world_dirty = false;
                        refresh_key = false;
                    }
                    host_log!(Category::Login, Level::Info, "relog decision wait queue");
                    show_title_message(&mut client, "", "Waiting to log in...");
                    let wait = wait_for_permit(
                        &slot_queue,
                        &slot_statuses,
                        &username,
                        uid,
                        &arm,
                        &mut title_frames(wait_frames, &mut client, &username, &slot_frame),
                    );
                    if wait == PermitWait::Cancelled {
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
                    if !granted_permit_world_is_current(
                        &slot_queue,
                        uid,
                        world_round.as_ref(),
                        &arm,
                    ) {
                        continue;
                    }
                    // A withdrawal or stop that lands after the granting poll
                    // releases the unused reservation before any login call,
                    // as does a relog wanted only by a script that has since
                    // stopped.
                    sync_script_login(&arm, &slot_scripts, &username);
                    let Some(login_command) = granted_permit_may_start_login(
                        &slot_queue,
                        uid,
                        &arm,
                        client.ingame,
                    ) else {
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    };
                    let mut permit = GrantedReservation::new(&slot_queue, uid);
                    mark_login_started(&slot_statuses, &username);
                    let reconnect = arm.reconnect.load(Ordering::Relaxed);
                    host_log!(
                        Category::Login,
                        Level::Info,
                        "relog decision attempt reconnect={reconnect}"
                    );
                    // Read at each handshake, not captured at spawn: a
                    // password saved since then applies to this login.
                    let password = arm.login_password();
                    // An in-world reconnect is only a server-side socket swap:
                    // it retains the old mode and tabs. Only a clean logout
                    // permits the queued mode to apply at this handshake.
                    if arm.login_applies_memory() {
                        if let Some(lowmem) = arm.lowmem_handshake() {
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
                    let login = login_and_acknowledge_permit(&mut permit, || {
                        client.login(&username, &password, reconnect)
                    });
                    match login {
                        Ok(()) => {
                            show_title_message(&mut client, "", "");
                            backoff.reset();
                            if let Some(round) = world_round.as_mut() { round.reset(); }
                            key_refreshed = false;
                            on_login_success(&arm, login_command);
                            publish_login_lowmem(
                                &slot_statuses,
                                &username,
                                handshake_lowmem,
                            );
                            set_startup_phase(&slot_statuses, &username, StartupPhase::LoadingScene);
                            host_log!(Category::Login, Level::Info, "handshake ok");
                        }
                        Err(e) => {
                            show_title_message(&mut client, &e.mes1, &e.mes2);
                            if wait_for_transfer_response(
                                &e,
                                &arm,
                                &slot_statuses,
                                &username,
                                &mut title_frames(wait_frames, &mut client, &username, &slot_frame),
                            )
                            .is_some()
                            {
                                continue;
                            }
                            record_login_error(&slot_statuses, &username, &e);
                            let decision = world_round.as_mut().map(|round| {
                                let count = connection.profile().and_then(|p| p.public_worlds())
                                    .expect("bound public worlds").worlds.len();
                                round.on_login_error(e.code, count)
                            });
                            match decision {
                                Some(public_worlds::WorldErrorStep::SwitchNow) => {
                                    world_dirty = true;
                                    key_refreshed = false;
                                    continue;
                                }
                                Some(public_worlds::WorldErrorStep::SwitchAfterWait) => {
                                    world_dirty = true;
                                    key_refreshed = false;
                                }
                                _ => {}
                            }
                            if world_round.is_some()
                                && public_worlds::refresh_after_login_error(e.code, &mut key_refreshed)
                            {
                                refresh_key = true;
                                world_dirty = true;
                            }
                            let retry = login_retry_wait(&mut backoff, e.code);
                            host_log!(
                                Category::Login,
                                Level::Info,
                                "relog decision wait backoff_ms={} code={}",
                                retry.as_millis(),
                                e.code
                            );
                            if e.code == 16 {
                                slot_queue.lock().hold_for(Instant::now(), retry);
                            }
                            arm.wait_for_retry(
                                retry,
                                &mut title_frames(wait_frames, &mut client, &username, &slot_frame),
                            );
                            continue;
                        }
                    }
                }
                let title_park_reason = &mut last_relog_park;
                let title_wake = slot_wake.clone();
                let pump_had_session = client.ingame;
                // The pump's own frames take over the title from any wait.
                let left_pump_to_log_in = title_wait.take().is_some();
                if left_pump_to_log_in && !client.ingame {
                    // Back on the title after a login attempt: input made
                    // while no title frame ran (the handshake, or the moment
                    // a wait was cancelled) answers no screen the operator
                    // saw, so a stale Log In must not undo a Logout.
                    slot_input.discard_user(&mut client.shell);
                    show_title_message(&mut client, "", "");
                }
                let mut mainland_sent = false;
                let arm_obs = Arc::clone(&arm);
                let arm_latch_obs = Arc::clone(&arm_obs);
                let obs_name = username.clone();
                let obs_catalog_core = slot_catalog_core.clone();
                let obs_paired_core = slot_paired_core.clone();
                let knock_name = username.clone();
                let knock_scripts = Arc::clone(&slot_scripts);
                let knock = move |ev: &DetectedRandom| -> RandomClaim {
                    let Some(slot) = script_slot(&knock_scripts, &knock_name) else {
                        return RandomClaim::Host;
                    };
                    let Ok(mut slot) = slot.lock() else {
                        return RandomClaim::Host;
                    };
                    slot.on_random(ev)
                };
                run_policy = Host::run_client(
                    &mut client,
                    &username,
                    profile.settings.clone(),
                    Arc::clone(&arm_obs.random_events),
                    Arc::clone(&arm_obs.lamp_auto),
                    Arc::clone(&arm_obs.lamp_skill),
                    Some(slot_input.clone()),
                    slot_mailbox.clone(),
                    park.clone(),
                    run_policy,
                    {
                        let slot_frame = Arc::clone(&slot_frame);
                        let slot_statuses = Arc::clone(&slot_statuses);
                        let slot_scripts = Arc::clone(&slot_scripts);
                        let slot_input = Arc::clone(&slot_input);
                        let slot_cheats = Arc::clone(&slot_cheats);
                        let slot_wires = Arc::clone(&slot_wires);
                        let slot_obj_names = Arc::clone(&slot_obj_names);
                        let slot_cache = Arc::clone(&slot_cache);
                        let slot_navs = Arc::clone(&slot_navs);
                        let slot_manual_abort_pause = Arc::clone(&slot_manual_abort_pause);
                        let slot_world = slot_world.clone();
                        let observe_channels = slot_channels.clone();
                        let slot_canlight = connection.profile().and_then(|p| p.canlight());
                        let map_members = connection
                            .profile()
                            .map(|p| p.map_members())
                            .unwrap_or(false);
                        let mut pump = Pump::new();
                        let script_tick = &mut script_tick;
                        // Last `(player gen, here)` the nav bot stepped:
                        // skip until either changes so the hop budget counts
                        // server ticks, not 20 ms frames (panel `tick_latch`).
                        let mut last_nav_step: Option<NavStepKey> = None;
                        // The slot's per-tick nav snapshot: rebuilt each
                        // observe when a family's gen moved (the walk arm
                        // gates on its facts; the follow surface reads the
                        // canonical base + route-head tile from it).
                        let mut nav_snapshot = GameSnapshot::new();
                        let mut session_epoch = 0u64;
                        let mut welcome = login_readiness::LoginReadiness::default();
                        let mut debug_replies = super::debug_replies::DebugReplies::default();
                        // This frame's random status is published before
                        // observe; its hold freezes script tick and nav follow.
                        move |c, _ignored, run_sends, status: &RandomStatus, run_policy| {
                            let name = &obs_name;
                            if !c.ingame {
                                // The live title pump owns offline script/latch
                                // synchronization. A hosted title click is only
                                // operator intent: arm the ordinary vault/FIFO
                                // path and never use the client's title fields.
                                if c.take_title_login_request() {
                                    arm_latch_obs.arm_explicit_login();
                                    title_wake.wake();
                                }
                                sync_script_login(&arm_latch_obs, &slot_scripts, name);
                                apply_offline_logout_reset(
                                    name,
                                    &arm_latch_obs,
                                    &slot_scripts,
                                    &slot_cheats,
                                    &slot_wires,
                                    &slot_navs,
                                    &observe_channels,
                                );
                                if should_handshake(&arm_latch_obs, c.ingame) {
                                    *title_park_reason = None;
                                } else {
                                    let reason = arm_latch_obs.relog_park_reason();
                                    if *title_park_reason != Some(reason) {
                                        host_log!(
                                            Category::Login,
                                            Level::Info,
                                            "relog decision park reason={reason}"
                                        );
                                        *title_park_reason = Some(reason);
                                    }
                                }
                            }
                            let drain = pump.drain_client(c);
                            let session_boundary = publish_session_boundary_status(
                                &slot_statuses,
                                name,
                                drain.session_changed,
                                c.ingame,
                                nav_snapshot.ingame(),
                            );
                            if session_boundary {
                                debug_replies = super::debug_replies::DebugReplies::default();
                                session_epoch = session_epoch.wrapping_add(1);
                                if c.ingame {
                                    reset_slot_session_boundary(
                                        name,
                                        &arm_latch_obs,
                                        &slot_scripts,
                                        &slot_cheats,
                                        &slot_wires,
                                        &slot_navs,
                                        &observe_channels,
                                    );
                                } else {
                                    end_slot_session(
                                        name,
                                        &arm_latch_obs,
                                        &slot_scripts,
                                        &slot_cheats,
                                        &slot_wires,
                                        &slot_navs,
                                        &observe_channels,
                                    );
                                }
                                last_nav_step = None;
                            }
                            host::publish_snapshot(&mut nav_snapshot, c, drain);
                            api::hostlog::set_tick(nav_snapshot.tick());
                            debug_replies.observe(name, &nav_snapshot);
                            // `script_observe_with_npc_boxes` below reaps the
                            // isolate and can publish a Stop receipt. The next
                            // frame attaches that bounded value here before
                            // status publication; pending panel logs are never
                            // consumed by this read.
                            // `status` is this frame's client_frame publication,
                            // copied onto the slot row below.
                            // The hunt cards also read the slot's own record
                            // of the requests host-play dispatched for it.
                            let copies_inspect = obs_catalog_core.copies_route_inspect();
                            let copies_hunt = obs_catalog_core.copies_hunt();
                            let (inspect, acts) = if copies_inspect || copies_hunt {
                                let navs = slot_navs.lock().unwrap();
                                let bot = navs.get(name);
                                (
                                    bot.filter(|_| copies_inspect)
                                        .map(|bot| bot.inspect.published_core_facts()),
                                    copies_hunt.then(|| {
                                        bot.map(|bot| bot.acts.published()).unwrap_or_default()
                                    }),
                                )
                            } else {
                                (None, None)
                            };
                            let catalog_paint = if obs_catalog_core.copies_line_of_sight()
                                || obs_catalog_core.copies_actor_observation()
                                || obs_catalog_core.copies_fight_field()
                                || copies_hunt
                            {
                                script_paint_of(&slot_scripts, name)
                            } else {
                                None
                            };
                            observe_slot_catalog_and_paired(
                                &obs_catalog_core,
                                &obs_paired_core,
                                name,
                                &nav_snapshot,
                                slot_obj_names.as_ref(),
                                session_boundary,
                                || {
                                    script_slot(&slot_scripts, name).and_then(|slot| {
                                        slot.lock()
                                            .ok()
                                            .and_then(|slot| slot.lifecycle_receipt())
                                    })
                                },
                                || bounded_guardian_fact(status),
                                inspect,
                                catalog_paint.as_deref(),
                                acts,
                            );
                            let ready = c.ingame && c.scene_state == 2
                                && nav_snapshot.local_player().is_some();
                            let welcome_obs = login_readiness::WelcomeObservation {
                                session_epoch,
                                tick: *script_tick,
                                now: Instant::now(),
                                ingame: c.ingame,
                                scene_state: c.scene_state,
                                welcome_interface_id: c.welcome_interface_id,
                                main_modal_id: c.main_modal_id,
                                allow_close: c.ingame && c.scene_state == 2 && !session_boundary,
                            };
                            let welcome_step = welcome.step(&welcome_obs, || {
                                login_readiness::try_close_welcome(&nav_snapshot, c)
                            });
                            if let Some(line) = welcome_step.notice.as_deref() {
                                host_log!(stderr; Category::Echo, Level::Info, "{line}");
                            }
                            #[cfg(test)]
                            let injected_maze_hold = if ready
                                && !status.hold
                                && !session_boundary
                                && crate::combat_proof::maze_injection_pending(name)
                            {
                                if crate::combat_proof::protect_from_melee_active(&nav_snapshot) {
                                    match script_slot(&slot_scripts, name) {
                                        Some(slot) => match slot.lock() {
                                            Ok(mut slot) => {
                                                let event = DetectedRandom {
                                                    kind: host::RandomKind::Maze,
                                                    name: "combat-live-proof synthetic Maze hold"
                                                        .to_owned(),
                                                    ours: true,
                                                    npc_index: None,
                                                };
                                                let owner_live_before =
                                                    crate::combat_proof::maze_attack_owner_live(
                                                        name,
                                                    );
                                                let claim = slot.on_random(&event);
                                                let owner_live_after =
                                                    crate::combat_proof::maze_attack_owner_live(
                                                        name,
                                                    );
                                                crate::combat_proof::record_maze_injection(
                                                    name,
                                                    Some(*script_tick),
                                                    &nav_snapshot,
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
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            #[cfg(test)]
                            let hold = status.hold
                                || !ready
                                || session_boundary
                                || welcome_step.hold
                                || injected_maze_hold;
                            #[cfg(not(test))]
                            let hold = status.hold || !ready || session_boundary || welcome_step.hold;
                            // Collect intent before either follow pump or script
                            // dispatch; this is the shared takeover ordering seam.
                            // Keep these commands for their normal late send/hold gate.
                            let (frame_input, wires) =
                                take_slot_frame_input(&slot_input, name, &slot_wires, hold);
                            take_manual_walk_ownership(
                                &slot_scripts,
                                &slot_navs,
                                name,
                                frame_input,
                                c.ingame && !session_boundary,
                                *script_tick,
                                slot_manual_abort_pause.load(Ordering::Relaxed),
                            );
                            #[cfg(feature = "memory-profile")]
                            memory::client_frame(c, name, hold);
                            slot_frame(c, name, frame_input);
                            if !mainland_sent && mainland && ready {
                                api::interact::mainland_hop(c);
                                mainland_sent = true;
                                host_log!(Category::Lifecycle, Level::Info, "mainland hop queued");
                            }
                            let tick_edge = should_emit_tick(drain.player_info);
                            if tick_edge {
                                *script_tick = script_tick.wrapping_add(1);
                            }
                            // The slot's paint frame is read before the
                            // status lock (scripts -> statuses is the only
                            // order the two mutexes may nest).
                            let paint = script_paint_of(&slot_scripts, name);
                            let login_latch_reason = arm_latch_obs.login_latch_reason();
                            let (up, here, active_world) = {
                                let mut all = lock_statuses(&slot_statuses);
                                let mut up = false;
                                let mut here = None;
                                let mut active_world = None;
                                for s in all.iter_mut() {
                                    if s.username == *name {
                                        // Keep the producer gate closed until a current
                                        // player observation can authorize game actions.
                                        s.ingame = ready;
                                        s.scene_state = nav_snapshot.scene_state();
                                        s.login_latched = login_latch_reason.is_some();
                                        s.login_latch_reason = login_latch_reason;
                                        apply_startup_phase(s, name, ready, c.ingame);
                                        s.runenergy = if ready { c.runenergy } else { 0 };
                                        s.run_sends = run_sends;
                                        s.main_modal_id = nav_snapshot.modals().main;
                                        s.welcome_hold = welcome_step.hold;
                                        s.welcome_failure = welcome_step.failure.clone();
                                        if welcome_step.notice.is_some() {
                                            s.welcome_notice = welcome_step.notice.clone();
                                        }
                                        copy_stream_bytes(c, s);
                                        s.chat_head = if ready { c.chat_text[0].clone() } else { String::new() };
                                        // The snapshot fact, not the chat head: the box
                                        // survives chat-ring turnover and clears on a
                                        // click that bumps no family. Bool copy.
                                        s.tutorial_pending = nav_snapshot.tutorial_pending();
                                        s.random = if session_boundary { RandomStatus::default() } else { status.clone() };
                                        publish_script_paint(s, paint.clone());
                                        here = nav_snapshot.tile();
                                        let (tx, tz, level) = here.unwrap_or((-1, -1, -1));
                                        s.tile_x = tx;
                                        s.tile_z = tz;
                                        s.tile_level = level;
                                        s.player = nav_snapshot.local_player()
                                            .and_then(|p| p.player.actor.name.clone())
                                            .unwrap_or_default();
                                        up = s.is_up();
                                        active_world = s.world;
                                    }
                                }
                                (up, here, active_world)
                            };
                            let broker_world = if public_world_profile {
                                active_world.map_or(
                                    super::script_channels::BrokerWorld::Unavailable,
                                    super::script_channels::BrokerWorld::Public,
                                )
                            } else {
                                super::script_channels::BrokerWorld::Local
                            };
                            let running = script_running(&slot_scripts, name);
                            let inv = observe_script_inv(running, tick_edge, &nav_snapshot);
                            let nav_armed = slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                b.route.is_some() || b.bank_fetch.is_some()
                            });
                            let nav_state = nav_world_state_for_observe(
                                here,
                                &nav_snapshot,
                                running,
                                nav_armed,
                                map_members,
                            );
                            let npc_boxes = project_npc_boxes_for_isolate_snapshot(
                                &slot_scripts,
                                name,
                                tick_edge,
                                || projected_npc_boxes(c),
                            );
                            let crate::script_runtime::ScriptObservation {
                                wrote: _wrote,
                                journal_paint_hidden,
                                exclusive,
                            } = script_observe_cached_with_channels(
                                c,
                                name,
                                up,
                                tick_edge,
                                drain.dirty.iface || drain.dirty.inv,
                                *script_tick,
                                here,
                                inv,
                                nav_state,
                                Some(&nav_snapshot),
                                npc_boxes.as_deref(),
                                Some(slot_obj_names.as_ref()),
                                &slot_scripts,
                                &slot_cheats,
                                &slot_navs,
                                &slot_world,
                                hold,
                                status.ours,
                                slot_canlight.as_deref(),
                                Some(slot_input.as_ref()),
                                Some(Arc::clone(&slot_cache)),
                                Some(Arc::clone(&slot_obj_names)),
                                Some(&observe_channels),
                                broker_world,
                                Some(run_policy),
                                Some(&mut debug_replies),
                            );
                            #[cfg(test)]
                            if crate::combat_proof::capture_enabled(name) {
                                crate::combat_proof::record_frame(name, *script_tick, &nav_snapshot);
                                crate::combat_proof::record_observation(
                                    name,
                                    *script_tick,
                                    &nav_snapshot,
                                    exclusive,
                                );
                                if let Some(status) = script_slot(&slot_scripts, name)
                                    .and_then(|slot| slot.lock().ok()?.native_status())
                                {
                                    crate::combat_proof::record_status(name, &status);
                                }
                            }
                            c.set_journal_paint_hidden(c.ingame && journal_paint_hidden);
                            // TUI chat / WASD sends: run the queued wire
                            // commands through `Interactions` on this
                            // slot's own Client, so Continue/Answer/Walk
                            // respect the same preconditions as the
                            // guardian and the scenario runner. The slot
                            // must not be frozen by the guardian's hold
                            // when it presses a dialog the guardian is
                            // talking through, but a walk while held is
                            // dropped (the hold freezes the follow too).
                            #[cfg(test)]
                            if crate::combat_proof::capture_enabled(name) {
                                for wire in &wires {
                                    if !hold || !matches!(wire, WireCmd::Walk { .. }) {
                                        crate::combat_proof::record_other_request(
                                            name,
                                            Some(*script_tick),
                                            &nav_snapshot,
                                            "play-slot-wire",
                                            &format!("{wire:?}"),
                                        );
                                    }
                                }
                            }
                            if !wires.is_empty() {
                                dispatch_wires(c, &nav_snapshot, wires.into(), hold);
                            }
                            // Per-uid nav step on the pump, gated on the
                            // player-gen/tile latch like the panel's WalkTo
                            // hook so a hop is sent once per server tick,
                            // not re-sent every 20 ms frame. The snapshot
                            // was already rebuilt above. Guardian hold and
                            // a dispatched batch freeze follow for this key;
                            // the armed route resumes at the next key.
                            let nav_key = (c.gens.player, here);
                            if nav_step_due(&mut last_nav_step, nav_key, hold, exclusive, || {
                                slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                    b.route.is_some() || b.bank_fetch.is_some() || b.walk_guard_off.is_some()
                                })
                            }) {
                                step_nav_bot(
                                    c,
                                    name,
                                    here,
                                    &nav_snapshot,
                                    &slot_navs,
                                    &slot_statuses,
                                    slot_world.as_ref(),
                                    hold,
                                    map_members,
                                    || {
                                        slot_arrival_reach(
                                            &slot_scripts,
                                            name,
                                            &nav_snapshot,
                                            here,
                                            slot_world.as_deref(),
                                            slot_canlight.as_deref(),
                                        )
                                    },
                                );
                            }
                            // Busy flag for the idle scheduler: a slot with
                            // a running script, queued cheats, or an armed
                            // nav bot must keep ticking (never parked), so
                            // the observe hook reports it every frame.
                            running
                                || slot_cheats
                                    .lock()
                                    .unwrap()
                                    .get(name)
                                    .is_some_and(|q| !q.is_empty())
                                || slot_wires
                                    .lock()
                                    .unwrap()
                                    .get(name)
                                    .is_some_and(|q| !q.is_empty())
                                || slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                    b.route.is_some() || b.bank_fetch.is_some() || b.walk_guard_off.is_some()
                                })
                        }
                    },
                    {
                        let ifaces_template = ifaces_template.clone();
                        move |c| slot_client_pump_should_exit(c, &ifaces_template, &arm_obs)
                    },
                    knock,
                );
                if pump_had_session && arm.session_online() {
                    publish_slot_disconnected(&slot_statuses, &username);
                    end_slot_session(
                        &username,
                        &arm,
                        &slot_scripts,
                        &slot_cheats,
                        &slot_wires,
                        &slot_navs,
                        &slot_channels,
                    );
                }
                if arm.stop.load(Ordering::Relaxed) {
                    return;
                }
            }
            }));
            // No later session observes this slot, however the worker
            // ended (a Stop while parked at the title never reaches the
            // boundary above): its channel member leaves the party now.
            if slot_channels.tracks() {
                deliver_channel_events(&slot_scripts, slot_channels.leave());
            }
            if !arm.stop.load(Ordering::Relaxed) {
                match worker_outcome {
                    Ok(()) => publish_worker_terminal(
                        &slot_statuses,
                        &username,
                        WorkerTerminal::Failed,
                        None,
                    ),
                    Err(payload) => {
                        let detail = payload
                            .downcast_ref::<&str>()
                            .map(|message| (*message).to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic payload".to_string());
                        publish_worker_terminal(
                            &slot_statuses,
                            &username,
                            WorkerTerminal::Panicked,
                            Some(format!("slot worker panicked: {detail}")),
                        );
                    }
                }
            }
            })
            .expect("failed to spawn slot thread"),
    );
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
fn title_frames<'a>(
    wait: &'a mut TitleWait,
    client: &'a mut Client,
    username: &'a str,
    slot_frame: &'a SlotFrame,
) -> impl FnMut() -> Duration + 'a {
    move || {
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

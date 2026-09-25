use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::snapshot::GameSnapshot;
use client::client::Client;
use client::config::{Cache, IfType, IfTypeMut};
use host::login_queue::LoginBackoff;
use host::{
    prepare_client, should_emit_tick, wake_channel, DetectedRandom, FrameBuf, Host, Pump,
    SlotInput, SlotPark,
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
    apply_startup_phase, clear_startup_progress, copy_stream_bytes, mark_login_started,
    publish_session_boundary_status, publish_slot_disconnected, publish_startup_phase,
    publish_startup_progress, record_login_error, set_startup_phase, SlotStatus, StartupPhase,
};
use crate::play_wires::{dispatch_wires, WireCmd};
use crate::script_runtime::{
    nav_world_state_for_observe, observe_script_inv, project_npc_boxes_for_isolate_snapshot,
    projected_npc_boxes, publish_script_paint, reset_script_nav, script_observe_cached,
    script_paint_of, script_running, script_slot, script_slot_or_insert, slot_arrival_reach,
    step_nav_bot, NavBot, ScriptWall,
};
use crate::{
    catalog_core, debug_enabled, login_readiness, paired_core, public_worlds, Play, RandomClaim,
    RandomStatus,
};

/// Per-slot hook invoked by the slot thread after every mainloop pass.
/// Per-frame hook: `(client, username, hold)`. `hold` is the guardian's
/// published hold from the previous frame (same lag as `step_nav_bot`) —
/// panel/TUI skip scenario follow and WalkArm follow while it is set.
pub(super) type SlotFrame = Arc<dyn Fn(&mut Client, &str, bool) + Send + Sync>;
/// Slot thread stack: 1 MiB (the Java client thread default).
const THREAD_STACK: usize = 1024 * 1024;
/// Per-slot nav latch key: the `(player gen, here)` pair the pump last
/// pump last stepped. The step is skipped until either half changes, so a
/// hop is sent once per server tick, not every 20 ms frame (panel
/// `tick_latch`).
type NavStepKey = (u64, Option<(i32, i32, i32)>);
impl Play {
    /// Stop one running slot: flag its arm `stop`, drop its login-FIFO
    /// place immediately (a queued slot must not keep later slots behind
    /// it even if the thread is still blocked in `wait_for_permit`),
    /// drop the status row and arm, then join the thread. The slot body
    /// checks `stop` every 20 ms in `run_client`; startup retry progress
    /// also transfers the arm stop into the client's shell, so an HTTP
    /// countdown adds at most its one-second tick. Do **not** abort the TCP
    /// link here — the caller sends a clean IF logout before calling this.
    pub fn stop_slot(&mut self, name: &str) {
        if let Some(arm) = self.arms.get(name) {
            arm.stop.store(true, Ordering::Relaxed);
            arm.notify_retry_wait();
            let mut queue = self.queue.lock();
            queue.leave_owner(arm.queue_owner);
        }
        self.spawned.remove(name);
        self.statuses.lock().unwrap().retain(|s| s.username != name);
        self.arms.remove(name);
        // Release the wall lock before the slot lock (wall-then-slot order).
        let removed = self.scripts.lock().unwrap().remove(name);
        if let Some(slot) = removed {
            slot.lock().unwrap().stop();
        }
        self.cheats.lock().unwrap().remove(name);
        self.wires.lock().unwrap().remove(name);
        if self.focused.as_deref() == Some(name) {
            self.focused = None;
            self.queue.lock().set_preferred_owner(None);
        }
        // Wake a parked thread so its next probe sees `stop`; the wake end
        // stays alive (removed after the join) so the poll cannot miss it.
        if let Some(handle) = self.handles.remove(name) {
            self.wake(name);
            let _ = handle.join();
        }
        self.wakes.remove(name);
    }

    /// Register a control arm without spawning a slot thread (panel unit
    /// tests that drive login/logout flags through [`Play::arm`]).
    pub fn attach_arm(&mut self, name: &str, arm: Arc<SlotArm>) {
        self.arms.insert(name.to_string(), arm);
    }

    /// Poll until `name` reports `!ingame` (or is absent), or `timeout`
    /// elapses. Used by rail ✕ after arming a clean logout so `stop_slot`
    /// does not cut the TCP link while still ingame.
    pub fn wait_until_not_ingame(&self, name: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !self
                .statuses()
                .iter()
                .any(|s| s.username == name && s.ingame)
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
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
        if let Err(error) = self.try_spawn_slot(profile, input, mailbox, arm) {
            eprintln!("[host-play] {error}");
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
        self.wakes.insert(profile.username.clone(), wake);
        spawn_slot_thread(
            &self.connection,
            profile,
            world_round,
            input,
            mailbox,
            Some(park),
            arm,
            Arc::clone(&self.cache),
            self.ifaces.clone(),
            self.ifaces_mut_template.clone(),
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.scripts),
            Arc::clone(&self.cheats),
            Arc::clone(&self.wires),
            Arc::clone(&self.navs),
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
/// script); packets, isolate interactions, route workers and navigation state
/// belong to the disconnected session and are discarded.
pub(super) fn reset_slot_session_work(
    name: &str,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
) {
    if let Some(slot) = script_slot(scripts, name) {
        let mut slot = slot.lock().unwrap();
        slot.reset_session_work();
    }
    if let Some(queue) = cheats.lock().unwrap().get_mut(name) {
        queue.clear();
    }
    if let Some(queue) = wires.lock().unwrap().get_mut(name) {
        queue.clear();
    }
    reset_script_nav(navs, name);
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
#[allow(clippy::too_many_arguments)]
fn spawn_slot_thread(
    connection: &PlayConnection,
    profile: Profile,
    mut world_round: Option<public_worlds::WorldRound>,
    slot_input: Option<Arc<SlotInput>>,
    slot_mailbox: Option<Arc<FrameBuf>>,
    park: Option<SlotPark>,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    slot_queue: SharedLoginQueue,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: ScriptWall,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    slot_navs: Arc<Mutex<HashMap<String, NavBot>>>,
    slot_world: Option<Arc<NavWorld>>,
    slot_obj_names: Arc<api::obj_names::ObjNames>,
    slot_catalog_core: catalog_core::CoreWatch,
    slot_paired_core: paired_core::PairWatch,
    slot_frame: SlotFrame,
    handles: &mut HashMap<String, thread::JoinHandle<()>>,
) {
    let username = profile.username.clone();
    let uid = profile.uid;
    let password = profile.password.clone();
    let connection = connection.clone();
    let mainland = match &connection {
        PlayConnection::Legacy(options) => options.mainland,
        PlayConnection::Bound { mainland, .. } => *mainland,
    };

    handles.insert(
        username.clone(),
        thread::Builder::new()
            .name(username.clone())
            .stack_size(THREAD_STACK)
            .spawn(move || {
            let _queue_retirement = QueuePlaceRetirement {
                queue: &slot_queue,
                statuses: &slot_statuses,
                username: &username,
                arm: &arm,
            };
            {
                // Publish the row before `prepare_client`/`maininit`
                // (a slow cache fetch can stall for seconds), so the
                // queue card shows the slot while it loads.
                let mut all = slot_statuses.lock().unwrap();
                all.push(SlotStatus {
                    username: username.clone(),
                    world: world_round.as_ref().and_then(|round| connection.profile()
                        .and_then(|p| p.public_worlds())
                        .map(|worlds| worlds.worlds[round.index].number)),
                    ..SlotStatus::default()
                });
            }
            let slot_script = script_slot_or_insert(&slot_scripts, &username);
            let slot_input = slot_input.unwrap_or_else(SlotInput::new);
            {
                let mut slot_script = slot_script.lock().unwrap();
                slot_script.bind_native_input(slot_input.authority());
            }
            slot_cheats
                .lock()
                .unwrap()
                .entry(username.clone())
                .or_default();
            slot_wires
                .lock()
                .unwrap()
                .entry(username.clone())
                .or_default();
            // Preparation has no determinate client percentage, but publish
            // a phase immediately so a slow cache fetch is visibly active.
            publish_startup_phase(&slot_statuses, &username, "Preparing client");
            // The park end survives re-login rounds (run_client is entered
            // once per ingame stretch), so wrap it once here.
            let park = park.map(Arc::new);
            let mut client = match &connection {
                PlayConnection::Legacy(options) => prepare_client(
                    bot_client_config(options, &profile), uid, Arc::clone(&slot_cache),
                    ifaces_template.clone(), ifaces_mut_template.clone(),
                ),
                PlayConnection::Bound { template, .. } => match template.prepare_client(uid, profile.settings.lowmem) {
                    Ok(client) => client,
                    Err(error) => {
                        if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                            row.startup_phase = StartupPhase::Error;
                            row.startup_phase_started = Instant::now();
                            row.error = Some(error);
                        }
                        clear_startup_progress(&slot_statuses, &username);
                        return;
                    }
                },
            };
            // The host owns every reconnect attempt so each fresh socket
            // returns through the shared FIFO and reservation accounting.
            client.set_external_reconnect_owner(true);
            if debug_enabled() {
                eprintln!("[host-play] slot {username}: thread up");
            }

            // Jag/anim/model/map prefetch (mirrors client-play; the scene
            // cannot reach scene_state 2 until the loc models are in).
            // `maininit` is renderer-free now: progress recording lives on
            // the Client, and no `Renderer` is constructed for a headless
            // slot.
            #[cfg(test)]
            let bypass_asset_startup = arm.bypass_asset_startup.load(Ordering::Relaxed);
            #[cfg(not(test))]
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
                if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                    row.startup_phase = StartupPhase::Error;
                    row.startup_phase_started = Instant::now();
                    row.error = Some(format!("profile asset initialization failed: {}", client.last_progress_message));
                }
                clear_startup_progress(&slot_statuses, &username);
                return;
            }
            clear_startup_progress(&slot_statuses, &username);
            set_startup_phase(&slot_statuses, &username, StartupPhase::Queueing);
            if client.error_loading && debug_enabled() {
                eprintln!("[host-play] slot {username}: maininit failed");
            }

            let mut backoff = LoginBackoff::new();
            let mut world_dirty = world_round.is_some();
            let mut refresh_key = false;
            let mut key_refreshed = false;
            let mut script_tick: u64 = 0;
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    return;
                }
                if !client.ingame {
                    if !should_handshake(&arm, client.ingame) {
                        // No pending intent (title hold, latched logout, or a
                        // withdrawn wait): a parked slot holds no FIFO place
                        // and publishes no `k of n`.
                        drop_queue_place(
                            &slot_queue,
                            &slot_statuses,
                            &username,
                            arm.queue_owner,
                        );
                        publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    // Leaving title park for handshake: refresh latch from the
                    // arm so an explicit Log in cannot keep a stale TRUE from
                    // the last park publish through queue/connect.
                    publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                    enqueue_queue_place(&slot_queue, &slot_statuses, &username, uid, &arm);
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
                                if let Some(row) = slot_statuses
                                    .lock()
                                    .unwrap()
                                    .iter_mut()
                                    .find(|s| s.username == username)
                                {
                                    row.startup_phase = StartupPhase::Error;
                                    row.startup_phase_started = Instant::now();
                                    row.error = Some(format!(
                                        "public world preference failed: {error}"
                                    ));
                                }
                                arm.withdraw_login();
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
                            if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
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
                        if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                            row.world = Some(world.number);
                        }
                        if debug_enabled() {
                            eprintln!(
                                "[host-play] slot {username}: world w{} {}:{} node {}",
                                world.number, world.host, world.port, world.node_id
                            );
                        }
                        world_dirty = false;
                        refresh_key = false;
                    }
                    let wait = wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm);
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
                    // releases the unused reservation before any login call.
                    if !granted_permit_may_start_login(
                        &slot_queue,
                        uid,
                        &arm,
                        client.ingame,
                    ) {
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
                    let mut permit = GrantedReservation::new(&slot_queue, uid);
                    mark_login_started(&slot_statuses, &username);
                    let reconnect = arm.reconnect.load(Ordering::Relaxed);
                    if debug_enabled() {
                        eprintln!(
                            "[host-play] slot {username}: handshake begin reconnect={reconnect}"
                        );
                    }
                    let login = login_and_acknowledge_permit(&mut permit, || {
                        client.login(&username, &password, reconnect)
                    });
                    match login {
                        Ok(()) => {
                            backoff.reset();
                            if let Some(round) = world_round.as_mut() { round.reset(); }
                            key_refreshed = false;
                            on_login_success(&arm);
                            set_startup_phase(&slot_statuses, &username, StartupPhase::LoadingScene);
                            if debug_enabled() {
                                eprintln!("[host-play] slot {username}: handshake ok");
                            }
                        }
                        Err(e) => {
                            if wait_for_transfer_response(
                                &e,
                                &arm,
                                &slot_statuses,
                                &username,
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
                            if e.code == 16 {
                                slot_queue.lock().hold_for(Instant::now(), retry);
                            }
                            arm.wait_for_retry(retry);
                            continue;
                        }
                    }
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
                    let mut slot = slot.lock().unwrap();
                    slot.on_random(ev)
                };
                let run_policy_override = slot_script.lock().unwrap().run_policy_override_cell();
                Host::run_client(
                    &mut client,
                    &username,
                    profile.settings.clone(),
                    Arc::clone(&arm_obs.random_events),
                    Arc::clone(&arm_obs.lamp_auto),
                    Arc::clone(&arm_obs.lamp_skill),
                    Some(slot_input.clone()),
                    slot_mailbox.clone(),
                    park.clone(),
                    run_policy_override,
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
                        let slot_world = slot_world.clone();
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
                        // The random status `client_frame` published last
                        // frame: copied onto the slot status row, and its
                        // hold freezes script tick and the nav follow.
                        move |c, _ignored, run_sends, status: &RandomStatus| {
                            let name = &obs_name;
                            let drain = pump.drain_client(c);
                            let session_boundary = publish_session_boundary_status(
                                &slot_statuses,
                                name,
                                drain.session_changed,
                                c.ingame,
                                nav_snapshot.ingame(),
                            );
                            if session_boundary {
                                session_epoch = session_epoch.wrapping_add(1);
                                reset_slot_session_work(name, &slot_scripts, &slot_cheats, &slot_wires, &slot_navs);
                                last_nav_step = None;
                            }
                            host::publish_snapshot(&mut nav_snapshot, c, drain);
                            // `script_observe_with_npc_boxes` below reaps the
                            // isolate and can publish a Stop receipt. The next
                            // frame attaches that bounded value here before
                            // status publication; pending panel logs are never
                            // consumed by this read.
                            // `status` is last frame's client_frame publication.
                            // Snapshot observe runs before this frame copies it
                            // onto the slot row.
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
                                        slot.lock().unwrap().lifecycle_receipt()
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
                                eprintln!("[host-play] slot {name}: {line}");
                            }
                            let hold = status.hold || !ready || session_boundary || welcome_step.hold;
                            #[cfg(feature = "memory-profile")]
                            memory::client_frame(c, name, hold);
                            slot_frame(c, name, hold);
                            if !mainland_sent && mainland && ready {
                                api::interact::mainland_hop(c);
                                mainland_sent = true;
                                if debug_enabled() {
                                    eprintln!("[host-play] slot {name}: queued mainland tele+setvar (scene 2)");
                                }
                            }
                            let tick_edge = should_emit_tick(drain.player_info);
                            if tick_edge {
                                *script_tick = script_tick.wrapping_add(1);
                            }
                            // The slot's paint frame is read before the
                            // status lock (scripts -> statuses is the only
                            // order the two mutexes may nest).
                            let paint = script_paint_of(&slot_scripts, name);
                            let (up, here) = {
                                let mut all = slot_statuses.lock().unwrap();
                                let mut up = false;
                                let mut here = None;
                                for s in all.iter_mut() {
                                    if s.username == *name {
                                        // Keep the producer gate closed until a current
                                        // player observation can authorize game actions.
                                        s.ingame = ready;
                                        s.scene_state = nav_snapshot.scene_state();
                                        s.login_latched =
                                            arm_latch_obs.latch.load(Ordering::Relaxed);
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
                                    }
                                }
                                (up, here)
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
                            script_observe_cached(
                                c,
                                name,
                                up,
                                tick_edge,
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
                            );
                            // TUI chat / WASD sends: run the queued wire
                            // commands through `Interactions` on this
                            // slot's own Client, so Continue/Answer/Walk
                            // respect the same preconditions as the
                            // guardian and the scenario runner. The slot
                            // must not be frozen by the guardian's hold
                            // when it presses a dialog the guardian is
                            // talking through, but a walk while held is
                            // dropped (the hold freezes the follow too).
                            let wires = {
                                let mut all = slot_wires.lock().unwrap();
                                all.get_mut(name)
                                    .map(std::mem::take)
                                    .unwrap_or_default()
                            };
                            if !wires.is_empty() {
                                dispatch_wires(c, &nav_snapshot, wires.into(), hold);
                            }
                            // Per-uid nav step on the pump, gated on the
                            // player-gen/tile latch like the panel's WalkTo
                            // hook so a hop is sent once per server tick,
                            // not re-sent every 20 ms frame. The snapshot
                            // was already rebuilt above. The guardian's
                            // hold freezes the follow — the armed route
                            // stays latched and resumes when it lifts.
                            let nav_key = (c.gens.player, here);
                            if last_nav_step != Some(nav_key) {
                                last_nav_step = Some(nav_key);
                                if here.is_some()
                                    && !hold
                                    && slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                        b.route.is_some() || b.bank_fetch.is_some()
                                    })
                                {
                                    step_nav_bot(
                                        c,
                                        name,
                                        here,
                                        &nav_snapshot,
                                        &slot_navs,
                                        &slot_statuses,
                                        slot_world.as_deref(),
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
                                    b.route.is_some() || b.bank_fetch.is_some()
                                })
                        }
                    },
                    {
                        let ifaces_template = ifaces_template.clone();
                        move |c| tick_flags(c, &ifaces_template, &arm_obs) || !c.ingame
                    },
                    knock,
                );
                publish_slot_disconnected(&slot_statuses, &username);
                reset_slot_session_work(
                    &username,
                    &slot_scripts,
                    &slot_cheats,
                    &slot_wires,
                    &slot_navs,
                );
                if arm.stop.load(Ordering::Relaxed) {
                    return;
                }
            }
            })
            .expect("failed to spawn slot thread"),
    );
}

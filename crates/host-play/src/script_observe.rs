use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use api::host_log;
use api::hostlog::{Category, Level};
use api::interact::Driver;
use api::snapshot::GameSnapshot;
#[cfg(any(feature = "js", test))]
use client::client::Client;
use client::config::Cache;
use host::{ScriptRunPolicy, SlotInput};
use nav_planner::world::NavWorld;
use nav_planner::WorldState;
use script::slot::SeatWalkTerminal;
use script::{ScriptCtx, SlotScript};

use super::{
    abort_script_walk, apply_watchdog_nav_action, dispatch_native_bank_deposit,
    dispatch_observed_bank_op, dispatch_script_interact_cached_scoped, pack_cached_reach,
    recovery_walk_idle, reset_script_nav, resumed_walk, route_inspect, script_slot,
    take_carried_walk, BankDepositDispatch, InteractContext, NavBot, ScriptWall,
};
#[cfg(feature = "js")]
use super::{with_script_snapshot_input_shorts, PostedWalkOutcome};
use crate::debug_enabled;
#[cfg(feature = "memory-profile")]
use crate::memory_diagnostics;
use crate::play_login::SlotArm;
#[cfg(test)]
use std::sync::{Condvar, LazyLock, Weak};

#[cfg(test)]
pub(crate) struct DispatchBarrier {
    entered: (Mutex<bool>, Condvar),
    release: (Mutex<bool>, Condvar),
    target: Mutex<Option<std::thread::ThreadId>>,
}
#[cfg(test)]
impl DispatchBarrier {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: (Mutex::new(false), Condvar::new()),
            release: (Mutex::new(false), Condvar::new()),
            target: Mutex::new(None),
        })
    }

    pub(crate) fn wait_entered(&self) {
        let (lock, signal) = &self.entered;
        let (entered, timeout) = signal
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |entered| {
                !*entered
            })
            .unwrap();
        assert!(
            *entered && !timeout.timed_out(),
            "dispatch barrier was not entered"
        );
    }

    pub(crate) fn release(&self) {
        let (lock, signal) = &self.release;
        *lock.lock().unwrap() = true;
        signal.notify_all();
    }

    fn wait_release(&self) {
        let (lock, signal) = &self.release;
        let (released, timeout) = signal
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |released| {
                !*released
            })
            .unwrap();
        assert!(
            *released && !timeout.timed_out(),
            "dispatch barrier was not released"
        );
    }

    pub(crate) fn arm_for_current_thread(&self) {
        *self.target.lock().unwrap() = Some(std::thread::current().id());
    }

    fn enter(&self) {
        if *self.target.lock().unwrap() != Some(std::thread::current().id()) {
            return;
        }
        let (lock, signal) = &self.entered;
        *lock.lock().unwrap() = true;
        signal.notify_all();
        self.wait_release();
    }
}

#[cfg(test)]
static DISPATCH_BARRIERS: LazyLock<Mutex<Vec<Weak<DispatchBarrier>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

#[cfg(test)]
pub(crate) fn install_dispatch_barrier(barrier: Arc<DispatchBarrier>) {
    DISPATCH_BARRIERS
        .lock()
        .unwrap()
        .push(Arc::downgrade(&barrier));
}

#[cfg(test)]
fn wait_dispatch_barrier() {
    let barriers = {
        let mut registered = DISPATCH_BARRIERS.lock().unwrap();
        let live: Vec<_> = registered.iter().filter_map(Weak::upgrade).collect();
        registered.retain(|barrier| barrier.strong_count() != 0);
        live
    };
    for barrier in barriers {
        barrier.enter();
    }
}

#[cfg(test)]
type OrderedDispatchHook = Box<dyn FnOnce(&script::native::HostAuthority)>;

#[cfg(test)]
std::thread_local! {
    static ORDERED_DISPATCH_HOOK: std::cell::RefCell<Option<OrderedDispatchHook>> =
        const { std::cell::RefCell::new(None) };
    static DELAY_ORDERED_DRAIN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn install_ordered_dispatch_hook(hook: OrderedDispatchHook) {
    ORDERED_DISPATCH_HOOK.with(|cell| {
        assert!(cell.borrow_mut().replace(hook).is_none());
    });
}

#[cfg(test)]
fn run_ordered_dispatch_hook(authority: &script::native::HostAuthority) {
    let hook = ORDERED_DISPATCH_HOOK.with(|cell| cell.borrow_mut().take());
    if let Some(hook) = hook {
        hook(authority);
    }
}

#[cfg(test)]
pub(crate) fn delay_ordered_drain_for_test() {
    DELAY_ORDERED_DRAIN.with(|delay| assert!(!delay.replace(true)));
}

#[cfg(feature = "js")]
/// Post the slot's snapshot. Only a post the isolate accepted counts as the
/// isolate having seen the walk outcome it carries (`walk_seq`), so only
/// then is the live refusal guard released. A refused post leaves the guard
/// in place, and the isolate refuses the paired tick.
pub(crate) fn post_script_snapshot(
    slot: &mut SlotScript,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    walk_seq: u64,
    post: impl FnOnce(&mut SlotScript) -> bool,
) -> bool {
    let accepted = post(slot);
    if accepted {
        if let Some(bot) = navs.lock().unwrap().get_mut(name) {
            bot.mark_walk_outcome_posted(walk_seq);
        }
    }
    accepted
}

fn watchdog_pending_event(slot: &SlotScript) -> Option<Arc<str>> {
    slot.native_status().and_then(|status| {
        status.fields.iter().find_map(|field| {
            if field.key != "last_event" {
                return None;
            }
            match &field.value {
                script::native::StatusValue::Text(text)
                    if !text.is_empty() && text.as_ref() != "started" =>
                {
                    Some(Arc::clone(text))
                }
                _ => None,
            }
        })
    })
}

fn watchdog_restart_line(pending: Option<&str>) -> String {
    match pending.filter(|reason| !reason.is_empty() && *reason != "started") {
        Some(reason) => format!("restarted the script ({reason})"),
        None => "restarted the script".to_owned(),
    }
}

fn trace_watchdog_action(slot: &SlotScript, name: &str, tick: u64, action: script::WatchdogAction) {
    if action == script::WatchdogAction::None || !api::hostlog::debug_enabled() {
        return;
    }
    host_log!(
        Category::Watchdog,
        Level::Debug,
        slot = name,
        "watchdog-action run={:?} tick={} action={:?}",
        slot.native_run(),
        tick,
        action
    );
}

/// Restart a script the watchdog gave up on and log the outcome.
fn watchdog_restart(slot: &mut SlotScript, name: &str, now: Instant) {
    let pending = watchdog_pending_event(slot);
    match slot.restart_from_identity(now) {
        Ok(()) => host_log!(
            Category::Watchdog,
            Level::Warn,
            slot = name,
            "{}",
            watchdog_restart_line(pending.as_deref())
        ),
        Err(e) => host_log!(
            stderr;
            Category::Watchdog,
            Level::Error,
            slot = name,
            "restart failed: {e}"
        ),
    }
}

/// Drain isolate `this.log` / tick-error lines onto stderr when
/// `BOT_DEBUG=1`. Tick errors also become [`SlotScript::last_error`].
fn emit_script_debug_logs(slot: &mut SlotScript, name: &str) {
    let logs = slot.drain_logs();
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::logs(name, &logs);
    if !debug_enabled() {
        return;
    }
    for line in logs {
        host_log!(Category::Echo, Level::Debug, slot = name, "{line}");
    }
}

/// One observe of a slot's script wiring: gate [`SlotScript::on_is_up`],
/// dispatch [`SlotScript::on_game_tick`] on the PLAYER_INFO edge, then run
/// any cheats the panel queued. `driver` is the slot body's own `Client`
/// (the only `Driver`); `here` is the local player's world tile
/// `(x, z, level)` when the body decoded one, else `None` (then the walk
/// hooks refuse to arm). `state` is the slot's gating facts at observe
/// time (built from its live snapshot); `None` when no player is decoded
/// — the walk arm then routes with the fail-closed empty [`WorldState`],
/// so an edge whose requirements the state cannot prove is never relaxed.
/// `snapshot` is the same per-tick [`GameSnapshot`] the native borrowed view and
/// isolate post read. Queued effects require that observed frame and pass the
/// single shared host dispatch fence. Walk find runs off-pump and follow stays
/// on the slot pump; compiled cards no longer receive direct route closures.
/// `hold` is the slot loop's frame hold (the scene is not ready, the
/// welcome modal is up, the random-event guardian holds, or a session
/// boundary applies): while true the tick is not dispatched (follow is
/// frozen by the pump too), so a script cannot walk through an in-flight
/// dialog or a trapped maze/mime/box — the snapshot blob still posts on
/// the held tick edge, so EventSignal reads the freeze the script is
/// frozen by. `guardian_hold` is the guardian's part of `hold`; only it
/// freezes the seat (`SessionHeld`). `ours` is the guardian's published
/// detected-ours flag (posted into the isolate for `EventSignal.pending()`).
/// Returns whether the driver's
/// out buffer was written (the slot's own `Client` sends on its next
/// mainloop pass). A slot whose script is Idle/Paused publishes nothing
/// — no dispatch, no flush.
// Slot threads pass the same shared handles everywhere; a context struct
// would churn every call site, so the arg count is allowed on purpose.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn script_observe_with_npc_boxes(
    driver: &mut dyn Driver,
    name: &str,
    up: bool,
    tick_edge: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    inv: Option<&[(i32, i32)]>,
    state: Option<WorldState>,
    snapshot: Option<&GameSnapshot>,
    npc_boxes: Option<&[script::native::snapshot_source::NpcBoxInput]>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    ours: bool,
    canlight: Option<&[u64]>,
    slot_input: Option<&SlotInput>,
) -> bool {
    script_observe_cached(
        driver, name, up, tick_edge, tick, here, inv, state, snapshot, npc_boxes, obj_names,
        scripts, cheats, navs, world, hold, ours, canlight, slot_input, None, None, None,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn script_observe_cached(
    driver: &mut dyn Driver,
    name: &str,
    up: bool,
    tick_edge: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    inv: Option<&[(i32, i32)]>,
    state: Option<WorldState>,
    snapshot: Option<&GameSnapshot>,
    npc_boxes: Option<&[script::native::snapshot_source::NpcBoxInput]>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    ours: bool,
    canlight: Option<&[u64]>,
    slot_input: Option<&SlotInput>,
    cache: Option<Arc<Cache>>,
    obj_names_arc: Option<Arc<api::obj_names::ObjNames>>,
    run_policy: Option<&mut ScriptRunPolicy>,
) -> bool {
    #[cfg(not(feature = "js"))]
    let _ = npc_boxes;
    script_observe_cached_with_channels(
        driver,
        name,
        up,
        tick_edge,
        false,
        tick,
        here,
        inv,
        state,
        snapshot,
        None,
        #[cfg(feature = "js")]
        npc_boxes,
        obj_names,
        scripts,
        cheats,
        navs,
        world,
        hold,
        // The test wrappers model the guardian's hold.
        hold,
        ours,
        canlight,
        slot_input,
        cache,
        obj_names_arc,
        #[cfg(feature = "js")]
        None,
        #[cfg(feature = "js")]
        super::script_channels::BrokerWorld::Unavailable,
        run_policy,
        None,
        None,
        None,
    )
    .wrote
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DrainedRunPolicyUpdate {
    producer_generation: u64,
    policy: Option<api::run_policy::RunPolicyOverride>,
}

impl DrainedRunPolicyUpdate {
    pub(crate) fn apply(self, run_policy: &mut ScriptRunPolicy) {
        run_policy.apply_override(self.producer_generation, self.policy);
    }
}

pub(crate) fn drain_observed_host_interacts(
    slot: &mut SlotScript,
) -> (
    Option<DrainedRunPolicyUpdate>,
    Vec<script::interact::QueuedInteract>,
    bool,
) {
    let producer_generation = slot.runtime_generation();
    let (update, interacts, owned) = slot.drain_host_interacts();
    (
        update.map(|policy| DrainedRunPolicyUpdate {
            producer_generation,
            policy,
        }),
        interacts,
        owned,
    )
}

/// Paint ownership leaves the same locked observation that admits and drains
/// game work. The client flag is applied before this pump can rasterize.
#[derive(Default)]
pub(crate) struct ScriptObservation {
    pub wrote: bool,
    pub journal_paint_hidden: bool,
    /// A live nonzero native batch was drained; freeze follow for this key.
    pub exclusive: bool,
}

/// The observed run ended in this frame and its navigation is reset, once per
/// run: its published rows end with it, as in an explicit Stop
/// (`retire_script_run_status`). Runs under the slot lock, which keeps the
/// order script, then statuses. `None` is a caller without status rows.
fn retire_run_status(statuses: Option<&Arc<Mutex<Vec<crate::SlotStatus>>>>, name: &str) {
    if let Some(statuses) = statuses {
        crate::play_scripts::retire_script_run_status(statuses, name);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn script_observe_cached_with_channels(
    driver: &mut dyn Driver,
    name: &str,
    up: bool,
    tick_edge: bool,
    wait_families_dirty: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    inv: Option<&[(i32, i32)]>,
    state: Option<WorldState>,
    snapshot: Option<&GameSnapshot>,
    bank_memory: Option<&parking_lot::RwLock<api::bank_memory::BankMemory>>,
    #[cfg(feature = "js")] npc_boxes: Option<&[script::native::snapshot_source::NpcBoxInput]>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    guardian_hold: bool,
    ours: bool,
    canlight: Option<&[u64]>,
    slot_input: Option<&SlotInput>,
    cache: Option<Arc<Cache>>,
    obj_names_arc: Option<Arc<api::obj_names::ObjNames>>,
    #[cfg(feature = "js")] channels: Option<&super::script_channels::SlotChannels>,
    #[cfg(feature = "js")] channel_world: super::script_channels::BrokerWorld,
    run_policy: Option<&mut ScriptRunPolicy>,
    mut debug_replies: Option<&mut super::debug_replies::DebugReplies>,
    service_arm: Option<&SlotArm>,
    statuses: Option<&Arc<Mutex<Vec<crate::SlotStatus>>>>,
) -> ScriptObservation {
    if let Some(inp) = slot_input {
        inp.set_host_consume_allowed(up && !hold);
        if let Some(snapshot) = snapshot {
            inp.observe_input_session(driver.revision(), snapshot.gens().session);
        }
        if !up || hold || guardian_hold {
            inp.with_server_ordered_input(|state| state.invalidate_content_expectation());
        }
    }
    // One relaxed load of the slot's Cure poison mirror per frame: a
    // durable settings write is visible on the next frame the slot
    // observes. The NavBot copy serves host consumers with no script
    // context (the walk guard); the same value posts to native
    // machines in the compiled tick below.
    let cure_poison = service_arm.map(SlotArm::cure_poison).unwrap_or_default();
    {
        let mut navs = navs.lock().unwrap();
        if let Some(bot) = navs.get_mut(name) {
            bot.cure_poison = cure_poison;
        }
    }
    let mut wrote = false;
    let mut journal_wall_now = None;
    let mut journal_paint_hidden = false;
    let mut exclusive = false;
    let mut interact = Vec::new();
    let mut pending_withdraw_x_active = false;
    let mut pending_bank_op_active = false;
    route_inspect::poll_inspection(navs, name);
    // Advance the host service even though only the native seat publishes the
    // public selection. Asynchronous picks otherwise never deliver a receipt.
    let _ = super::bank::poll_bank_pick(navs, name, Instant::now());
    let observed_slot = script_slot(scripts, name);
    let mut slot_work_epoch = None;
    let mut has_native_actions = false;
    let mut walk_permissions = None;
    #[cfg(feature = "js")]
    let mut channel_generation = 0;
    #[cfg(feature = "js")]
    let mut channel_active = false;
    let mut policy_runtime_generation = None;
    let mut run_policy_update = None;
    'script_slot: {
        let Some(slot) = observed_slot.as_ref() else {
            break 'script_slot;
        };
        let Ok(mut slot) = slot.lock() else {
            break 'script_slot;
        };
        slot.observe_lifecycle();
        let bank_selection = slot.bank_selection_input();
        let work_scope = slot.work_scope();
        if let Some(bot) = navs
            .lock()
            .expect("navigation state mutex is not poisoned")
            .get_mut(name)
        {
            bot.work_scope = work_scope;
            bot.quest_context = super::script_nav::route_quest_context(&slot, tick);
        }
        // Only the random-event guardian holds the seat (`SessionHeld`,
        // `28-seat-sg.md:65`; `27-decision-r5.md` §6). Every other frame
        // hold is a held tick (`28-seat-sg.md:64`): the seat is not frozen,
        // it drops the held frames' gameplay, and it keeps the abort releases
        // for the first frame after the hold (operator decision
        // 2026-10-16, `SlotScript::seat_host_hold`). A session boundary
        // reconnects or resets the seat (`SlotScript::reconnect_session_work`,
        // `reset_session_work`); a native run's seat also holds through
        // `align_seat_session`.
        slot.seat_guardian_hold(guardian_hold);
        slot.seat_host_hold(hold && !guardian_hold);
        let stop_prayers = slot.take_stop_prayer_cleanup();
        let prayer_cleanup_hold = if let Some(snapshot) = snapshot.filter(|_| up) {
            let mut bots = navs.lock().unwrap();
            if !stop_prayers.is_empty() {
                let bot = bots.entry(name.to_owned()).or_default();
                super::script_walk::owe_combat_prayers_off(
                    bot,
                    stop_prayers,
                    snapshot.tick() as u16,
                );
            }
            if let Some(bot) = bots.get_mut(name) {
                super::script_walk::finish_combat_prayers(driver, snapshot, bot, Some(name));
                bot.combat_prayer_off.is_some()
            } else {
                false
            }
        } else {
            false
        };
        if matches!(
            slot.state(),
            script::RunState::Starting | script::RunState::Paused
        ) && slot.surface_active()
        {
            let (update, queued, _) = drain_observed_host_interacts(&mut slot);
            if let Some(update) = update {
                run_policy_update = Some(update);
            }
            // Ownership already removed game rows; retained non-game rows
            // follow the same pause rule as an unowned batch in every frame.
            slot.restore_host_interacts(queued);
        }
        // Reap a script-requested Stop before advancing host continuations.
        emit_script_debug_logs(&mut slot, name);
        let terminal_slot = matches!(
            slot.state(),
            script::RunState::Idle | script::RunState::Error
        );
        if terminal_slot {
            if let Some(generation) = slot.terminal_lifecycle_generation() {
                if reset_script_nav(navs, name, Some(generation)) {
                    retire_run_status(statuses, name);
                }
            } else if navs
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(NavBot::script_walk_armed)
            {
                // Legacy self-stops without a terminal receipt can only own a walk.
                abort_script_walk(navs, name);
                retire_run_status(statuses, name);
            }
        }
        slot.on_is_up(up);
        // Compiled clue machine: apply the owed abort and this frame's
        // pause/hold freeze here, on the slot's own thread, whether or not
        // this frame dispatches a tick. A Load slot's isolate thread runs the
        // same hooks for its own instance.
        slot.sync_compiled_clue(
            hold || ours
                || !up
                || !snapshot.is_some_and(|snap| snap.ingame() && snap.scene_state() == 2),
        );
        // Deliver only the terminal belonging to this still-live native owner.
        // The slot repeats the run/action fence before accepting the receipt.
        let mut host_results_dirty = false;
        if let Some(bot) = navs.lock().unwrap().get_mut(name) {
            host_results_dirty |= host_completion_changed(&slot, bot, bank_selection);
            let native_terminal =
                bot.native_end.as_ref().is_some_and(|(owner, _)| {
                    owner.request_id().get() == bot.walk_outcome_request_id
                }) || bot.native_walk.as_ref().is_some_and(|owner| {
                    owner.live() && owner.request_id().get() == bot.walk_outcome_request_id
                });
            deliver_native_walk_end(&mut slot, bot, tick);
            host_results_dirty |= deliver_native_advice(&mut slot, bot);
            // A newly armed walk may already have replaced the completed
            // route. Its predecessor still owns a release slot. Only when
            // the same request id remains armed must the route generation
            // match, so a stale outcome cannot free the new arm.
            let terminal_request = api::identity::RequestId::from_raw(bot.walk_outcome_request_id)
                .filter(|request| {
                    request.get() != bot.walk_request_id
                        || bot.walk_outcome_generation == bot.route_generation
                })
                .map(|request| {
                    if native_terminal {
                        SeatWalkTerminal::Native(request)
                    } else {
                        SeatWalkTerminal::Load(request)
                    }
                });
            host_results_dirty |= slot.observe_walk_outcome(
                bot.walk_outcome_seq,
                bot.compiled_walk_cancel_reason(),
                terminal_request,
            );
        }
        slot_work_epoch = Some(slot.work_epoch());
        #[cfg(feature = "js")]
        {
            channel_generation = slot.runtime_generation();
            // Region loads briefly clear `up`; the isolate and its browser-style
            // channel survive them. Session boundaries suspend the broker in the
            // slot loop before this observation runs.
            channel_active = slot.surface_active()
                && matches!(
                    slot.state(),
                    script::RunState::Running | script::RunState::Paused
                );
        }
        if let Some(pending) = slot.pending_bank_op() {
            if hold || slot.state() == script::RunState::Paused {
                slot.freeze_pending_bank_op();
                pending_bank_op_active = true;
            } else {
                slot.resume_pending_bank_op();
                let valid_session = snapshot.is_some_and(|snap| {
                    snap.bank_component_id() >= 0
                        && snap.bank_loaded()
                        && snap.bank_session_generation() == pending.bank_generation
                });
                if !valid_session {
                    slot.complete_bank_op(false);
                } else if matches!(
                    pending.kind,
                    script::slot::PendingBankOpKind::WithdrawXAction
                ) {
                    // Raw open-only `Withdraw X`: the accepted menu action is
                    // the result the frozen caller awaits (it types the amount
                    // + Enter itself), so no bank or inventory delta can exist
                    // yet. Acknowledge on the same generation/bank-open gate
                    // the counted ops settle through.
                    slot.complete_bank_op(true);
                } else {
                    let current = snapshot.map_or(0, |snap| match pending.kind {
                        script::slot::PendingBankOpKind::Deposit => snap
                            .bank_side()
                            .iter()
                            .filter(|item| item.def.id == pending.item_id)
                            .map(|item| item.count)
                            .sum(),
                        // The open-only X kind never reaches the delta
                        // branch: it acknowledged on the session gate above.
                        script::slot::PendingBankOpKind::Withdraw
                        | script::slot::PendingBankOpKind::WithdrawXAction => snap
                            .bank()
                            .iter()
                            .filter(|item| item.def.id == pending.item_id)
                            .map(|item| item.count)
                            .sum(),
                    });
                    let inventory_increased =
                        matches!(pending.kind, script::slot::PendingBankOpKind::Withdraw)
                            && inv.is_some_and(|items| {
                                items
                                    .iter()
                                    .filter(|(id, _)| *id == pending.item_id)
                                    .map(|(_, count)| *count)
                                    .sum::<i32>()
                                    > pending.before_inventory_count
                            });
                    if current < pending.before_count || inventory_increased {
                        slot.complete_bank_op(true);
                    } else if pending.expired() {
                        slot.complete_bank_op(false);
                    } else {
                        pending_bank_op_active = true;
                    }
                }
            }
        }
        if let Some(pending) = slot.pending_withdraw_x() {
            if hold || slot.state() == script::RunState::Paused {
                slot.freeze_pending_withdraw_x();
                pending_withdraw_x_active = true;
            } else {
                slot.resume_pending_withdraw_x();
                let valid_session = snapshot.is_some_and(|snap| {
                    snap.bank_component_id() >= 0
                        && snap.bank_loaded()
                        && snap.bank_session_generation() == pending.bank_generation
                });
                if !valid_session {
                    slot.complete_withdraw_x(false);
                } else {
                    match pending.phase {
                        script::slot::PendingWithdrawXPhase::Dialog => {
                            if pending.expired() {
                                slot.complete_withdraw_x(false);
                            } else if snapshot.is_some_and(GameSnapshot::count_dialog_open) {
                                let sent = snapshot.is_some_and(|snap| {
                                    matches!(
                                        api::interact::Interactions::new(snap, driver)
                                            .answer_count(pending.count),
                                        api::interact::SendResult::Sent { .. }
                                    )
                                });
                                if sent {
                                    wrote = true;
                                    slot.set_pending_withdraw_x(Some(pending.waiting_settlement()));
                                    pending_withdraw_x_active = true;
                                } else {
                                    slot.complete_withdraw_x(false);
                                }
                            } else {
                                pending_withdraw_x_active = true;
                            }
                        }
                        script::slot::PendingWithdrawXPhase::Settlement => {
                            let (current, full) = if let Some(rows) = inv {
                                (
                                    rows.iter()
                                        .filter(|(id, _)| *id == pending.item_id)
                                        .map(|(_, count)| *count)
                                        .sum::<i32>(),
                                    snapshot.is_some_and(|snap| {
                                        snap.inventory_size() > 0
                                            && rows.len() >= snap.inventory_size() as usize
                                    }),
                                )
                            } else {
                                snapshot.map_or((0, false), |snap| {
                                    (
                                        snap.inventory()
                                            .iter()
                                            .filter(|item| item.def.id == pending.item_id)
                                            .map(|item| item.count)
                                            .sum::<i32>(),
                                        snap.inventory_size() > 0
                                            && snap.inventory().len()
                                                >= snap.inventory_size() as usize,
                                    )
                                })
                            };
                            if current >= pending.target || (current > pending.before && full) {
                                slot.complete_withdraw_x(true);
                            } else if pending.expired() {
                                slot.complete_withdraw_x(false);
                            } else {
                                pending_withdraw_x_active = true;
                            }
                        }
                    }
                }
            }
        }
        host_results_dirty |= slot.posted_snapshot_facts().is_some_and(|last| {
            last.withdraw_x_result_seq != slot.withdraw_x_result().0
                || last.bank_op_result_seq != slot.bank_op_result().0
        });
        // Evidence wakes use the latest observed PLAYER_INFO tick. They poll
        // compiled/API/compat machines, never dispatch another JS game tick.
        // Held runs remain frozen, and frames before the first tick cannot
        // obtain an interaction budget.
        let wake_waits = !tick_edge
            && tick != 0
            && (wait_families_dirty || host_results_dirty)
            && !prayer_cleanup_hold
            && !hold
            && !slot.watchdog().holds_script_actions();
        // Load Gather owns its own committed permissions; other native
        // ticks use the compiled card's current permissions. Pass the
        // same callback value to selection and route admission.
        walk_permissions = slot
            .api_gather_walk_permissions()
            .or_else(|| slot.native_walk_permissions());
        if (tick_edge || wake_waits) && slot.state() == script::RunState::Running {
            // Task 9b: post the FlatBuffer snapshot blob on every tick
            // edge — held or not — so the isolate's
            // Game/Inventory/Skills/Bank/Banking/EventSignal read what
            // this observe saw (only these fields — no World clone).
            // The blob's `hold` mirrors onto `__rs2b0t_host.hold` so a
            // posted hold freezes the isolate without a probe poke.
            // Task 9c: the post is a DELTA — `tick` always, other
            // fields only when changed vs the slot's last post (a
            // 50+ isolate wall never resends unchanged tables). The
            // packed banks are re-posted when the NavWorld identity
            // changed (identity = the shared Arc's pointer), not when
            // the stand list merely rebuilds identical.
            #[cfg(feature = "js")]
            let world_id = world.as_ref().map(|w| Arc::as_ptr(w) as usize);
            #[cfg(feature = "js")]
            let force_banks = world_id.is_some_and(|id| slot.last_world_id() != Some(id));
            // Guardian `hold` freezes clocks/nav and cancels owned recovery.
            // Recovery hold is host-owned and must still reach the isolate so
            // loop/pump freeze, without being treated as that external freeze.
            let recovery_hold = slot.watchdog().holds_script_actions();
            let isolate_hold = hold || recovery_hold || prayer_cleanup_hold;
            #[cfg(feature = "js")]
            if slot.surface_active() {
                let (
                    teleports_enabled,
                    walk_seq,
                    walk_generation,
                    walk_request_id,
                    walk_failed,
                    walk_x,
                    walk_z,
                    walk_level,
                    walk_radius,
                    walk_allow_teleports,
                    walk_blocked,
                    walk_cancel_reason,
                    user_move_intent_seq,
                    walk_missing_carry,
                    inspect_posted,
                    bank_selection,
                ) = {
                    let mut all = navs.lock().unwrap();
                    match all.get_mut(name) {
                        Some(b) => {
                            let posted = (
                                b.allow_teleports,
                                b.walk_outcome_seq,
                                b.walk_outcome_generation,
                                b.walk_outcome_request_id,
                                b.walk_outcome_failed,
                                b.walk_outcome_x,
                                b.walk_outcome_z,
                                b.walk_outcome_level,
                                b.walk_outcome_radius,
                                b.walk_outcome_allow_teleports,
                                b.walk_outcome_blocked,
                                b.walk_outcome_cancel_reason,
                                b.user_move_intent_seq,
                                b.walk_missing_carry.clone(),
                                b.inspect.posted(),
                                bank_selection,
                            );
                            // The live refusal guard is released only once
                            // the isolate accepts the snapshot carrying this
                            // outcome (`post_script_snapshot`).
                            posted
                        }
                        None => (
                            false,
                            0,
                            0,
                            0,
                            false,
                            0,
                            0,
                            0,
                            0,
                            false,
                            false,
                            script::native::WalkCancelReason::None,
                            0,
                            Vec::new(),
                            route_inspect::PostedInspect::default(),
                            script::native::snapshot_source::BankSelectionInput::default(),
                        ),
                    }
                };
                let (withdraw_x_result_seq, withdraw_x_result) = slot.withdraw_x_result();
                let (bank_op_result_seq, bank_op_result) = slot.bank_op_result();
                // The published failure's shopping list, as the isolate reads
                // it: the diagnosis' own ids and counts, with the host obj
                // table's display name beside them when it has one. Always
                // supplied — an empty list is the observed "no named short",
                // so a clear is never omitted.
                let carry_rows: Vec<script::native::snapshot_source::CarryInput<'_>> =
                    walk_missing_carry
                        .iter()
                        .map(|row| script::native::snapshot_source::CarryInput {
                            id: row.id,
                            count: row.count,
                            name: obj_names.and_then(|names| names.name(row.id)),
                        })
                        .collect();
                let packed = pack_cached_reach(
                    slot.reach_pack_cache(),
                    snapshot,
                    here,
                    world.as_deref(),
                    canlight,
                );

                with_script_snapshot_input_shorts(
                    tick,
                    here,
                    up,
                    inv,
                    snapshot,
                    obj_names,
                    world.as_deref(),
                    npc_boxes,
                    isolate_hold,
                    ours,
                    teleports_enabled,
                    withdraw_x_result_seq,
                    withdraw_x_result,
                    bank_op_result_seq,
                    bank_op_result,
                    canlight,
                    PostedWalkOutcome {
                        seq: walk_seq,
                        generation: walk_generation,
                        request_id: walk_request_id,
                        failed: walk_failed,
                        x: walk_x,
                        z: walk_z,
                        level: walk_level,
                        radius: walk_radius,
                        allow_teleports: walk_allow_teleports,
                        blocked: walk_blocked,
                        cancel_reason: walk_cancel_reason,
                        user_move_intent_seq,
                    },
                    &carry_rows,
                    inspect_posted,
                    bank_selection,
                    Some(packed.view.as_ref()),
                    packed.flood.as_deref(),
                    packed.stamp,
                    |input, native| {
                        post_script_snapshot(&mut slot, navs, name, walk_seq, |slot| {
                            if wake_waits {
                                slot.post_snapshot_wake(
                                    input,
                                    native,
                                    force_banks,
                                    inv.is_none() && input.inv_size == 0,
                                )
                            } else {
                                slot.post_snapshot_delta(input, native, force_banks)
                            }
                        })
                    },
                );
                slot.store_last_world_id(world_id);
            }
            if isolate_hold {
                if !guardian_hold {
                    slot.seat_held_tick();
                }
                // Isolate: tick for onPaint only (hold gate inside V8 via
                // snapshot.hold). Recovery hold is distinct from guardian
                // hold: nav follow of the owned recovery route continues,
                // but script walk arms are not provided. Compiled: skip on
                // guardian hold.
                #[cfg(feature = "js")]
                if slot.surface_active() {
                    slot.on_game_tick(&mut ScriptCtx {
                        driver,
                        tick,
                        here,
                        walk: None,
                        walk_with: None,
                        inv,
                        snapshot,
                        obj_names,
                        compiled: script::CompiledTick {
                            hold: isolate_hold,
                            ..Default::default()
                        },
                    });
                    wrote = true;
                }
            } else {
                let selected = slot.compiled_game_data();
                let world_members = slot.world_members();
                let packed = selected.as_ref().map(|_| {
                    pack_cached_reach(
                        slot.reach_pack_cache(),
                        snapshot,
                        here,
                        world.as_deref(),
                        canlight,
                    )
                });
                // The compiled tick borrows the account's bank memory
                // through this read guard and nothing after it does
                // (design-bank-snapshot §1.2): the guard ends with this
                // block, before effect dispatch, logging and queued cheats.
                let bank_memory = bank_memory.map(parking_lot::RwLock::read);
                // Keep the real lifecycle gate stable through this callback.
                // Its guard drops before host effect dispatch below.
                let service_gate_guard = if slot.native_run().is_some() || slot.surface_active() {
                    service_arm.map(SlotArm::session_gate)
                } else {
                    None
                };
                let service_gate = service_gate_guard.as_deref();
                let walk_danger = world.as_deref().map(|world| {
                    crate::walk_permissions::SelectionDangerQuery::new(
                        navs,
                        name,
                        world,
                        snapshot,
                        walk_permissions,
                    )
                });
                // Copy only the small fact, not the slot owner or its tables.
                let (condition, poison_unknown_pulse) = {
                    let navs = navs.lock().unwrap();
                    let bot = navs.get(name);
                    (
                        bot.and_then(|bot| bot.condition.current().copied()),
                        bot.map(|bot| bot.condition.unknown_pulse()),
                    )
                };
                let mut ctx = ScriptCtx {
                    driver,
                    tick,
                    here,
                    walk: None,
                    walk_with: None,
                    inv,
                    snapshot,
                    obj_names,
                    compiled: script::CompiledTick {
                        service_gate,
                        selected: selected.as_deref(),
                        reach: packed.as_ref().map(|packed| packed.view.as_ref()),
                        reach_flood: packed.as_ref().and_then(|packed| packed.flood.as_deref()),
                        bank_memory: bank_memory.as_deref(),
                        condition,
                        poison_unknown_pulse,
                        cure_poison,
                        collision: world
                            .as_deref()
                            .and_then(|world| nav_planner::producer::collision_view(world).ok()),
                        walk_danger: walk_danger
                            .as_ref()
                            .map(|query| query as &dyn script::native::WalkDangerQuery),
                        hold: hold || ours,
                        world_members,
                        ..Default::default()
                    },
                };
                if let Some(gate) = service_gate {
                    slot.align_seat_session(gate.ready());
                }
                if wake_waits {
                    slot.on_snapshot_change(&mut ctx);
                } else {
                    slot.on_game_tick(&mut ctx);
                }
                wrote = true;
            }
        }
        {
            let mut navs = navs.lock().unwrap();
            let bot = navs.entry(name.to_owned()).or_default();
            bot.work_scope = slot.work_scope();
            bot.native_permissions = slot.native_walk_permissions();
            bot.compat_v1 = slot.surface_family() == Some(script::surface::SurfaceFamily::JsV1);
            bot.cure_poison = cure_poison;
        }
        emit_script_debug_logs(&mut slot, name);
        if slot.surface_active() && slot.state() != script::RunState::Running {
            slot.retain_held_seat_batches();
        }
        // Ordinary Pause leaves the isolate queue untouched. Guardian hold
        // has already discarded gameplay and bounded repeated control pages.
        // Running frames drain those controls through the seat below.
        if slot.surface_active() || slot.native_run().is_some() {
            let mut lifecycle = Vec::new();
            if slot.state() == script::RunState::Running {
                lifecycle.extend(slot.drain_lifecycle());
            }
            let ready = up
                && here.is_some()
                && snapshot.is_some_and(|snap| snap.ingame() && snap.scene_state() == 2);
            let frozen = hold || !ready || slot.state() == script::RunState::Paused;
            if ready {
                if let Some(snapshot) = snapshot {
                    slot.note_levels(snapshot.stats().iter().map(|stat| stat.base));
                }
            }
            let mut xp_slots = [0; 25];
            let xp = if let Some(snapshot) = snapshot {
                for (xp, stat) in xp_slots.iter_mut().zip(snapshot.stats()) {
                    *xp = stat.xp;
                }
                &xp_slots[..snapshot.stats().len()]
            } else {
                &[]
            };
            let now = Instant::now();
            journal_wall_now = Some(now);
            if tick_edge {
                slot.note_game_packet_tick(now);
            }
            if frozen {
                if slot.watchdog().recovering_anchor().is_some() {
                    if hold {
                        let _ = slot.notify_hold_during_walk();
                    } else {
                        let _ = slot.abort_owned_recovery();
                    }
                    abort_script_walk(navs, name);
                }
            } else if slot.watchdog().recovering_anchor().is_some() {
                let far = here
                    .map(|(x, z, level)| {
                        slot.watchdog().recovering_anchor().is_some_and(|anchor| {
                            level != anchor.level
                                || script::watchdog::chebyshev_xz((x, z), (anchor.x, anchor.z))
                                    > script::watchdog::WALK_RADIUS
                        })
                    })
                    .unwrap_or(true);
                // A recovery Resume re-entered is idle until `feed_watchdog`
                // re-arms its walk below: that is not a failed walk.
                if far && !slot.watchdog().rearm_pending() && recovery_walk_idle(navs, name) {
                    match slot.notify_walk_failed(now) {
                        script::WatchdogAction::Restart { .. } => {
                            interact.clear();
                            watchdog_restart(&mut slot, name, now);
                        }
                        other => apply_watchdog_nav_action(
                            other,
                            driver,
                            snapshot,
                            here,
                            navs,
                            world,
                            state.as_ref(),
                            bank_memory,
                            name,
                        ),
                    }
                }
            }
            let running = slot.state() == script::RunState::Running;
            let action = slot.feed_watchdog(now, here, xp, frozen, running, &lifecycle);
            trace_watchdog_action(&slot, name, tick, action);
            match action {
                script::WatchdogAction::Restart { .. } => {
                    interact.clear();
                    if frozen {
                        host_log!(
                            stderr;
                            Category::Watchdog,
                            Level::Warn,
                            slot = name,
                            "restart ignored: frozen"
                        );
                    } else {
                        watchdog_restart(&mut slot, name, now);
                    }
                }
                script::WatchdogAction::RequestAnchor => slot.request_recovery_anchor(),
                script::WatchdogAction::WarnHungLoop => {
                    // The slot stages its own "watchdog: hung loop" line, which
                    // the operator session moves onto the slot log.
                    host_log!(
                        Category::Echo,
                        Level::Warn,
                        slot = name,
                        "watchdog hung loop (10s)"
                    );
                }
                script::WatchdogAction::WarnNoGamePackets => {
                    host_log!(
                        Category::Echo,
                        Level::Warn,
                        slot = name,
                        "watchdog no game packets for 10 s"
                    );
                }
                script::WatchdogAction::HoldInFightPen => {
                    host_log!(
                        Category::Echo,
                        Level::Warn,
                        slot = name,
                        "watchdog: holding recovery in a Duel Arena fight pen; the recovery anchor is outside the pen"
                    );
                }
                other => apply_watchdog_nav_action(
                    other,
                    driver,
                    snapshot,
                    here,
                    navs,
                    world,
                    state.as_ref(),
                    bank_memory,
                    name,
                ),
            }
            slot.sync_native_input_gate();
        }
        if slot.surface_active() {
            if slot.state() == script::RunState::Running {
                // The seat admits controls and filters captured gameplay before
                // this host stage. A live API Session still fences a carried
                // pre-seat route, not releases or already-admitted rows.
                let (update, reqs, owned) = drain_observed_host_interacts(&mut slot);
                if slot.watchdog().holds_script_actions() || owned {
                    if owned {
                        let _ = take_carried_walk(navs, name, slot.runtime_generation());
                        interact.extend(take_script_interacts(reqs, slot_input));
                    }
                    if let Some(update) = update {
                        run_policy_update = Some(update);
                    }
                } else {
                    let mut carried = None;
                    if up && !hold && here.is_some() && snapshot.is_some() {
                        carried = take_carried_walk(navs, name, slot.runtime_generation());
                    }
                    if let Some(update) = update {
                        run_policy_update = Some(update);
                    }
                    let queued = take_script_interacts(reqs, slot_input);
                    let resumed =
                        resumed_walk(carried, queued.iter().map(|q| &q.req), |dest, radius| {
                            let (Some((x, z, level)), Some(snapshot)) = (here, snapshot) else {
                                return false;
                            };
                            let view = pack_cached_reach(
                                slot.reach_pack_cache(),
                                Some(snapshot),
                                here,
                                world.as_deref(),
                                canlight,
                            )
                            .view;
                            api::query::is_arrived(
                                api::snapshot::WorldTile { x, z, level },
                                dest,
                                radius,
                                || view,
                            )
                        });
                    interact.extend(resumed.map(|req| {
                        script::interact::QueuedInteract {
                            req,
                            observed_walk_outcome_seq: navs
                                .lock()
                                .unwrap()
                                .get(name)
                                .map_or(0, |bot| bot.walk_outcome_seq),
                        }
                    }));
                    interact.extend(queued);
                }
            }
        } else if (slot.state() == script::RunState::Running
            && !slot.watchdog().holds_script_actions()
            && !prayer_cleanup_hold)
            || slot.completed_seat_dispatch_open()
        {
            let (update, reqs, _) = drain_observed_host_interacts(&mut slot);
            if let Some(update) = update {
                run_policy_update = Some(update);
            }
            interact.extend(take_script_interacts(reqs, slot_input));
        }
        // Lifecycle/log/watchdog work above can stop or replace the runtime.
        // Sync to the state that leaves this observation; each drained policy
        // row retains its producer generation so a pre-restart row fails the
        // host fence instead of being relabelled as the replacement runtime.
        policy_runtime_generation = matches!(
            slot.state(),
            script::RunState::Starting | script::RunState::Running | script::RunState::Paused
        )
        .then_some(slot.runtime_generation());
        has_native_actions = slot.has_native_actions();
        journal_paint_hidden = journal_wall_now.is_some_and(|now| slot.journal_paint_hidden(now));
    }
    if let Some(run_policy) = run_policy {
        run_policy.sync_runtime(policy_runtime_generation);
        if let Some(update) = run_policy_update {
            update.apply(run_policy);
        }
    }
    #[cfg(feature = "js")]
    if let Some(channels) = channels {
        // Channel requests are the broker's, never the game's. A frame with
        // none allocates nothing, and an untracked slot takes no lock.
        let is_channel = |queued: &script::interact::QueuedInteract| {
            matches!(
                queued.req,
                script::shim::InteractReq::ChannelOpen { .. }
                    | script::shim::InteractReq::ChannelPost { .. }
                    | script::shim::InteractReq::ChannelClose { .. }
            )
        };
        let channel_reqs = if interact.iter().any(is_channel) {
            let (reqs, game) = interact.into_iter().partition(is_channel);
            interact = game;
            reqs
        } else {
            Vec::new()
        };
        if !channel_reqs.is_empty() || (channels.tracks() && (up || !channel_active)) {
            let deliveries = channels.pump(
                channel_generation,
                channel_world,
                channel_active,
                channel_reqs.into_iter().map(|queued| queued.req).collect(),
            );
            deliver_channel_events(scripts, deliveries);
        }
    }
    // Dispatch the shim's interact requests through the slot's own Driver
    // (open/deposit/withdraw) and the shared walk arm (bank-stand walks
    // with default FindOptions, so wilderness/quest gates fail closed).
    // The guardian's hold drops them: the script's parked wait stays
    // frozen, and a later retry re-queues what still matters.
    #[cfg(test)]
    if DELAY_ORDERED_DRAIN.with(|delay| delay.replace(false)) {
        has_native_actions = false;
    }
    if up && !hold && (!interact.is_empty() || has_native_actions) {
        if let Some(snapshot) = snapshot {
            #[cfg(test)]
            wait_dispatch_barrier();
            if let Some(dispatch_slot) = script_slot(scripts, name) {
                let same_lifetime = observed_slot
                    .as_ref()
                    .is_some_and(|observed| Arc::ptr_eq(observed, &dispatch_slot));
                let Ok(mut slot) = dispatch_slot.lock() else {
                    return ScriptObservation {
                        wrote,
                        journal_paint_hidden,
                        exclusive,
                    };
                };
                if same_lifetime
                    && (slot.state() == script::RunState::Running
                        || slot.completed_seat_dispatch_open())
                    && Some(slot.work_epoch()) == slot_work_epoch
                {
                    let compatibility_work_scope = slot.work_scope();
                    {
                        let mut navs = navs.lock().unwrap();
                        let bot = navs.entry(name.to_owned()).or_default();
                        bot.work_scope = slot.work_scope();
                        bot.quest_context = super::script_nav::route_quest_context(&slot, tick);
                        bot.native_permissions = slot.native_walk_permissions();
                        bot.compat_v1 =
                            slot.surface_family() == Some(script::surface::SurfaceFamily::JsV1);
                    }
                    let interact_context = InteractContext {
                        snapshot,
                        obj_names,
                        here,
                        navs,
                        world,
                        state: state.as_ref(),
                        bank: bank_memory,
                        name,
                        compatibility_scope: compatibility_work_scope,
                        cache: &cache,
                        obj_names_arc: &obj_names_arc,
                    };
                    // The native seat encodes release rows ahead of gameplay,
                    // but its two host consumers drain separately. Deliver JS
                    // Walk releases before the native action loop as well, so
                    // a fresh native drive cannot overtake a reserved stop.
                    if interact.iter().any(|queued| {
                        matches!(queued.req, script::shim::InteractReq::AbortWalk { .. })
                    }) {
                        let (releases, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut interact)
                            .into_iter()
                            .partition(|queued| {
                                matches!(queued.req, script::shim::InteractReq::AbortWalk { .. })
                            });
                        interact = remaining;
                        wrote |= dispatch_script_interact_cached_scoped(
                            driver,
                            &interact_context,
                            releases
                                .into_iter()
                                .map(|queued| queued.req)
                                .collect::<Vec<_>>(),
                        );
                    }
                    let mut refused_batch = None;
                    while let Some(action) = slot.take_native_action() {
                        let batch = action.batch;
                        let authority = action.authority();
                        let bank_deposit = action.bank_deposit;
                        if !authority.live() {
                            continue;
                        }
                        if batch != 0 {
                            exclusive = true;
                        }
                        if batch != 0 && refused_batch == Some(batch) {
                            slot.complete_native_interaction(
                                &authority,
                                script::native::InteractionReceipt {
                                    request_id: authority.request_id().get(),
                                    evidence: api::quest_progress::EvidenceStamp {
                                        run: authority.run(),
                                        tick,
                                        sequence: tick,
                                    },
                                    accepted: false,
                                    chat_since: snapshot
                                        .chat_lines()
                                        .first()
                                        .map_or(0, |line| line.sequence),
                                },
                            );
                            continue;
                        }
                        let current = navs.lock().unwrap().get(name).is_none_or(|bot| {
                            bot.walking_decision_is_current(action.observed_walk_outcome_seq)
                        });
                        if !current
                            && matches!(
                                &action.effect,
                                script::native::HostEffect::Walk(_)
                                    | script::native::HostEffect::Interaction(
                                        script::shim::InteractReq::WalkTo { .. }
                                    )
                            )
                        {
                            continue;
                        }
                        match action.effect {
                            script::native::HostEffect::ServerOrdered(envelope) => {
                                #[cfg(test)]
                                run_ordered_dispatch_hook(&authority);
                                let selected = slot.compiled_game_data();
                                let dispatch = super::dispatch_native_server_ordered(
                                    driver,
                                    super::OrderedDispatchContext {
                                        snapshot,
                                        selected: selected.as_deref(),
                                        tick,
                                        input: slot_input,
                                    },
                                    &authority,
                                    &envelope,
                                );
                                wrote |= dispatch.wire_events != 0;
                                slot.complete_native_server_ordered(
                                    &authority,
                                    script::native::ServerOrderedCompletion {
                                        evidence: api::quest_progress::EvidenceStamp {
                                            run: authority.run(),
                                            tick,
                                            sequence: tick,
                                        },
                                        dispatch,
                                    },
                                );
                            }
                            script::native::HostEffect::Interaction(request) => {
                                #[cfg(test)]
                                let proof_capture = crate::combat_proof::capture_enabled(name);
                                #[cfg(test)]
                                let packet_trace_enabled = bank_deposit.is_some()
                                    || api::hostlog::enabled(Category::InteractTrace)
                                    || proof_capture;
                                #[cfg(not(test))]
                                let packet_trace_enabled = bank_deposit.is_some()
                                    || api::hostlog::enabled(Category::InteractTrace);
                                let packet_trace = packet_trace_enabled
                                    .then(|| driver.packet_checkpoint())
                                    .flatten();
                                #[cfg(test)]
                                let proof_request = proof_capture.then(|| request.clone());
                                #[cfg(test)]
                                let mut proof_wire_opcodes = proof_capture.then(Vec::new);
                                #[cfg(test)]
                                let mut proof_decoded = false;
                                #[cfg(all(windows, test, feature = "journal-paint-proof"))]
                                let proof_close =
                                    matches!(&request, script::shim::InteractReq::CloseModal);
                                let bank_dispatch = bank_deposit.as_ref().map(|trace| {
                                    let matches = matches!(
                                        &request,
                                        script::shim::InteractReq::InvButton {
                                            id,
                                            slot,
                                            component,
                                            operation,
                                            bank_generation,
                                        } if *id == trace.id
                                            && *slot == trace.slot
                                            && *component == trace.component
                                            && *operation == trace.operation
                                            && *bank_generation == trace.generation
                                    );
                                    if matches {
                                        #[cfg(test)]
                                        if super::gatherer_recovery_live::refuse_deposit_once(
                                            name, &authority, trace, tick,
                                        ) {
                                            return BankDepositDispatch {
                                                accepted: false,
                                                refusal: Some(
                                                    "bank deposit dispatch was refused".to_owned(),
                                                ),
                                            };
                                        }
                                        dispatch_native_bank_deposit(driver, snapshot, trace)
                                    } else {
                                        BankDepositDispatch {
                                            accepted: false,
                                            refusal: Some("metadata-mismatch".to_owned()),
                                        }
                                    }
                                });
                                let accepted = if let Some(dispatch) = bank_dispatch.as_ref() {
                                    dispatch.accepted
                                } else if matches!(
                                    request,
                                    script::shim::InteractReq::WithdrawX { .. }
                                ) {
                                    if pending_withdraw_x_active {
                                        false
                                    } else if let Some(pending) =
                                        super::bank::dispatch_observed_withdraw_x(
                                            driver, snapshot, obj_names, inv, &request,
                                        )
                                    {
                                        slot.set_pending_withdraw_x(Some(pending));
                                        pending_withdraw_x_active = true;
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    dispatch_script_interact_cached_scoped(
                                        driver,
                                        &interact_context,
                                        [request],
                                    )
                                };
                                let mut emitted_packets = 0;
                                let mut opcode = None;
                                if let Some(checkpoint) = packet_trace {
                                    let decoded = driver.trace_packets(*checkpoint, &mut |packet_opcode| {
                                        emitted_packets += 1;
                                        opcode.get_or_insert(packet_opcode);
                                        #[cfg(test)]
                                        if let Some(opcodes) = proof_wire_opcodes.as_mut() {
                                            opcodes.push(packet_opcode);
                                        }
                                        host_log!(
                                            Category::InteractTrace,
                                            Level::Debug,
                                            "native-packet account={name} run={:?} tick={tick} request={} opcode={packet_opcode}",
                                            authority.run(),
                                            authority.request_id(),
                                        );
                                    });
                                    #[cfg(test)]
                                    {
                                        proof_decoded = decoded;
                                    }
                                    host_log!(
                                        Category::InteractTrace,
                                        Level::Debug,
                                        "native-packets account={name} run={:?} tick={tick} request={} count={emitted_packets} decoded={decoded} accepted={accepted}",
                                        authority.run(),
                                        authority.request_id(),
                                    );
                                }
                                if let (Some(trace), Some(dispatch)) =
                                    (bank_deposit.as_ref(), bank_dispatch.as_ref())
                                {
                                    host_log!(
                                        Category::BankOp,
                                        Level::Debug,
                                        "bank-deposit-dispatch run={:?} bank_generation={} transfer={} request={} click_tick={} tick={} id={} slot={} component={} option={} operation={} accepted={} refusal={:?} emitted_packets={} opcode={:?}",
                                        authority.run(),
                                        trace.generation,
                                        trace.transfer,
                                        authority.request_id(),
                                        trace.click_tick,
                                        tick,
                                        trace.id,
                                        trace.slot,
                                        trace.component,
                                        trace.option,
                                        trace.operation,
                                        dispatch.accepted,
                                        dispatch.refusal.as_deref(),
                                        emitted_packets,
                                        opcode,
                                    );
                                }
                                #[cfg(test)]
                                if let Some(request) = proof_request.as_ref() {
                                    crate::combat_proof::record_interaction(
                                        name,
                                        tick,
                                        (authority.request_id().get(), batch),
                                        &authority,
                                        request,
                                        (
                                            accepted,
                                            proof_decoded,
                                            proof_wire_opcodes.as_deref().unwrap_or_default(),
                                        ),
                                        snapshot,
                                    );
                                }
                                wrote |= accepted;
                                #[cfg(all(windows, test, feature = "journal-paint-proof"))]
                                if proof_close {
                                    eprintln!(
                                        "journal-proof-close origin=host-native effect=CloseModal accepted={accepted} run={:?} request={} tick={tick} snapshot_root={:?} unix_ns={}",
                                        authority.run(),
                                        authority.request_id().get(),
                                        snapshot.modals().main,
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .expect("journal proof clock")
                                            .as_nanos(),
                                    );
                                }
                                slot.complete_native_interaction(
                                    &authority,
                                    script::native::InteractionReceipt {
                                        request_id: authority.request_id().get(),
                                        evidence: api::quest_progress::EvidenceStamp {
                                            run: authority.run(),
                                            tick,
                                            sequence: tick,
                                        },
                                        accepted,
                                        chat_since: snapshot
                                            .chat_lines()
                                            .first()
                                            .map_or(0, |line| line.sequence),
                                    },
                                );
                                if batch != 0 && !accepted {
                                    refused_batch = Some(batch);
                                }
                            }
                            script::native::HostEffect::Walk(request) => {
                                #[cfg(test)]
                                crate::combat_proof::record_walk(
                                    name,
                                    tick,
                                    authority.run(),
                                    authority.request_id().get(),
                                    authority.action_id().get(),
                                    &request,
                                    snapshot,
                                );
                                let arm = super::ScriptWalkArm {
                                    here,
                                    world: world.clone(),
                                    navs: Arc::clone(navs),
                                    name: name.to_owned(),
                                    state: state.clone(),
                                    bank: super::slot_bank_memory::planner_rows(bank_memory),
                                };
                                // A refusal reaches the owner as a typed
                                // `Refused` receipt on the next observation.
                                arm.queue_native_route(
                                    snapshot,
                                    request,
                                    authority,
                                    walk_permissions,
                                );
                            }
                            script::native::HostEffect::AssessWalk(request) => {
                                let arm = super::ScriptWalkArm {
                                    here,
                                    world: world.clone(),
                                    navs: Arc::clone(navs),
                                    name: name.to_owned(),
                                    state: state.clone(),
                                    bank: super::slot_bank_memory::planner_rows(bank_memory),
                                };
                                arm.queue_native_assess(
                                    snapshot,
                                    request,
                                    authority.clone(),
                                    api::quest_progress::EvidenceStamp {
                                        run: authority.run(),
                                        tick,
                                        sequence: tick,
                                    },
                                    walk_permissions,
                                );
                            }
                            script::native::HostEffect::BankPick(request) => {
                                let evidence = api::quest_progress::EvidenceStamp {
                                    run: authority.run(),
                                    tick,
                                    sequence: tick,
                                };
                                let Some(scope) = compatibility_work_scope else {
                                    slot.complete_native_bank_pick(
                                        &authority,
                                        script::native_bank::BankPickReceipt {
                                            request_id: authority.request_id().get(),
                                            evidence,
                                            selected: super::bank::native_no_candidate(),
                                        },
                                    );
                                    continue;
                                };
                                super::bank::queue_native_bank_pick_scoped(
                                    navs,
                                    name,
                                    scope,
                                    world,
                                    state.as_ref(),
                                    super::bank::NativeBankPickInput {
                                        request,
                                        authority,
                                        evidence,
                                    },
                                    walk_permissions,
                                );
                            }
                        }
                    }
                    let mut dispatchable = Vec::with_capacity(interact.len());
                    let mut armed = None;
                    let mut armed_bank_op = None;
                    let mut rejected_withdraw_x = 0usize;
                    let mut rejected_bank_op = 0usize;
                    for queued in interact {
                        if queued.req.is_automatic_walk()
                            && navs.lock().unwrap().get(name).is_some_and(|bot| {
                                !bot.walking_decision_is_current(queued.observed_walk_outcome_seq)
                            })
                        {
                            continue;
                        }
                        let req = queued.req;
                        #[cfg(test)]
                        crate::combat_proof::record_shim_interactions(
                            name,
                            tick,
                            std::slice::from_ref(&req),
                            snapshot,
                        );
                        match req {
                            req @ (script::shim::InteractReq::Deposit { .. }
                            | script::shim::InteractReq::Withdraw { .. }) => {
                                if pending_bank_op_active || armed_bank_op.is_some() {
                                    rejected_bank_op += 1;
                                } else if let Some(pending) = dispatch_observed_bank_op(
                                    driver, snapshot, obj_names, inv, &req,
                                ) {
                                    armed_bank_op = Some(pending);
                                    pending_bank_op_active = true;
                                    wrote = true;
                                } else {
                                    rejected_bank_op += 1;
                                }
                            }
                            req @ script::shim::InteractReq::WithdrawX { .. } => {
                                let pending = (armed.is_none() && !pending_withdraw_x_active)
                                    .then(|| {
                                        super::bank::dispatch_observed_withdraw_x(
                                            driver, snapshot, obj_names, inv, &req,
                                        )
                                    })
                                    .flatten();
                                if let Some(pending) = pending {
                                    armed = Some(pending);
                                    pending_withdraw_x_active = true;
                                    wrote = true;
                                } else {
                                    rejected_withdraw_x += 1;
                                }
                            }
                            req => dispatchable.push(req),
                        }
                    }
                    wrote |= dispatch_script_interact_cached_scoped(
                        driver,
                        &interact_context,
                        dispatchable,
                    );
                    if (armed.is_some()
                        || armed_bank_op.is_some()
                        || rejected_withdraw_x != 0
                        || rejected_bank_op != 0)
                        && Some(slot.work_epoch()) == slot_work_epoch
                    {
                        for _ in 0..rejected_withdraw_x {
                            slot.complete_withdraw_x(false);
                        }
                        for _ in 0..rejected_bank_op {
                            slot.complete_bank_op(false);
                        }
                        if let Some(pending) = armed_bank_op {
                            if matches!(
                                slot.state(),
                                script::RunState::Running | script::RunState::Paused
                            ) {
                                slot.set_pending_bank_op(Some(pending));
                                if slot.state() == script::RunState::Paused {
                                    slot.freeze_pending_bank_op();
                                }
                            }
                        }
                        if let Some(pending) = armed {
                            if matches!(
                                slot.state(),
                                script::RunState::Running | script::RunState::Paused
                            ) {
                                slot.set_pending_withdraw_x(Some(pending));
                                if slot.state() == script::RunState::Paused {
                                    slot.freeze_pending_withdraw_x();
                                }
                            }
                        }
                    }
                } else if same_lifetime
                    && Some(slot.work_epoch()) != slot_work_epoch
                    && has_native_actions
                {
                    // The existing session-work revocation owns stale
                    // outbox cleanup. Do not leave an admitted batch to be
                    // dispatched against a later observation key.
                    slot.reset_session_work();
                } else if !same_lifetime && has_native_actions {
                    // A replacement slot must not inherit the old object's
                    // admitted batch rows; revoke through its lifecycle path.
                    if let Some(stale_slot) = observed_slot.as_ref() {
                        if let Ok(mut stale_slot) = stale_slot.lock() {
                            stale_slot.reset_session_work();
                        }
                    }
                } else if same_lifetime
                    && slot.state() == script::RunState::Paused
                    && Some(slot.work_epoch()) == slot_work_epoch
                {
                    slot.restore_host_interacts(interact);
                }
            }
        } else {
            let rejected_x = interact
                .iter()
                .filter(|queued| matches!(&queued.req, script::shim::InteractReq::WithdrawX { .. }))
                .count();
            let rejected_bank = interact
                .iter()
                .filter(|req| {
                    matches!(
                        &req.req,
                        script::shim::InteractReq::Deposit { .. }
                            | script::shim::InteractReq::Withdraw { .. }
                    )
                })
                .count();
            if rejected_x != 0 || rejected_bank != 0 {
                if let Some(slot) = script_slot(scripts, name) {
                    let same_lifetime = observed_slot
                        .as_ref()
                        .is_some_and(|observed| Arc::ptr_eq(observed, &slot));
                    let Ok(mut slot) = slot.lock() else {
                        return ScriptObservation {
                            wrote,
                            journal_paint_hidden,
                            exclusive,
                        };
                    };
                    if same_lifetime && Some(slot.work_epoch()) == slot_work_epoch {
                        for _ in 0..rejected_x {
                            slot.complete_withdraw_x(false);
                        }
                        for _ in 0..rejected_bank {
                            slot.complete_bank_op(false);
                        }
                    }
                }
            }
        }
    } else if !interact.is_empty() {
        let rejected_x = interact
            .iter()
            .filter(|req| matches!(req.req, script::shim::InteractReq::WithdrawX { .. }))
            .count();
        let rejected_bank = interact
            .iter()
            .filter(|req| {
                matches!(
                    &req.req,
                    script::shim::InteractReq::Deposit { .. }
                        | script::shim::InteractReq::Withdraw { .. }
                )
            })
            .count();
        if rejected_x != 0 || rejected_bank != 0 {
            if let Some(slot) = script_slot(scripts, name) {
                let same_lifetime = observed_slot
                    .as_ref()
                    .is_some_and(|observed| Arc::ptr_eq(observed, &slot));
                let Ok(mut slot) = slot.lock() else {
                    return ScriptObservation {
                        wrote,
                        journal_paint_hidden,
                        exclusive,
                    };
                };
                if same_lifetime && Some(slot.work_epoch()) == slot_work_epoch {
                    for _ in 0..rejected_x {
                        slot.complete_withdraw_x(false);
                    }
                    for _ in 0..rejected_bank {
                        slot.complete_bank_op(false);
                    }
                }
            }
        }
    }
    let cmds = {
        let mut all = cheats.lock().unwrap();
        all.get_mut(name).map(std::mem::take).unwrap_or_default()
    };
    for queued in cmds.into_iter().filter(|_| up) {
        let (cmd, observe_replies) = match queued.strip_prefix(crate::Play::INTERNAL_CHEAT_PREFIX) {
            Some(cmd) => (cmd.to_string(), false),
            None => (queued, true),
        };
        if api::interact::cheat(driver, &cmd) == client::CheatSend::Sent {
            #[cfg(test)]
            if let Some(snapshot) = snapshot {
                crate::combat_proof::record_other_request(
                    name,
                    Some(tick),
                    snapshot,
                    "cheat",
                    &cmd,
                );
            } else {
                crate::combat_proof::mark_invalid(name, "cheat emitted without a capture snapshot");
            }
            wrote = true;
            if observe_replies {
                if let (Some(replies), Some(snapshot)) = (debug_replies.as_deref_mut(), snapshot) {
                    replies.sent(name, cmd, snapshot);
                }
            }
        } else {
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Warn,
                slot = name,
                "debug ::{cmd}: refused by client cheat admission"
            );
        }
    }
    ScriptObservation {
        wrote,
        journal_paint_hidden,
        exclusive,
    }
}

/// Hand a native walk's terminal to its owner's ledger. Two sources: a
/// terminal no walk outcome carries (host refusal, or a Pause, displacement
/// or abort that ended the follow), and the published outcome of the route
/// the owner still holds. A revoked owner receives neither.
fn deliver_native_walk_end(slot: &mut script::SlotScript, bot: &mut NavBot, tick: u64) {
    for (owner, event) in bot.walk_guard_events.drain(..) {
        slot.notify_native_walk(&owner, event);
    }
    let receipt = |owner: &script::native::HostAuthority,
                   end,
                   blocked,
                   detail,
                   assessment,
                   refusal,
                   escape,
                   goal| script::native::WalkReceipt {
        goal,
        request_id: owner.request_id().get(),
        evidence: api::quest_progress::EvidenceStamp {
            run: owner.run(),
            tick,
            sequence: tick,
        },
        end,
        blocked,
        detail,
        assessment,
        refusal,
        escape,
    };
    if let Some((owner, end)) = bot.native_end.take() {
        let request_id = owner.request_id().get();
        let detail =
            if bot.walk_outcome_request_id == 0 || bot.walk_outcome_request_id == request_id {
                bot.walk_outcome_detail.take()
            } else {
                None
            };
        let risk = bot.native_end_risk.take();
        let assessment = risk.as_deref().and_then(|risk| risk.assessment.clone());
        let refusal = risk.as_deref().and_then(|risk| risk.refusal);
        let escape = risk.as_deref().and_then(|risk| risk.escape);
        let blocked = risk.as_deref().and_then(|risk| risk.blocked.clone());
        slot.complete_native_walk(
            &owner,
            receipt(
                &owner, end, blocked, detail, assessment, refusal, escape, None,
            ),
        );
        if bot.walk_outcome_request_id == request_id {
            bot.walk_outcome_detail = None;
        }
    }
    let Some(owner) = bot.native_walk.as_ref() else {
        return;
    };
    // Compatibility walks and native actions can reuse request ids. Only
    // the current route generation can complete the owner's walk.
    if !owner.live()
        || bot.walk_outcome_seq == 0
        || bot.native_receipt_seq == bot.walk_outcome_seq
        || bot.walk_outcome_request_id != owner.request_id().get()
        || bot.walk_outcome_generation != bot.route_generation
    {
        return;
    }
    let request_id = owner.request_id().get();
    let typed_failure = bot
        .native_walk_failure
        .as_ref()
        .filter(|(request, _)| *request == request_id)
        .map(|(_, end)| end.clone());
    let end = typed_failure.unwrap_or({
        if bot.walk_outcome_blocked {
            script::native::WalkEnd::Blocked
        } else if bot.walk_outcome_failed {
            script::native::WalkEnd::Failed
        } else {
            script::native::WalkEnd::RouteEnded
        }
    });
    let blocked = bot
        .native_walk_blocked
        .as_ref()
        .filter(|(request, _)| *request == request_id)
        .map(|(_, keys)| Arc::clone(keys));
    let detail = (bot.walk_outcome_request_id == request_id)
        .then(|| bot.walk_outcome_detail.clone())
        .flatten();
    let refusal = bot
        .risk_refusal
        .as_deref()
        .filter(|(request, _)| *request == request_id)
        .map(|(_, refusal)| *refusal);
    let assessment = bot.assessment.clone();
    let escape = bot
        .admission
        .as_deref()
        .and_then(|admission| admission.escape);
    // The existing correlated outcome already retains the chosen exact Area
    // goal. No additional per-bot destination state is needed. Other walk
    // modes keep their original target/radius settlement contract.
    let goal = (matches!(
        &end,
        script::native::WalkEnd::Arrived | script::native::WalkEnd::RouteEnded
    ) && bot.route_arrival == nav_planner::contract::ArrivalKind::Area
        && bot.route_loc_id.is_none()
        && bot.walk_outcome_radius == 0)
        .then_some(api::snapshot::WorldTile {
            x: bot.walk_outcome_x,
            z: bot.walk_outcome_z,
            level: bot.walk_outcome_level,
        });
    slot.complete_native_walk(
        owner,
        receipt(
            owner, end, blocked, detail, assessment, refusal, escape, goal,
        ),
    );
    bot.native_receipt_seq = bot.walk_outcome_seq;
    bot.native_walk_blocked = None;
    bot.walk_outcome_detail = None;
}

fn deliver_native_advice(slot: &mut script::SlotScript, bot: &mut NavBot) -> bool {
    let mut completed = false;
    if let Some((authority, receipt)) = bot.bank_pick.take_native_receipt() {
        slot.complete_native_bank_pick(&authority, receipt);
        completed = true;
    }
    if let Some(result) = bot.assess_result.take() {
        let (authority, receipt) = *result;
        slot.complete_native_assess_walk(&authority, receipt);
        completed = true;
    }
    completed
}

/// Compare only completion identities against the snapshot already retained
/// by the slot. Route progress does not allocate a posted route on clean frames.
fn host_completion_changed(
    slot: &script::SlotScript,
    bot: &NavBot,
    bank_selection: script::native::snapshot_source::BankSelectionInput,
) -> bool {
    slot.posted_snapshot_facts().is_some_and(|last| {
        let inspect = &bot.inspect;
        let route = last.route_inspect;
        last.bank_selection != bank_selection
            || last.walk_outcome_seq != bot.walk_outcome_seq
            || route.latest
                != inspect
                    .latest
                    .as_ref()
                    .map_or((0, 0, 0), |end| (end.seq, end.generation, end.request_id))
            || route.prev
                != inspect
                    .prev
                    .as_ref()
                    .map_or((0, 0, 0), |end| (end.seq, end.generation, end.request_id))
            || route.replaced_id != inspect.replaced_id
            || route.replaced_prev_id != inspect.replaced_prev_id
            || route.refused != inspect.refused
    })
}

#[cfg(feature = "js")]
pub(crate) fn deliver_channel_events(
    scripts: &ScriptWall,
    deliveries: Vec<super::script_channels::Delivery>,
) {
    let mut batches: HashMap<(String, u64), Vec<script::interact::InteractReq>> = HashMap::new();
    for delivery in deliveries {
        batches
            .entry((delivery.account, delivery.generation))
            .or_default()
            .push(delivery.event);
    }
    for ((account, generation), events) in batches {
        let Some(slot) = script_slot(scripts, &account) else {
            continue;
        };
        let Ok(mut slot) = slot.lock() else {
            continue;
        };
        if slot.runtime_generation() != generation
            || !slot.surface_active()
            || !matches!(
                slot.state(),
                script::RunState::Running | script::RunState::Paused
            )
        {
            continue;
        }
        let _ = slot.post_channel_events(&events);
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn script_observe(
    driver: &mut dyn Driver,
    name: &str,
    up: bool,
    tick_edge: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    inv: Option<&[(i32, i32)]>,
    state: Option<WorldState>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    ours: bool,
) -> bool {
    script_observe_with_npc_boxes(
        driver, name, up, tick_edge, tick, here, inv, state, snapshot, None, obj_names, scripts,
        cheats, navs, world, hold, ours, None, None,
    )
}

pub(crate) fn take_script_interacts<T: std::borrow::Borrow<script::shim::InteractReq>>(
    reqs: Vec<T>,
    slot_input: Option<&SlotInput>,
) -> Vec<T> {
    let mut out = Vec::with_capacity(reqs.len());
    for req in reqs {
        match req.borrow() {
            script::shim::InteractReq::Mouse {
                down,
                x,
                y,
                button,
                identity,
            } => {
                if let Some(inp) = slot_input {
                    inp.enqueue_script_mouse_at(*identity, *down, *x, *y, *button);
                }
            }
            script::shim::InteractReq::RunPolicyOverride { .. } => {
                // Host callers split these before dispatch. A raw/direct caller
                // cannot turn host policy into a game interact.
            }
            _ => out.push(req),
        }
    }
    out
}
/// Whether this observe pass needs a [`WorldState`] from the slot snapshot.
/// Built for a Starting or Running script (walk arm) or an armed nav bot
/// (route / BankBudget session — interact dispatch may walk with gating
/// facts). Starting is included because compiled cards can emit their first
/// walk on that frame, before the outer pump's Running flag flips.
pub(crate) fn nav_world_state_for_observe(
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    script_running: bool,
    nav_armed: bool,
    map_members: bool,
) -> Option<WorldState> {
    if here.is_some() && (script_running || nav_armed) {
        Some(WorldState::from_snapshot(snapshot).with_map_members(map_members))
    } else {
        None
    }
}
/// Borrow the current session's inventory only on running tick edges.
pub(crate) fn observe_script_inv(
    running: bool,
    tick_edge: bool,
    snapshot: &GameSnapshot,
) -> Option<&[(i32, i32)]> {
    (running && tick_edge).then(|| snapshot.inv())
}

#[cfg(any(feature = "js", test))]
/// Project only the client's bounded active-NPC list. A ready scene with no
/// projectable NPCs is an available empty update; a non-ready scene is
/// unavailable so isolates clear any prior geometry.
pub(crate) fn projected_npc_boxes(
    client: &Client,
) -> Option<Vec<script::native::snapshot_source::NpcBoxInput>> {
    if !client.ingame || client.scene_state != 2 {
        return None;
    }
    Some(
        client
            .npc_ids
            .iter()
            .take(usize::try_from(client.npc_count).unwrap_or(0))
            .filter_map(|&index| {
                let slot = usize::try_from(index).ok()?;
                client::render::npc_overlay_box(client, slot)
                    .map(|points| script::native::snapshot_source::NpcBoxInput { index, points })
            })
            .collect(),
    )
}

/// Evaluate native NPC projection only for a frame that can post an isolate
/// snapshot. Held running isolates still satisfy this gate because they post
/// the snapshot before dispatching their paint-only tick; idle, paused,
/// compiled-only, and non-tick frames do not consume one.
#[cfg(any(feature = "js", test))]
pub(crate) fn project_npc_boxes_for_isolate_snapshot<F>(
    scripts: &ScriptWall,
    name: &str,
    tick_edge: bool,
    project: F,
) -> Option<Vec<script::native::snapshot_source::NpcBoxInput>>
where
    F: FnOnce() -> Option<Vec<script::native::snapshot_source::NpcBoxInput>>,
{
    if !tick_edge {
        return None;
    }
    let consumes_snapshot = script_slot(scripts, name).is_some_and(|slot| {
        slot.lock()
            .ok()
            .is_some_and(|slot| slot.state() == script::RunState::Running && slot.surface_active())
    });
    if consumes_snapshot {
        project()
    } else {
        None
    }
}

#[cfg(test)]
mod watchdog_restart_tests {
    #[test]
    fn restart_line_includes_pending_event() {
        assert_eq!(
            super::watchdog_restart_line(Some("waiting for world")),
            "restarted the script (waiting for world)"
        );
        assert_eq!(super::watchdog_restart_line(None), "restarted the script");
        assert_eq!(
            super::watchdog_restart_line(Some("")),
            "restarted the script"
        );
        assert_eq!(
            super::watchdog_restart_line(Some("started")),
            "restarted the script"
        );
    }

    #[test]
    fn watchdog_action_trace_is_debug_only_and_none_is_silent() {
        const CHILD: &str = "GRINCH_WATCHDOG_ACTION_LOG_TEST";
        if std::env::var(CHILD).as_deref() != Ok("1") {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "script_runtime::script_observe::watchdog_restart_tests::watchdog_action_trace_is_debug_only_and_none_is_silent",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env_remove("BOT_DEBUG")
                .output()
                .expect("run isolated diagnostic sink test");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed; 0 failed;"),
                "diagnostic child failed: {stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        struct Capture(std::sync::Mutex<Vec<String>>);
        impl api::hostlog::Sink for Capture {
            fn record(&self, record: &api::hostlog::Record<'_>) {
                assert_eq!(record.source, api::hostlog::Source::Watchdog);
                assert_eq!(record.level, api::hostlog::Level::Debug);
                self.0.lock().unwrap().push(record.message.to_owned());
            }
        }
        static CAPTURE: Capture = Capture(std::sync::Mutex::new(Vec::new()));
        assert!(api::hostlog::install_sink(&CAPTURE));
        let slot = script::SlotScript::new();
        let action = script::WatchdogAction::ArmWalk {
            x: 1,
            z: 2,
            level: 0,
        };
        let interaction_formats = std::cell::Cell::new(0);
        let interaction_traces = || {
            api::host_log!(
                api::hostlog::Category::InteractTrace,
                api::hostlog::Level::Debug,
                "native-packet account=grinch-trace run={:?} tick=7 request={} opcode=110",
                slot.native_run(),
                {
                    interaction_formats.set(interaction_formats.get() + 1);
                    1
                },
            );
            api::host_log!(
                api::hostlog::Category::InteractTrace,
                api::hostlog::Level::Debug,
                "native-packets account=grinch-trace run={:?} tick=7 request={} count=1 decoded=true accepted=true",
                slot.native_run(),
                {
                    interaction_formats.set(interaction_formats.get() + 1);
                    1
                },
            );
        };

        api::hostlog::set_debug(true);
        let none = allocation_counter::measure(|| {
            super::trace_watchdog_action(
                &slot,
                "grinch-watchdog-trace",
                7,
                script::WatchdogAction::None,
            );
        });
        assert_eq!(none.count_total, 0, "{none:?}");
        assert!(CAPTURE.0.lock().unwrap().is_empty());

        api::hostlog::set_debug(false);
        assert!(!api::hostlog::debug_enabled());
        let disabled = allocation_counter::measure(|| {
            super::trace_watchdog_action(&slot, "grinch-watchdog-trace", 7, action);
            interaction_traces();
        });
        assert_eq!(disabled.count_total, 0, "{disabled:?}");
        assert!(CAPTURE.0.lock().unwrap().is_empty());
        assert_eq!(interaction_formats.get(), 0);

        api::hostlog::set_debug(true);
        interaction_traces();
        assert_eq!(interaction_formats.get(), 2);
        assert!(CAPTURE.0.lock().unwrap().is_empty());
        super::trace_watchdog_action(&slot, "grinch-watchdog-trace", 7, action);
        assert_eq!(
            *CAPTURE.0.lock().unwrap(),
            ["watchdog-action run=None tick=7 action=ArmWalk { x: 1, z: 2, level: 0 }"]
        );
    }
}

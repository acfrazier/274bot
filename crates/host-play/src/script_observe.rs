use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use api::host_log;
use api::hostlog::{Category, Level};
use api::interact::Driver;
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::config::Cache;
use host::{ScriptRunPolicy, SlotInput};
use nav::world::NavWorld;
use nav::WorldState;
use script::{ScriptCtx, SlotScript};

use super::{
    abort_script_walk, apply_watchdog_nav_action, dispatch_observed_bank_op,
    dispatch_script_interact_cached, fill_withdraw_action, pack_cached_reach, recovery_walk_idle,
    reset_script_nav, resumed_walk, route_inspect, script_slot, take_carried_walk,
    with_script_snapshot_input_shorts, NavBot, PostedWalkOutcome, ScriptWall,
};
use crate::debug_enabled;
#[cfg(feature = "memory-profile")]
use crate::memory_diagnostics;
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

/// Post the slot's snapshot. Only a post the isolate accepted counts as the
/// isolate having seen the walk outcome it carries (`walk_seq`), so only
/// then is the live refusal guard released. A refused post leaves the guard
/// in place, and the isolate refuses the paired tick.
pub(crate) fn post_script_snapshot(
    slot: &mut SlotScript,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    walk_seq: u64,
    bytes: Vec<u8>,
) -> bool {
    let accepted = slot.post_snapshot(bytes);
    if accepted {
        if let Some(bot) = navs.lock().unwrap().get_mut(name) {
            bot.mark_walk_outcome_posted(walk_seq);
        }
    }
    accepted
}

/// Restart a script the watchdog gave up on and log the outcome.
fn watchdog_restart(slot: &mut SlotScript, name: &str, now: Instant) {
    match slot.restart_from_identity(now) {
        Ok(()) => host_log!(
            Category::Watchdog,
            Level::Warn,
            slot = name,
            "restarted the script"
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
/// `hold` is the guardian's
/// random-event freeze: while true the tick is not dispatched (follow is
/// frozen by the pump too), so a script cannot walk through an in-flight
/// dialog or a trapped maze/mime/box — the snapshot blob still posts on
/// the held tick edge, so EventSignal reads the freeze the script is
/// frozen by. `ours` is the guardian's published
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
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
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
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
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
        npc_boxes,
        obj_names,
        scripts,
        cheats,
        navs,
        world,
        hold,
        ours,
        canlight,
        slot_input,
        cache,
        obj_names_arc,
        None,
        super::script_channels::BrokerWorld::Unavailable,
        run_policy,
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
    Vec<script::load::QueuedInteract>,
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
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
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
    channels: Option<&super::script_channels::SlotChannels>,
    channel_world: super::script_channels::BrokerWorld,
    run_policy: Option<&mut ScriptRunPolicy>,
    mut debug_replies: Option<&mut super::debug_replies::DebugReplies>,
) -> ScriptObservation {
    if let Some(inp) = slot_input {
        inp.set_host_consume_allowed(up && !hold);
    }
    let mut wrote = false;
    let mut journal_wall_now = None;
    let mut journal_paint_hidden = false;
    let mut exclusive = false;
    let mut interact = Vec::new();
    let mut pending_withdraw_x_active = false;
    let mut pending_bank_op_active = false;
    let observed_slot = script_slot(scripts, name);
    let mut slot_work_epoch = None;
    let mut has_native_actions = false;
    let mut channel_generation = 0;
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
        ) && slot.load_active()
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
                reset_script_nav(navs, name, Some(generation));
            } else if navs
                .lock()
                .unwrap()
                .get(name)
                .is_some_and(NavBot::script_walk_armed)
            {
                // Legacy self-stops without a terminal receipt can only own a walk.
                abort_script_walk(navs, name);
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
            let bank_selection = bot.bank_pick.poll(std::time::Instant::now());
            host_results_dirty |= host_completion_changed(&slot, bot, bank_selection);
            deliver_native_walk_end(&mut slot, bot, tick);
            host_results_dirty |= deliver_native_advice(&mut slot, bot);
            host_results_dirty |= slot.observe_walk_outcome_seq(bot.walk_outcome_seq);
        }
        slot_work_epoch = Some(slot.work_epoch());
        channel_generation = slot.runtime_generation();
        // Region loads briefly clear `up`; the isolate and its browser-style
        // channel survive them. Session boundaries suspend the broker in the
        // slot loop before this observation runs.
        channel_active = slot.load_active()
            && matches!(
                slot.state(),
                script::RunState::Running | script::RunState::Paused
            );
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
                    slot.complete_current_withdrawal(false);
                } else {
                    match pending.phase {
                        script::slot::PendingWithdrawXPhase::Dialog => {
                            if pending.expired() {
                                slot.complete_current_withdrawal(false);
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
                                    slot.complete_current_withdrawal(false);
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
                            let load_settled = pending.fill.is_some_and(|fill| {
                                let (used, count) = inv.map_or_else(
                                    || {
                                        snapshot.map_or((0, 0), |snap| {
                                            let inventory = snap.inventory();
                                            (
                                                inventory.len(),
                                                inventory
                                                    .iter()
                                                    .filter(|item| item.def.id == fill.bank_item_id)
                                                    .map(|item| item.count)
                                                    .sum::<i32>(),
                                            )
                                        })
                                    },
                                    |rows| {
                                        (
                                            rows.len(),
                                            rows.iter()
                                                .filter(|(id, _)| *id == fill.bank_item_id)
                                                .map(|(_, count)| *count)
                                                .sum::<i32>(),
                                        )
                                    },
                                );
                                let stock_decreased = snapshot
                                    .and_then(|snap| {
                                        snap.bank()
                                            .iter()
                                            .find(|item| item.def.id == fill.bank_item_id)
                                    })
                                    .is_some_and(|item| item.count < fill.before_stock);
                                used > fill.before_used
                                    || count > fill.before_count
                                    || stock_decreased
                            });
                            if load_settled
                                || (pending.fill.is_none()
                                    && (current >= pending.target
                                        || (current > pending.before && full)))
                            {
                                slot.complete_current_withdrawal(true);
                            } else if pending.expired() {
                                slot.complete_current_withdrawal(false);
                            } else {
                                pending_withdraw_x_active = true;
                            }
                        }
                    }
                }
            }
        }
        host_results_dirty |= slot.last_snapshot_fingerprint().is_some_and(|last| {
            last.withdraw_x_result_seq != slot.withdraw_x_result().0
                || last.withdraw_load_result_seq != slot.withdraw_load_result().0
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
            let world_id = world.as_ref().map(|w| Arc::as_ptr(w) as usize);
            let force_banks = world_id.is_some_and(|id| slot.last_world_id() != Some(id));
            // Guardian `hold` freezes clocks/nav and cancels owned recovery.
            // Recovery hold is host-owned and must still reach the isolate so
            // loop/pump freeze, without being treated as that external freeze.
            let recovery_hold = slot.watchdog().holds_script_actions();
            let isolate_hold = hold || recovery_hold || prayer_cleanup_hold;
            if slot.load_active() {
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
                                b.bank_pick.poll(std::time::Instant::now()),
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
                            script::isolate_fb::WalkCancelReason::None,
                            0,
                            Vec::new(),
                            route_inspect::PostedInspect::default(),
                            script::isolate_fb::BankSelectionInput::default(),
                        ),
                    }
                };
                let (withdraw_x_result_seq, withdraw_x_result) = slot.withdraw_x_result();
                let (withdraw_load_result_seq, withdraw_load_result) = slot.withdraw_load_result();
                let (bank_op_result_seq, bank_op_result) = slot.bank_op_result();
                // The published failure's shopping list, as the isolate reads
                // it: the diagnosis' own ids and counts, with the host obj
                // table's display name beside them when it has one. Always
                // supplied — an empty list is the observed "no named short",
                // so a clear is never omitted.
                let carry_rows: Vec<script::isolate_fb::CarryInput<'_>> = walk_missing_carry
                    .iter()
                    .map(|row| script::isolate_fb::CarryInput {
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

                let bytes = with_script_snapshot_input_shorts(
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
                    withdraw_load_result_seq,
                    withdraw_load_result,
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
                    Some(packed.view.as_ref()),
                    packed.flood.as_deref(),
                    packed.stamp,
                    |input, mut native| {
                        native.bank_selection = bank_selection;
                        if wake_waits {
                            slot.encode_snapshot_wake_with_native(
                                input,
                                native,
                                force_banks,
                                inv.is_none() && input.inv_size == 0,
                            )
                        } else {
                            slot.encode_snapshot_delta_with_native(input, native, force_banks)
                        }
                    },
                );
                post_script_snapshot(&mut slot, navs, name, walk_seq, bytes);
                slot.store_last_world_id(world_id);
            }
            if isolate_hold {
                // Isolate: tick for onPaint only (hold gate inside V8 via
                // snapshot.hold). Recovery hold is distinct from guardian
                // hold: nav follow of the owned recovery route continues,
                // but script walk arms are not provided. Compiled: skip on
                // guardian hold.
                if slot.load_active() {
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
                        selected: selected.as_deref(),
                        reach: packed.as_ref().map(|packed| packed.view.as_ref()),
                        reach_flood: packed.as_ref().and_then(|packed| packed.flood.as_deref()),
                        bank_memory: bank_memory.as_deref(),
                        collision: world.as_ref().map(|world| &world.collision),
                        hold: hold || ours,
                        world_members,
                        ..Default::default()
                    },
                };
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
            bot.native_permissions = slot.native_walk_permissions();
            bot.api_gather_permissions = slot.api_gather_walk_permissions();
            bot.compat_v1 = slot.api_family() == Some(script::ApiFamily::V1);
        }
        emit_script_debug_logs(&mut slot, name);
        // Fold forwarded shim requests on running frames. Pause leaves the
        // isolate queue untouched so Resume can dispatch it; guardian hold
        // retains the existing drain/drop policy below.
        if slot.load_active() || slot.native_run().is_some() {
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
                            state.clone(),
                            bank_memory,
                            name,
                        ),
                    }
                }
            }
            let running = slot.state() == script::RunState::Running;
            let action = slot.feed_watchdog(now, here, xp, frozen, running, &lifecycle);
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
                other => apply_watchdog_nav_action(
                    other,
                    driver,
                    snapshot,
                    here,
                    navs,
                    world,
                    state.clone(),
                    bank_memory,
                    name,
                ),
            }
            slot.sync_native_input_gate();
        }
        if slot.load_active() {
            if slot.state() == script::RunState::Running {
                // Admit controls before taking any carried walks. Both ownership
                // edges (and a run/Stop pair within this batch) suppress replay.
                let (update, reqs, owned) = drain_observed_host_interacts(&mut slot);
                if slot.watchdog().holds_script_actions() || owned {
                    if owned {
                        // Admission already discarded held script walks. The
                        // host's carried walk is also discarded, but is not a
                        // script row and must not inflate its drop count.
                        let _ = take_carried_walk(navs, name, slot.runtime_generation());
                        interact.extend(reqs);
                    }
                    if let Some(update) = update {
                        run_policy_update = Some(update);
                    }
                } else {
                    let mut queued = Vec::new();
                    let mut carried = None;
                    if up && !hold && here.is_some() && snapshot.is_some() {
                        carried = take_carried_walk(navs, name, slot.runtime_generation());
                        queued.extend(slot.take_held_host_walks());
                    }
                    if let Some(update) = update {
                        run_policy_update = Some(update);
                    }
                    queued.extend(take_script_interacts(reqs, slot_input));
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
                        script::load::QueuedInteract {
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
        } else if slot.state() == script::RunState::Running
            && !slot.watchdog().holds_script_actions()
            && !prayer_cleanup_hold
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
    if let Some(channels) = channels {
        // Channel requests are the broker's, never the game's. A frame with
        // none allocates nothing, and an untracked slot takes no lock.
        let is_channel = |queued: &script::load::QueuedInteract| {
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
                    && slot.state() == script::RunState::Running
                    && Some(slot.work_epoch()) == slot_work_epoch
                {
                    {
                        let mut navs = navs.lock().unwrap();
                        let bot = navs.entry(name.to_owned()).or_default();
                        bot.native_permissions = slot.native_walk_permissions();
                        bot.api_gather_permissions = slot.api_gather_walk_permissions();
                        bot.compat_v1 = slot.api_family() == Some(script::ApiFamily::V1);
                    }
                    let mut refused_batch = None;
                    while let Some(action) = slot.take_native_action() {
                        let batch = action.batch;
                        let authority = action.authority();
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
                            script::native::HostEffect::Interaction(request) => {
                                #[cfg(test)]
                                let proof_capture = crate::combat_proof::capture_enabled(name);
                                #[cfg(test)]
                                let packet_trace_enabled =
                                    api::hostlog::enabled(Category::InteractTrace) || proof_capture;
                                #[cfg(not(test))]
                                let packet_trace_enabled =
                                    api::hostlog::enabled(Category::InteractTrace);
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
                                let accepted = if matches!(
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
                                    dispatch_script_interact_cached(
                                        driver,
                                        snapshot,
                                        obj_names,
                                        here,
                                        navs,
                                        world,
                                        state.clone(),
                                        bank_memory,
                                        name,
                                        [request],
                                        cache.clone(),
                                        obj_names_arc.clone(),
                                    )
                                };
                                if let Some(checkpoint) = packet_trace {
                                    let mut count = 0;
                                    let decoded = driver.trace_packets(*checkpoint, &mut |opcode| {
                                        count += 1;
                                        #[cfg(test)]
                                        if let Some(opcodes) = proof_wire_opcodes.as_mut() {
                                            opcodes.push(opcode);
                                        }
                                        host_log!(
                                            Category::InteractTrace,
                                            Level::Debug,
                                            "native-packet account={name} run={:?} tick={tick} request={} opcode={opcode}",
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
                                        "native-packets account={name} run={:?} tick={tick} request={} count={count} decoded={decoded} accepted={accepted}",
                                        authority.run(),
                                        authority.request_id(),
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
                                arm.queue_native_route(snapshot, request, authority);
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
                                );
                            }
                            script::native::HostEffect::BankPick(request) => {
                                super::bank::queue_native_bank_pick(
                                    navs,
                                    name,
                                    world,
                                    state.clone(),
                                    request,
                                    authority.clone(),
                                    api::quest_progress::EvidenceStamp {
                                        run: authority.run(),
                                        tick,
                                        sequence: tick,
                                    },
                                );
                            }
                        }
                    }
                    let mut dispatchable = Vec::with_capacity(interact.len());
                    let mut armed = None;
                    let mut armed_bank_op = None;
                    let mut rejected_withdraw_x = 0usize;
                    let mut rejected_withdraw_load = 0usize;
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
                            script::shim::InteractReq::WithdrawLoad {
                                name: item_name,
                                bank_generation,
                            } if armed.is_none()
                                && !pending_withdraw_x_active
                                && snapshot.bank_component_id() >= 0
                                && snapshot.bank_loaded()
                                && !snapshot.count_dialog_open()
                                && snapshot.bank_session_generation() == bank_generation =>
                            {
                                let capacity = snapshot.inventory_size().max(0) as usize;
                                let used =
                                    inv.map_or_else(|| snapshot.inventory().len(), <[_]>::len);
                                let free = capacity.saturating_sub(used);
                                let mut accepted = false;
                                if free > 0 {
                                    let mut ix = api::interact::Interactions::new(snapshot, driver);
                                    let wanted = item_name.to_lowercase();
                                    if let Some(item) = snapshot.bank().iter().find(|item| {
                                        item.count > 0
                                            && obj_names
                                                .and_then(|names| names.name(item.def.id))
                                                .is_some_and(|name| {
                                                    name.eq_ignore_ascii_case(&wanted)
                                                })
                                    }) {
                                        let bank_item_id = item.def.id;
                                        let count = item.count.min(free as i32);
                                        if let Some((op, needs_dialog)) =
                                            fill_withdraw_action(&item.actions, count, item.count)
                                        {
                                            let sent = matches!(
                                                ix.interact(
                                                    api::interact::OpTarget::Item(item),
                                                    api::interact::ActionSpec::Operation(op),
                                                ),
                                                api::interact::SendResult::Sent { .. }
                                            );
                                            if sent {
                                                let before_count = snapshot
                                                    .inventory()
                                                    .iter()
                                                    .filter(|held| held.def.id == bank_item_id)
                                                    .map(|held| held.count)
                                                    .sum();
                                                let pending =
                                            script::slot::PendingWithdrawX::waiting_load_dialog(
                                                bank_item_id,
                                                count,
                                                used,
                                                before_count,
                                                item.count,
                                                bank_generation,
                                            );
                                                armed = Some(if needs_dialog {
                                                    pending
                                                } else {
                                                    pending.waiting_settlement()
                                                });
                                                wrote = true;
                                                pending_withdraw_x_active = true;
                                                accepted = true;
                                            }
                                        }
                                    }
                                }
                                if !accepted {
                                    rejected_withdraw_load += 1;
                                }
                            }
                            script::shim::InteractReq::WithdrawLoad { .. } => {
                                rejected_withdraw_load += 1;
                            }
                            req => dispatchable.push(req),
                        }
                    }
                    wrote |= dispatch_script_interact_cached(
                        driver,
                        snapshot,
                        obj_names,
                        here,
                        navs,
                        world,
                        state.clone(),
                        bank_memory,
                        name,
                        dispatchable,
                        cache.clone(),
                        obj_names_arc.clone(),
                    );
                    if (armed.is_some()
                        || armed_bank_op.is_some()
                        || rejected_withdraw_x != 0
                        || rejected_withdraw_load != 0
                        || rejected_bank_op != 0)
                        && Some(slot.work_epoch()) == slot_work_epoch
                    {
                        for _ in 0..rejected_withdraw_x {
                            slot.complete_withdraw_x(false);
                        }
                        for _ in 0..rejected_withdraw_load {
                            slot.complete_withdraw_load(false);
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
            let rejected_load = interact
                .iter()
                .filter(|queued| {
                    matches!(&queued.req, script::shim::InteractReq::WithdrawLoad { .. })
                })
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
            if rejected_x != 0 || rejected_load != 0 || rejected_bank != 0 {
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
                        for _ in 0..rejected_load {
                            slot.complete_withdraw_load(false);
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
        let rejected_load = interact
            .iter()
            .filter(|req| matches!(req.req, script::shim::InteractReq::WithdrawLoad { .. }))
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
        if rejected_x != 0 || rejected_load != 0 || rejected_bank != 0 {
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
                    for _ in 0..rejected_load {
                        slot.complete_withdraw_load(false);
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
                   escape| script::native::WalkReceipt {
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
            receipt(&owner, end, blocked, detail, assessment, refusal, escape),
        );
        if bot.walk_outcome_request_id == request_id {
            bot.walk_outcome_detail = None;
        }
    }
    let Some(owner) = bot.native_walk.as_ref() else {
        return;
    };
    if !owner.live()
        || bot.walk_outcome_seq == 0
        || bot.native_receipt_seq == bot.walk_outcome_seq
        || bot.walk_outcome_request_id != owner.request_id().get()
    {
        return;
    }
    let end = if bot.walk_outcome_blocked {
        script::native::WalkEnd::Blocked
    } else if bot.walk_outcome_failed {
        bot.native_walk_failure
            .as_ref()
            .filter(|(request, _)| *request == owner.request_id().get())
            .map_or(script::native::WalkEnd::Failed, |(_, end)| end.clone())
    } else {
        script::native::WalkEnd::RouteEnded
    };
    let request_id = owner.request_id().get();
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
    slot.complete_native_walk(
        owner,
        receipt(owner, end, blocked, detail, assessment, refusal, escape),
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
    bank_selection: script::isolate_fb::BankSelectionInput,
) -> bool {
    slot.last_snapshot_fingerprint().is_some_and(|last| {
        let inspect = &bot.inspect;
        let old = &last.route_inspect;
        last.bank_selection != bank_selection
            || last.walk_outcome_seq != bot.walk_outcome_seq
            || (old.latest.seq, old.latest.generation, old.latest.request_id)
                != inspect
                    .latest
                    .as_ref()
                    .map_or((0, 0, 0), |end| (end.seq, end.generation, end.request_id))
            || (old.prev.seq, old.prev.generation, old.prev.request_id)
                != inspect
                    .prev
                    .as_ref()
                    .map_or((0, 0, 0), |end| (end.seq, end.generation, end.request_id))
            || old.replaced_id != inspect.replaced_id
            || old.replaced_prev_id != inspect.replaced_prev_id
            || [old.refused_id, old.refused_id_2, old.refused_id_3] != inspect.refused
    })
}

pub(crate) fn deliver_channel_events(
    scripts: &ScriptWall,
    deliveries: Vec<super::script_channels::Delivery>,
) {
    let mut batches: HashMap<(String, u64), Vec<script::shim::InteractReq>> = HashMap::new();
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
            || !slot.load_active()
            || !matches!(
                slot.state(),
                script::RunState::Running | script::RunState::Paused
            )
        {
            continue;
        }
        let bytes = script::isolate_fb::encode_interact_batch(&events);
        let _ = slot.post_channel_events(bytes);
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
/// Built only for a Running script (walk arm) or an armed nav bot (route /
/// BankBudget session — interact dispatch may walk with gating facts).
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

/// Project only the client's bounded active-NPC list. A ready scene with no
/// projectable NPCs is an available empty update; a non-ready scene is
/// unavailable so isolates clear any prior geometry.
pub(crate) fn projected_npc_boxes(client: &Client) -> Option<Vec<script::isolate_fb::NpcBoxInput>> {
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
                    .map(|points| script::isolate_fb::NpcBoxInput { index, points })
            })
            .collect(),
    )
}

/// Evaluate native NPC projection only for a frame that can post an isolate
/// snapshot. Held running isolates still satisfy this gate because they post
/// the snapshot before dispatching their paint-only tick; idle, paused,
/// compiled-only, and non-tick frames do not consume one.
pub(crate) fn project_npc_boxes_for_isolate_snapshot<F>(
    scripts: &ScriptWall,
    name: &str,
    tick_edge: bool,
    project: F,
) -> Option<Vec<script::isolate_fb::NpcBoxInput>>
where
    F: FnOnce() -> Option<Vec<script::isolate_fb::NpcBoxInput>>,
{
    if !tick_edge {
        return None;
    }
    let consumes_snapshot = script_slot(scripts, name).is_some_and(|slot| {
        slot.lock()
            .ok()
            .is_some_and(|slot| slot.state() == script::RunState::Running && slot.load_active())
    });
    if consumes_snapshot {
        project()
    } else {
        None
    }
}

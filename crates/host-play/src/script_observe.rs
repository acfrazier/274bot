use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use api::interact::Driver;
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::config::Cache;
use host::SlotInput;
use nav::router::FindOptions;
use nav::world::NavWorld;
use nav::WorldState;
use script::{ScriptCtx, SlotScript};

use super::{
    abort_script_walk, action_slot, apply_watchdog_nav_action, dispatch_observed_bank_op,
    dispatch_script_interact_cached, fill_withdraw_action, pack_cached_reach, recovery_walk_idle,
    route_inspect, script_slot, with_script_snapshot_input_shorts, NavBot, PostedWalkOutcome,
    ScriptWalkArm, ScriptWall,
};
use crate::debug_enabled;
#[cfg(feature = "memory-profile")]
use crate::memory_diagnostics;
#[cfg(test)]
use std::sync::{Condvar, LazyLock};

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
        let mut entered = lock.lock().unwrap();
        while !*entered {
            entered = signal.wait(entered).unwrap();
        }
    }

    pub(crate) fn release(&self) {
        let (lock, signal) = &self.release;
        *lock.lock().unwrap() = true;
        signal.notify_all();
    }

    fn wait_release(&self) {
        let (lock, signal) = &self.release;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = signal.wait(released).unwrap();
        }
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
static DISPATCH_BARRIER: LazyLock<Mutex<Option<Arc<DispatchBarrier>>>> =
    LazyLock::new(|| Mutex::new(None));

#[cfg(test)]
fn dispatch_barrier_cell() -> &'static Mutex<Option<Arc<DispatchBarrier>>> {
    &DISPATCH_BARRIER
}

#[cfg(test)]
pub(crate) fn install_dispatch_barrier(barrier: Arc<DispatchBarrier>) {
    *dispatch_barrier_cell().lock().unwrap() = Some(barrier);
}

#[cfg(test)]
pub(crate) fn clear_dispatch_barrier() {
    *dispatch_barrier_cell().lock().unwrap() = None;
}

#[cfg(test)]
fn wait_dispatch_barrier() {
    let barrier = dispatch_barrier_cell().lock().unwrap().clone();
    if let Some(barrier) = barrier {
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
        eprintln!("[script {name}] {line}");
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
/// `snapshot` is the same per-tick [`GameSnapshot`] the ctx getters read
/// (`varp`, `stat_level`, `chat`, `bank`, …); it stays `None` only when
/// no snapshot was built, and the getters fail closed on it.
/// `navs`/`world` back the `ctx.walk` and `ctx.walk_with` closures — one
/// shared arm ([`ScriptWalkArm`]) takes the [`FindOptions`] (`walk` passes the
/// defaults): the arm refuses synchronously only when there is no tile,
/// no nav world, or a route already queued; `find_with` runs off-pump on
/// a short-lived worker per request, storing the route in the uid's nav
/// bot when one exists (a walk that would panic on the first follow step
/// must not succeed when no route can arm). `hold` is the guardian's
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
        scripts, cheats, navs, world, hold, ours, canlight, slot_input, None, None,
    )
}

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
) -> bool {
    if let Some(inp) = slot_input {
        inp.set_host_consume_allowed(up && !hold);
    }
    let mut wrote = false;
    let mut interact = Vec::new();
    let mut pending_withdraw_x_active = false;
    let mut pending_bank_op_active = false;
    let mut slot_work_epoch = None;
    if let Some(slot) = script_slot(scripts, name) {
        let mut slot = slot.lock().unwrap();
        slot.observe_lifecycle();
        // Reap a script-requested Stop before advancing host continuations.
        emit_script_debug_logs(&mut slot, name);
        slot.on_is_up(up);
        // Compiled clue machine: apply the owed abort and this frame's
        // pause/hold freeze here, on the slot's own thread, whether or not
        // this frame dispatches a tick. A Load slot's isolate thread runs the
        // same hooks for its own instance.
        slot.sync_compiled_clue(hold || ours);
        slot_work_epoch = Some(slot.work_epoch());
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
        // Post the snapshot only while the slot script is Running.
        // While the guardian holds: still dispatch the isolate tick so
        // `onPaint` runs (loop/pump stay frozen inside the isolate);
        // compiled scripts stay fully frozen (0.1.2). The blob still
        // posts while held so EventSignal reads the freeze.
        if tick_edge && slot.state() == script::RunState::Running {
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
            let recovery_hold = slot.load_active() && slot.watchdog().holds_script_actions();
            let isolate_hold = hold || recovery_hold;
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
                    },
                    &carry_rows,
                    inspect_posted,
                    Some(packed.view.as_ref()),
                    packed.flood.as_deref(),
                    packed.stamp,
                    |input, mut native| {
                        native.bank_selection = bank_selection;
                        slot.encode_snapshot_delta_with_native(input, native, force_banks)
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
                // One shared arm for both hooks: `walk_with` carries the
                // script's options through to `find_with`; `walk` is the
                // default-options adapter (rs2b0t `walk` semantics stay
                // default-off for teleports and wilderness). Each closure
                // owns its own clone of the arm.
                let bank_rows: Vec<(i32, i32)> = snapshot
                    .map(|s| s.bank().iter().map(|it| (it.def.id, it.count)).collect())
                    .unwrap_or_default();
                let arm = ScriptWalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                    state: state.clone(),
                    bank: bank_rows,
                };
                let mut walk_with = {
                    let arm = arm.clone();
                    move |x: i32, z: i32, level: i32, o: script::FindOptions| -> bool {
                        arm.route(
                            x,
                            z,
                            level,
                            FindOptions {
                                allow_teleports: o.allow_teleports,
                                allow_wilderness: o.allow_wilderness,
                                allow_bank_fetch: o.allow_bank_fetch,
                                ..FindOptions::default()
                            },
                        )
                    }
                };
                let mut walk = {
                    let arm = arm.clone();
                    move |x: i32, z: i32, level: i32| -> bool {
                        arm.route(x, z, level, FindOptions::default())
                    }
                };
                let selected = slot.compiled_game_data();
                slot.on_game_tick(&mut ScriptCtx {
                    driver,
                    tick,
                    here,
                    walk: Some(&mut walk),
                    walk_with: Some(&mut walk_with),
                    inv,
                    snapshot,
                    obj_names,
                    compiled: script::CompiledTick {
                        selected: selected.as_deref(),
                        hold: hold || ours,
                        ..Default::default()
                    },
                });
                wrote = true;
            }
        }
        emit_script_debug_logs(&mut slot, name);
        // Fold forwarded shim requests on running frames. Pause leaves the
        // isolate queue untouched so Resume can dispatch it; guardian hold
        // retains the existing drain/drop policy below.
        if slot.load_active() {
            let mut lifecycle = Vec::new();
            if slot.state() == script::RunState::Running {
                lifecycle.extend(slot.drain_lifecycle());
            }
            let ready = up
                && here.is_some()
                && snapshot.is_some_and(|snap| snap.ingame() && snap.scene_state() == 2);
            let frozen = hold || !ready || slot.state() == script::RunState::Paused;
            let xp: Vec<i32> = snapshot
                .map(|snap| snap.stats().iter().map(|stat| stat.xp).collect())
                .unwrap_or_default();
            let now = Instant::now();
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
                    .map(|(x, z, _)| {
                        slot.watchdog().recovering_anchor().is_some_and(|anchor| {
                            script::watchdog::chebyshev_xz((x, z), (anchor.x, anchor.z))
                                > script::watchdog::WALK_RADIUS
                        })
                    })
                    .unwrap_or(true);
                if far && recovery_walk_idle(navs, name) {
                    match slot.notify_walk_failed(now) {
                        script::WatchdogAction::Restart { .. } => {
                            interact.clear();
                            if let Err(e) = slot.restart_load_from_identity(now) {
                                eprintln!("[script {name}] watchdog restart failed: {e}");
                            }
                        }
                        other => apply_watchdog_nav_action(
                            other,
                            driver,
                            snapshot,
                            here,
                            navs,
                            world,
                            state.clone(),
                            name,
                        ),
                    }
                }
            }
            let running = slot.state() == script::RunState::Running;
            let action = slot.feed_watchdog(now, here, &xp, frozen, running, &lifecycle);
            match action {
                script::WatchdogAction::Restart { .. } => {
                    interact.clear();
                    if frozen {
                        eprintln!("[script {name}] watchdog restart ignored: frozen");
                    } else if let Err(e) = slot.restart_load_from_identity(now) {
                        eprintln!("[script {name}] watchdog restart failed: {e}");
                    }
                }
                script::WatchdogAction::RequestAnchor => slot.request_recovery_anchor(),
                script::WatchdogAction::WarnHungLoop => {
                    if debug_enabled() {
                        eprintln!("[script {name}] watchdog hung loop (10s)");
                    }
                }
                other => apply_watchdog_nav_action(
                    other,
                    driver,
                    snapshot,
                    here,
                    navs,
                    world,
                    state.clone(),
                    name,
                ),
            }
            slot.sync_native_input_gate();
            if slot.state() == script::RunState::Running {
                if slot.watchdog().holds_script_actions() {
                    let _dropped = slot.drain_interacts();
                } else {
                    interact.extend(take_script_interacts(slot.drain_interacts(), slot_input));
                }
            }
        } else if slot.state() == script::RunState::Running {
            interact.extend(take_script_interacts(slot.drain_interacts(), slot_input));
        }
    }
    // Dispatch the shim's interact requests through the slot's own Driver
    // (open/deposit/withdraw) and the shared walk arm (bank-stand walks
    // with default FindOptions, so wilderness/quest gates fail closed).
    // The guardian's hold drops them: the script's parked wait stays
    // frozen, and a later retry re-queues what still matters.
    if up && !hold && !interact.is_empty() {
        if let Some(snapshot) = snapshot {
            #[cfg(test)]
            wait_dispatch_barrier();
            if let Some(dispatch_slot) = script_slot(scripts, name) {
                let mut slot = dispatch_slot.lock().unwrap();
            if slot.state() == script::RunState::Running
                && Some(slot.work_epoch()) == slot_work_epoch
            {
            let mut dispatchable = Vec::with_capacity(interact.len());
            let mut armed = None;
            let mut armed_bank_op = None;
            let mut rejected_withdraw_x = 0usize;
            let mut rejected_withdraw_load = 0usize;
            let mut rejected_bank_op = 0usize;
            for req in interact {
                match req {
                    req @ (script::shim::InteractReq::Deposit { .. }
                    | script::shim::InteractReq::Withdraw { .. }) => {
                        if pending_bank_op_active || armed_bank_op.is_some() {
                            rejected_bank_op += 1;
                        } else if let Some(pending) =
                            dispatch_observed_bank_op(driver, snapshot, obj_names, inv, &req)
                        {
                            armed_bank_op = Some(pending);
                            pending_bank_op_active = true;
                            wrote = true;
                        } else {
                            rejected_bank_op += 1;
                        }
                    }
                    script::shim::InteractReq::WithdrawX {
                        name: item_name,
                        count,
                        bank_item_id,
                        lands_as_id,
                        action,
                        bank_generation,
                    } if armed.is_none()
                        && !pending_withdraw_x_active
                        && count > 0
                        && snapshot.bank_component_id() >= 0
                        && snapshot.bank_loaded()
                        && !snapshot.count_dialog_open()
                        && snapshot.bank_session_generation() == bank_generation =>
                    {
                        let mut accepted = false;
                        let wanted = item_name.to_lowercase();
                        let mut ix = api::interact::Interactions::new(snapshot, driver);
                        if let Some(item) = snapshot.bank().iter().find(|item| {
                            item.def.id == bank_item_id
                                && obj_names
                                    .and_then(|names| names.name(item.def.id))
                                    .is_some_and(|name| name.eq_ignore_ascii_case(&wanted))
                        }) {
                            if let Some(op) = action_slot(&item.actions, &action) {
                                let sent = matches!(
                                    ix.interact(
                                        api::interact::OpTarget::Item(item),
                                        api::interact::ActionSpec::Operation(op),
                                    ),
                                    api::interact::SendResult::Sent { .. }
                                );
                                if sent {
                                    let before = inv.map_or_else(
                                        || {
                                            snapshot
                                                .inventory()
                                                .iter()
                                                .filter(|held| held.def.id == lands_as_id)
                                                .map(|held| held.count)
                                                .sum()
                                        },
                                        |rows| {
                                            rows.iter()
                                                .filter(|(id, _)| *id == lands_as_id)
                                                .map(|(_, count)| *count)
                                                .sum()
                                        },
                                    );
                                    wrote = true;
                                    let pending = script::slot::PendingWithdrawX::waiting_dialog(
                                        lands_as_id,
                                        count,
                                        before,
                                        before.saturating_add(count.min(item.count)),
                                        bank_generation,
                                    );
                                    armed = Some(
                                        if matches!(count, 1 | 5 | 10)
                                            && action_slot(
                                                &item.actions,
                                                &format!("Withdraw {count}"),
                                            ) == Some(op)
                                        {
                                            pending.waiting_settlement()
                                        } else {
                                            pending
                                        },
                                    );
                                    pending_withdraw_x_active = true;
                                    accepted = true;
                                }
                            }
                        }
                        if !accepted {
                            rejected_withdraw_x += 1;
                        }
                    }
                    script::shim::InteractReq::WithdrawX { .. } => {
                        rejected_withdraw_x += 1;
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
                        let used = inv.map_or_else(|| snapshot.inventory().len(), <[_]>::len);
                        let free = capacity.saturating_sub(used);
                        let mut accepted = false;
                        if free > 0 {
                            let mut ix = api::interact::Interactions::new(snapshot, driver);
                            let wanted = item_name.to_lowercase();
                            if let Some(item) = snapshot.bank().iter().find(|item| {
                                item.count > 0
                                    && obj_names
                                        .and_then(|names| names.name(item.def.id))
                                        .is_some_and(|name| name.eq_ignore_ascii_case(&wanted))
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
                name,
                dispatchable,
                cache.clone(),
                obj_names_arc.clone(),
            );
            if armed.is_some()
                || armed_bank_op.is_some()
                || rejected_withdraw_x != 0
                || rejected_withdraw_load != 0
                || rejected_bank_op != 0
            {
                if Some(slot.work_epoch()) == slot_work_epoch {
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
            }
            }
            }
        } else {
            let rejected_x = interact
                .iter()
                .filter(|req| matches!(req, script::shim::InteractReq::WithdrawX { .. }))
                .count();
            let rejected_load = interact
                .iter()
                .filter(|req| matches!(req, script::shim::InteractReq::WithdrawLoad { .. }))
                .count();
            let rejected_bank = interact
                .iter()
                .filter(|req| {
                    matches!(
                        req,
                        script::shim::InteractReq::Deposit { .. }
                            | script::shim::InteractReq::Withdraw { .. }
                    )
                })
                .count();
            if rejected_x != 0 || rejected_load != 0 || rejected_bank != 0 {
                if let Some(slot) = script_slot(scripts, name) {
                    let mut slot = slot.lock().unwrap();
                    if Some(slot.work_epoch()) == slot_work_epoch {
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
            .filter(|req| matches!(req, script::shim::InteractReq::WithdrawX { .. }))
            .count();
        let rejected_load = interact
            .iter()
            .filter(|req| matches!(req, script::shim::InteractReq::WithdrawLoad { .. }))
            .count();
        let rejected_bank = interact
            .iter()
            .filter(|req| {
                matches!(
                    req,
                    script::shim::InteractReq::Deposit { .. }
                        | script::shim::InteractReq::Withdraw { .. }
                )
            })
            .count();
        if rejected_x != 0 || rejected_load != 0 || rejected_bank != 0 {
            if let Some(slot) = script_slot(scripts, name) {
                let mut slot = slot.lock().unwrap();
                if Some(slot.work_epoch()) == slot_work_epoch {
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
    for cmd in cmds.into_iter().filter(|_| up) {
        api::interact::cheat(driver, &cmd);
        wrote = true;
    }
    wrote
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

pub(crate) fn take_script_interacts(
    reqs: Vec<script::shim::InteractReq>,
    slot_input: Option<&SlotInput>,
) -> Vec<script::shim::InteractReq> {
    let mut out = Vec::with_capacity(reqs.len());
    for req in reqs {
        match req {
            script::shim::InteractReq::Mouse {
                down,
                x,
                y,
                button,
                identity,
            } => {
                if let Some(inp) = slot_input {
                    inp.enqueue_script_mouse_at(identity, down, x, y, button);
                }
            }
            other => out.push(other),
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
        let slot = slot.lock().unwrap();
        slot.state() == script::RunState::Running && slot.load_active()
    });
    if consumes_snapshot {
        project()
    } else {
        None
    }
}

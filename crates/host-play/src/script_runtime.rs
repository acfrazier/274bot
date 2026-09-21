//! Per-slot script observe/dispatch/hold transaction and script navigation continuation.

use super::*;

#[path = "route_inspect.rs"]
mod route_inspect;
pub(super) use route_inspect::PostedInspect;

/// Per-uid script cell on the wall. Encode/post/drain take the slot lock
/// only — the wall map lock is held briefly for lookup/insert.
pub(super) type ScriptSlot = Arc<Mutex<SlotScript>>;
/// Lock order: wall before slot; never hold slot then wall.
pub(super) type ScriptWall = Arc<Mutex<HashMap<String, ScriptSlot>>>;

pub(super) fn script_slot(wall: &ScriptWall, name: &str) -> Option<ScriptSlot> {
    wall.lock().unwrap().get(name).cloned()
}

pub(super) fn script_slot_or_insert(wall: &ScriptWall, name: &str) -> ScriptSlot {
    wall.lock()
        .unwrap()
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(SlotScript::new())))
        .clone()
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
#[allow(clippy::too_many_arguments)]
pub(super) fn script_observe_with_npc_boxes(
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
        driver,
        name,
        up,
        tick_edge,
        tick,
        here,
        inv,
        state,
        snapshot,
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
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn script_observe_cached(
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
        // Reap a script-requested Stop before advancing host continuations.
        emit_script_debug_logs(&mut slot, name);
        slot.on_is_up(up);
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
                    inspect_posted,
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
                                b.inspect.posted(),
                            );
                            b.mark_walk_outcome_posted();
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
                            route_inspect::PostedInspect::default(),
                        ),
                    }
                };
                let (withdraw_x_result_seq, withdraw_x_result) = slot.withdraw_x_result();
                let (withdraw_load_result_seq, withdraw_load_result) = slot.withdraw_load_result();
                let (bank_op_result_seq, bank_op_result) = slot.bank_op_result();
                let bytes = with_script_snapshot_input(
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
                    inspect_posted,
                    |input, native| {
                        slot.encode_snapshot_delta_with_native(input, native, force_banks)
                    },
                );
                slot.post_snapshot(bytes);
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
                slot.on_game_tick(&mut ScriptCtx {
                    driver,
                    tick,
                    here,
                    walk: Some(&mut walk),
                    walk_with: Some(&mut walk_with),
                    inv,
                    snapshot,
                    obj_names,
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
                if let Some(slot) = script_slot(scripts, name) {
                    let mut slot = slot.lock().unwrap();
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
pub(super) fn script_observe(
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

pub(super) fn take_script_interacts(
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

fn dispatch_observed_bank_op(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    inventory: Option<&[(i32, i32)]>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingBankOp> {
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    use script::shim::InteractReq;
    use script::slot::{PendingBankOp, PendingBankOpKind};

    if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
        return None;
    }
    let generation = snapshot.bank_session_generation();
    let inventory_count = |id| {
        inventory.map_or(0, |items| {
            items
                .iter()
                .filter(|(item_id, _)| *item_id == id)
                .map(|(_, count)| *count)
                .sum()
        })
    };
    match req {
        InteractReq::Deposit { name } => {
            let item = snapshot.bank_side().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = all_slot(&item.actions)?;
            let before_count = snapshot
                .bank_side()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    PendingBankOpKind::Deposit,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        InteractReq::Withdraw { name, action } => {
            let item = snapshot.bank().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = action_slot(&item.actions, action)?;
            let kind = if open_only_withdraw_x(&item.actions, op) {
                PendingBankOpKind::WithdrawXAction
            } else {
                PendingBankOpKind::Withdraw
            };
            let before_count = snapshot
                .bank()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    kind,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        _ => None,
    }
}

pub(super) fn nearest_bank_booth(
    world: &NavWorld,
    (x, z, level): (i32, i32, i32),
) -> Option<WorldTile> {
    world
        .banks()
        .iter()
        .filter(|stand| matches!(stand.access, nav::pack::BankAccess::Booth { .. }))
        .min_by_key(|stand| {
            let distance = stand.tile.x.abs_diff(x).max(stand.tile.z.abs_diff(z));
            if stand.tile.level == level {
                u64::from(distance)
            } else {
                (u64::MAX / 2).saturating_add(u64::from(distance))
            }
        })
        .map(|stand| stand.tile)
}

/// Dispatch one isolate's shim interact requests. Open/close/deposit/
/// withdraw run through [`api::interact::Interactions`] on the slot's
/// snapshot + Driver — a request whose target is missing (no loc at the
/// tile, no bank-side row with the resolved name, no bank open) fails
/// closed with no send. `walk` / `walk-near` route through the shared
/// [`ScriptWalkArm`] with the request's three FindOptions bits (serde/old
/// wire default off). `walk-to` (scene `DirectNavigator`) is the
/// [`Interactions::walk`] packet. Returns whether the driver's out buffer
/// was written.
#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_script_interact(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    name: &str,
    reqs: Vec<script::shim::InteractReq>,
) -> bool {
    dispatch_script_interact_cached(
        driver,
        snapshot,
        obj_names,
        here,
        navs,
        world,
        state,
        name,
        reqs,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_script_interact_cached(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    name: &str,
    reqs: Vec<script::shim::InteractReq>,
    cache: Option<Arc<Cache>>,
    obj_names_arc: Option<Arc<api::obj_names::ObjNames>>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    use script::shim::InteractReq;
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::requests(name, &reqs);
    let mut wrote = false;
    let camera_yaws: Vec<i32> = reqs
        .iter()
        .filter_map(|req| {
            if let InteractReq::SetCameraYaw { yaw } = req {
                Some(*yaw)
            } else {
                None
            }
        })
        .collect();
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    if debug_enabled() {
        for req in &reqs {
            eprintln!("[script {name}] interact {req:?}");
        }
    }
    for req in reqs {
        match req {
            InteractReq::OpenBooth {
                x,
                z,
                level,
                id,
                name: booth_name,
                action,
            } => {
                let target = api::snapshot::WorldTile { x, z, level };
                let accepted = match (booth_name, action) {
                    (Some(name), Some(action)) => matches!(
                        ix.open_named_booth_at(target, id, &name, &action),
                        SendResult::Sent { .. }
                    ),
                    (None, None) => {
                        matches!(ix.open_booth_at(target, id), SendResult::Sent { .. })
                    }
                    _ => false,
                };
                if accepted {
                    // The interaction supersedes any earlier scripted walk.
                    // Cancel after validation/dispatch only: rejected or
                    // malformed requests must not disturb an armed route.
                    abort_script_walk(navs, name);
                }
                wrote |= accepted;
            }
            InteractReq::OpenStand {
                x,
                z,
                level,
                kind,
                name,
                stand_op,
                ..
            } => {
                if kind == "booth" {
                    let loc = snapshot.locs().iter().find(|loc| {
                        loc.tile.x == x
                            && loc.tile.z == z
                            && loc.tile.level == level
                            && name.as_deref().is_none_or(|wanted| {
                                loc.name
                                    .as_deref()
                                    .is_some_and(|actual| actual.eq_ignore_ascii_case(wanted))
                            })
                    });
                    if let Some(loc) = loc {
                        let op = stand_op.or_else(|| action_slot(&loc.actions, "Use-quickly"));
                        if let Some(op) = op {
                            wrote |= matches!(
                                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            );
                        }
                    }
                } else if kind == "npc" {
                    let npc = snapshot.npcs().iter().find(|n| {
                        name.as_deref().is_some_and(|wanted| {
                            n.name
                                .as_deref()
                                .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                        })
                    });
                    if let Some(npc) = npc {
                        if let Some(op) = stand_op {
                            wrote |= matches!(
                                ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            );
                        }
                    }
                }
            }
            InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                request_id,
            } => {
                let bank_rows: Vec<(i32, i32)> = snapshot
                    .bank()
                    .iter()
                    .map(|it| (it.def.id, it.count))
                    .collect();
                let arm = ScriptWalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                    state: state.clone(),
                    bank: bank_rows,
                };
                wrote |= arm.queue_route(
                    x,
                    z,
                    level,
                    FindOptions {
                        allow_teleports,
                        allow_wilderness,
                        allow_bank_fetch,
                        ..FindOptions::default()
                    },
                    0,
                    false,
                    request_id,
                );
            }
            InteractReq::WalkNear {
                x,
                z,
                level,
                radius,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                request_id,
            } => {
                let arm = ScriptWalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                    state: state.clone(),
                    bank: snapshot
                        .bank()
                        .iter()
                        .map(|it| (it.def.id, it.count))
                        .collect(),
                };
                wrote |= arm.queue_route(
                    x,
                    z,
                    level,
                    FindOptions {
                        allow_teleports,
                        allow_wilderness,
                        allow_bank_fetch,
                        ..FindOptions::default()
                    },
                    radius,
                    true,
                    request_id,
                );
            }
            InteractReq::WalkNearestBank => {
                if let (Some((hx, hz, hl)), Some(nav_world)) = (here, world.as_deref()) {
                    if let Some(tile) = nearest_bank_booth(nav_world, (hx, hz, hl)) {
                        let arm = ScriptWalkArm {
                            here,
                            world: world.clone(),
                            navs: Arc::clone(navs),
                            name: name.to_string(),
                            state: state.clone(),
                            bank: snapshot
                                .bank()
                                .iter()
                                .map(|item| (item.def.id, item.count))
                                .collect(),
                        };
                        wrote |= arm.route_with_radius(
                            tile.x,
                            tile.z,
                            tile.level,
                            FindOptions::default(),
                            1,
                        );
                    }
                }
            }
            InteractReq::WalkTo { x, z, level } => {
                wrote |= matches!(ix.walk(WorldTile { x, z, level }), SendResult::Sent { .. });
            }
            InteractReq::InspectRoute {
                x,
                z,
                level,
                from_x,
                from_z,
                from_level,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                avoid,
                request_id,
                inspect_ack_seq,
            } => {
                let mut invalid_args = false;
                let mut rects = Vec::new();
                for zone in avoid {
                    match zone {
                        script::shim::InspectAvoidWire::Rect {
                            min_x,
                            max_x,
                            min_z,
                            max_z,
                            level: zone_level,
                        } => {
                            if min_x > max_x || min_z > max_z {
                                invalid_args = true;
                            }
                            rects.push(nav::router::AvoidRect {
                                min_x,
                                max_x,
                                min_z,
                                max_z,
                                level: zone_level,
                            });
                        }
                        script::shim::InspectAvoidWire::Unsupported => invalid_args = true,
                    }
                }
                let bank_rows: Vec<(i32, i32)> = snapshot
                    .bank()
                    .iter()
                    .map(|it| (it.def.id, it.count))
                    .collect();
                route_inspect::queue_inspect(
                    navs,
                    name,
                    world,
                    state.clone(),
                    bank_rows,
                    cache.clone(),
                    obj_names_arc.clone(),
                    route_inspect::InspectRequest {
                        from: WorldTile {
                            x: from_x,
                            z: from_z,
                            level: from_level,
                        },
                        to: WorldTile { x, z, level },
                        allow_teleports,
                        allow_wilderness,
                        allow_bank_fetch,
                        avoid: rects,
                        request_id,
                        inspect_ack_seq,
                        invalid_args,
                    },
                );
                wrote = true;
            }
            InteractReq::InspectAck { seq, generation } => {
                if let Some(bot) = navs.lock().unwrap().get_mut(name) {
                    bot.inspect.apply_ack(seq, generation);
                }
            }
            InteractReq::Deposit { name } => {
                let wanted = name.to_lowercase();
                for item in snapshot.bank_side() {
                    let resolved = obj_names.and_then(|n| n.name(item.def.id));
                    if resolved.is_some_and(|n| n.eq_ignore_ascii_case(&wanted)) {
                        if let Some(op) = all_slot(&item.actions) {
                            wrote |= matches!(
                                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            );
                        }
                    }
                }
            }
            InteractReq::Withdraw { name, action } => {
                let wanted = name.to_lowercase();
                if let Some(item) = snapshot.bank().iter().find(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                }) {
                    if let Some(op) = action_slot(&item.actions, &action) {
                        let res = ix.interact(OpTarget::Item(item), ActionSpec::Operation(op));
                        if host::debug_enabled() {
                            let outcome = match &res {
                                SendResult::Sent { .. } => "sent".to_string(),
                                SendResult::Refused { reason, .. } => {
                                    format!("refused {reason:?}")
                                }
                            };
                            eprintln!("[shim-withdraw] {name} {action} -> {outcome}");
                        }
                        wrote |= matches!(res, SendResult::Sent { .. });
                    } else if host::debug_enabled() {
                        eprintln!("[shim-withdraw] {name} {action} -> no op slot");
                    }
                } else if host::debug_enabled() {
                    eprintln!("[shim-withdraw] {name} {action} -> no bank row");
                }
            }
            // `script_observe` consumes this variant so it can arm the
            // slot-owned continuation only after the X action was sent.
            // Direct callers cannot safely create host pending state.
            InteractReq::WithdrawX { .. } | InteractReq::WithdrawLoad { .. } => {}
            InteractReq::Held { name, action } => {
                // rs2b0t `Item.interact` / `Inventory.first`: one name → one
                // held row (same as Withdraw's `.find`). A name the table
                // does not know or an item that is no longer held fails
                // closed — nothing is sent.
                let wanted = name.to_lowercase();
                if let Some(item) = snapshot.inventory().iter().find(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                }) {
                    let res = ix.interact(OpTarget::Item(item), ActionSpec::Label(action.clone()));
                    if host::debug_enabled() {
                        let outcome = match &res {
                            SendResult::Sent { .. } => "sent".to_string(),
                            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
                        };
                        eprintln!(
                            "[shim-held] {name} {action} slot={} -> {outcome}",
                            item.slot
                        );
                    }
                    wrote |= matches!(res, SendResult::Sent { .. });
                }
            }
            InteractReq::InvButton {
                id,
                slot,
                component,
                operation,
                bank_generation,
            } => {
                // Selected bank withdraw or bank-side deposit identity, or the
                // open trade offer/side row. Exact id/slot/component only —
                // no same-name fallback, no count-dialog answer.
                if snapshot.bank_component_id() >= 0
                    && snapshot.bank_loaded()
                    && snapshot.bank_session_generation() == bank_generation
                {
                    if let Some(item) = snapshot
                        .bank()
                        .iter()
                        .chain(snapshot.bank_side().iter())
                        .find(|item| {
                            item.def.id == id && item.slot == slot && item.component_id == component
                        })
                    {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                            SendResult::Sent { .. }
                        );
                    }
                } else if snapshot.trade().offer_open {
                    if let Some(item) = snapshot
                        .trade()
                        .side_pack
                        .iter()
                        .chain(snapshot.trade().my_offer.iter())
                        .find(|item| {
                            item.def.id == id && item.slot == slot && item.component_id == component
                        })
                    {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                            SendResult::Sent { .. }
                        );
                    }
                }
            }
            InteractReq::ShopButton {
                kind,
                name,
                id,
                slot,
                component,
                chunk,
            } => {
                // Sell acts on the shop's own player pack and Buy on its
                // stock: never the other container, never the backpack. The
                // exact posted row must still be there (no same-name
                // fallback), then the api op re-resolves and sends it.
                let rows: &[api::snapshot::ItemView] = if kind == "sell" {
                    &snapshot.shop().player
                } else {
                    &snapshot.shop().stock
                };
                let present = rows
                    .iter()
                    .any(|it| it.def.id == id && it.slot == slot && it.component_id == component);
                if present {
                    let sent = if kind == "sell" {
                        ix.shop_sell(&name, chunk)
                    } else {
                        ix.shop_buy(&name, chunk)
                    };
                    wrote |= matches!(sent, SendResult::Sent { .. });
                }
            }
            InteractReq::MakePanel {
                id,
                slot,
                component,
                operation,
            } => {
                // Anvil/main skill-multi identity only. Do not fall back to
                // another same-name row, the backpack, or a count dialog.
                let present = snapshot.main_make().iter().any(|item| {
                    item.def.id == id && item.slot == slot && item.component_id == component
                });
                if present {
                    if let Some(item) = snapshot.main_make().iter().find(|item| {
                        item.def.id == id && item.slot == slot && item.component_id == component
                    }) {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                            SendResult::Sent { .. }
                        );
                    }
                }
            }
            InteractReq::Close => {
                let res = ix.close_modal();
                if host::debug_enabled() {
                    match &res {
                        SendResult::Sent { .. } => eprintln!("[shim-close] sent"),
                        SendResult::Refused { reason, .. } => {
                            eprintln!("[shim-close] refused {reason:?}")
                        }
                    }
                }
                wrote |= matches!(res, SendResult::Sent { .. });
            }
            InteractReq::Npc {
                name,
                action,
                index,
            } => {
                let wanted = name.to_lowercase();
                let npc = snapshot.npcs().iter().find(|n| {
                    if let Some(i) = index {
                        n.index as i32 == i
                    } else {
                        n.name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                    }
                });
                if let Some(npc) = npc {
                    wrote |= matches!(
                        ix.interact(OpTarget::Npc(npc), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::Loc {
                x,
                z,
                level,
                action,
                id,
            } => {
                // Selected `id` must match a current row at this tile.
                // Do not fall back to another co-located loc. Absent id
                // keeps the previous first-row coordinate match.
                let loc = snapshot.locs().iter().find(|l| {
                    l.tile.x == x
                        && l.tile.z == z
                        && l.tile.level == level
                        && id.is_none_or(|wanted| l.id == wanted)
                });
                if let Some(loc) = loc {
                    wrote |= matches!(
                        ix.interact(OpTarget::Loc(loc), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::Obj {
                x,
                z,
                level,
                name,
                action,
            } => {
                let obj = snapshot.ground_items().iter().find(|it| {
                    it.tile.x == x
                        && it.tile.z == z
                        && it.tile.level == level
                        && name.as_deref().is_none_or(|wanted| {
                            obj_names
                                .and_then(|n| n.name(it.def.id))
                                .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                        })
                });
                if let Some(obj) = obj {
                    wrote |= matches!(
                        ix.interact(OpTarget::GroundItem(obj), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::Player { name, action } => {
                let wanted = name.to_lowercase();
                let player = snapshot.players().iter().find(|p| {
                    p.actor
                        .name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                });
                if let Some(player) = player {
                    wrote |= matches!(
                        ix.interact(OpTarget::Player(player), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::UseOn {
                name,
                kind,
                target_name,
                x,
                z,
                level,
                index,
                source_item_id,
                source_item_slot,
                target_item_id,
                target_item_slot,
            } => {
                let item = resolve_inventory_item(
                    snapshot,
                    obj_names,
                    Some(&name),
                    source_item_id,
                    source_item_slot,
                );
                if let Some(item) = item {
                    if let Some(target) = resolve_op_target(
                        snapshot,
                        obj_names,
                        &kind,
                        target_name.as_deref(),
                        target_item_id,
                        target_item_slot,
                        x,
                        z,
                        level,
                        index,
                    ) {
                        wrote |= matches!(ix.use_item_on(item, target), SendResult::Sent { .. });
                    }
                }
            }
            InteractReq::UseWidgetOn {
                component_id,
                kind,
                target_name,
                x,
                z,
                level,
                index,
            } => {
                let ctx = api::snapshot::ReadContext::new(snapshot);
                if let Some(widget) = ctx.component(component_id) {
                    if let Some(target) = resolve_op_target(
                        snapshot,
                        obj_names,
                        &kind,
                        target_name.as_deref(),
                        None,
                        None,
                        x,
                        z,
                        level,
                        index,
                    ) {
                        wrote |=
                            matches!(ix.use_widget_on(widget, target), SendResult::Sent { .. });
                    }
                }
            }
            InteractReq::ContinueDialog => {
                wrote |= matches!(ix.continue_dialog(), SendResult::Sent { .. });
            }
            InteractReq::Answer { option } => {
                wrote |= matches!(ix.answer_choice(option), SendResult::Sent { .. });
            }
            InteractReq::IfButton { component_id } => {
                let ctx = api::snapshot::ReadContext::new(snapshot);
                if let Some(widget) = ctx.component(component_id) {
                    wrote |= matches!(ix.press(widget), SendResult::Sent { .. });
                }
            }
            InteractReq::CloseModal => {
                wrote |= matches!(ix.close_modal(), SendResult::Sent { .. });
            }
            InteractReq::AnswerCount { value } => {
                let res = ix.answer_count(value);
                if host::debug_enabled() {
                    match &res {
                        SendResult::Sent { .. } => {
                            eprintln!("[shim-count] {value} sent")
                        }
                        SendResult::Refused { reason, .. } => {
                            eprintln!("[shim-count] {value} refused {reason:?}")
                        }
                    }
                }
                wrote |= matches!(res, SendResult::Sent { .. });
            }
            InteractReq::SideTab { tab } => {
                wrote |= matches!(ix.click_side_tab(tab), SendResult::Sent { .. });
            }
            InteractReq::Wear { name } => {
                let wanted = name.to_lowercase();
                let id = snapshot.inventory().iter().find_map(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .filter(|n| n.eq_ignore_ascii_case(&wanted))
                        .map(|_| it.def.id)
                });
                if let Some(id) = id {
                    let res = ix.wear(id);
                    if host::debug_enabled() {
                        let outcome = match &res {
                            SendResult::Sent { .. } => "sent".to_string(),
                            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
                        };
                        eprintln!(
                            "[shim-wear] {name} id={id} inv={} zip={} -> {outcome}",
                            snapshot.inventory().len(),
                            snapshot.inv().len()
                        );
                    }
                    wrote |= matches!(res, SendResult::Sent { .. });
                } else if host::debug_enabled() {
                    eprintln!(
                        "[shim-wear] {name} no inventory() row inv={} zip={:?}",
                        snapshot.inventory().len(),
                        snapshot.inv()
                    );
                }
            }
            InteractReq::SetRun { on } => {
                wrote |= matches!(ix.set_run(on), SendResult::Sent { .. });
            }
            InteractReq::SetRetaliate { on } => {
                wrote |= matches!(ix.set_retaliate(on), SendResult::Sent { .. });
            }
            InteractReq::SetNoteMode { on } => {
                let res = ix.set_note_mode(on);
                if host::debug_enabled() {
                    match &res {
                        SendResult::Sent { .. } => eprintln!("[shim-note] on={on} sent"),
                        SendResult::Refused { reason, .. } => {
                            eprintln!("[shim-note] on={on} refused {reason:?}")
                        }
                    }
                }
                wrote |= matches!(res, SendResult::Sent { .. });
            }
            InteractReq::Key { down, key, .. } => {
                wrote |= ix.apply_amount_key(down, &key);
            }
            InteractReq::Mouse { .. } => {}
            InteractReq::SetCameraYaw { .. } => {}
            InteractReq::NoteProgress
            | InteractReq::LoopSettled
            | InteractReq::WaitEnqueued
            | InteractReq::WaitSettled
            | InteractReq::RecoveryAnchor { .. }
            | InteractReq::RecoveryAnchorNone => {}
        }
    }
    for yaw in camera_yaws {
        wrote |= driver.set_orbit_camera_yaw(yaw);
    }
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::sent(name, wrote);
    wrote
}

pub(super) fn abort_script_walk(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        bot.route_generation = bot.route_generation.wrapping_add(1);
        bot.route = None;
        bot.route_worker = None;
        bot.pending_route = None;
        bot.requested_route = None;
        bot.walk_request_id = 0;
        bot.clear_walk_outcome();
    }
}

fn recovery_walk_idle(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) -> bool {
    let all = navs.lock().unwrap();
    let Some(bot) = all.get(name) else {
        return true;
    };
    bot.route_worker.is_none() && bot.route.is_none() && bot.pending_route.is_none()
}

fn apply_watchdog_nav_action(
    action: script::WatchdogAction,
    _driver: &mut dyn Driver,
    snapshot: Option<&GameSnapshot>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    name: &str,
) {
    match action {
        script::WatchdogAction::AbortWalk => abort_script_walk(navs, name),
        script::WatchdogAction::ArmWalk { x, z, level } => {
            let Some(snapshot) = snapshot else {
                return;
            };
            let arm = ScriptWalkArm {
                here,
                world: world.clone(),
                navs: Arc::clone(navs),
                name: name.to_string(),
                state,
                bank: snapshot
                    .bank()
                    .iter()
                    .map(|item| (item.def.id, item.count))
                    .collect(),
            };
            let armed = arm.route_with_radius(
                x,
                z,
                level,
                FindOptions::default(),
                script::watchdog::WALK_RADIUS,
            );
            if !armed {
                abort_script_walk(navs, name);
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_op_target<'a>(
    snapshot: &'a GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    kind: &str,
    target_name: Option<&str>,
    target_item_id: Option<i32>,
    target_item_slot: Option<i32>,
    x: i32,
    z: i32,
    level: i32,
    index: Option<i32>,
) -> Option<api::interact::OpTarget<'a>> {
    use api::interact::OpTarget;
    match kind {
        "npc" => {
            let wanted = target_name.map(|n| n.to_lowercase());
            snapshot
                .npcs()
                .iter()
                .find(|n| {
                    if let Some(i) = index {
                        n.index as i32 == i
                    } else {
                        wanted.as_ref().is_some_and(|w| {
                            n.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(w))
                        })
                    }
                })
                .map(OpTarget::Npc)
        }
        "loc" => snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)
            .map(OpTarget::Loc),
        "obj" => snapshot
            .ground_items()
            .iter()
            .find(|it| {
                it.tile.x == x
                    && it.tile.z == z
                    && it.tile.level == level
                    && target_name.is_none_or(|wanted| {
                        obj_names
                            .and_then(|n| n.name(it.def.id))
                            .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                    })
            })
            .map(OpTarget::GroundItem),
        "player" => {
            let wanted = target_name.map(|n| n.to_lowercase());
            snapshot
                .players()
                .iter()
                .find(|p| {
                    wanted.as_ref().is_some_and(|w| {
                        p.actor
                            .name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(w))
                    })
                })
                .map(OpTarget::Player)
        }
        "inv" | "held" => resolve_inventory_item(
            snapshot,
            obj_names,
            target_name,
            target_item_id,
            target_item_slot,
        )
        .map(OpTarget::Item),
        _ => None,
    }
}

fn resolve_inventory_item<'a>(
    snapshot: &'a GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: Option<&str>,
    item_id: Option<i32>,
    item_slot: Option<i32>,
) -> Option<&'a api::snapshot::ItemView> {
    match (item_id, item_slot) {
        (Some(id), Some(slot)) => snapshot
            .inventory()
            .iter()
            .find(|item| item.def.id == id && item.slot == slot),
        (None, None) => {
            let wanted = name?.to_lowercase();
            snapshot.inventory().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(&wanted))
            })
        }
        _ => None,
    }
}

/// Action-label lookup matching rs2b0t's `norm` (lowercase, whitespace and
/// `-`/`_` separators gone): `Withdraw All`, `Withdraw-All` and
/// `Withdraw  All` all resolve to the same slot.
fn action_slot(actions: &[Option<String>], wanted: &str) -> Option<i32> {
    let wanted = norm_action(wanted);
    actions
        .iter()
        .position(|a| a.as_deref().map(norm_action).as_deref() == Some(wanted.as_str()))
        .map(|i| i as i32 + 1)
}

/// Whether the resolved bank op is the open-only `Withdraw X` label: it
/// opens the amount dialog and cannot move inventory on its own, so the
/// accepted send *is* the result (frozen `Bank.withdraw` returns
/// `Input.invButton` → `actions.menuAction`). Every other withdraw label
/// settles on the observed delta.
fn open_only_withdraw_x(actions: &[Option<String>], op: i32) -> bool {
    let index = match usize::try_from(op) {
        Ok(index) if index >= 1 => index - 1,
        _ => return false,
    };
    actions
        .get(index)
        .and_then(|label| label.as_deref())
        .is_some_and(|label| norm_action(label) == norm_action("Withdraw X"))
}

/// The bank-side op slot whose label contains "all" (Deposit All / the
/// deposit window's bulk op), 1-based.
fn all_slot(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|a| a.as_deref().is_some_and(|s| norm_action(s).contains("all")))
        .map(|i| i as i32 + 1)
}

/// Select one bounded fill operation in compatibility order: an exact
/// amount, then All only when the requested fill consumes current stock,
/// then the host-owned X continuation.
pub(super) fn fill_withdraw_action(
    actions: &[Option<String>],
    count: i32,
    stock: i32,
) -> Option<(i32, bool)> {
    action_slot(actions, &format!("Withdraw {count}"))
        .map(|op| (op, false))
        .or_else(|| {
            (count == stock)
                .then(|| all_slot(actions).map(|op| (op, false)))
                .flatten()
        })
        .or_else(|| action_slot(actions, "Withdraw X").map(|op| (op, true)))
}

fn norm_action(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect()
}

/// The FlatBuffer snapshot blob posted into a Load isolate each
/// PLAYER_INFO (schema: `crates/script/schema/isolate.fbs`): `tick, here,
/// ingame, inv, stats, booths, banks, bank, bank_side, bank_open,
/// bank_loaded, hold, ours` — the exact fields the shim
/// Game/Inventory/Skills/Bank/Banking/EventSignal read, and nothing else
/// (no World clone). `here` is the local player's tile `{x, z, level}`
/// (absent when the body decoded none); `inv` rows carry the obj's
/// resolved name (`None` when the shared table has none — a name a script
/// queries never matches); `stats` rows carry the snapshot's stat
/// index/name/xp; `booths` are the scene locs whose actions include
/// `Use-quickly` (a name/action a script interacts with never appears
/// otherwise); `banks` are the packed bank stands (`{name, x, z, level,
/// kind: booth|npc, op, choose}`) the shim walks to; `bank`/`bank_side`
/// are the open bank's withdraw/deposit rows with the obj's resolved
/// name (`None` when the table has none — a deposit/withdraw by that name
/// never matches); `hold`/`ours` are the guardian's published status that
/// `EventSignal.pending()` reads.
///
/// Posts are DELTAS: `tick` is always carried; every other field only
/// when it changed vs `last` (the per-slot last-post fingerprint, `None`
/// right after Start — the first post is then the full keyframe). A 50+
/// isolate wall never resends unchanged inv/bank/stats/booths/packed
/// banks. Packed `banks` are additionally re-posted when `force_banks`
/// (the `NavWorld` identity changed) even though the stand list is
/// byte-identical. Returns the blob and the fingerprint to store as the
/// new last-post baseline.
/// Tests encode through a one-shot builder; the live observe path uses
/// [`with_script_snapshot_input`] + the slot's [`script::isolate_fb::IsolateBuf`].
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn script_snapshot_fb(
    last: Option<&script::isolate_fb::SnapshotFingerprint>,
    force_banks: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    hold: bool,
    ours: bool,
    teleports_enabled: bool,
) -> (Vec<u8>, script::isolate_fb::SnapshotFingerprint) {
    with_script_snapshot_input(
        tick,
        here,
        ingame,
        inv,
        snapshot,
        obj_names,
        world,
        None,
        hold,
        ours,
        teleports_enabled,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        route_inspect::PostedInspect::default(),
        |input, native| {
            script::isolate_fb::encode_snapshot_delta_with_native(last, input, native, force_banks)
        },
    )
}

/// Note-mode landing id for an unnoted obj (`ObjNames` reverse cert), else
/// the snapshot def's raw `certlink`.
fn posted_cert(
    obj_names: Option<&api::obj_names::ObjNames>,
    def: &api::obj_names::ItemDefView,
) -> i32 {
    obj_names
        .and_then(|n| n.item(def.id))
        .map(|d| d.certificate_link)
        .filter(|&c| c >= 0)
        .unwrap_or(def.certificate_link)
}

fn posted_cert_id(obj_names: Option<&api::obj_names::ObjNames>, id: i32) -> i32 {
    obj_names
        .and_then(|n| n.item(id))
        .map(|d| d.certificate_link)
        .filter(|&c| c >= 0)
        .unwrap_or(-1)
}

/// Host-published walk outcome copied onto the isolate snapshot.
#[derive(Clone, Copy, Default)]
pub(super) struct PostedWalkOutcome {
    pub(super) seq: u64,
    pub(super) generation: u64,
    pub(super) request_id: u64,
    pub(super) failed: bool,
    pub(super) x: i32,
    pub(super) z: i32,
    pub(super) level: i32,
    pub(super) radius: i32,
    pub(super) allow_teleports: bool,
}

/// Build the observed snapshot input and hand it to `f`. The live observe
/// path encodes through the slot's reusable [`script::isolate_fb::IsolateBuf`];
/// tests encode through a one-shot builder via [`script_snapshot_fb`].
#[allow(clippy::too_many_arguments, unused_assignments)]
pub(super) fn with_script_snapshot_input<R>(
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
    hold: bool,
    ours: bool,
    teleports_enabled: bool,
    withdraw_x_result_seq: u64,
    withdraw_x_result: bool,
    withdraw_load_result_seq: u64,
    withdraw_load_result: bool,
    bank_op_result_seq: u64,
    bank_op_result: bool,
    canlight: Option<&[u64]>,
    walk_outcome: PostedWalkOutcome,
    inspect: route_inspect::PostedInspect,
    f: impl FnOnce(
        &script::isolate_fb::SnapshotInput<'_>,
        script::isolate_fb::NativeFactsInput<'_>,
    ) -> R,
) -> R {
    use script::isolate_fb::{
        BankApproachInput, BankStandInput, ChatLineInput, ChatOptionInput, CombatStyleInput,
        ItemRowInput, MakeButtonInput, MakeProductInput, NativeFactsInput, NearestBoothInput,
        QuestStatusInput, ReachViewInput, SceneEntityInput, SideTabIfaceInput, SnapshotInput,
        StatInput, TileInput, VarpInput, WidgetTextInput,
    };

    let flood = snapshot.and_then(|s| {
        let (x, z, level) = here?;
        if !s.scene().available {
            return None;
        }
        api::query::SceneQuery::new(s.scene(), Some(WorldTile { x, z, level })).flood_reach()
    });
    let canlight_plane = canlight.and_then(|bits| {
        world.map(|w| api::query::CanlightPlane {
            bits,
            origin_x: w.collision.origin.x,
            origin_z: w.collision.origin.z,
            width: w.collision.width as i32,
            height: w.collision.height as i32,
        })
    });
    let reach_pack = snapshot
        .map(|s| api::query::pack_reach_query_plane(s.scene(), flood.as_ref(), canlight_plane))
        .unwrap_or_else(api::query::ReachQueryView::unavailable);
    let reach = ReachViewInput {
        available: reach_pack.available,
        base_x: reach_pack.base_x,
        base_z: reach_pack.base_z,
        level: reach_pack.level,
        width: reach_pack.width,
        height: reach_pack.height,
        walkable: &reach_pack.walkable,
        reachable: &reach_pack.reachable,
        reachable_adj: &reach_pack.reachable_adj,
        exact_rank: &reach_pack.exact_rank,
        adjacent_rank: &reach_pack.adjacent_rank,
        step: &reach_pack.step,
        canlight: &reach_pack.canlight,
    };
    let here = here.map(|(x, z, level)| TileInput { x, z, level });
    let entity_reach = |x: i32, z: i32, level: i32| -> (bool, bool) {
        flood
            .as_ref()
            .map(|f| f.at(&WorldTile { x, z, level }))
            .unwrap_or((false, false))
    };
    let scene_entity_target = |target: Option<&api::snapshot::ActorTargetView>| -> (i32, i32) {
        match target {
            None => (0, -1),
            Some(t) => {
                let kind = match t.kind {
                    api::snapshot::ActorKind::Npc => 1,
                    api::snapshot::ActorKind::Player => 2,
                };
                (kind, t.index as i32)
            }
        }
    };
    let inv_ops_store: Vec<Vec<String>>;
    let inv: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        if s.inventory().is_empty() {
            inv_ops_store = Vec::new();
            inv.map(|rows| {
                rows.iter()
                    .map(|(id, count)| ItemRowInput {
                        name: obj_names.and_then(|names| names.name(*id)),
                        count: *count,
                        id: *id,
                        ops: &[],
                        noted: false,
                        cert: posted_cert_id(obj_names, *id),
                        component_id: -1,
                        slot: -1,
                    })
                    .collect()
            })
            .unwrap_or_default()
        } else {
            inv_ops_store = s
                .inventory()
                .iter()
                .map(|it| {
                    it.actions
                        .iter()
                        .filter_map(|a| a.as_deref().map(str::to_string))
                        .collect()
                })
                .collect();
            s.inventory()
                .iter()
                .enumerate()
                .map(|(i, it)| ItemRowInput {
                    name: obj_names
                        .and_then(|names| names.name(it.def.id))
                        .or(it.def.name.as_deref()),
                    count: it.count,
                    id: it.def.id,
                    ops: &inv_ops_store[i],
                    noted: it.def.noted,
                    cert: posted_cert(obj_names, &it.def),
                    component_id: -1,
                    slot: it.slot,
                })
                .collect()
        }
    } else {
        inv_ops_store = Vec::new();
        inv.map(|rows| {
            rows.iter()
                .map(|(id, count)| ItemRowInput {
                    name: obj_names.and_then(|names| names.name(*id)),
                    count: *count,
                    id: *id,
                    ops: &[],
                    noted: false,
                    cert: posted_cert_id(obj_names, *id),
                    component_id: -1,
                    slot: -1,
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let stats = snapshot.map(|s| {
        s.stats()
            .iter()
            .map(|st| StatInput {
                index: st.index,
                name: &st.name,
                xp: st.xp,
                base: st.base,
                effective: st.effective,
            })
            .collect::<Vec<_>>()
    });
    let bank_ops_store: Vec<Vec<String>>;
    let bank: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        bank_ops_store = s
            .bank()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.bank()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &bank_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        bank_ops_store = Vec::new();
        Vec::new()
    };
    let bank_side_ops_store: Vec<Vec<String>>;
    let bank_side: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        bank_side_ops_store = s
            .bank_side()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.bank_side()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &bank_side_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                // Real deposit component (e.g. 2006 / fixture 701). Input.invButton
                // revalidates this id; posting -1 rejects every bank-side deposit.
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        bank_side_ops_store = Vec::new();
        Vec::new()
    };
    let npc_action_store: Vec<Vec<String>>;
    let npcs: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        npc_action_store = s
            .npcs()
            .iter()
            .map(|npc| {
                npc.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.npcs()
            .iter()
            .enumerate()
            .map(|(i, npc)| {
                let (reachable, reachable_adj) =
                    entity_reach(npc.tile.x, npc.tile.z, npc.tile.level);
                let (target_kind, target_index) = scene_entity_target(npc.target.as_ref());
                SceneEntityInput {
                    index: npc.index as i32,
                    id: npc.r#type.map(|t| t as i32).unwrap_or(-1),
                    name: npc.name.as_deref(),
                    x: npc.tile.x,
                    z: npc.tile.z,
                    level: npc.tile.level,
                    distance: npc.distance,
                    health: npc.health,
                    max_health: npc.total_health,
                    in_combat: npc.in_combat,
                    animating: npc.moving || npc.animation != -1,
                    actions: &npc_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: npc.level,
                    target_kind,
                    target_index,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let loc_action_store: Vec<Vec<String>>;
    let locs: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        loc_action_store = s
            .locs()
            .iter()
            .map(|loc| {
                loc.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.locs()
            .iter()
            .enumerate()
            .map(|(i, loc)| {
                let (reachable, reachable_adj) =
                    entity_reach(loc.tile.x, loc.tile.z, loc.tile.level);
                SceneEntityInput {
                    index: loc.id,
                    id: loc.id,
                    name: loc.name.as_deref(),
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    distance: loc.distance,
                    health: -1,
                    max_health: -1,
                    in_combat: false,
                    animating: loc.animation != -1,
                    actions: &loc_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: 0,
                    target_kind: 0,
                    target_index: -1,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let player_action_store: Vec<Vec<String>>;
    let players: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        player_action_store = s
            .players()
            .iter()
            .map(|player| {
                player
                    .actor
                    .actions
                    .iter()
                    // FlatBuffers cannot carry null entries in a string
                    // vector. Keep each native op's position with the
                    // existing empty-string sentinel; the shim filters it
                    // from the public actions while opIndex still sees the
                    // original one-based slot numbers.
                    .map(|a| a.as_deref().unwrap_or_default().to_string())
                    .collect::<Vec<String>>()
            })
            .collect();
        s.players()
            .iter()
            .enumerate()
            .map(|(i, player)| {
                let (reachable, reachable_adj) = entity_reach(
                    player.actor.tile.x,
                    player.actor.tile.z,
                    player.actor.tile.level,
                );
                let (target_kind, target_index) = scene_entity_target(player.actor.target.as_ref());
                SceneEntityInput {
                    index: player.index as i32,
                    id: player.index as i32,
                    name: player.actor.name.as_deref(),
                    x: player.actor.tile.x,
                    z: player.actor.tile.z,
                    level: player.actor.tile.level,
                    distance: player.actor.distance,
                    health: player.actor.health,
                    max_health: player.actor.total_health,
                    in_combat: player.actor.in_combat,
                    animating: player.actor.moving || player.actor.animation != -1,
                    actions: &player_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: player.combat_level,
                    target_kind,
                    target_index,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let ground_action_store: Vec<Vec<String>>;
    let ground: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        ground_action_store = s
            .ground_items()
            .iter()
            .map(|item| {
                item.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.ground_items()
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (reachable, reachable_adj) =
                    entity_reach(item.tile.x, item.tile.z, item.tile.level);
                SceneEntityInput {
                    index: item.def.id,
                    id: item.def.id,
                    name: obj_names.and_then(|names| names.name(item.def.id)),
                    x: item.tile.x,
                    z: item.tile.z,
                    level: item.tile.level,
                    distance: item.distance,
                    health: -1,
                    max_health: -1,
                    in_combat: false,
                    animating: false,
                    actions: &ground_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: 0,
                    target_kind: 0,
                    target_index: -1,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let equip_ops_store: Vec<Vec<String>>;
    let equipment: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        equip_ops_store = s
            .equipment()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.equipment()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &equip_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: -1,
                slot: it.slot,
            })
            .collect()
    } else {
        equip_ops_store = Vec::new();
        Vec::new()
    };
    let trade_mine_ops_store: Vec<Vec<String>>;
    let trade_mine: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_mine_ops_store = s
            .trade()
            .my_offer
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .my_offer
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_mine_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_mine_ops_store = Vec::new();
        Vec::new()
    };
    let trade_theirs_ops_store: Vec<Vec<String>>;
    let trade_theirs: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_theirs_ops_store = s
            .trade()
            .their_offer
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .their_offer
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_theirs_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_theirs_ops_store = Vec::new();
        Vec::new()
    };
    let trade_side_ops_store: Vec<Vec<String>>;
    let trade_side: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_side_ops_store = s
            .trade()
            .side_pack
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .side_pack
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_side_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_side_ops_store = Vec::new();
        Vec::new()
    };
    let shop_player_ops_store: Vec<Vec<String>>;
    let shop_player_rows: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        shop_player_ops_store = s
            .shop()
            .player
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.shop()
            .player
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &shop_player_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        shop_player_ops_store = Vec::new();
        Vec::new()
    };
    // `None` = the shop side interface's player pack was not decoded: the
    // Sell path fails closed rather than acting on an empty stand-in.
    let shop_player = snapshot
        .filter(|s| s.shop().player_available)
        .map(|_| shop_player_rows.as_slice());
    let main_make_ops_store: Vec<Vec<String>>;
    let main_make_rows: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        main_make_ops_store = s
            .main_make()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.main_make()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &main_make_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        main_make_ops_store = Vec::new();
        Vec::new()
    };
    // Always decoded when a snapshot exists: empty means no Make TYPE_INV
    // on the open main modal, not an unpublished field.
    let main_make = snapshot.map(|_| main_make_rows.as_slice());
    let shop_stock_ops_store: Vec<Vec<String>>;
    let shop_stock: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        shop_stock_ops_store = s
            .shop()
            .stock
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.shop()
            .stock
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &shop_stock_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        shop_stock_ops_store = Vec::new();
        Vec::new()
    };
    let chat_options = snapshot.map(|s| {
        s.chat_options()
            .iter()
            .map(|o| ChatOptionInput { text: &o.text })
            .collect::<Vec<_>>()
    });
    let varps = snapshot.map(|s| {
        let magic = api::snapshot::ReadContext::new(s).varp(108);
        let energy = api::snapshot::ReadContext::new(s).varp(300);
        let armed = api::snapshot::ReadContext::new(s).varp(301);
        // attackstyle_magic is packed as varp 108 on both selected caches.
        // sa_energy/sa_attack are packed as 300/301. Always post them,
        // including 0, so Special.energy/armed and Autocast.armed/selected
        // observe real state instead of a missing-row default.
        let mut rows: Vec<VarpInput> = vec![
            VarpInput {
                index: 108,
                value: magic,
            },
            VarpInput {
                index: 300,
                value: energy,
            },
            VarpInput {
                index: 301,
                value: armed,
            },
        ];
        rows.extend(
            s.varps()
                .iter()
                .filter(|v| v.value != 0 && v.index != 108 && v.index != 300 && v.index != 301)
                .take(29)
                .map(|v| VarpInput {
                    index: v.index,
                    value: v.value,
                }),
        );
        rows
    });
    let combat_style_store = snapshot.map(|s| {
        let root = s
            .side_tabs()
            .iter()
            .find(|t| t.index == 0)
            .map(|t| t.root_component_id)
            .unwrap_or(-1);
        if root == -1 {
            Vec::new()
        } else {
            api::query::widget_search::combat_style_labels(s, root, 43)
        }
    });
    let combat_styles: Vec<CombatStyleInput<'_>> = combat_style_store
        .as_ref()
        .map(|labels| {
            labels
                .iter()
                .map(|l| CombatStyleInput {
                    mode: l.mode,
                    label: &l.label,
                    component_id: l.component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let local = snapshot.and_then(|s| s.local_player());
    let my_name = local.and_then(|lp| lp.player.actor.name.as_deref());
    let in_combat = local.is_some_and(|lp| lp.player.actor.in_combat);
    let attacked_by_player =
        local.is_some_and(|lp| api::snapshot::attacked_by_player(lp.player.actor.face_entity));
    let animating =
        local.is_some_and(|lp| lp.player.actor.moving || lp.player.actor.animation != -1);
    let modals = snapshot.map(|s| s.modals());
    // The scene bank booths: the openable locs (`Use-quickly` is the
    // bankbooth op the pack bakes from `scripts/interface_bank/configs/
    // bank_booth.loc`). Only the tile is posted — the shim never reads a
    // loc definition.
    let booths = snapshot.map(|s| {
        s.locs()
            .iter()
            .filter(|l| {
                l.actions.iter().any(|a| {
                    a.as_deref()
                        .is_some_and(|a| a.eq_ignore_ascii_case("Use-quickly"))
                })
            })
            .map(|l| TileInput {
                x: l.tile.x,
                z: l.tile.z,
                level: l.tile.level,
            })
            .collect::<Vec<_>>()
    });
    let nearest_booth = snapshot.and_then(|s| {
        s.nearest_use_quickly_booth().map(|loc| NearestBoothInput {
            x: loc.tile.x,
            z: loc.tile.z,
            level: loc.tile.level,
            id: loc.id,
            name: loc.name.as_deref().unwrap_or("Bank booth"),
            op: "Use-quickly",
        })
    });
    let banks = world.map(|w| {
        use nav::pack::BankAccess;
        w.banks()
            .iter()
            .map(|b| {
                let (kind, op, choose) = match &b.access {
                    BankAccess::Booth { op } => ("booth", *op, None),
                    BankAccess::Npc { op, choose, .. } => ("npc", *op, choose.as_deref()),
                };
                BankStandInput {
                    name: &b.name,
                    x: b.tile.x,
                    z: b.tile.z,
                    level: b.tile.level,
                    kind,
                    op,
                    choose,
                }
            })
            .collect::<Vec<_>>()
    });
    let mut make_button_store: Vec<Vec<MakeButtonInput>> = Vec::new();
    let make_products: Vec<MakeProductInput<'_>> = snapshot
        .map(|s| {
            make_button_store = s
                .make_products()
                .iter()
                .map(|p| {
                    p.buttons
                        .iter()
                        .map(|b| MakeButtonInput {
                            qty: b.quantity,
                            com_id: b.component_id,
                        })
                        .collect()
                })
                .collect();
            s.make_products()
                .iter()
                .enumerate()
                .map(|(i, p)| MakeProductInput {
                    object_id: p.object_id,
                    name: p.name.as_str(),
                    buttons: &make_button_store[i],
                })
                .collect()
        })
        .unwrap_or_default();
    let side_tab_ifaces: Vec<SideTabIfaceInput> = snapshot
        .map(|s| {
            s.side_tabs()
                .iter()
                .map(|t| SideTabIfaceInput {
                    index: t.index,
                    id: t.root_component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let spell_store = snapshot.map(|s| {
        let root = s
            .side_tabs()
            .iter()
            .find(|t| t.index == 6)
            .map(|t| t.root_component_id)
            .unwrap_or(-1);
        if root == -1 {
            Vec::new()
        } else {
            s.widgets()
                .iter()
                .chain(s.side_tabs().iter().flat_map(|t| t.widgets.iter()))
                .filter(|w| {
                    w.root_component_id == root
                        && w.button_type == 2
                        && w.target_base.as_deref().is_some_and(|t| !t.is_empty())
                })
                .map(|w| (w.target_base.clone().unwrap_or_default(), w.component_id))
                .collect::<Vec<_>>()
        }
    });
    let spell_buttons: Vec<CombatStyleInput<'_>> = spell_store
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|(label, component_id)| CombatStyleInput {
                    mode: 0,
                    label,
                    component_id: *component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let chat_lines: Vec<ChatLineInput<'_>> = snapshot
        .map(|s| {
            s.chat_lines()
                .iter()
                .map(|l| ChatLineInput {
                    seq: l.sequence,
                    text: l.text.as_str(),
                    type_: l.type_,
                    username: l.username.as_deref(),
                })
                .collect()
        })
        .unwrap_or_default();
    let widget_store: Vec<(i32, String)> = snapshot
        .map(|s| {
            s.widgets()
                .iter()
                .filter_map(|w| w.text.as_ref().map(|text| (w.component_id, text.clone())))
                .collect()
        })
        .unwrap_or_default();
    let widgets: Vec<WidgetTextInput<'_>> = widget_store
        .iter()
        .map(|(component_id, text)| WidgetTextInput {
            component_id: *component_id,
            text,
        })
        .collect();
    let quest_statuses: Vec<QuestStatusInput<'_>> = snapshot
        .map(|s| {
            s.quest_statuses()
                .iter()
                .map(|quest| QuestStatusInput {
                    name: quest.name.as_str(),
                    status: quest.status().as_str(),
                })
                .collect()
        })
        .unwrap_or_default();
    let quest_statuses = snapshot
        .filter(|s| s.quest_statuses_available())
        .map(|_| quest_statuses.as_slice());
    let input = SnapshotInput {
        tick,
        here,
        ingame,
        inv: &inv,
        // The inv tab's slot count (28 when bound, 0 while the side icons
        // stay tutorial-locked): the `reader.inventorySize()` gate a
        // script's onStart parks on.
        inv_size: snapshot.map_or(0, |s| s.inventory_size()),
        stats: stats.as_deref().unwrap_or(&[]),
        booths: booths.as_deref().unwrap_or(&[]),
        nearest_booth,
        banks: banks.as_deref().unwrap_or(&[]),
        bank: &bank,
        bank_side: &bank_side,
        bank_open: snapshot.is_some_and(|s| s.bank_component_id() != -1),
        bank_loaded: snapshot.is_some_and(GameSnapshot::bank_loaded),
        bank_generation: snapshot.map_or(0, GameSnapshot::bank_session_generation),
        count_dialog_open: snapshot.is_some_and(GameSnapshot::count_dialog_open),
        withdraw_x_result_seq,
        withdraw_x_result,
        withdraw_load_result_seq,
        withdraw_load_result,
        bank_op_result_seq,
        bank_op_result,
        hold,
        ours,
        npcs: &npcs,
        locs: &locs,
        players: &players,
        ground: &ground,
        equipment: &equipment,
        chat_open: modals.is_some_and(|m| m.chat != -1),
        chat_continue: snapshot.is_some_and(|s| s.chat_continue_component_id() != -1),
        chat_text: snapshot.and_then(|s| s.chat()),
        chat_options: chat_options.as_deref().unwrap_or(&[]),
        side_tab: snapshot.map(|s| s.active_side_tab()).unwrap_or(-1),
        varps: varps.as_deref().unwrap_or(&[]),
        combat_styles: &combat_styles,
        run_energy: snapshot.map(|s| s.runenergy()).unwrap_or(0),
        run_enabled: snapshot.is_some_and(|s| api::snapshot::ReadContext::new(s).varp(173) != 0),
        retaliate_enabled: snapshot
            .is_some_and(|s| api::snapshot::ReadContext::new(s).varp(172) == 0),
        my_name,
        in_combat,
        animating,
        main_modal_id: modals.map(|m| m.main).unwrap_or(-1),
        chat_modal_id: modals.map(|m| m.chat).unwrap_or(-1),
        make_products: &make_products,
        side_tab_ifaces: &side_tab_ifaces,
        spell_buttons: &spell_buttons,
        chat_lines: &chat_lines,
        bank_note_on: snapshot
            .and_then(|s| s.bank_note_controls())
            .map(|c| c.on_component_id)
            .unwrap_or(-1),
        bank_note_off: snapshot
            .and_then(|s| s.bank_note_controls())
            .map(|c| c.off_component_id)
            .unwrap_or(-1),
        scene_state: snapshot.map(|s| s.scene_state()).unwrap_or(0),
        weight: snapshot
            .and_then(|s| s.local_player())
            .map(|lp| lp.weight)
            .unwrap_or(0),
        combat_level: snapshot
            .and_then(|s| s.local_player())
            .map(|lp| lp.player.combat_level)
            .unwrap_or(0),
        camera_yaw: snapshot.map(|s| s.camera().orbit_yaw).unwrap_or(0),
        camera_pitch: snapshot.map(|s| s.camera().orbit_pitch).unwrap_or(0),
        teleports_enabled,
        self_slot: snapshot.map(|s| s.self_slot()).unwrap_or(-1),
        trade_offer_open: snapshot.is_some_and(|s| s.trade().offer_open),
        trade_confirm_open: snapshot.is_some_and(|s| s.trade().confirm_open),
        trade_partner: snapshot.and_then(|s| s.trade().partner.as_deref()),
        trade_mine: &trade_mine,
        trade_theirs: &trade_theirs,
        trade_side: &trade_side,
        trade_accept_id: snapshot
            .map(|s| s.trade().accept_component_id)
            .unwrap_or(-1),
        trade_decline_id: snapshot
            .map(|s| s.trade().decline_component_id)
            .unwrap_or(-1),
        shop_open: snapshot.is_some_and(|s| s.shop().open),
        shop_stock: &shop_stock,
        reach,
        attacked_by_player,
        widgets: &widgets,
    };
    let bank_approach_store: Vec<BankApproachInput> = match (snapshot, here, flood.as_ref()) {
        (Some(s), Some(tile), Some(flood)) => s
            .locs()
            .iter()
            .filter_map(|loc| {
                let name = loc.name.as_deref()?;
                if !name
                    .as_bytes()
                    .windows(4)
                    .any(|word| word.eq_ignore_ascii_case(b"bank"))
                {
                    return None;
                }
                let approach = api::query::loc_approach::booth_approach(
                    loc,
                    s.scene(),
                    WorldTile {
                        x: tile.x,
                        z: tile.z,
                        level: tile.level,
                    },
                    flood,
                )?;
                Some(BankApproachInput {
                    loc_id: loc.id,
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    can_operate: approach.can_operate,
                    dest_ok: approach.dest.is_some(),
                    dest_x: approach.dest.map(|t| t.x).unwrap_or(0),
                    dest_z: approach.dest.map(|t| t.z).unwrap_or(0),
                    dest_level: approach.dest.map(|t| t.level).unwrap_or(0),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    let latest_hops = inspect
        .latest
        .as_ref()
        .map(|t| route_inspect::hop_inputs(&t.hops))
        .unwrap_or_default();
    let prev_hops = inspect
        .prev
        .as_ref()
        .map(|t| route_inspect::hop_inputs(&t.hops))
        .unwrap_or_default();
    let latest_in = inspect
        .latest
        .as_ref()
        .map(|t| route_inspect::terminal_to_input(t, &latest_hops))
        .unwrap_or_default();
    let prev_in = inspect
        .prev
        .as_ref()
        .map(|t| route_inspect::terminal_to_input(t, &prev_hops))
        .unwrap_or_default();
    let native = NativeFactsInput {
        self_chat: snapshot.and_then(GameSnapshot::local_overhead_text),
        hint_tile: snapshot
            .and_then(GameSnapshot::hint_tile)
            .map(|tile| (tile.x, tile.z)),
        retaliate_controls: snapshot
            .and_then(GameSnapshot::retaliate_controls)
            .map(|controls| (controls.on_component_id, controls.off_component_id)),
        quest_statuses,
        npc_boxes,
        shop_player,
        main_make,
        bank_approaches: Some(&bank_approach_store),
        walk_outcome_seq: walk_outcome.seq,
        walk_outcome_generation: walk_outcome.generation,
        walk_outcome_request_id: walk_outcome.request_id,
        walk_outcome_failed: walk_outcome.failed,
        walk_outcome_x: walk_outcome.x,
        walk_outcome_z: walk_outcome.z,
        walk_outcome_level: walk_outcome.level,
        walk_outcome_radius: walk_outcome.radius,
        walk_outcome_allow_teleports: walk_outcome.allow_teleports,
        route_inspect: script::isolate_fb::RouteInspectFactsInput {
            latest: latest_in,
            prev: prev_in,
            running_id: inspect.running_id,
            pending_id: inspect.pending_id,
            accepted_id: inspect.accepted_id,
            replaced_id: inspect.replaced_id,
            replaced_prev_id: inspect.replaced_prev_id,
        },
    };
    f(&input, native)
}
/// Whether this observe pass needs a [`WorldState`] from the slot snapshot.

/// Built only for a Running script (walk arm) or an armed nav bot (route /
/// BankBudget session — interact dispatch may walk with gating facts).
pub(super) fn nav_world_state_for_observe(
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

/// the per-observe inventory view (the observe re-checks the gate inside).
pub(super) fn script_running(scripts: &ScriptWall, name: &str) -> bool {
    script_slot(scripts, name)
        .is_some_and(|s| s.lock().unwrap().state() == script::RunState::Running)
}

/// `name`'s slot script's latest paint frame, `None` for a slot with no
/// script or a script that has not painted. Copied onto the status row
/// each observe so the TUI can show paint-as-chat without a probe
/// round-trip (the isolate forwards the frame after each tick).
pub(super) fn script_paint_of(
    scripts: &ScriptWall,
    name: &str,
) -> Option<script::shim::ScriptPaint> {
    script_slot(scripts, name).and_then(|s| s.lock().unwrap().paint())
}

/// Copy `paint` onto `status.script_paint` only when the frame changed.
pub(super) fn publish_script_paint(
    status: &mut SlotStatus,
    paint: Option<&script::shim::ScriptPaint>,
) {
    match (&status.script_paint, paint) {
        (Some(cur), Some(next)) if cur == next => {}
        (None, None) => {}
        _ => status.script_paint = paint.cloned(),
    }
}

/// Whether `paint` advertises `select_name` on the persistent chrome store
/// `key` (`strip:{id}`, `rail:{id}`, or `tabs:{id}`).
pub(super) fn script_paint_select_advertised(
    paint: &script::shim::ScriptPaint,
    key: &str,
    select_name: &str,
) -> bool {
    if select_name.is_empty() {
        return false;
    }
    if let Some(id) = key.strip_prefix("strip:") {
        return paint.strip.as_ref().is_some_and(|band| {
            band.id == id && band.names.iter().any(|n| n == select_name)
        });
    }
    if let Some(id) = key.strip_prefix("rail:") {
        return paint.rail.as_ref().is_some_and(|band| {
            band.id == id && band.names.iter().any(|n| n == select_name)
        });
    }
    if let Some(id) = key.strip_prefix("tabs:") {
        return paint.tabs.iter().any(|band| {
            band.id == id && band.names.iter().any(|n| n == select_name)
        });
    }
    false
}

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route (found off-pump over the shared
/// [`NavWorld`]); the slot pump polls [`Traveller::follow`] with a clone of
/// it one step per player-info tick. `route` being set is the "armed"
/// gate the walk hook and the busy flag read. A pending BankBudget
/// session freezes follow until its steps finish.
#[derive(Default)]
pub(super) struct NavBot {
    pub(super) route_generation: u64,
    pub(super) route_worker: Option<Arc<()>>,
    pub(super) pending_route: Option<ScriptRouteRequest>,
    /// Dest, radius, allow_teleports, allow_wilderness, allow_bank_fetch.
    pub(super) requested_route: Option<(WorldTile, i32, bool, bool, bool)>,
    pub(super) traveller: Traveller,
    pub(super) route: Option<Route>,
    pub(super) bank_fetch: Option<PendingBankFetch>,
    /// Isolate-allocated walk request id for the armed / in-flight find.
    /// Distinct from `route_generation`, which remains the worker / retained-route token.
    pub(super) walk_request_id: u64,
    /// Unpublished current-wait refusal id, distinct from `walk_request_id`.
    /// Older armed-route NoPath / mid-follow terminals must not overwrite this
    /// published outcome until a snapshot copies it.
    pub(super) walk_live_refusal_id: u64,
    /// Last packed-walk `allow_teleports` opt-in (`Traversal.teleportsEnabled`).
    pub(super) allow_teleports: bool,
    /// Bounded published walk outcome. Seq `0` means never published.
    pub(super) walk_outcome_seq: u64,
    pub(super) walk_outcome_generation: u64,
    pub(super) walk_outcome_request_id: u64,
    pub(super) walk_outcome_failed: bool,
    pub(super) walk_outcome_x: i32,
    pub(super) walk_outcome_z: i32,
    pub(super) walk_outcome_level: i32,
    pub(super) walk_outcome_radius: i32,
    pub(super) walk_outcome_allow_teleports: bool,
    pub(super) inspect: route_inspect::InspectNav,
}

/// The shared script walk arm: both `ctx.walk` (default options) and
/// `ctx.walk_with` (explicit options) route through
/// [`ScriptWalkArm::route`]. Each observe clones the arm once per hook
/// (all fields are `Clone`), so the two `&mut` hooks never share a
/// mutable borrow.
#[derive(Clone)]
pub(super) struct ScriptWalkArm {
    pub(super) here: Option<(i32, i32, i32)>,
    pub(super) world: Option<Arc<NavWorld>>,
    pub(super) navs: Arc<Mutex<HashMap<String, NavBot>>>,
    pub(super) name: String,
    /// The slot's gating facts at arm time (from its live snapshot);
    /// `None` when no player is decoded — the worker then routes with
    /// the fail-closed empty [`WorldState`].
    pub(super) state: Option<WorldState>,
    /// Open bank rows (obj id, count) at arm time — empty when closed.
    pub(super) bank: Vec<(i32, i32)>,
}

fn walk_arm_outcome_tag(outcome: &RouteOutcome) -> &'static str {
    match outcome {
        RouteOutcome::Routed(_) => "Routed",
        RouteOutcome::BankSession { .. } => "BankSession",
        RouteOutcome::NoPath => "NoPath",
    }
}

fn log_walk_arm(name: &str, build: impl FnOnce() -> String) {
    if debug_enabled() {
        eprintln!("[nav-walk-arm {name}] {}", build());
    }
}

fn walk_arm_worker_slot() -> String {
    thread::current()
        .name()
        .and_then(|n| n.strip_prefix("nav-find-").map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}

fn log_walk_arm_bot(build: impl FnOnce() -> String) {
    if debug_enabled() {
        eprintln!("[nav-walk-arm] {}", build());
    }
}

impl ScriptWalkArm {
    /// Queue one walk toward `(x, z, level)` with `opts`, routing off-pump
    /// on a short-lived worker (`find_with` over the shared [`NavWorld`]).
    /// When `allow_bank_fetch` is on and the strict find fails only on
    /// missing item/worn reqs, latches a [`PendingBankFetch`] session and
    /// the post-session route. Refuses synchronously only when there is
    /// no player tile, no nav world, or a route/session already queued;
    /// the worker stores the outcome on the uid's nav bot. Returns whether
    /// the worker was spawned — not whether a path exists.
    pub(super) fn route(&self, x: i32, z: i32, level: i32, opts: FindOptions) -> bool {
        self.queue_route(x, z, level, opts, 0, false, 0)
    }
    /// Explicit WalkNear, including radius 0. Unlike [`Self::route`], an
    /// armed or in-flight route is replaced through the existing generation /
    /// pending-route coalescing path. A latched bank-fetch session still refuses.
    pub(super) fn route_with_radius(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
    ) -> bool {
        self.queue_route(x, z, level, opts, radius, true, 0)
    }
    fn publish_refusal(&self, to: WorldTile, radius: i32, allow_teleports: bool, request_id: u64) {
        let mut navs = self.navs.lock().unwrap();
        let bot = navs.entry(self.name.clone()).or_default();
        // Do not bump route_generation or clear a retained route / bank-fetch.
        bot.note_failure(
            bot.route_generation,
            request_id,
            to,
            radius,
            allow_teleports,
        );
    }

    pub(super) fn queue_route(
        &self,
        x: i32,
        z: i32,
        level: i32,
        mut opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        let to = WorldTile { x, z, level };
        let Some((hx, hz, hl)) = self.here else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-here dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.publish_refusal(to, radius, opts.allow_teleports, request_id);
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-world dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.publish_refusal(to, radius, opts.allow_teleports, request_id);
            return false;
        };
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let token = {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if bot.bank_fetch.is_some()
                || (!retarget && (bot.route.is_some() || bot.route_worker.is_some()))
            {
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route refused-in-flight dest={to:?} r={radius} request_id={request_id} \
                         bank_fetch={} retarget={retarget} route={} worker={}",
                        bot.bank_fetch.is_some(),
                        bot.route.is_some(),
                        bot.route_worker.is_some()
                    )
                });
                bot.note_failure(
                    bot.route_generation,
                    request_id,
                    to,
                    radius,
                    opts.allow_teleports,
                );
                return false;
            }
            let key = (
                to,
                radius,
                opts.allow_teleports,
                opts.allow_wilderness,
                opts.allow_bank_fetch,
            );
            if bot.requested_route == Some(key)
                && (bot.route_worker.is_some()
                    || bot.route.is_some()
                    || bot.pending_route.is_some())
            {
                // Same-id retransmission and legacy request_id 0 keep the
                // in-flight find. A later distinct nonzero wait is refused
                // without restarting search or reassigning the armed id.
                if request_id != 0 && request_id != bot.walk_request_id {
                    log_walk_arm(&self.name, || {
                        format!(
                            "queue_route coalesced-refused-distinct-id dest={to:?} r={radius} \
                             request_id={request_id} armed_id={}",
                            bot.walk_request_id
                        )
                    });
                    bot.note_failure(
                        bot.route_generation,
                        request_id,
                        to,
                        radius,
                        opts.allow_teleports,
                    );
                    return false;
                }
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route coalesced dest={to:?} r={radius} request_id={request_id} \
                         generation={}",
                        bot.route_generation
                    )
                });
                return true;
            }
            if let Some(ess) = bot.traveller.essence() {
                opts.essence = Some(ess);
            }
            bot.route_generation = bot.route_generation.wrapping_add(1);
            bot.walk_request_id = request_id;
            bot.requested_route = Some(key);
            bot.pending_route = Some(ScriptRouteRequest {
                generation: bot.route_generation,
                request_id,
                world: Arc::clone(world),
                from,
                to,
                radius,
                opts,
                state: self.state.clone(),
                bank: self.bank.clone(),
            });
            if bot.route_worker.is_some() {
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route handed-pending dest={to:?} r={radius} request_id={request_id} \
                         generation={} from={from:?}",
                        bot.route_generation
                    )
                });
                return true;
            }
            let token = Arc::new(());
            bot.route_worker = Some(Arc::clone(&token));
            token
        };
        let navs = Arc::clone(&self.navs);
        let name = self.name.clone();
        let worker_token = Arc::clone(&token);
        log_walk_arm(&self.name, || {
            format!(
                "queue_route spawned dest={to:?} r={radius} request_id={request_id} from={from:?} \
                 allow_teleports={} allow_wilderness={} allow_bank_fetch={}",
                opts.allow_teleports, opts.allow_wilderness, opts.allow_bank_fetch
            )
        });
        let spawned = thread::Builder::new()
            .name(format!("nav-find-{name}"))
            .spawn(move || loop {
                let request = {
                    let mut all = navs.lock().unwrap();
                    let Some(bot) = all.get_mut(&name) else {
                        log_walk_arm(&name, || "worker exit bot-gone".to_string());
                        return;
                    };
                    if !bot
                        .route_worker
                        .as_ref()
                        .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                    {
                        log_walk_arm(&name, || {
                            "worker discard stale-token before dequeue".to_string()
                        });
                        return;
                    }
                    let Some(request) = bot.pending_route.take() else {
                        bot.route_worker = None;
                        log_walk_arm(&name, || "worker exit no-pending-route".to_string());
                        return;
                    };
                    request
                };
                let debug = debug_enabled();
                if debug {
                    log_walk_arm(&name, || {
                        format!(
                            "worker calculate begin generation={} request_id={} from={:?} dest={:?} r={}",
                            request.generation,
                            request.request_id,
                            request.from,
                            request.to,
                            request.radius
                        )
                    });
                }
                let started = debug.then(Instant::now);
                let outcome = request.calculate();
                if debug {
                    let elapsed_ms = started.unwrap().elapsed().as_millis();
                    log_walk_arm(&name, || {
                        format!(
                            "worker calculate end generation={} request_id={} elapsed_ms={elapsed_ms} \
                             outcome={}",
                            request.generation,
                            request.request_id,
                            walk_arm_outcome_tag(&outcome)
                        )
                    });
                }
                let mut all = navs.lock().unwrap();
                let Some(bot) = all.get_mut(&name) else {
                    log_walk_arm(&name, || "worker exit bot-gone after calculate".to_string());
                    return;
                };
                if !bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                {
                    log_walk_arm(&name, || {
                        format!(
                            "worker discard stale-token after calculate generation={} \
                             live_generation={}",
                            request.generation, bot.route_generation
                        )
                    });
                    return;
                }
                bot.publish_route(
                    request.generation,
                    request.request_id,
                    request.opts.allow_teleports,
                    outcome,
                );
            })
            .is_ok();
        if !spawned {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route spawn-failed dest={to:?} r={radius} request_id={request_id}"
                )
            });
            if let Some(bot) = self.navs.lock().unwrap().get_mut(&self.name) {
                if bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &token))
                {
                    if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                        bot.note_failure(
                            bot.route_generation,
                            bot.walk_request_id,
                            to,
                            radius,
                            allow_teleports,
                        );
                    }
                    bot.route_worker = None;
                    bot.pending_route = None;
                    bot.requested_route = None;
                }
            }
        }
        spawned
    }
}

/// Mid-follow Stall / Refused / Blocked / GaveUp publish a failed outcome
/// with the armed walk's isolate request id so the matching wait returns
/// false. Arrival still settles from `here`. Only genuinely pending follow
/// work (None) keeps the caller timeout.
pub(super) fn apply_nav_follow_outcome(
    bot: &mut NavBot,
    outcome: Option<nav::traveller::TravelOutcome>,
    walking_stand: bool,
) {
    match outcome {
        Some(nav::traveller::TravelOutcome::Arrived { .. }) => {
            bot.route = None;
        }
        Some(_) => {
            if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                if bot.armed_outcome_may_publish(bot.walk_request_id) {
                    bot.note_failure(
                        bot.route_generation,
                        bot.walk_request_id,
                        to,
                        radius,
                        allow_teleports,
                    );
                }
            } else if let Some(route) = bot.route.as_ref() {
                if bot.armed_outcome_may_publish(bot.walk_request_id) {
                    bot.note_failure(
                        bot.route_generation,
                        bot.walk_request_id,
                        route.dest,
                        0,
                        bot.allow_teleports,
                    );
                }
            }
            bot.route = None;
            if walking_stand {
                bot.bank_fetch = None;
            }
        }
        None => {}
    }
}

/// One pump step of a uid's nav bot: advance a pending BankBudget session
/// first (Wear / deposit / withdraw), then poll the armed route through
/// [`Traveller::follow`] one step against `snapshot`. `here` is the
/// player's world tile when the body decoded one (else the bot stands
/// still). `world` is the shared nav world, `None` when no pack loaded —
/// its packed any-tile teleport list rides into the follow so a jewellery
/// rub hop answers the destination dialog's choice for its landing (the
/// same pass-through the panel and scenario follow make). Mirrors the
/// armed route's dest into the status row's `walk_*` fields (`-1` when
/// idle); any terminal outcome clears the route — arrival and stall alike
/// — so the status flips back to idle and a script may arm a fresh walk.
/// Mid-follow Stall / Refused / Blocked / GaveUp publish the armed walk's
/// isolate request id as a failed outcome. Arrival still settles from `here`.
// Shared handles threaded like `script_observe`; the arg count is allowed.
#[allow(clippy::too_many_arguments)]
pub(super) fn step_nav_bot<D: Driver>(
    driver: &mut D,
    name: &str,
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<&NavWorld>,
    hold: bool,
    map_members: bool,
) {
    // The random-event freeze: the follow is not stepped while the
    // guardian holds the slot, and the armed route stays latched so it
    // resumes when the hold lifts. BankBudget steps freeze the same way.
    if here.is_none() || hold {
        return;
    }
    {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            if bot.bank_fetch.is_some() {
                step_bank_fetch_on_bot(driver, snapshot, bot, world, here, map_members);
                // Freeze follow for Open / Deposit / Withdraw / Wear /
                // Close. Walk with a stand sub-route armed falls through
                // to Traveller::follow — never final_route mid-session.
                if bank_fetch_freezes_follow(bot) {
                    let queued = bot.route.as_ref().map(|r| r.dest);
                    drop(all);
                    let mut rows = statuses.lock().unwrap();
                    if let Some(s) = rows.iter_mut().find(|s| s.username == name) {
                        match queued {
                            Some(d) => {
                                s.walk_x = d.x;
                                s.walk_z = d.z;
                                s.walk_level = d.level;
                            }
                            None => {
                                s.walk_x = -1;
                                s.walk_z = -1;
                                s.walk_level = -1;
                            }
                        }
                    }
                    return;
                }
            }
        }
    }
    let mut options = TravelOptions {
        // Exact arrival: the armed dest must be stood on before the route
        // clears (the v1 traveller arrived the same way).
        close_enough: 0,
        teleports: world.map(|w| w.graph.teleports.as_slice()),
        edges: world.map(|w| w.graph.edges.as_slice()),
        ..TravelOptions::default()
    };
    let queued = {
        let mut all = navs.lock().unwrap();
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        let Some(route) = bot.route.clone() else {
            return;
        };
        let walking_stand = bot.bank_fetch.as_ref().is_some_and(|p| {
            matches!(
                p.steps.front(),
                Some(BankStep::Walk { x, z, level })
                    if route.dest.x == *x && route.dest.z == *z && route.dest.level == *level
            )
        });
        let follow_outcome = bot.traveller.follow(driver, snapshot, route, &mut options);
        apply_nav_follow_outcome(bot, follow_outcome, walking_stand);
        bot.route.as_ref().map(|r| r.dest)
    };
    let mut rows = statuses.lock().unwrap();
    if let Some(s) = rows.iter_mut().find(|s| s.username == name) {
        match queued {
            Some(d) => {
                s.walk_x = d.x;
                s.walk_z = d.z;
                s.walk_level = d.level;
            }
            None => {
                s.walk_x = -1;
                s.walk_z = -1;
                s.walk_level = -1;
            }
        }
    }
}

/// Advance one BankBudget session step on a [`NavBot`]. Walk completes
/// when the player is already on the stand tile (or a sub-route is
/// armed for follow); Open is a no-op while the bank is already
/// open+loaded; DepositAll / Withdraw / Wear / Close dispatch through
/// [`api::interact::Interactions`]. Clears the pending session when
/// steps are exhausted, or on walk/open/withdraw failure (NoPath).
/// Returns whether the driver was written.
pub(super) fn step_bank_fetch_on_bot<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    bot: &mut NavBot,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
    map_members: bool,
) -> bool {
    let Some(pending) = bot.bank_fetch.as_mut() else {
        return false;
    };
    let Some(step) = pending.steps.front().cloned() else {
        bot.bank_fetch = None;
        return false;
    };
    let mut abort = false;
    let wrote = match step {
        BankStep::Walk { x, z, level } => {
            if here == Some((x, z, level)) {
                pending.steps.pop_front();
                log_walk_arm_bot(|| {
                    format!("bank_fetch phase done Walk on-stand ({x},{z},{level})")
                });
                // Restore the post-session route for follow / status.
                bot.route = Some(pending.final_route.clone());
                false
            } else if bot
                .route
                .as_ref()
                .is_some_and(|r| r.dest.x == x && r.dest.z == z && r.dest.level == level)
            {
                // Stand sub-route armed; follow polls it outside this step.
                false
            } else if let Some(w) = world {
                let from = match here {
                    Some((hx, hz, hl)) => WorldTile {
                        x: hx,
                        z: hz,
                        level: hl,
                    },
                    None => return false,
                };
                let to = WorldTile { x, z, level };
                // Live snapshot facts (same fail-closed gates as execute),
                // not an empty WorldState that would refuse gated walks.
                let state = WorldState::from_snapshot(snapshot).with_map_members(map_members);
                let opts = FindOptions {
                    allow_bank_fetch: false,
                    ..pending.opts
                };
                match find_with(&w.collision, &w.graph, from, to, opts, &state) {
                    Ok(route) => {
                        log_walk_arm_bot(|| {
                            format!("bank_fetch Walk armed sub-route dest={to:?}")
                        });
                        bot.route = Some(route);
                        false
                    }
                    Err(_) => {
                        abort = true;
                        false
                    }
                }
            } else {
                abort = true;
                false
            }
        }
        BankStep::Open => {
            if snapshot.bank_loaded() {
                pending.steps.pop_front();
                log_walk_arm_bot(|| "bank_fetch phase done Open already-loaded".to_string());
                false
            } else if snapshot.bank_component_id() == -1 {
                let sent = open_bank_at_here(driver, snapshot, here, world);
                if sent {
                    log_walk_arm_bot(|| "bank_fetch Open sent Use-quickly".to_string());
                }
                sent
            } else {
                false
            }
        }
        BankStep::DepositAll => {
            let wrote = deposit_all_backpack(driver, snapshot);
            pending.steps.pop_front();
            log_walk_arm_bot(|| format!("bank_fetch phase done DepositAll wrote={wrote}"));
            wrote
        }
        BankStep::Withdraw { id, count } => {
            let wrote = withdraw_id(driver, snapshot, id, count);
            if wrote {
                pending.steps.pop_front();
                log_walk_arm_bot(|| {
                    format!("bank_fetch phase done Withdraw id={id} count={count}")
                });
            } else {
                abort = true;
            }
            wrote
        }
        BankStep::Wear { id } => {
            let mut ix = api::interact::Interactions::new(snapshot, driver);
            let wrote = matches!(ix.wear(id), api::interact::SendResult::Sent { .. });
            pending.steps.pop_front();
            log_walk_arm_bot(|| format!("bank_fetch phase done Wear id={id} wrote={wrote}"));
            wrote
        }
        BankStep::Close => {
            let mut ix = api::interact::Interactions::new(snapshot, driver);
            let wrote = matches!(ix.close_modal(), api::interact::SendResult::Sent { .. });
            pending.steps.pop_front();
            log_walk_arm_bot(|| format!("bank_fetch phase done Close wrote={wrote}"));
            wrote
        }
    };
    if abort {
        let session_dest = pending.dest;
        let remaining = pending.steps.len();
        log_walk_arm_bot(|| {
            format!(
                "bank_fetch abort front={step:?} session_dest={session_dest:?} remaining={remaining}"
            )
        });
        bot.bank_fetch = None;
        bot.route = None;
        return wrote;
    }
    if pending.steps.is_empty() {
        let session_dest = pending.dest;
        log_walk_arm_bot(|| {
            format!("bank_fetch session cleared complete session_dest={session_dest:?}")
        });
        bot.bank_fetch = None;
    }
    wrote
}

/// Whether a latched BankBudget session must freeze [`Traveller::follow`].
/// Walk with the stand sub-route armed does **not** freeze; Open /
/// Deposit / Withdraw / Wear / Close do. Mid-session `final_route` is
/// never followed.
pub(super) fn bank_fetch_freezes_follow(bot: &NavBot) -> bool {
    let Some(pending) = bot.bank_fetch.as_ref() else {
        return false;
    };
    match pending.steps.front() {
        Some(BankStep::Walk { x, z, level }) => {
            // Freeze only until the stand sub-route is armed; once armed,
            // follow that route. If route still points at final_route,
            // stay frozen this tick (Walk arms next pump / this pump).
            !bot.route
                .as_ref()
                .is_some_and(|r| r.dest.x == *x && r.dest.z == *z && r.dest.level == *level)
        }
        Some(_) => true,
        None => false,
    }
}

fn open_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    // Prefer a packed booth stand's tile; else any Use-quickly loc.
    let target_tile = world.and_then(|w| {
        here.and_then(|(hx, hz, hl)| {
            w.banks()
                .iter()
                .min_by_key(|s| {
                    (
                        s.tile.level != hl,
                        (s.tile.x - hx).abs().max((s.tile.z - hz).abs()),
                    )
                })
                .map(|s| (s.tile.x, s.tile.z, s.tile.level))
        })
    });
    if let Some((x, z, level)) = target_tile {
        if let Some(loc) = snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)
        {
            if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
                return matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                );
            }
        }
    }
    for loc in snapshot.locs() {
        if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
            return matches!(
                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    false
}

fn deposit_all_backpack<D: Driver>(driver: &mut D, snapshot: &GameSnapshot) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let mut wrote = false;
    for item in snapshot.bank_side() {
        if let Some(op) = all_slot(&item.actions) {
            wrote |= matches!(
                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    wrote
}

fn withdraw_id<D: Driver>(driver: &mut D, snapshot: &GameSnapshot, id: i32, count: i32) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let Some(item) = snapshot.bank().iter().find(|it| it.def.id == id) else {
        return false;
    };
    let label = match count {
        1 => "Withdraw 1",
        5 => "Withdraw 5",
        10 => "Withdraw 10",
        _ => "Withdraw All",
    };
    if let Some(op) = action_slot(&item.actions, label)
        .or_else(|| action_slot(&item.actions, "Withdraw 1"))
        .or_else(|| action_slot(&item.actions, "Withdraw All"))
    {
        return matches!(
            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
            SendResult::Sent { .. }
        );
    }
    false
}

/// Borrow the current session's inventory only on running tick edges.
pub(super) fn observe_script_inv(
    running: bool,
    tick_edge: bool,
    snapshot: &GameSnapshot,
) -> Option<&[(i32, i32)]> {
    (running && tick_edge).then(|| snapshot.inv())
}

/// Project only the client's bounded active-NPC list. A ready scene with no
/// projectable NPCs is an available empty update; a non-ready scene is
/// unavailable so isolates clear any prior geometry.
pub(super) fn projected_npc_boxes(client: &Client) -> Option<Vec<script::isolate_fb::NpcBoxInput>> {
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
pub(super) fn project_npc_boxes_for_isolate_snapshot<F>(
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

/// Candidate destinations for an explicit radius request. Exact walks retain
/// their old routing behavior. Bound enumeration to the loaded scene size.
pub(super) fn approach_tiles(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
) -> Vec<WorldTile> {
    let r = radius.clamp(0, 104);
    let mut tiles = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            let t = WorldTile {
                x: to.x + dx,
                z: to.z + dz,
                level: to.level,
            };
            if world.collision.standable(t) {
                tiles.push(t);
            }
        }
    }
    if world.collision.standable(to) {
        let connected = nav::router::local_step_component(&world.collision, to, r);
        tiles.retain(|tile| connected.contains(tile));
    }
    tiles.sort_by_key(|t| {
        (
            (t.x - from.x).abs().max((t.z - from.z).abs()),
            (t.x - to.x).abs().max((t.z - to.z).abs()),
            t.x,
            t.z,
        )
    });
    tiles
}

pub(super) struct ScriptRouteRequest {
    pub(super) generation: u64,
    pub(super) request_id: u64,
    pub(super) world: Arc<NavWorld>,
    pub(super) from: WorldTile,
    pub(super) to: WorldTile,
    pub(super) radius: i32,
    pub(super) opts: FindOptions,
    pub(super) state: Option<WorldState>,
    pub(super) bank: Vec<(i32, i32)>,
}
impl ScriptRouteRequest {
    pub(super) fn calculate(&self) -> RouteOutcome {
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        if self.radius <= 0 {
            return route_or_bank_fetch(
                &self.world,
                self.from,
                self.to,
                self.opts,
                state,
                &self.bank,
            );
        }
        let candidates = approach_tiles(&self.world, self.from, self.to, self.radius);
        let debug = debug_enabled();
        let slot = walk_arm_worker_slot();
        if debug {
            log_walk_arm(&slot, || {
                format!(
                    "approach enumerate dest={:?} r={} candidates={}",
                    self.to, self.radius, candidates.len()
                )
            });
        }
        for (idx, target) in candidates.into_iter().enumerate() {
            if debug {
                log_walk_arm(&slot, || {
                    format!(
                        "approach begin idx={idx} target={target:?} generation={} request_id={}",
                        self.generation, self.request_id
                    )
                });
            }
            let started = debug.then(Instant::now);
            let outcome =
                route_or_bank_fetch(&self.world, self.from, target, self.opts, state, &self.bank);
            if debug {
                let elapsed_ms = started.unwrap().elapsed().as_millis();
                log_walk_arm(&slot, || {
                    format!(
                        "approach end idx={idx} target={target:?} elapsed_ms={elapsed_ms} \
                         outcome={}",
                        walk_arm_outcome_tag(&outcome)
                    )
                });
            }
            if !matches!(outcome, RouteOutcome::NoPath) {
                return outcome;
            }
        }
        RouteOutcome::NoPath
    }
}
impl NavBot {
    fn bump_walk_outcome_seq(&mut self) {
        self.walk_outcome_seq = self.walk_outcome_seq.wrapping_add(1);
        if self.walk_outcome_seq == 0 {
            self.walk_outcome_seq = 1;
        }
    }

    /// Older armed-route results may publish only after the current wait
    /// refusal has been copied into a snapshot, or when they *are* that wait.
    fn armed_outcome_may_publish(&self, request_id: u64) -> bool {
        self.walk_live_refusal_id == 0 || request_id == self.walk_live_refusal_id
    }

    pub(super) fn mark_walk_outcome_posted(&mut self) {
        self.walk_live_refusal_id = 0;
    }

    pub(super) fn note_failure(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
    ) {
        // Legacy request id 0 never settles a wait. Do not replace a live
        // nonzero refusal with it before the isolate can observe the refusal.
        if request_id == 0 && self.walk_live_refusal_id != 0 {
            log_walk_arm_bot(|| {
                format!(
                    "note_failure skipped legacy-id dest={to:?} r={radius} request_id={request_id} \
                     live_refusal_id={}",
                    self.walk_live_refusal_id
                )
            });
            return;
        }
        log_walk_arm_bot(|| {
            format!(
                "note_failure generation={generation} walk_request_id={} request_id={request_id} \
                 dest={to:?} r={radius} allow_teleports={allow_teleports} seq={}",
                self.walk_request_id,
                self.walk_outcome_seq.wrapping_add(1)
            )
        });
        self.bump_walk_outcome_seq();
        self.walk_outcome_generation = generation;
        self.walk_outcome_request_id = request_id;
        self.walk_outcome_failed = true;
        self.walk_outcome_x = to.x;
        self.walk_outcome_z = to.z;
        self.walk_outcome_level = to.level;
        self.walk_outcome_radius = radius;
        self.walk_outcome_allow_teleports = allow_teleports;
        if request_id != 0 && request_id != self.walk_request_id {
            self.walk_live_refusal_id = request_id;
        } else {
            self.walk_live_refusal_id = 0;
        }
    }

    pub(super) fn clear_walk_outcome(&mut self) {
        self.bump_walk_outcome_seq();
        self.walk_outcome_failed = false;
        self.walk_outcome_generation = self.route_generation;
        self.walk_outcome_request_id = 0;
        self.walk_outcome_x = 0;
        self.walk_outcome_z = 0;
        self.walk_outcome_level = 0;
        self.walk_outcome_radius = 0;
        self.walk_outcome_allow_teleports = false;
        self.walk_live_refusal_id = 0;
    }

    pub(super) fn publish_route(
        &mut self,
        generation: u64,
        request_id: u64,
        allow_teleports: bool,
        outcome: RouteOutcome,
    ) {
        // Superseded workers and aborted/restarted runs keep a newer
        // route_generation. A late NoPath for the same dest must not publish.
        if self.route_generation != generation {
            log_walk_arm_bot(|| {
                format!(
                    "publish_route discard-stale-generation published={generation} \
                     live={} request_id={request_id} outcome={}",
                    self.route_generation,
                    walk_arm_outcome_tag(&outcome)
                )
            });
            return;
        }
        let (route, pending) = match outcome {
            RouteOutcome::Routed(route) => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route Routed request_id={request_id} walk_request_id={} \
                         route_dest={:?}",
                        self.walk_request_id, route.dest
                    )
                });
                (route, None)
            }
            RouteOutcome::BankSession { pending, route } => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route BankSession request_id={request_id} walk_request_id={} \
                         route_dest={:?} session_steps={} session_dest={:?}",
                        self.walk_request_id,
                        route.dest,
                        pending.steps.len(),
                        pending.dest
                    )
                });
                (route, Some(pending))
            }
            RouteOutcome::NoPath => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route NoPath request_id={request_id} walk_request_id={} \
                         may_publish={}",
                        self.walk_request_id,
                        self.armed_outcome_may_publish(request_id)
                    )
                });
                if let Some((to, radius, allow_teleports, ..)) = self.requested_route {
                    if self.armed_outcome_may_publish(request_id) {
                        self.note_failure(generation, request_id, to, radius, allow_teleports);
                    }
                }
                // The retained route belongs to the previous request. A later
                // request for this failed destination must be allowed to retry.
                self.requested_route = None;
                return;
            }
        };
        self.traveller.clear();
        self.route = Some(route);
        self.bank_fetch = pending;
        self.allow_teleports = allow_teleports;
    }
}

pub(super) fn reset_script_nav(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        nav.route_generation = nav.route_generation.wrapping_add(1);
        nav.route_worker = None;
        nav.pending_route = None;
        nav.requested_route = None;
        nav.traveller.clear();
        nav.route = None;
        nav.bank_fetch = None;
        nav.walk_request_id = 0;
        nav.clear_walk_outcome();
        route_inspect::reset_inspect(nav);
    }
}

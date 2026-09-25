use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use client::config::Cache;
use nav::router::FindOptions;
use nav::world::NavWorld;
use nav::WorldState;

use super::{
    abort_script_walk, action_slot, all_slot, route_inspect, NavBot,
    ScriptWalkArm,
};
use crate::catalog_core::ScriptAct;
use crate::debug_enabled;
#[cfg(feature = "memory-profile")]
use crate::memory_diagnostics;
fn act_tile(x: i32, z: i32, level: i32) -> crate::catalog_core::LineOfSightTile {
    crate::catalog_core::LineOfSightTile { x, z, level }
}

/// Note one game request host-play dispatched for `slot`'s script. The
/// catalog hunt watch reads this record; the isolate never sees it.
fn record_script_act(navs: &Arc<Mutex<HashMap<String, NavBot>>>, slot: &str, act: ScriptAct) {
    let mut navs = navs.lock().unwrap();
    match navs.get_mut(slot) {
        Some(bot) => bot.acts.record(act),
        None => navs.entry(slot.to_string()).or_default().acts.record(act),
    }
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
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_script_interact(
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
        driver, snapshot, obj_names, here, navs, world, state, name, reqs, None, None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_script_interact_cached(
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
    for req in &reqs {
        if let InteractReq::InspectAck { seq, generation } = req {
            if let Some(bot) = navs.lock().unwrap().get_mut(name) {
                bot.inspect.apply_ack(*seq, *generation);
            }
        }
    }
    // Arms below bind a request field called `name`; this is the slot's.
    let slot = name;
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
                let queued = arm.queue_route(
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
                if queued {
                    record_script_act(
                        navs,
                        slot,
                        ScriptAct::Walk {
                            dest: act_tile(x, z, level),
                            radius: 0,
                            exact: true,
                            allow_teleports,
                            allow_wilderness,
                            allow_bank_fetch,
                            request_id,
                        },
                    );
                }
                wrote |= queued;
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
                let queued = arm.queue_route(
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
                if queued {
                    record_script_act(
                        navs,
                        slot,
                        ScriptAct::Walk {
                            dest: act_tile(x, z, level),
                            radius,
                            exact: false,
                            allow_teleports,
                            allow_wilderness,
                            allow_bank_fetch,
                            request_id,
                        },
                    );
                }
                wrote |= queued;
            }
            // Not a game packet: the follow stops, nothing is written.
            InteractReq::AbortWalk { request_id } => {
                // Only the walk the machine itself armed; a later walk has
                // replaced it and is not the machine's to stop.
                let owned = request_id != 0
                    && navs
                        .lock()
                        .unwrap()
                        .get(name)
                        .is_some_and(|bot| bot.walk_request_id == request_id);
                if owned {
                    abort_script_walk(navs, name);
                }
            }
            InteractReq::SelectBank { x, z, level, allow_wilderness, use_mage_bank, use_zanaris_bank, request_id } => {
                super::bank::queue_bank_pick(
                    navs, name, world, state.clone(), WorldTile { x, z, level },
                    allow_wilderness, request_id,
                    api::named_banks::BankPreferences { use_mage_bank, use_zanaris_bank },
                    snapshot.stats().iter().find(|stat| stat.index == 10).map(|stat| stat.base),
                );
            }
            InteractReq::WalkNearestBank => {
                if let Some((x, z, level)) = here {
                    wrote |= super::bank::queue_bank_walk(
                        navs, name, world, state.clone(), WorldTile { x, z, level },
                        snapshot.stats().iter().find(|stat| stat.index == 10).map(|stat| stat.base),
                    );
                }
            }
            InteractReq::WalkTo { x, z, level } => {
                let sent = matches!(ix.walk(WorldTile { x, z, level }), SendResult::Sent { .. });
                if sent {
                    record_script_act(
                        navs,
                        slot,
                        ScriptAct::WalkTo {
                            dest: act_tile(x, z, level),
                        },
                    );
                }
                wrote |= sent;
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
            InteractReq::PuzzleMove {
                id,
                slot,
                component,
                generation,
            } => {
                // The posted board's own row under its own session
                // generation: never a name, never a co-located widget, and
                // never the bank session. A refused or stale move sends
                // nothing and closes nothing.
                wrote |= dispatch_puzzle_move(
                    &mut ix,
                    snapshot,
                    cache.as_deref(),
                    id,
                    slot,
                    component,
                    generation,
                );
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
                    let sent = matches!(
                        ix.interact(OpTarget::Npc(npc), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                    if sent {
                        record_script_act(
                            navs,
                            slot,
                            ScriptAct::Npc {
                                name: npc.name.clone().unwrap_or_default(),
                                action,
                                index: npc.index as i32,
                            },
                        );
                    }
                    wrote |= sent;
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
                    let sent = matches!(
                        ix.interact(OpTarget::Loc(loc), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                    if sent {
                        record_script_act(
                            navs,
                            slot,
                            ScriptAct::Loc {
                                tile: act_tile(loc.tile.x, loc.tile.z, loc.tile.level),
                                id: loc.id,
                                action,
                            },
                        );
                    }
                    wrote |= sent;
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
                        let on_loc = match &target {
                            OpTarget::Loc(loc) => {
                                Some(act_tile(loc.tile.x, loc.tile.z, loc.tile.level))
                            }
                            _ => None,
                        };
                        let item_name = obj_names
                            .and_then(|names| names.name(item.def.id))
                            .map_or_else(|| name.clone(), str::to_string);
                        let sent = matches!(ix.use_item_on(item, target), SendResult::Sent { .. });
                        if let (true, Some(tile)) = (sent, on_loc) {
                            record_script_act(
                                navs,
                                slot,
                                ScriptAct::UseOnLoc {
                                    item: item_name,
                                    tile,
                                },
                            );
                        }
                        wrote |= sent;
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
            InteractReq::Unequip { name } => {
                let id = snapshot.equipment().iter().find_map(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .filter(|n| n.eq_ignore_ascii_case(&name))
                        .map(|_| it.def.id)
                });
                if let Some(id) = id {
                    let res = ix.unequip(id);
                    if host::debug_enabled() {
                        let outcome = match &res {
                            SendResult::Sent { .. } => "sent".to_string(),
                            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
                        };
                        eprintln!(
                            "[shim-unequip] {name} id={id} worn={} -> {outcome}",
                            snapshot.equipment().len()
                        );
                    }
                    wrote |= matches!(res, SendResult::Sent { .. });
                } else if host::debug_enabled() {
                    eprintln!(
                        "[shim-unequip] {name} no equipment() row worn={}",
                        snapshot.equipment().len()
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

/// The frozen `PUZZLE_SIZE` (5×5) a live puzzle board reports. The posted
/// size stays the observation; this is only the puzzle-move send gate.
const PUZZLE_SIZE: i32 = 25;

/// One `puzzle-move` click: the exact posted board row (id/slot/component)
/// under its own `puzzle_session_generation`, sent as a send-time
/// Held-family view (`Move`, else the frozen index-5 op) through the
/// OPHELD opcode family. A closed board, a board whose observed size is not
/// the frozen 25, a stale generation, a row the board no longer holds, a
/// missing obj table and a fifth op that is only the padded `Drop` default
/// all send nothing — and a sent packet is not board progress, so nothing
/// here waits on or rewrites the board.
fn dispatch_puzzle_move(
    ix: &mut api::interact::Interactions<'_>,
    snapshot: &GameSnapshot,
    cache: Option<&Cache>,
    id: i32,
    slot: i32,
    component: i32,
    generation: u64,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let board = snapshot.puzzle_board();
    if component < 0 || board.component_id != component {
        return false;
    }
    if board.generation != generation || board.size != PUZZLE_SIZE {
        return false;
    }
    let Some(row) = board
        .items
        .iter()
        .find(|item| item.def.id == id && item.slot == slot && item.component_id == component)
    else {
        return false;
    };
    let Some(cache) = cache else {
        return false;
    };
    let view = api::snapshot::ItemView {
        def: row.def.clone(),
        container: api::snapshot::ItemContainer::Widget,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot: row.slot,
        count: row.count,
        actions: api::snapshot::cache_held_ops(cache, id),
        component_id: row.component_id,
    };
    let target = OpTarget::Item(&view);
    let action = if api::interact::operation_of(&target, "Move").is_some() {
        // The frozen click's labelled path: `Move` wins wherever the obj
        // table has it, and `interact` re-resolves the same slot.
        ActionSpec::Label("Move".into())
    } else {
        // Every piece defines `iop5=Move` (the frozen click's no-label
        // path), but `cache_held_ops` pads a `Drop` into the fifth slot
        // when the obj table has none and `offers_operation(5)` treats that
        // default as usable. Only a def-defined, non-drop fifth op is the
        // index-5 fallback; the padded default is never sent as OPHELD5.
        let fifth = cache
            .objs
            .get(id as usize)
            .and_then(|obj| obj.iop.get(4))
            .and_then(|op| op.as_deref());
        if !fifth.is_some_and(|label| !label.trim().eq_ignore_ascii_case("Drop")) {
            return false;
        }
        ActionSpec::Operation(5)
    };
    matches!(ix.interact(target, action), SendResult::Sent { .. })
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

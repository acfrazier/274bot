use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::bank_fetch::BankStep;
use nav::router::{find_with, FindOptions};
use nav::traveller::TravelOptions;
use nav::world::NavWorld;
use nav::WorldState;

use super::play_status::lock_statuses;
use super::{
    deposit_all_backpack, log_walk_arm_bot, open_bank_at_here, withdraw_id, NavBot, ScriptWalkArm,
    SlotStatus,
};

pub(crate) fn abort_script_walk(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
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

pub(super) fn recovery_walk_idle(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) -> bool {
    let all = navs.lock().unwrap();
    let Some(bot) = all.get(name) else {
        return true;
    };
    bot.route_worker.is_none() && bot.route.is_none() && bot.pending_route.is_none()
}

#[allow(clippy::too_many_arguments)] // watchdog nav action packs driver/snapshot/nav handles
pub(super) fn apply_watchdog_nav_action(
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

/// Mid-follow Stall / Refused / Blocked / GaveUp publish a failed outcome
/// with the armed walk's isolate request id so the matching wait returns
/// false. A follow that reaches the end of the armed walk's route publishes
/// a settled (not failed) outcome for that id: frozen `WalkExecutor`
/// returns true at the path terminal whether or not `isArrived` holds there
/// (`WalkExecutor.ts:316-325`, `'closest'`), and a radius route ends on its
/// approach tile, which the reach-aware rule may not call arrival. Both
/// publish only for a route installed for the armed request id
/// (`route_request_id`): an old route still followed after a retarget
/// neither settles nor fails the new walk. A bank fetch's stand sub-route is
/// not the armed walk's end, and request id 0 never publishes a route end.
/// Only genuinely pending follow work (None) keeps the caller timeout.
pub(crate) fn apply_nav_follow_outcome(
    bot: &mut NavBot,
    outcome: Option<nav::traveller::TravelOutcome>,
    walking_stand: bool,
) {
    match outcome {
        Some(nav::traveller::TravelOutcome::Arrived { .. }) => {
            bot.route = None;
            if bot.bank_fetch.is_none() {
                if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                    if bot.walk_request_id != 0
                        && bot.route_request_id == bot.walk_request_id
                        && bot.armed_outcome_may_publish(bot.walk_request_id)
                    {
                        bot.note_route_end(
                            bot.route_generation,
                            bot.walk_request_id,
                            to,
                            radius,
                            allow_teleports,
                        );
                    }
                }
            }
        }
        Some(_) => {
            let owned = bot.route_request_id == bot.walk_request_id;
            if !owned {
                log_walk_arm_bot(|| {
                    format!(
                        "follow failure of a superseded route route_request_id={} \
                         walk_request_id={} not published",
                        bot.route_request_id, bot.walk_request_id
                    )
                });
            } else if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
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
/// isolate request id as a failed outcome. Arrival still settles from `here`,
/// and the same arrival ends the follow: once [`api::query::is_arrived`]
/// (frozen `isArrived`, the rule `walk_wait` settles on) holds for the
/// requested dest and radius, the route clears without another hop, even
/// short of the approach tile the route aimed at. `reach` yields the slot's
/// cached reach view for `here`; it is asked only when that rule needs a
/// probe (`0 < dist <= radius` on the dest's level).
// Shared handles threaded like `script_observe`; the arg count is allowed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn step_nav_bot<D: Driver>(
    driver: &mut D,
    name: &str,
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<&NavWorld>,
    hold: bool,
    map_members: bool,
    reach: impl FnOnce() -> Arc<api::query::ReachQueryView>,
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
                    let mut rows = lock_statuses(statuses);
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
    // Resolve arrival before the follow lock: the reach view lives behind
    // the script slot, and the slot locks before navs, never after.
    let armed = {
        let all = navs.lock().unwrap();
        all.get(name)
            .filter(|bot| bot.route.is_some() && bot.bank_fetch.is_none())
            .and_then(|bot| bot.requested_route)
    };
    let arrived = here
        .zip(armed)
        .is_some_and(|((x, z, level), (to, radius, ..))| {
            api::query::is_arrived(WorldTile { x, z, level }, to, radius, reach)
        });
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
        if bot.route.is_none() {
            return;
        }
        // `requested_route == armed`: the arrival above is for this walk.
        if arrived && bot.bank_fetch.is_none() && bot.requested_route == armed {
            // The isolate settles the walk wait on this same arrival rule
            // (walk_wait.rs), so the card has already moved on. Every
            // further hop would be a click the card never sent; a stall
            // re-send walks the player out of the fight the card started at
            // the edge of the radius.
            bot.traveller.clear();
            bot.route = None;
        } else if let Some(route) = bot.route.clone() {
            let walking_stand = bot.bank_fetch.as_ref().is_some_and(|p| {
                matches!(
                    p.steps.front(),
                    Some(BankStep::Walk { x, z, level })
                        if route.dest.x == *x && route.dest.z == *z && route.dest.level == *level
                )
            });
            let follow_outcome = bot.traveller.follow(driver, snapshot, route, &mut options);
            apply_nav_follow_outcome(bot, follow_outcome, walking_stand);
        }
        bot.route.as_ref().map(|r| r.dest)
    };
    let mut rows = lock_statuses(statuses);
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
pub(crate) fn step_bank_fetch_on_bot<D: Driver>(
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
                bot.map_route_generation = crate::walk_map::next_map_route_generation();
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
                        log_walk_arm_bot(|| format!("bank_fetch Walk armed sub-route dest={to:?}"));
                        bot.route = Some(route);
                        bot.map_route_generation = crate::walk_map::next_map_route_generation();
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
pub(crate) fn bank_fetch_freezes_follow(bot: &NavBot) -> bool {
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

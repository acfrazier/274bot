use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::bank_fetch::{bank_access_tiles, is_bank_access, BankStep, SAME_BANK};
use nav::router::{
    find_first_blocking_zones, find_first_with_avoid, find_with_avoid, FindOptions, Route,
    RouteError,
};
use nav::traveller::TravelOptions;
use nav::world::NavWorld;
use nav::WorldState;

use super::play_status::lock_statuses;
use super::{
    deposit_all_backpack, log_walk_arm_bot, open_bank_at_here, withdraw_id, BankFetchFlight,
    FlightTarget, NavBot, PendingBankFetch, ScriptWalkArm, SlotStatus,
};

/// Pumps a non-Walk BankBudget step may wait before a truthful abort.
const BANK_STEP_ATTEMPTS: u32 = 32;

struct LiveRouteRefresh {
    to: WorldTile,
    radius: i32,
    loc_id: Option<i32>,
    options: FindOptions,
    request_id: u64,
    exclusions: Option<Arc<super::script_nav::ScriptRouteExclusions>>,
    authority: Option<script::native::HostAuthority>,
    quest_evidence: Option<nav::quest_gates::QuestEvidence>,
}

pub(crate) fn abort_script_walk(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        abort_walk_on_bot(bot);
    }
}

/// End the uid's walk follow and everything that shares its ownership. A
/// still-live native owner receives `Cancelled`; a revoked one nothing.
pub(super) fn abort_walk_on_bot(bot: &mut NavBot) {
    bot.route_generation = bot.route_generation.wrapping_add(1);
    bot.route = None;
    bot.route_worker = None;
    bot.pending_route = None;
    bot.requested_route = None;
    bot.end_native_walk(script::native::WalkEnd::Cancelled);
    bot.route_loc_id = None;
    bot.route_loc_geometry = (false, false);
    bot.route_quest_evidence = None;
    bot.walk_request_id = 0;
    bot.clear_walk_outcome();
    bot.traveller.clear();
    // Bank work and carried routes share the revoked walk's ownership.
    bot.bank_fetch = None;
    bot.carried_walk = None;
}

/// Operator Pause of the slot's script. Frozen stops clicking at the paused
/// `await` and carries on the same walk after Resume, so the host ends the
/// route it is following and carries it ([`hold_script_nav`], as a
/// reconnect does); the first dispatch after Resume re-sends it once
/// ([`resumed_walk`]). A BankBudget session latched on the route ends with
/// the follow and is re-planned by the re-sent walk: its deposits and
/// withdrawals were planned from the pack and bank at arm time, which the
/// operator may change while paused. A watchdog recovery walk is not the
/// script's: the watchdog re-arms it on Resume itself.
pub(crate) fn pause_script(
    slot: &mut script::SlotScript,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
) {
    let recovery = slot.watchdog().recovering_anchor().is_some();
    let generation = slot.runtime_generation();
    slot.pause();
    if recovery {
        abort_script_walk(navs, name);
    } else {
        super::hold_script_nav(navs, name, Some(generation));
    }
}

/// The carried walk the first dispatch after a Resume or a relog sends,
/// if it still applies: not when a request in `queued` replaces it (a
/// newer walk, a nearest-bank walk, an abort) and not when the player
/// already stands within its arrival radius (`arrived`).
pub(crate) fn resumed_walk(
    carried: Option<script::shim::InteractReq>,
    queued: &[script::shim::InteractReq],
    arrived: impl FnOnce(WorldTile, i32) -> bool,
) -> Option<script::shim::InteractReq> {
    use script::shim::InteractReq;
    let carried = carried?;
    let superseded = queued.iter().any(|op| {
        matches!(
            op,
            InteractReq::Walk { .. }
                | InteractReq::WalkNear { .. }
                | InteractReq::WalkNearestBank
                | InteractReq::AbortWalk { .. }
        )
    });
    if superseded {
        return None;
    }
    let target = match &carried {
        InteractReq::Walk { x, z, level, .. } => Some((
            WorldTile {
                x: *x,
                z: *z,
                level: *level,
            },
            0,
        )),
        InteractReq::WalkNear {
            x,
            z,
            level,
            radius,
            ..
        } => Some((
            WorldTile {
                x: *x,
                z: *z,
                level: *level,
            },
            *radius,
        )),
        _ => None,
    };
    if target.is_some_and(|(dest, radius)| arrived(dest, radius)) {
        return None;
    }
    Some(carried)
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
            let armed = arm.queue_route_in_snapshot(
                snapshot,
                x,
                z,
                level,
                FindOptions::default(),
                script::watchdog::WALK_RADIUS,
                true,
                0,
            );
            if !armed {
                abort_script_walk(navs, name);
            }
        }
        _ => {}
    }
}

/// The armed walk's route ended (arrived, or frozen `'blocked'`): clear it
/// and publish the settled outcome for the armed request.
fn settle_route_end(bot: &mut NavBot, blocked: bool) {
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
                    blocked,
                );
            }
        }
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
        Some(nav::traveller::TravelOutcome::Arrived { .. }) => settle_route_end(bot, false),
        // Frozen `'blocked'` (`WalkExecutor.ts:1092-1094`): the follow ended
        // next to a route end the live scene refuses. The armed walk settles
        // as its route end, flagged blocked, and walkResilient returns true
        // (`Traversal.ts:171-174`). A bank-fetch stand walk is not the armed
        // walk's end and fails below.
        Some(nav::traveller::TravelOutcome::Stalled {
            why: nav::traveller::HopFailure::EndBlocked,
            ..
        }) if bot.bank_fetch.is_none() => settle_route_end(bot, true),
        Some(failure) => {
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
                    bot.note_native_follow_failure(&failure);
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
                    bot.note_native_follow_failure(&failure);
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
/// first, then poll the armed route through [`Traveller::follow`] one step
/// against `snapshot`. `here` is the player's world tile when the body
/// decoded one (else the bot stands still). `world` is the shared, owned
/// nav-world handle so a stale offscene endpoint can be re-queued on the
/// existing background route worker without retaining a per-bot world copy.
/// Mirrors the armed route's dest into the status row's `walk_*` fields
/// (`-1` when idle). A live modeled loc's full-footprint arrival gates both
/// early settlement and route-end receipts; an estimated endpoint is
/// refreshed off-pump when the target enters scene. `reach` yields the slot's
/// cached reach view for ordinary destinations and is asked only when that
/// rule needs a probe (`0 < dist <= radius` on the dest's level).
// Shared handles threaded like `script_observe`; the arg count is allowed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn step_nav_bot<D: Driver>(
    driver: &mut D,
    name: &str,
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<&Arc<NavWorld>>,
    hold: bool,
    map_members: bool,
    reach: impl FnOnce() -> Arc<api::query::ReachQueryView>,
) {
    // The random-event freeze: the follow is not stepped while the
    // guardian holds the slot, and the armed route stays latched so it
    // resumes when the hold lifts. BankBudget steps freeze the same way.
    if hold {
        return;
    }
    if here.is_none() {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            // A missing player cannot deposit/open/wear; abort the latched
            // non-Walk step so disconnect does not hang with follow frozen.
            if bank_fetch_freezes_follow(bot) {
                abort_bank_fetch(bot, "no player tile (disconnected)");
            }
        }
        return;
    }
    let borrowed_world = world.map(Arc::as_ref);
    {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            if bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
                abort_walk_on_bot(bot);
            }
            if bot.bank_fetch.is_some() {
                step_bank_fetch_on_bot(driver, snapshot, bot, borrowed_world, here, map_members);
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
    let (armed, endpoint, loc_id, estimated_geometry) = {
        let all = navs.lock().unwrap();
        all.get(name)
            .filter(|bot| bot.route.is_some() && bot.bank_fetch.is_none())
            .map(|bot| {
                (
                    bot.requested_route,
                    bot.route.as_ref().map(|route| route.dest),
                    bot.route_loc_id,
                    bot.route_loc_geometry,
                )
            })
            .unwrap_or((None, None, None, (false, false)))
    };
    let endpoint_arrival = armed.zip(endpoint).and_then(|((to, radius, ..), from)| {
        loc_id.and_then(|id| api::query::loc_approach::arrived_at(snapshot, from, to, radius, id))
    });
    let target_gone = endpoint_arrival.is_none()
        && armed
            .zip(loc_id)
            .is_some_and(|((to, ..), id)| api::query::loc_approach::target_gone(snapshot, to, id));
    let live_geometry = (
        loc_id.is_some()
            && armed.is_some_and(|(to, ..)| {
                api::query::SceneQuery::new(snapshot.scene(), None).contains(to)
            }),
        endpoint_arrival.is_some(),
    );
    let estimated_endpoint = loc_id.is_some() && endpoint_arrival.is_none() && !target_gone;
    let arrived = here
        .zip(armed)
        .is_some_and(|((x, z, level), (to, radius, ..))| {
            let from = WorldTile { x, z, level };
            match loc_id {
                Some(id) if !target_gone => {
                    api::query::loc_approach::arrived_at(snapshot, from, to, radius, id)
                        == Some(true)
                }
                _ => api::query::is_arrived(from, to, radius, reach),
            }
        });
    // Refresh newly observable geometry, or a vanished footprint whose old
    // stand no longer satisfies tile arrival. An unchanged estimate never
    // restarts its worker.
    let invalid_endpoint = !arrived
        && ((live_geometry.0 && !estimated_geometry.0)
            || (live_geometry.1 && !estimated_geometry.1)
            || (target_gone && estimated_geometry.1));
    let (refresh, suppress_follow) = if invalid_endpoint {
        let mut all = navs.lock().unwrap();
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        if bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
            abort_walk_on_bot(bot);
        }
        let still_owns_endpoint = bot.route_request_id == bot.walk_request_id
            && bot.requested_route == armed
            && bot.route.as_ref().map(|route| route.dest) == endpoint
            && bot.route_loc_id == loc_id
            && bot.bank_fetch.is_none();
        if !still_owns_endpoint {
            (None, false)
        } else if bot.route_worker.is_some() || bot.pending_route.is_some() || world.is_none() {
            // A stale route cannot settle while an existing worker is
            // calculating, or before a world is available to refresh it.
            (None, true)
        } else {
            let (to, radius, allow_teleports, allow_wilderness, allow_bank_fetch, zones) =
                armed.expect("invalid endpoint has an armed request");
            let refresh = LiveRouteRefresh {
                to,
                radius,
                loc_id,
                options: FindOptions {
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    zones,
                    ..FindOptions::default()
                },
                request_id: bot.walk_request_id,
                exclusions: bot.requested_exclusions.clone(),
                authority: bot.native_walk.clone(),
                quest_evidence: bot.route_quest_evidence.clone(),
            };
            // Keep the live owner and request correlation; only the old
            // endpoint and its Traveller run are replaced by the worker.
            bot.traveller.clear();
            bot.route = None;
            (Some(refresh), true)
        }
    } else {
        (None, false)
    };
    if let Some(refresh) = refresh {
        let mut state = WorldState::from_snapshot(snapshot).with_map_members(map_members);
        state.quest_evidence = refresh.quest_evidence;
        let bank = snapshot
            .bank()
            .iter()
            .map(|item| (item.def.id, item.count))
            .collect();
        ScriptWalkArm {
            here,
            world: world.cloned(),
            navs: Arc::clone(navs),
            name: name.to_string(),
            state: Some(state),
            bank,
        }
        .refresh_route_in_snapshot(
            snapshot,
            refresh.to,
            refresh.radius,
            refresh.options,
            refresh.request_id,
            refresh.exclusions.as_deref().cloned().unwrap_or_default(),
            refresh.authority,
            refresh.loc_id,
        );
    }
    let defer_estimated_end = estimated_endpoint;
    let queued = {
        let mut all = navs.lock().unwrap();
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        if bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
            abort_walk_on_bot(bot);
        }
        if bot.route.is_some() {
            // `requested_route == armed`: the arrival above is for this walk.
            if arrived && bot.bank_fetch.is_none() && bot.requested_route == armed {
                bot.traveller.clear();
                settle_route_end(bot, false);
            } else if !suppress_follow {
                if let Some(route) = bot.route.clone() {
                    let walking_stand = bot.bank_fetch.as_ref().is_some_and(|p| {
                        matches!(
                            p.steps.front(),
                            Some(BankStep::Walk { x, z, level })
                                if route.dest.x == *x && route.dest.z == *z && route.dest.level == *level
                        )
                    });
                    let follow_outcome = {
                        let mut options = TravelOptions {
                            // Exact arrival matches the armed walk's destination.
                            close_enough: 0,
                            teleports: borrowed_world.map(|world| world.graph.teleports.as_slice()),
                            edges: borrowed_world.map(|world| world.graph.edges.as_slice()),
                            quest_evidence: bot.route_quest_evidence.as_ref(),
                            ..TravelOptions::default()
                        };
                        bot.traveller.follow(driver, snapshot, route, &mut options)
                    };
                    let owned_estimated_end = defer_estimated_end
                        && bot.requested_route == armed
                        && bot.route_request_id == bot.walk_request_id
                        && bot.bank_fetch.is_none();
                    let reached_endpoint = follow_outcome.as_ref().is_some_and(|outcome| {
                        matches!(outcome, nav::traveller::TravelOutcome::Arrived { .. })
                    });
                    if !owned_estimated_end || !reached_endpoint {
                        apply_nav_follow_outcome(bot, follow_outcome, walking_stand);
                    }
                }
            }
        }
        bot.route.as_ref().map(|route| route.dest)
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
/// on a standable access tile (never the stand's interact tile) or once
/// a sub-route to that tile is armed for follow; Open is a no-op while
/// the bank is already open+loaded and honors packed booth or NPC access
/// (falling back to another access of the same bank); DepositAll /
/// Withdraw / Wear / Close dispatch through
/// [`api::interact::Interactions`] and pop only after the snapshot
/// shows the step landed. Each non-Walk step has a pump budget and does
/// not re-send while the snapshot is unchanged; both live on the session
/// ([`PendingBankFetch::progress`]), so a caller that re-wraps the session
/// every pump (the panel/TUI `WalkArm`) keeps them. Clears the pending
/// session when steps are exhausted, or on a truthful, logged failure.
/// Returns whether the driver was written.
pub(crate) fn step_bank_fetch_on_bot<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    bot: &mut NavBot,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
    map_members: bool,
) -> bool {
    let Some(pending) = bot.bank_fetch.as_ref() else {
        return false;
    };
    let Some(step) = pending.steps.front().cloned() else {
        bot.bank_fetch = None;
        return false;
    };
    let (wrote, end) = match step {
        BankStep::Walk { x, z, level } => step_walk(
            snapshot,
            bot,
            world,
            here,
            map_members,
            WorldTile { x, z, level },
        ),
        _ => {
            let pending = bot
                .bank_fetch
                .as_mut()
                .expect("the session holds the front step");
            step_bank_action(driver, snapshot, pending, &step, here, world)
        }
    };
    match end {
        StepEnd::Waiting => {}
        StepEnd::Abort(why) => {
            abort_bank_fetch(bot, why);
            return wrote;
        }
        StepEnd::Landed(how) => {
            log_walk_arm_bot(|| format!("bank_fetch phase done {step:?} {how}"));
            if let Some(pending) = bot.bank_fetch.as_mut() {
                pending.pop_step();
            }
        }
    }
    if let Some(pending) = bot.bank_fetch.as_ref() {
        if pending.steps.is_empty() {
            let session_dest = pending.dest;
            log_walk_arm_bot(|| {
                format!("bank_fetch session cleared complete session_dest={session_dest:?}")
            });
            bot.bank_fetch = None;
        }
    }
    wrote
}

/// How a pump left the front step.
enum StepEnd {
    /// Still waiting for the snapshot to show it (or for the sub-route).
    Waiting,
    /// The snapshot shows it done; the detail goes in the log.
    Landed(&'static str),
    /// It cannot land: the session ends with this reason.
    Abort(&'static str),
}

/// End the BankBudget session truthfully: log the step it stopped at and
/// why, then clear the session and the route it armed.
fn abort_bank_fetch(bot: &mut NavBot, why: &str) {
    if let Some(pending) = bot.bank_fetch.take() {
        log_walk_arm_bot(|| {
            format!(
                "bank_fetch abort front={:?} why={why} session_dest={:?} remaining={}",
                pending.steps.front(),
                pending.dest,
                pending.steps.len()
            )
        });
    }
    bot.route = None;
}

/// The Walk step: arrived on an access tile of the planned bank, or a
/// sub-route to it armed for follow, or a fallback to the nearest routable
/// access tile of that bank (the step's dest is rewritten to it).
fn step_walk(
    snapshot: &GameSnapshot,
    bot: &mut NavBot,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
    map_members: bool,
    dest: WorldTile,
) -> (bool, StepEnd) {
    if walk_arrived(here, dest, world) {
        // A stand follow may have left a stale Traveller run armed against the
        // private route. The next poll belongs to the restored post-session
        // route, so drop that run first.
        bot.traveller.clear();
        if let Some(pending) = bot.bank_fetch.as_ref() {
            bot.route = Some(pending.final_route.clone());
        }
        bot.map_route_generation = crate::walk_map::next_map_route_generation();
        return (false, StepEnd::Landed("on-access"));
    }
    if bot.route.as_ref().is_some_and(|r| r.dest == dest) {
        return (false, StepEnd::Waiting);
    }
    let Some(w) = world else {
        return (false, StepEnd::Abort("no nav world to route the walk"));
    };
    let (Some((x, z, level)), Some(pending)) = (here, bot.bank_fetch.as_ref()) else {
        return (false, StepEnd::Waiting);
    };
    let from = WorldTile { x, z, level };
    let state = WorldState::from_snapshot(snapshot).with_map_members(map_members);
    let opts = FindOptions {
        allow_bank_fetch: false,
        ..pending.opts
    };
    let route = match find_with_avoid(
        &w.collision,
        &w.graph,
        from,
        dest,
        opts,
        &state,
        &pending.avoid,
    ) {
        Ok(route) => {
            log_walk_arm_bot(|| format!("bank_fetch Walk armed sub-route dest={dest:?}"));
            route
        }
        Err(_) => match arm_access_fallback(w, from, dest, opts, &state, &pending.avoid) {
            Ok(route) => {
                let reached = route.dest;
                if let Some(BankStep::Walk { x, z, level }) =
                    bot.bank_fetch.as_mut().and_then(|p| p.steps.front_mut())
                {
                    *x = reached.x;
                    *z = reached.z;
                    *level = reached.level;
                }
                log_walk_arm_bot(|| format!("bank_fetch Walk fallback access dest={reached:?}"));
                route
            }
            Err(blocked) => {
                if let (Some(keys), Some(table)) = (blocked.as_deref(), w.graph.zones.as_ref()) {
                    api::host_log!(
                        api::hostlog::Category::NavTrace,
                        api::hostlog::Level::Warn,
                        "{}",
                        super::script_nav::compat_zone_no_route_line(table, keys)
                    );
                }
                return (false, StepEnd::Abort("no route to a bank access tile"));
            }
        },
    };
    bot.route = Some(route);
    bot.map_route_generation = crate::walk_map::next_map_route_generation();
    (false, StepEnd::Waiting)
}

/// One pump of a non-Walk step on the session: landed on snapshot
/// evidence, else (unless a send is still in flight) send it, within the
/// step's pump budget.
fn step_bank_action<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    pending: &mut PendingBankFetch,
    step: &BankStep,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> (bool, StepEnd) {
    let bank_open = snapshot.bank_component_id() >= 0;
    let landed = match step {
        BankStep::Open => snapshot.bank_loaded().then_some("loaded"),
        BankStep::DepositAll => backpack_empty(snapshot).then_some("observed-empty"),
        BankStep::Withdraw { id, count } => {
            (backpack_count(snapshot, *id) >= *count).then_some("observed in the backpack")
        }
        BankStep::Wear { id } => wearing(snapshot, *id).then_some("observed worn"),
        BankStep::Close => (!bank_open).then_some("observed closed"),
        BankStep::Walk { .. } => unreachable!("Walk steps in step_walk"),
    };
    if let Some(how) = landed {
        return (false, StepEnd::Landed(how));
    }
    // The obj a Withdraw or Wear moves leaves its source (the bank row, the
    // backpack slot) before the server's next update shows it arrived (the
    // backpack, the worn set), so those two refusals hold only until the
    // step's first send; after it, the step waits for the landing within
    // its budget.
    let sent = pending.progress.flight.is_some();
    let refused = match step {
        BankStep::DepositAll | BankStep::Withdraw { .. } if !bank_open => {
            Some("the bank is closed")
        }
        BankStep::Withdraw { id, .. } if !sent && !bank_holds(snapshot, *id) => {
            Some("the open bank does not hold the obj")
        }
        BankStep::Wear { .. } if bank_open => Some("the bank is open; wearing needs it closed"),
        BankStep::Wear { id } if !sent && backpack_count(snapshot, *id) < 1 => {
            Some("the obj is not in the backpack")
        }
        _ => None,
    };
    if let Some(why) = refused {
        return (false, StepEnd::Abort(why));
    }
    // Open also waits while a bank component is up but not yet loaded.
    let in_flight = pending
        .progress
        .flight
        .as_ref()
        .is_some_and(|flight| flight.matches(snapshot))
        || (matches!(step, BankStep::Open) && snapshot.bank_component_id() != -1);
    let wrote = !in_flight && {
        let wrote = match step {
            BankStep::Open => open_bank_at_here(driver, snapshot, here, world),
            BankStep::DepositAll => deposit_all_backpack(driver, snapshot),
            BankStep::Withdraw { id, count } => withdraw_id(driver, snapshot, *id, *count),
            BankStep::Wear { id } => {
                let mut ix = api::interact::Interactions::new(snapshot, driver);
                matches!(ix.wear(*id), api::interact::SendResult::Sent { .. })
            }
            BankStep::Close => {
                let mut ix = api::interact::Interactions::new(snapshot, driver);
                matches!(ix.close_modal(), api::interact::SendResult::Sent { .. })
            }
            BankStep::Walk { .. } => unreachable!("Walk steps in step_walk"),
        };
        if wrote {
            pending.progress.flight = Some(Box::new(BankFetchFlight::of(snapshot, step)));
        }
        log_walk_arm_bot(|| format!("bank_fetch {step:?} sent={wrote}"));
        wrote
    };
    pending.progress.attempts = pending.progress.attempts.saturating_add(1);
    if pending.progress.attempts >= BANK_STEP_ATTEMPTS {
        return (
            wrote,
            StepEnd::Abort("the step did not land within its pump budget"),
        );
    }
    (wrote, StepEnd::Waiting)
}

/// Whether a latched BankBudget session must freeze [`Traveller::follow`].
/// Walk with the access sub-route armed does **not** freeze; Open /
/// Deposit / Withdraw / Wear / Close do. Mid-session `final_route` is
/// never followed.
pub(crate) fn bank_fetch_freezes_follow(bot: &NavBot) -> bool {
    session_freezes_follow(bot.bank_fetch.as_ref(), bot.route.as_ref())
}

/// [`bank_fetch_freezes_follow`] over a session and the route armed with
/// it, for an owner that keeps them apart (the panel/TUI `WalkArm`).
pub(crate) fn session_freezes_follow(
    pending: Option<&PendingBankFetch>,
    route: Option<&Route>,
) -> bool {
    let Some(pending) = pending else {
        return false;
    };
    match pending.steps.front() {
        Some(BankStep::Walk { x, z, level }) => {
            // Freeze only until the access sub-route is armed; once armed,
            // follow that route. If route still points at final_route,
            // stay frozen this tick (Walk arms next pump / this pump).
            !route.is_some_and(|r| r.dest.x == *x && r.dest.z == *z && r.dest.level == *level)
        }
        Some(_) => true,
        None => false,
    }
}

fn walk_arrived(here: Option<(i32, i32, i32)>, dest: WorldTile, world: Option<&NavWorld>) -> bool {
    let Some((hx, hz, hl)) = here else {
        return false;
    };
    if (hx, hz, hl) == (dest.x, dest.z, dest.level) {
        return true;
    }
    let Some(world) = world else {
        return false;
    };
    let here_tile = WorldTile {
        x: hx,
        z: hz,
        level: hl,
    };
    world.banks().iter().any(|stand| {
        stand.tile.level == dest.level
            && (stand.tile.x - dest.x)
                .abs()
                .max((stand.tile.z - dest.z).abs())
                <= SAME_BANK
            && is_bank_access(&world.collision, stand, here_tile)
    })
}

fn arm_access_fallback(
    world: &NavWorld,
    from: WorldTile,
    dest: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[nav::router::AvoidRect],
) -> Result<Route, Option<Vec<nav::zones::ZoneKey>>> {
    let mut targets: Vec<WorldTile> = world
        .banks()
        .iter()
        .filter(|stand| {
            stand.tile.level == dest.level
                && (stand.tile.x - dest.x)
                    .abs()
                    .max((stand.tile.z - dest.z).abs())
                    <= SAME_BANK * 2
        })
        .flat_map(|stand| bank_access_tiles(&world.collision, stand))
        .collect();
    if targets.is_empty() {
        targets = world
            .banks()
            .iter()
            .flat_map(|stand| bank_access_tiles(&world.collision, stand))
            .collect();
    }
    if targets.is_empty() {
        return Err(None);
    }
    match find_first_with_avoid(
        &world.collision,
        &world.graph,
        from,
        &targets,
        opts,
        state,
        avoid,
    )
    .into_route()
    {
        Ok(route) => Ok(route),
        Err(RouteError::NoPath) => Err(find_first_blocking_zones(
            &world.collision,
            &world.graph,
            from,
            &targets,
            opts,
            state,
            avoid,
        )
        .filter(|keys| !keys.is_empty())),
        Err(_) => Err(None),
    }
}

impl BankFetchFlight {
    /// The bank session and `step`'s own target a send is judged against.
    fn of(snapshot: &GameSnapshot, step: &BankStep) -> Self {
        let target = match *step {
            BankStep::DepositAll => FlightTarget::Backpack(backpack(snapshot).collect()),
            BankStep::Withdraw { id, .. } | BankStep::Wear { id } => FlightTarget::Obj {
                id,
                carried: backpack_count(snapshot, id),
                worn: wearing(snapshot, id),
            },
            BankStep::Open | BankStep::Close | BankStep::Walk { .. } => FlightTarget::Bank,
        };
        BankFetchFlight {
            bank_gen: snapshot.bank_session_generation(),
            bank_com: snapshot.bank_component_id(),
            target,
        }
    }

    /// Whether `snapshot` still shows the latched bank session and target,
    /// so the send is still in flight. Compares in place, without a
    /// per-pump copy.
    fn matches(&self, snapshot: &GameSnapshot) -> bool {
        self.bank_gen == snapshot.bank_session_generation()
            && self.bank_com == snapshot.bank_component_id()
            && match &self.target {
                FlightTarget::Bank => true,
                FlightTarget::Backpack(rows) => rows.iter().copied().eq(backpack(snapshot)),
                &FlightTarget::Obj { id, carried, worn } => {
                    backpack_count(snapshot, id) == carried && wearing(snapshot, id) == worn
                }
            }
    }
}

/// The backpack's own rows `(obj id, count)`, one per filled slot. While a
/// bank is open the client shows the pack in the bank's side panel
/// (`bank_side`); otherwise it is the inv tab (`inventory`). Not
/// [`GameSnapshot::inv`]: with no inv tab bound, its fallback reads the
/// first filled TYPE_INV, which need not be the pack.
fn backpack(snapshot: &GameSnapshot) -> impl Iterator<Item = (i32, i32)> + '_ {
    let rows = if snapshot.bank_component_id() >= 0 {
        snapshot.bank_side()
    } else {
        snapshot.inventory()
    };
    rows.iter()
        .filter(|item| item.count >= 1)
        .map(|item| (item.def.id, item.count))
}

fn backpack_empty(snapshot: &GameSnapshot) -> bool {
    backpack(snapshot).next().is_none()
}

/// The obj's total over the backpack's slots (an unstackable obj fills one
/// slot per item).
fn backpack_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    backpack(snapshot)
        .filter(|&(got, _)| got == id)
        .fold(0, |total, (_, n)| total.saturating_add(n))
}

fn worn(snapshot: &GameSnapshot) -> impl Iterator<Item = i32> + '_ {
    snapshot
        .equipment()
        .iter()
        .filter(|item| item.count >= 1)
        .map(|item| item.def.id)
}

fn bank_holds(snapshot: &GameSnapshot, id: i32) -> bool {
    snapshot
        .bank()
        .iter()
        .any(|item| item.def.id == id && item.count >= 1)
}

fn wearing(snapshot: &GameSnapshot, id: i32) -> bool {
    worn(snapshot).any(|got| got == id)
}

#[cfg(test)]
mod zone_diagnostic_tests {
    use super::*;
    use nav::collision::{pack_walk, WorldCollision};
    use nav::pack::{BankAccess, BankStand};
    use nav::transport::TransportGraph;
    use nav::zones::{Zone, ZoneClass, ZoneKey, ZoneKind, ZoneTable};

    fn world_with_bank_access_barrier() -> NavWorld {
        let origin = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let flags = vec![0u32; 40 * 4];
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin,
            width: 40,
            height: 1,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = TransportGraph::default();
        graph.zones = Some(
            ZoneTable::from_parts(
                vec![Zone::npc(
                    WorldTile {
                        x: 2,
                        z: 0,
                        level: 0,
                    },
                    0,
                    ZoneClass::Always,
                    u16::MAX,
                    0,
                )],
                vec![ZoneKind::new(
                    "test-barrier",
                    "Test barrier",
                    123,
                    0,
                    false,
                    false,
                )],
                vec![],
                vec![],
                vec![],
                origin,
                40,
                1,
                &graph.wilderness,
            )
            .unwrap(),
        );
        NavWorld::from_parts(
            collision,
            graph,
            vec![BankStand {
                name: "Test bank".into(),
                tile: WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
                access: BankAccess::Booth { op: 2 },
            }],
        )
    }

    #[test]
    fn bank_access_fallback_returns_its_zone_witness() {
        let world = world_with_bank_access_barrier();
        let result = arm_access_fallback(
            &world,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
            FindOptions::default(),
            &WorldState::empty(),
            &[],
        );
        match result {
            Err(Some(keys)) => assert_eq!(keys, vec![ZoneKey::Zone(0)]),
            other => panic!("expected a bank-access zone witness, got {other:?}"),
        }
    }
}

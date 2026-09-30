use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::bank_fetch::{bank_access_tiles, is_bank_access, BankStep, SAME_BANK};
use nav::router::{find_first_with_avoid, find_with_avoid, FindOptions, Route};
use nav::traveller::TravelOptions;
use nav::world::NavWorld;
use nav::WorldState;

use super::play_status::lock_statuses;
use super::{
    deposit_all_backpack, log_walk_arm_bot, open_bank_at_here, withdraw_id, BankFetchFlight,
    NavBot, PendingBankFetch, ScriptWalkArm, SlotStatus,
};

/// Pumps a non-Walk BankBudget step may wait before a truthful abort.
const BANK_STEP_ATTEMPTS: u32 = 32;

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
    {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            if bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
                abort_walk_on_bot(bot);
            }
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
    let queued = {
        let mut all = navs.lock().unwrap();
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        if bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
            abort_walk_on_bot(bot);
        }
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
            let follow_outcome = {
                let mut options = TravelOptions {
                    // Exact arrival matches the armed walk's destination.
                    close_enough: 0,
                    teleports: world.map(|w| w.graph.teleports.as_slice()),
                    edges: world.map(|w| w.graph.edges.as_slice()),
                    quest_evidence: bot.route_quest_evidence.as_ref(),
                    ..TravelOptions::default()
                };
                bot.traveller.follow(driver, snapshot, route, &mut options)
            };
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
            Some(route) => {
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
            None => return (false, StepEnd::Abort("no route to a bank access tile")),
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
            pending.progress.flight = Some(Box::new(BankFetchFlight::of(snapshot)));
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
) -> Option<nav::router::Route> {
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
        return None;
    }
    find_first_with_avoid(
        &world.collision,
        &world.graph,
        from,
        &targets,
        opts,
        state,
        avoid,
    )
    .into_route()
    .ok()
}

impl BankFetchFlight {
    /// The bank, backpack and worn facts a send is judged against.
    fn of(snapshot: &GameSnapshot) -> Self {
        BankFetchFlight {
            bank_gen: snapshot.bank_session_generation(),
            bank_com: snapshot.bank_component_id(),
            backpack: backpack(snapshot).collect(),
            worn: worn(snapshot).collect(),
        }
    }

    /// Whether `snapshot` still shows exactly the latched facts, so the
    /// send is still in flight. Compares in place, without a per-pump copy.
    fn matches(&self, snapshot: &GameSnapshot) -> bool {
        self.bank_gen == snapshot.bank_session_generation()
            && self.bank_com == snapshot.bank_component_id()
            && self.backpack.iter().copied().eq(backpack(snapshot))
            && self.worn.iter().copied().eq(worn(snapshot))
    }
}

/// The backpack's own rows `(obj id, count)`, one per filled slot. While a
/// bank is open the client shows the pack only in the bank's side panel
/// (`bank_side`); otherwise it is the inv tab (`inventory`). Never
/// [`GameSnapshot::inv`]: with no inv tab bound, its fallback reads the
/// first filled TYPE_INV, which is the bank's withdraw grid while the bank
/// is open and the pack is empty.
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

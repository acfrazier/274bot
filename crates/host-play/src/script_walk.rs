use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::{ActionSpec, Driver, Interactions, OpTarget, SendResult};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, ReadContext, SnapshotView, WorldTile};
use nav::arrival::ArrivalKind;
use nav::bank_fetch::{bank_access_tiles, is_bank_access, BankStep, SAME_BANK};
use nav::router::{
    find_first_blocking_zones, find_first_with_avoid, find_with_avoid, FindOptions, Route,
    RouteError,
};
use nav::traveller::TravelOptions;
use nav::world::NavWorld;
use nav::WorldState;
use script::combat::guard::GUARD_PRAYER_WINDOW_TICKS;
use script::combat::schedule::elapsed;
use script::combat::{GuardFailure, GuardOp, GuardProtect};

use super::play_status::lock_statuses;
use super::{
    log_walk_arm_bot, open_bank_at_here, BankFetchFlight, FlightTarget, NavBot, PendingBankFetch,
    ScriptWalkArm, SlotStatus,
};

/// Pumps a non-Walk BankBudget step may wait before a truthful abort.
const BANK_STEP_ATTEMPTS: u32 = 32;

struct LiveRouteRefresh {
    to: WorldTile,
    radius: i32,
    loc_id: Option<i32>,
    options: FindOptions,
    arrival: ArrivalKind,
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
    abort_walk_on_bot_with_end(bot, script::native::WalkEnd::Cancelled);
}

pub(super) fn abort_walk_on_bot_with_end(bot: &mut NavBot, end: script::native::WalkEnd) {
    let manual = end == script::native::WalkEnd::UserInput;
    if bot.bank_pick.walking(bot.route_generation) {
        bot.bank_pick.reset();
    }
    bot.route_generation = bot.route_generation.wrapping_add(1);
    bot.route = None;
    bot.route_worker = None;
    bot.pending_route = None;
    bot.requested_route = None;
    bot.end_native_walk(end);
    bot.route_loc_id = None;
    bot.route_loc_geometry = (false, false);
    bot.route_arrival = ArrivalKind::Reach;
    bot.route_quest_evidence = None;
    bot.walk_request_id = 0;
    if !manual {
        bot.clear_walk_outcome();
    }
    bot.traveller.clear();
    // Bank work and carried routes share the revoked walk's ownership.
    bot.bank_fetch = None;
    bot.carried_walk = None;
    owe_walk_guard_off(bot);
}

fn guard_view(snapshot: &GameSnapshot) -> SnapshotView<'_> {
    SnapshotView::new(
        Some(snapshot),
        EvidenceStamp {
            run: api::selected::RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick: u64::from(snapshot.tick()),
            sequence: 0,
        },
    )
}

fn unprotectable_detail(protect: GuardProtect, reason: GuardFailure) -> Arc<str> {
    let (level, name) = match protect {
        GuardProtect::Magic => (37, "Protect from Magic"),
        GuardProtect::Missiles => (40, "Protect from Missiles"),
        GuardProtect::Melee => (43, "Protect from Melee"),
    };
    Arc::from(match reason {
        GuardFailure::PrayerLevel => format!("Prayer {level} needed for {name}"),
        GuardFailure::NoPrayerPoints => {
            format!("No Prayer points or prayer potion available for {name}")
        }
    })
}

pub(crate) fn apply_guard_op(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    op: &GuardOp,
    account: Option<&str>,
) -> bool {
    let _ = account;
    let (sent, kind, component) = match op {
        GuardOp::IfButton { component } if *component > 0 => {
            let ctx = ReadContext::new(snapshot);
            let sent = ctx.component(*component).is_some_and(|widget| {
                matches!(
                    Interactions::new(snapshot, driver).if_button(widget),
                    api::interact::SendResult::Sent { .. }
                )
            });
            (sent, "if-button", Some(*component))
        }
        GuardOp::Drink { name } => {
            let sent = snapshot
                .inventory()
                .iter()
                .find(|row| {
                    row.def
                        .name
                        .as_deref()
                        .is_some_and(|got| got.eq_ignore_ascii_case(name.as_ref()))
                })
                .is_some_and(|item| {
                    matches!(
                        Interactions::new(snapshot, driver)
                            .interact(OpTarget::Item(item), ActionSpec::Label("Drink".into())),
                        api::interact::SendResult::Sent { .. }
                    )
                });
            (sent, "drink", None)
        }
        GuardOp::Eat { name } => {
            let sent = snapshot
                .inventory()
                .iter()
                .find(|row| {
                    row.def
                        .name
                        .as_deref()
                        .is_some_and(|got| got.eq_ignore_ascii_case(name.as_ref()))
                })
                .is_some_and(|item| {
                    matches!(
                        Interactions::new(snapshot, driver)
                            .interact(OpTarget::Item(item), ActionSpec::Label("Eat".into())),
                        api::interact::SendResult::Sent { .. }
                    )
                });
            (sent, "eat", None)
        }
        _ => return false,
    };
    #[cfg(test)]
    if sent {
        if let Some(account) = account {
            crate::combat_proof::record_guard(account, snapshot, kind, component);
        }
    }
    let _ = (kind, component);
    sent
}

/// A serialized protect cleanup, used by retired walk guards and Combat raises.
/// In-flight guard switches may retain a previously owned style as fallback.
pub(crate) struct WalkGuardOff {
    component: i32,
    fallback_component: Option<i32>,
    admitted_tick: u16,
    nav_defer_tick: u16,
    start_at_first_pump: bool,
    awaiting_on: bool,
    off_tick: Option<u16>,
    retried: bool,
}

impl WalkGuardOff {
    fn blocks_nav(&self, tick: u16) -> bool {
        elapsed(tick, self.nav_defer_tick) < GUARD_PRAYER_WINDOW_TICKS
    }
}

pub(super) fn owe_walk_guard_off(bot: &mut NavBot) {
    if let Some(guard) = bot.walk_guard.take() {
        let awaiting_on = guard.pending_protect().is_some();
        let admitted_tick = guard.pending_protect_tick().unwrap_or_default();
        let (end, fallback_component) = guard.end();
        if let GuardOp::IfButton { component } = end {
            if component > 0 && bot.walk_guard_off.is_none() {
                bot.walk_guard_off = Some(WalkGuardOff {
                    component,
                    fallback_component,
                    admitted_tick,
                    nav_defer_tick: admitted_tick,
                    start_at_first_pump: !awaiting_on,
                    awaiting_on,
                    off_tick: None,
                    retried: false,
                });
            }
        }
    }
}

/// Stop's accepted Combat raises, serviced one off-click at a time by the
/// same bounded retirement state used by WalkGuard. No idle-slot allocation.
pub(crate) struct CombatPrayerOff {
    owned: script::combat::RaisedPrayers,
    awaiting_on: script::combat::RaisedPrayers,
    admitted_tick: u16,
    current: Option<WalkGuardOff>,
}

pub(crate) fn owe_combat_prayers_off(
    bot: &mut NavBot,
    owned: script::combat::RaisedPrayers,
    tick: u16,
) {
    if owned.is_empty() {
        return;
    }
    if let Some(debt) = bot.combat_prayer_off.as_mut() {
        debt.owned.merge(owned);
        debt.awaiting_on.merge(owned);
    } else {
        bot.combat_prayer_off = Some(CombatPrayerOff {
            owned,
            awaiting_on: owned,
            admitted_tick: tick,
            current: None,
        });
    }
}

pub(crate) fn finish_combat_prayers(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    bot: &mut NavBot,
    account: Option<&str>,
) {
    if bot.combat_prayer_off.is_none() {
        return;
    }
    if snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.effective == 0)
    {
        bot.combat_prayer_off = None;
        return;
    }
    let Some(debt) = bot.combat_prayer_off.as_mut() else {
        return;
    };
    let Ok(data) = api::game_data::for_revision(api::selected::ClientRevision::R289) else {
        return;
    };
    // Observe the whole mask while clicks are serialized. Once an accepted
    // raise was seen on, its first off row relinquishes ownership immediately.
    let age = elapsed(snapshot.tick() as u16, debt.admitted_tick);
    for fact in data.prayers() {
        if !debt.owned.contains(fact.varp) {
            continue;
        }
        let value = (snapshot.ingame() && snapshot.scene_state() == 2)
            .then(|| {
                snapshot
                    .varps()
                    .iter()
                    .find(|row| row.index == fact.varp)
                    .map(|row| row.value)
            })
            .flatten();
        if debt.awaiting_on.contains(fact.varp) {
            if age > GUARD_PRAYER_WINDOW_TICKS
                || (age == GUARD_PRAYER_WINDOW_TICKS && value != Some(1))
            {
                debt.owned.remove(fact.varp);
            } else if value == Some(1) {
                debt.awaiting_on.remove(fact.varp);
            }
        } else if value == Some(0) {
            debt.owned.remove(fact.varp);
        }
    }
    if debt.current.as_ref().is_some_and(|off| {
        data.prayers()
            .iter()
            .any(|fact| fact.button_com == off.component && !debt.owned.contains(fact.varp))
    }) {
        debt.current = None;
    }
    if debt.current.is_none() {
        let Some(fact) = data
            .prayers()
            .iter()
            .find(|fact| debt.owned.contains(fact.varp))
        else {
            bot.combat_prayer_off = None;
            return;
        };
        debt.current = Some(WalkGuardOff {
            component: fact.button_com,
            admitted_tick: debt.admitted_tick,
            nav_defer_tick: debt.admitted_tick,
            awaiting_on: debt.awaiting_on.contains(fact.varp),
            off_tick: None,
            fallback_component: None,
            start_at_first_pump: false,
            retried: false,
        });
    }
    let off = debt.current.as_mut().expect("selected owed prayer");
    if !finish_prayer_off(driver, snapshot, off, account) {
        if let Some(fact) = data
            .prayers()
            .iter()
            .find(|fact| fact.button_com == off.component)
        {
            debt.owned.remove(fact.varp);
        }
        debt.current = None;
        if debt.owned.is_empty() {
            bot.combat_prayer_off = None;
        }
    }
}

pub(crate) fn finish_walk_guard<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    bot: &mut NavBot,
    account: Option<&str>,
) {
    if let Some(off) = bot.walk_guard_off.as_mut() {
        if !finish_prayer_off(driver, snapshot, off, account) {
            bot.walk_guard_off = None;
        }
    }
}

fn prayer_varp_value(snapshot: &GameSnapshot, component: i32) -> Option<i32> {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return None;
    }
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).ok()?;
    let fact = data
        .prayers()
        .iter()
        .find(|fact| fact.button_com == component)?;
    snapshot
        .varps()
        .iter()
        .find(|row| row.index == fact.varp)
        .map(|row| row.value)
}

fn log_prayer_off_drop(off: &WalkGuardOff, account: Option<&str>, reason: &'static str) {
    api::host_log!(
        api::hostlog::Category::NavEvent,
        api::hostlog::Level::Warn,
        slot = account.unwrap_or(""),
        "prayer cleanup drops protect debt component={} awaiting_on={} retry={} reason={}",
        off.component,
        off.awaiting_on,
        off.retried,
        reason
    );
}

fn finish_prayer_off(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    off: &mut WalkGuardOff,
    account: Option<&str>,
) -> bool {
    let tick = snapshot.tick() as u16;
    // Death deactivates all prayers on the server, and a disconnected session
    // cannot admit a click. Session reset discards its temporary-varp debt.
    if snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.effective == 0)
    {
        return false;
    }
    if off.start_at_first_pump {
        off.admitted_tick = tick;
        off.nav_defer_tick = tick;
        off.start_at_first_pump = false;
    }
    let mut age = elapsed(tick, off.admitted_tick);
    let mut value = prayer_varp_value(snapshot, off.component);
    if value == Some(0) && !off.awaiting_on {
        return false;
    }
    // An enable first seen after its admission window cannot safely be
    // attributed to this walk: a user may have raised it in the meantime.
    let expired_enable = off.awaiting_on
        && (age > GUARD_PRAYER_WINDOW_TICKS
            || (age == GUARD_PRAYER_WINDOW_TICKS && value != Some(1)));
    if expired_enable {
        if let Some(fallback_component) = off.fallback_component.filter(|_| value != Some(1)) {
            if prayer_varp_value(snapshot, fallback_component) == Some(1) {
                api::host_log!(
                    api::hostlog::Category::NavEvent,
                    api::hostlog::Level::Warn,
                    slot = account.unwrap_or(""),
                    "walk guard retires owned fallback component={} after dropped switch component={}",
                    fallback_component,
                    off.component
                );
                off.component = fallback_component;
                off.fallback_component = None;
                off.admitted_tick = tick;
                off.start_at_first_pump = false;
                off.awaiting_on = false;
                off.off_tick = None;
                off.retried = false;
                age = 0;
                value = Some(1);
            } else {
                log_prayer_off_drop(
                    off,
                    account,
                    "enable not observed within 3 ticks; owned fallback not observed on",
                );
                return false;
            }
        } else {
            log_prayer_off_drop(off, account, "enable not observed within 3 ticks");
            return false;
        }
    }
    let off_age = off.off_tick.map(|sent| elapsed(tick, sent));
    let expired_off = off_age.is_some_and(|age| {
        age >= GUARD_PRAYER_WINDOW_TICKS * 2
            || (age >= GUARD_PRAYER_WINDOW_TICKS && value.is_none())
    });
    if expired_off {
        log_prayer_off_drop(off, account, "off not observed within bounded retry window");
        return false;
    }
    if value != Some(1) {
        if !off.awaiting_on && off.off_tick.is_none() && age >= GUARD_PRAYER_WINDOW_TICKS {
            log_prayer_off_drop(
                off,
                account,
                "owned protect varp unavailable within cleanup window",
            );
            return false;
        }
        return true;
    }
    off.awaiting_on = false;
    off.fallback_component = None;
    if let Some(age) = off_age {
        if age < GUARD_PRAYER_WINDOW_TICKS || off.retried {
            return true;
        }
        // One bounded retry, only while this exact protect is still observed
        // on. A refusal also spends the retry, never an unbounded send loop.
        off.retried = true;
    }
    if apply_guard_op(
        driver,
        snapshot,
        &GuardOp::IfButton {
            component: off.component,
        },
        account,
    ) {
        off.off_tick.get_or_insert(tick);
    } else if off.off_tick.is_none() && age >= GUARD_PRAYER_WINDOW_TICKS {
        log_prayer_off_drop(off, account, "off click refused within cleanup window");
        return false;
    }
    true
}

pub(crate) fn tick_walk_guard<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    guard: &mut Option<script::combat::WalkGuard>,
    account: Option<&str>,
) -> Option<GuardOp> {
    let guard = guard.as_mut()?;
    let view = guard_view(snapshot);
    let op = guard.tick(&view)?;
    if apply_guard_op(driver, snapshot, &op, account) {
        guard.admitted(&op, &view);
    }
    Some(op)
}

/// The single takeover seam, called before frontend and script consumers.
/// `hold` freezes automatic follow but never suppresses human intent.
pub(crate) fn take_manual_walk_ownership(
    scripts: &super::ScriptWall,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    frame: crate::SlotFrameInput,
    client_active: bool,
    tick: u64,
    pause_owner: bool,
) -> bool {
    let count = frame.manual_move_count();
    if count == 0 {
        return false;
    }
    let script = super::script_slot(scripts, name);
    let mut slot = script.as_ref().map(|slot| slot.lock().unwrap());
    let eligible = client_active
        && slot
            .as_ref()
            .is_some_and(|slot| slot.want_run && slot.state() == script::RunState::Running);
    let live_family = eligible
        && slot
            .as_ref()
            .is_some_and(|slot| slot.live_walking_operation());
    let mut all = navs.lock().unwrap();
    if !all.contains_key(name) {
        all.insert(name.to_owned(), NavBot::default());
    }
    let bot = all
        .get_mut(name)
        .expect("the slot intent counter was inserted");
    bot.user_move_intent_seq = bot.user_move_intent_seq.saturating_add(count as u64);
    // Retained request metadata also deduplicates completed walks; it is
    // not evidence of active work. Route-less families keep their own owner.
    let active_walk = bot.route.is_some()
        || bot.route_worker.is_some()
        || bot.pending_route.is_some()
        || bot.bank_fetch.is_some()
        || bot.carried_walk.is_some()
        || bot.native_walk.as_ref().is_some_and(|owner| owner.live());
    if !eligible || !(active_walk || live_family) {
        return false;
    }
    bot.cancel_for_manual_input();
    // End watchdog ownership before the ordinary Pause entry can defer it.
    if let Some(slot) = slot.as_mut() {
        slot.note_manual_walk_takeover(bot.user_move_intent_seq, tick);
        let _ = slot.abort_owned_recovery();
    }
    api::host_log!(
        api::hostlog::Category::NavEvent,
        api::hostlog::Level::Info,
        slot = name,
        "walk outcome=cancelled reason=UserInput request={} generation={} intent_seq={} source={:?} manual_steps={}",
        bot.walk_outcome_request_id,
        bot.walk_outcome_generation,
        bot.user_move_intent_seq,
        frame.manual_move_intent,
        frame.manual_steps,
    );
    drop(all);
    if pause_owner {
        if let Some(slot) = slot.as_mut() {
            pause_script(slot, navs, name);
        }
    }
    true
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
pub(crate) fn resumed_walk<'a>(
    carried: Option<script::shim::InteractReq>,
    queued: impl IntoIterator<Item = &'a script::shim::InteractReq>,
    arrived: impl FnOnce(WorldTile, i32) -> bool,
) -> Option<script::shim::InteractReq> {
    use script::shim::InteractReq;
    let carried = carried?;
    let superseded = queued.into_iter().any(|op| {
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
pub(crate) fn apply_watchdog_nav_action(
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
            super::script_nav::preserve_recovery_walk_identity(navs, name);
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

fn arrived_in_area(snapshot: &GameSnapshot, here: WorldTile, to: WorldTile, radius: i32) -> bool {
    nav::arrival::arrived_in_area(here, to, radius, |tile| {
        let scene = api::query::SceneQuery::new(snapshot.scene(), None);
        scene.contains(tile) && scene.walkable(tile)
    })
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
    {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            if snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 3 && stat.effective == 0)
            {
                bot.walk_guard = None;
                bot.walk_guard_off = None;
                bot.combat_prayer_off = None;
            } else if bot.walk_guard_off.is_some() {
                finish_walk_guard(driver, snapshot, bot, Some(name));
                // Only the first pacing window defers navigation. Cleanup
                // may watch one off retry while later follow/bank work runs.
                if bot
                    .walk_guard_off
                    .as_ref()
                    .is_some_and(|off| off.blocks_nav(snapshot.tick() as u16))
                {
                    return;
                }
            }
            finish_combat_prayers(driver, snapshot, bot, Some(name));
            if bot.combat_prayer_off.is_some() {
                return;
            }
        }
    }
    // The random-event freeze: the follow is not stepped while the
    // guardian holds the slot, and the armed route stays latched so it
    // resumes when the hold lifts. BankBudget steps freeze the same way.
    if hold {
        return;
    }
    if here.is_none() {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            // A missing player cannot open/withdraw/wear; abort the latched
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
                // Freeze follow for Open / Withdraw / Withdraw-X / Wear /
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
    let (armed, endpoint, loc_id, estimated_geometry, arrival) = {
        let all = navs.lock().unwrap();
        all.get(name)
            .filter(|bot| bot.route.is_some() && bot.bank_fetch.is_none())
            .map(|bot| {
                (
                    bot.requested_route,
                    bot.route.as_ref().map(|route| route.dest),
                    bot.route_loc_id,
                    bot.route_loc_geometry,
                    bot.route_arrival,
                )
            })
            .unwrap_or((None, None, None, (false, false), ArrivalKind::Reach))
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
            match arrival {
                ArrivalKind::Area => arrived_in_area(snapshot, from, to, radius),
                ArrivalKind::Reach => match loc_id {
                    Some(id) if !target_gone => {
                        api::query::loc_approach::arrived_at(snapshot, from, to, radius, id)
                            == Some(true)
                    }
                    _ => api::query::is_arrived(from, to, radius, reach),
                },
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
            && bot.route_arrival == arrival
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
                arrival,
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
            refresh.arrival,
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
                owe_walk_guard_off(bot);
                finish_walk_guard(driver, snapshot, bot, Some(name));
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
                    // A later route may follow while the old off-click is
                    // watched, but cannot raise a protect that debt could clear.
                    let guard_op = if bot.walk_guard_off.is_none() {
                        tick_walk_guard(driver, snapshot, &mut bot.walk_guard, Some(name))
                    } else {
                        None
                    };
                    if let Some(GuardOp::Unprotectable { protect, reason }) = guard_op {
                        if let Some(owner) = bot.native_walk.as_ref().filter(|owner| owner.live()) {
                            bot.walk_guard_events.push((
                                owner.clone(),
                                script::native::WalkEvent {
                                    request_id: owner.request_id().get(),
                                    evidence: EvidenceStamp {
                                        run: owner.run(),
                                        tick: u64::from(snapshot.tick()),
                                        sequence: u64::from(snapshot.tick()),
                                    },
                                    kind: script::native::WalkEventKind::Unprotectable { protect },
                                    detail: unprotectable_detail(protect, reason),
                                },
                            ));
                        }
                    }
                    let skip_follow = bot.route.is_none()
                        || bot
                            .walk_guard
                            .as_ref()
                            .is_some_and(|guard| guard.blocks_follow(snapshot.tick() as u16));
                    if !skip_follow {
                        let follow_outcome = {
                            let mut options = TravelOptions {
                                // Exact arrival matches the armed walk's destination.
                                close_enough: 0,
                                teleports: borrowed_world
                                    .map(|world| world.graph.teleports.as_slice()),
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
                    if bot.route.is_none() {
                        owe_walk_guard_off(bot);
                        finish_walk_guard(driver, snapshot, bot, Some(name));
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
/// (falling back to another access of the same bank); Withdraw / Withdraw-X /
/// Wear / Close dispatch through [`api::interact::Interactions`] and pop
/// only after the snapshot shows the step landed. Each non-Walk step has a
/// pump budget and does not re-send while the snapshot is unchanged; both
/// live on the session ([`PendingBankFetch::progress`]), so a caller that
/// re-wraps the session every pump (the panel/TUI `WalkArm`) keeps them.
/// Clears the pending session when steps are exhausted, or on a truthful,
/// logged failure. Returns whether the driver was written.
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
        BankStep::Withdraw { id, count } | BankStep::WithdrawXAmount { id, count } => pending
            .progress
            .flight
            .as_ref()
            .and_then(|flight| match &flight.target {
                FlightTarget::Obj {
                    id: target_id,
                    carried,
                    ..
                } if target_id == id => Some(*carried),
                _ => None,
            })
            .is_some_and(|carried| backpack_count(snapshot, *id) >= carried.saturating_add(*count))
            .then_some("observed requested inventory increase"),
        BankStep::WithdrawX { .. } => (pending.progress.flight.is_some()
            && snapshot.count_dialog_open())
        .then_some("amount dialog opened"),
        BankStep::Wear { id } => wearing(snapshot, *id).then_some("observed worn"),
        BankStep::Close => (!bank_open).then_some("observed closed"),
        BankStep::Walk { .. } => unreachable!("Walk steps in step_walk"),
    };
    if let Some(how) = landed {
        return (false, StepEnd::Landed(how));
    }
    // A Withdraw / Wear moves its source before the server's next update
    // shows it arrive, so these refusals hold only until the first send.
    let sent = pending.progress.flight.is_some();
    let refused = match step {
        BankStep::Withdraw { .. }
        | BankStep::WithdrawX { .. }
        | BankStep::WithdrawXAmount { .. }
            if !bank_open =>
        {
            Some("the bank is closed")
        }
        BankStep::WithdrawX { .. } if !sent && snapshot.count_dialog_open() => {
            Some("a count dialog was already open before Withdraw-X")
        }
        BankStep::WithdrawXAmount { .. } if !sent && !snapshot.count_dialog_open() => {
            Some("the Withdraw-X amount dialog is not open")
        }
        BankStep::Withdraw { id, .. }
        | BankStep::WithdrawX { id }
        | BankStep::WithdrawXAmount { id, .. }
            if !sent && !bank_holds(snapshot, *id) =>
        {
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
    // Once Withdraw-X answered, keep it latched until its inventory delta
    // arrives; never submit the same count twice.
    let in_flight = pending
        .progress
        .flight
        .as_ref()
        .is_some_and(|flight| flight.matches(snapshot))
        || (matches!(step, BankStep::Open) && snapshot.bank_component_id() != -1)
        || (matches!(step, BankStep::WithdrawXAmount { .. })
            && sent
            && !snapshot.count_dialog_open());
    let wrote = !in_flight && {
        let wrote = match step {
            BankStep::Open => open_bank_at_here(driver, snapshot, here, world),
            BankStep::Withdraw { id, count } => withdraw_fixed_count(driver, snapshot, *id, *count),
            BankStep::WithdrawX { id } => open_withdraw_x(driver, snapshot, *id),
            BankStep::WithdrawXAmount { count, .. } => matches!(
                Interactions::new(snapshot, driver).answer_count(*count),
                SendResult::Sent { .. }
            ),
            BankStep::Wear { id } => {
                let mut ix = Interactions::new(snapshot, driver);
                matches!(ix.wear(*id), SendResult::Sent { .. })
            }
            BankStep::Close => {
                let mut ix = Interactions::new(snapshot, driver);
                matches!(ix.close_modal(), SendResult::Sent { .. })
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

fn withdraw_fixed_count<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    id: i32,
    count: i32,
) -> bool {
    let Some(item) = snapshot.bank().iter().find(|item| item.def.id == id) else {
        return false;
    };
    let Some((op, needs_amount_dialog)) =
        super::fill_withdraw_action(&item.actions, count, item.count)
    else {
        return false;
    };
    if needs_amount_dialog {
        return false;
    }
    matches!(
        Interactions::new(snapshot, driver)
            .interact(OpTarget::Item(item), ActionSpec::Operation(op),),
        SendResult::Sent { .. }
    )
}

fn open_withdraw_x<D: Driver>(driver: &mut D, snapshot: &GameSnapshot, id: i32) -> bool {
    let Some(item) = snapshot.bank().iter().find(|item| item.def.id == id) else {
        return false;
    };
    let Some(op) = super::action_slot(&item.actions, "Withdraw X") else {
        return false;
    };
    matches!(
        Interactions::new(snapshot, driver)
            .interact(OpTarget::Item(item), ActionSpec::Operation(op),),
        SendResult::Sent { .. }
    )
}

/// Whether a latched BankBudget session must freeze [`Traveller::follow`].
/// Walk with the access sub-route armed does **not** freeze; Open /
/// Withdraw / Withdraw-X / Wear / Close do. Mid-session `final_route` is
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
            BankStep::Withdraw { id, .. }
            | BankStep::WithdrawX { id }
            | BankStep::WithdrawXAmount { id, .. }
            | BankStep::Wear { id } => FlightTarget::Obj {
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav_executor::{TravelOptions, TravelOutcome, Traveller};
use nav_planner::router::{FindOptions, Route};
use nav_planner::tile::Tile;
use nav_planner::world::NavWorld;
use nav_planner::WorldState;

use crate::admission::{Admission, RouteBasis};
use crate::script_runtime::{
    session_freezes_follow, step_bank_fetch_on_bot, step_poison_hold_on_bot, NavBot,
    RoutePrereqContent,
};
use crate::walk_plan::{route_or_bank_fetch, PendingBankFetch, RouteOutcome};
use script::combat::risk::RouteAssessment;
use script::native::{HandoverEntry, OuterIdentity, OuterWalk, WalkHandover, WalkRefusal};
/// Per-username WalkTo arm: the whole-world [`Traveller`] plus the
/// [`Route`] it is following. [`arm_walk_on`] stores the route (found
/// over the shared [`NavWorld`]); the slot hook polls
/// [`Traveller::follow`] with a clone of it one step per player-info
/// tick. `route` being set is the "armed" gate the status row and the
/// overlay read; any terminal outcome clears it (arrival and stall
/// alike). A pending [`PendingBankFetch`] freezes follow for non-Walk
/// steps (Open / Withdraw / Wear / Close); Walk follows the
/// access-tile sub-route only (never `final_route` until the session clears).
/// Shared by the panel and the TUI so a walk armed from either view
/// drives the same follow path.
#[derive(Default)]
pub struct WalkArm {
    pub traveller: Traveller,
    pub route: Option<Arc<Route>>,
    /// Changes when a route is installed or cancelled, fencing the prior owner.
    pub route_generation: u64,
    pub bank_fetch: Option<PendingBankFetch>,
    pub admission: Option<Box<Admission>>,
    pub assessment: Option<Arc<RouteAssessment>>,
    pub(crate) poison_hold: Option<crate::poison_hold::PoisonHold>,
    pub last_assessment: Option<Arc<RouteAssessment>>,
    pub basis: Option<Arc<RouteBasis>>,
    pub refusal: Option<WalkRefusal>,
    pub refusal_detail: Option<Arc<str>>,
    pub admission_pending: bool,
    pub manual_prereq: Option<Box<ManualWalkPrereqContext>>,
}

#[derive(Clone)]
pub struct ManualWalkPrereqContext {
    outer: OuterWalk,
    route_run: api::selected::RunKey,
    content: Arc<RoutePrereqContent>,
    handover_enabled: bool,
    start: crate::ScriptStartHandle,
    slot: Arc<str>,
}

impl ManualWalkPrereqContext {
    pub(crate) fn new(
        outer: OuterWalk,
        route_run: api::selected::RunKey,
        content: Arc<RoutePrereqContent>,
        handover_enabled: bool,
        start: crate::ScriptStartHandle,
        slot: Arc<str>,
    ) -> Self {
        Self {
            outer,
            route_run,
            content,
            handover_enabled,
            start,
            slot,
        }
    }
}

#[cfg(feature = "test-support")]
impl WalkArm {
    /// Build an idle arm with the supplied essence-mine return session.
    pub fn with_latched_essence(session: Option<nav_planner::essence::EssenceSession>) -> Self {
        let mut traveller = Traveller::new();
        traveller.set_essence(session);
        Self {
            traveller,
            ..Default::default()
        }
    }
}

impl WalkArm {
    /// The current follow target, or `None` when no hop is active.
    pub fn current_aim(&self) -> Option<WorldTile> {
        self.traveller.current_aim()
    }

    /// The latched essence-mine return session, if one is known.
    pub fn essence(&self) -> Option<nav_planner::essence::EssenceSession> {
        self.traveller.essence()
    }

    /// The armed route's dest as a tile, `None` when idle.
    pub fn queued_tile(&self) -> Option<Tile> {
        self.route.as_ref().map(|r| Tile {
            x: r.dest.x,
            z: r.dest.z,
            level: r.dest.level,
        })
    }

    /// Current assessment or retained refusal; cloning the Arc needs no new text.
    pub fn risk_reason(&self) -> Option<&Arc<str>> {
        self.refusal_detail.as_ref().or_else(|| {
            self.assessment
                .as_ref()
                .or(self.last_assessment.as_ref())
                .map(|assessment| &assessment.reason)
        })
    }

    /// Whether a WalkArm / scenario follow may poll this frame. Guardian
    /// hold freezes follow; the armed route stays latched.
    pub fn may_follow(hold: bool) -> bool {
        !hold
    }
}

/// The per-slot walk arms keyed by username (shared with the panel and
/// TUI; [`arm_walk_on`] latches the picked route on the focused slot).
pub type WalkArms = Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>>;

/// `arm_walk_on` route failure: the picked dest has no route, or the strict
/// search was incomplete. The caller keeps its picked dest and shows detail.
#[derive(Debug)]
pub struct NoPath {
    pub assessment: Option<Arc<RouteAssessment>>,
    pub refusal: Option<WalkRefusal>,
    pub detail: Option<Arc<str>>,
    pub incomplete_search: bool,
    pub generation: u64,
}

#[derive(Debug)]
pub struct WalkRoute {
    pub route: Route,
    pub assessment: Arc<RouteAssessment>,
    pub generation: u64,
}
/// Route `from` → `dest` over `world` and latch the found route on the
/// focused slot's [`WalkArm`] (keyed by username) when one is named.
/// `options` carries the caller's nav settings (`allow_teleports` /
/// `allow_wilderness` / `allow_bank_fetch`); the focused arm's latched
/// essence-mine session is fed in after — a player inside the mine can
/// walk out through the exit portal's return hop, a slot with no latch
/// keeps the mine sealed. `state` gates payable edges: the focused slot's
/// last published snapshot facts, fail-closed [`WorldState::empty`] when
/// none. `bank` is the account's bank memory rows ([`crate::Play::bank_rows`])
/// the BankBudget session is planned over
/// ([`nav_planner::bank_fetch::planning_rows`]): a `Session` or `Unknown`
/// bank that lacks a missing item is `NoPath`; a `Hint` plans the one
/// verifying trip, which the open bank settles.
/// On success the caller's picked dest is stored by the arm's route;
/// when `allow_bank_fetch` is on and `find` fails only on missing
/// item/worn reqs, a [`PendingBankFetch`] is latched and the post-session
/// route is armed. Returns `Err(NoPath)` when no path (and no session)
/// exists.
#[allow(clippy::too_many_arguments)]
pub fn arm_walk_on(
    world: &NavWorld,
    from: Tile,
    dest: Tile,
    options: FindOptions,
    state: &WorldState,
    bank: &nav_planner::bank_fetch::BankRows,
    admission: Admission,
    travellers: &WalkArms,
    focused: Option<&str>,
) -> Result<WalkRoute, NoPath> {
    arm_walk_on_with_prereq(
        world,
        from,
        dest,
        options,
        state,
        bank,
        admission,
        travellers,
        focused,
        |_| None,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the shared arm boundary retains the route inputs and optional prerequisite context"
)]
pub(crate) fn arm_walk_on_with_prereq(
    world: &NavWorld,
    from: Tile,
    dest: Tile,
    options: FindOptions,
    state: &WorldState,
    bank: &nav_planner::bank_fetch::BankRows,
    mut admission: Admission,
    travellers: &WalkArms,
    focused: Option<&str>,
    manual_prereq: impl FnOnce(&Route) -> Option<Box<ManualWalkPrereqContext>>,
) -> Result<WalkRoute, NoPath> {
    let from_w = WorldTile {
        x: from.x,
        z: from.z,
        level: from.level,
    };
    let dest_w = WorldTile {
        x: dest.x,
        z: dest.z,
        level: dest.level,
    };
    // The focused slot's latched essence-mine session: a player standing
    // inside the mine can WalkTo out through the exit portal (the router
    // synthesizes the return hop from the latch). A slot with no latch
    // stays fail-closed — the mine is sealed, exactly like the packed
    // graph.
    let mut options = options;
    options.essence = focused.and_then(|name| {
        travellers
            .lock()
            .expect("a panic while holding the walk-arm registry invalidates its route ownership")
            .get(name)
            .and_then(|arm| {
                arm.lock()
                    .expect("a panic while holding a walk arm invalidates its traveller state")
                    .traveller
                    .essence()
            })
    });
    if let Some(escape) = focused
        .and_then(|name| {
            travellers
                .lock()
                .expect(
                    "a panic while holding the walk-arm registry invalidates its route ownership",
                )
                .get(name)
                .cloned()
        })
        .and_then(|arm| {
            arm.lock()
                .expect("a panic while holding a walk arm invalidates its admission state")
                .admission
                .as_deref()
                .and_then(|admission| admission.escape)
        })
    {
        admission.escape = Some(escape);
    }
    if admission.generation == 0 {
        admission.generation = crate::walk_map::next_map_route_generation();
    }
    let generation = admission.generation;
    let admitted = crate::admission::route(
        world,
        &admission,
        options,
        |options| {
            route_or_bank_fetch(
                world,
                from_w,
                dest_w,
                options,
                state,
                bank.origin,
                &bank.rows,
                &[],
            )
        },
        || {
            nav_planner::router::find_blocking_zones(
                &world.collision,
                &world.graph,
                from_w,
                dest_w,
                options,
                state,
                &[],
            )
        },
    );
    let search_result = match &admitted.outcome {
        RouteOutcome::IncompleteSearch { search_result } => Some(*search_result),
        RouteOutcome::NoPath if !admitted.blocked.is_empty() => {
            Some(crate::walk_plan::RouteSearchResultKind::Danger)
        }
        RouteOutcome::NoPath if admitted.refusal.is_some() => {
            Some(crate::walk_plan::RouteSearchResultKind::Refusal)
        }
        RouteOutcome::NoPath => Some(crate::walk_plan::RouteSearchResultKind::NoPath),
        RouteOutcome::Routed(_) | RouteOutcome::BankSession { .. } => None,
    };
    let incomplete_search = matches!(
        search_result,
        Some(crate::walk_plan::RouteSearchResultKind::BudgetExhausted { .. })
    );
    let assessment = admitted.assessment;
    let outcome = if admitted.refusal.is_some() || incomplete_search {
        RouteOutcome::NoPath
    } else {
        admitted.outcome
    };
    match outcome {
        outcome @ (RouteOutcome::Routed(_) | RouteOutcome::BankSession { .. }) => {
            let assessment = assessment.expect("a found route is always assessed");
            crate::admission::log(focused, &assessment, false);
            let (route, bank_fetch) = match outcome {
                RouteOutcome::Routed(route) => (Arc::new(route), None),
                RouteOutcome::BankSession { pending, .. } => (
                    Arc::clone(
                        pending
                            .final_route
                            .legacy_route()
                            .expect("panel route session retains its legacy route"),
                    ),
                    Some(pending),
                ),
                RouteOutcome::IncompleteSearch { .. } | RouteOutcome::NoPath => unreachable!(),
            };
            let manual_prereq = manual_prereq(&route);
            replace_walk_arm(
                travellers,
                focused,
                from_w,
                Arc::clone(&route),
                bank_fetch,
                admission,
                Arc::clone(&assessment),
                options,
                state,
                manual_prereq,
            );
            Ok(WalkRoute {
                route: route.as_ref().clone(),
                assessment,
                generation,
            })
        }
        RouteOutcome::IncompleteSearch { .. } | RouteOutcome::NoPath => {
            let refusal_detail = admitted.refusal.as_ref().map(|refusal| {
                assessment
                    .as_ref()
                    .map(|assessment| Arc::clone(&assessment.reason))
                    .unwrap_or_else(|| {
                        if let Some(escape) = admission
                            .escape
                            .filter(|escape| crate::admission::escape_live(*escape))
                        {
                            return Arc::from(crate::admission::escape_reason(escape));
                        }
                        let blocked = world
                            .graph
                            .zones
                            .as_ref()
                            .filter(|_| !admitted.blocked.is_empty());
                        Arc::from(match blocked {
                            Some(table) => format!(
                                "{refusal:?}; {}",
                                crate::blocked_zone_detail(table, &admitted.blocked)
                            ),
                            None => format!("{refusal:?}"),
                        })
                    })
            });
            let detail = match search_result {
                Some(search_result) => {
                    let search_detail = format!("search_result={search_result}");
                    Some(Arc::from(match refusal_detail {
                        Some(refusal_detail) => {
                            format!("{search_detail}; {refusal_detail}")
                        }
                        None => search_detail,
                    }))
                }
                None => refusal_detail,
            };
            // An incomplete search cannot decide whether the held walk is
            // invalid. Keep the existing arm unchanged.
            if !incomplete_search {
                // A held (non-enforcing) NoPath stays the pre-S2b bare refusal:
                // it neither creates an arm nor stops one already walking. Only
                // an enforcing refusal or a live escape is retained on the arm.
                let retain = admission.decision.enforce
                    || admission.escape.is_some_and(crate::admission::escape_live);
                if let Some(name) = focused
                    .filter(|_| retain && (assessment.is_some() || admitted.refusal.is_some()))
                {
                    if let Some(assessment) = assessment.as_ref() {
                        crate::admission::log(Some(name), assessment, false);
                    }
                    let arm = travellers
                        .lock()
                        .expect("a panic while holding the walk-arm registry invalidates its route ownership")
                        .entry(name.to_string())
                        .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
                        .clone();
                    let mut arm = arm
                        .lock()
                        .expect("a panic while holding a walk arm invalidates its admission state");
                    if let Some(assessment) = assessment.as_ref() {
                        arm.last_assessment = Some(Arc::clone(assessment));
                    }
                    if !admission.escape.is_some_and(crate::admission::escape_live) {
                        arm.traveller.clear();
                        arm.route = None;
                        arm.poison_hold = None;
                        arm.bank_fetch = None;
                        arm.basis = None;
                        arm.assessment = None;
                        arm.admission = Some(Box::new(admission));
                        arm.route_generation = admission.generation;
                        arm.admission_pending = false;
                    }
                    arm.refusal = admitted.refusal.clone();
                    arm.refusal_detail = detail.clone();
                }
            }
            Err(NoPath {
                assessment,
                refusal: admitted.refusal,
                detail,
                incomplete_search,
                generation,
            })
        }
    }
}

fn with_walk_arm_bot<R>(arm: &mut WalkArm, run: impl FnOnce(&mut NavBot) -> R) -> R {
    let mut bot = NavBot {
        route: arm.route.take(),
        bank_fetch: arm.bank_fetch.take(),
        traveller: std::mem::take(&mut arm.traveller),
        route_generation: arm.route_generation,
        admission: arm.admission.take(),
        assessment: arm.assessment.take(),
        poison_hold: arm.poison_hold.take(),
        last_assessment: arm.last_assessment.take(),
        route_basis: arm.basis.take(),
        admission_pending: std::mem::take(&mut arm.admission_pending),
        ..Default::default()
    };
    let result = run(&mut bot);
    arm.route = bot.route.take();
    arm.bank_fetch = bot.bank_fetch.take();
    arm.traveller = std::mem::take(&mut bot.traveller);
    arm.admission = bot.admission.take();
    arm.assessment = bot.assessment.take();
    arm.poison_hold = bot.poison_hold.take();
    arm.last_assessment = bot.last_assessment.take();
    arm.basis = bot.route_basis.take();
    arm.admission_pending = bot.admission_pending;
    if let Some(refusal) = bot.risk_refusal {
        arm.refusal = Some(refusal.1);
        arm.refusal_detail = arm
            .admission
            .as_deref()
            .and_then(|admission| admission.escape)
            .filter(|escape| crate::admission::escape_live(*escape))
            .map(|escape| Arc::from(crate::admission::escape_reason(escape)))
            .or_else(|| {
                arm.last_assessment
                    .as_ref()
                    .map(|assessment| Arc::clone(&assessment.reason))
            });
    }
    result
}
/// Public WalkArm BankBudget pump (panel / TUI follow path). Same step
/// semantics as the script [`NavBot`] pump: the session, with its step
/// budget and in-flight latch, moves through the pump and back, so both
/// persist across pumps.
pub fn step_walk_arm_bank_fetch<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
    map_members: bool,
) -> bool {
    with_walk_arm_bot(arm, |bot| {
        if let Some(world) = world {
            crate::admission::publish(bot, world, snapshot, map_members, None);
        }
        if step_poison_hold_on_bot(driver, snapshot, bot, world, None, true) {
            return false;
        }
        let wrote = step_bank_fetch_on_bot(driver, snapshot, bot, world, here, map_members);
        if let Some(world) = world {
            crate::admission::publish(bot, world, snapshot, map_members, None);
        }
        let _ = step_poison_hold_on_bot(driver, snapshot, bot, world, None, true);
        wrote
    })
}

/// Whether a WalkArm BankBudget session freezes follow this frame
/// (panel / TUI). Same rule as the script [`NavBot`] pump.
pub fn walk_arm_bank_fetch_freezes_follow(arm: &WalkArm) -> bool {
    session_freezes_follow(arm.bank_fetch.as_ref(), arm.route.as_deref())
}

fn walk_destination(arm: &WalkArm) -> Option<WorldTile> {
    arm.bank_fetch
        .as_ref()
        .map(|pending| pending.dest)
        .or_else(|| arm.route.as_ref().map(|route| route.dest))
}

fn bank_stand_route_active(arm: &WalkArm) -> bool {
    let Some(route) = arm.route.as_ref() else {
        return false;
    };
    arm.bank_fetch.as_ref().is_some_and(|pending| {
        matches!(
            pending.steps.front(),
            Some(nav_planner::bank_fetch::BankStep::Walk { x, z, level })
                if route.dest.x == *x && route.dest.z == *z && route.dest.level == *level
        )
    })
}

/// Replace the focused slot's in-flight WalkTo state with a newly routed
/// request, preserving one consistent cancellation receipt for both direct
/// routes and BankBudget sessions.
#[allow(clippy::too_many_arguments)]
fn replace_walk_arm(
    travellers: &WalkArms,
    focused: Option<&str>,
    from: WorldTile,
    route: Arc<Route>,
    bank_fetch: Option<PendingBankFetch>,
    admission: Admission,
    assessment: Arc<RouteAssessment>,
    options: FindOptions,
    state: &WorldState,
    manual_prereq: Option<Box<ManualWalkPrereqContext>>,
) {
    let Some(name) = focused else {
        return;
    };
    let arm = travellers
        .lock()
        .unwrap()
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
        .clone();
    let replaced = {
        let mut arm = arm.lock().unwrap();
        let replaced =
            walk_destination(&arm).map(|destination| (destination, arm.route_generation));
        // A fresh arm replaces any in-flight follow run and its prior
        // BankBudget session. Traveller::clear preserves the essence latch.
        arm.traveller.clear();
        arm.poison_hold = None;
        arm.bank_fetch = bank_fetch;
        arm.route = Some(Arc::clone(&route));
        arm.route_generation = admission.generation;
        arm.admission = Some(Box::new(admission));
        arm.assessment = Some(assessment);
        arm.basis = Some(Arc::new(RouteBasis {
            _route: crate::walk_plan::RoutePath::legacy(Arc::clone(&route)),
            options,
            avoid: Arc::from([]),
            quest_evidence: state.quest_evidence.clone(),
        }));
        arm.refusal = None;
        arm.refusal_detail = None;
        arm.admission_pending = true;
        arm.manual_prereq = manual_prereq;
        replaced
    };
    if let Some((destination, generation)) = replaced {
        crate::walk_map::emit_walk_cancelled(
            Some(name),
            destination,
            Some(from),
            generation,
            "Replaced",
        );
    }
}

/// Publish a manual worker result against the current observed snapshot.
/// Both frontends call this before their hold/mover gates: observation and
/// refusal do not emit a movement click. The runtime observer remains S2c.
pub fn observe_walk_arm_admission(
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    world: &NavWorld,
    map_members: bool,
    slot: Option<&str>,
) -> bool {
    if !arm.admission_pending {
        return false;
    }
    let before = walk_destination(arm);
    with_walk_arm_bot(arm, |bot| {
        crate::admission::publish(bot, world, snapshot, map_members, slot);
    });
    finish_walk_arm_refusal(before, snapshot, arm, slot, world.graph.zones.is_none())
}

fn finish_walk_arm_refusal(
    before: Option<WorldTile>,
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    slot: Option<&str>,
    zones_unknown: bool,
) -> bool {
    if let (Some(destination), None, Some(assessment)) =
        (before, walk_destination(arm), arm.last_assessment.as_ref())
    {
        if arm.refusal.is_some() {
            let escape = arm
                .admission
                .as_deref()
                .and_then(|admission| admission.escape)
                .filter(|escape| crate::admission::escape_live(*escape))
                .map(crate::admission::escape_reason);
            crate::walk_map::emit_walk_aborted(
                slot,
                destination,
                snapshot
                    .tile()
                    .map(|(x, z, level)| WorldTile { x, z, level }),
                escape.as_deref().unwrap_or(&assessment.reason),
                zones_unknown,
            );
            clear_admission(arm);
            return true;
        }
    }
    false
}

/// Panel/TUI WalkTo pump: advance BankBudget work first, then the manual
/// route. The exact traveller outcome is logged before the route is cleared.
/// A successful bank-stand sub-route is internal and therefore never emits a
/// terminal WalkTo receipt; its final route remains the operator's request.
pub fn step_walk_arm_follow<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    world: Option<&NavWorld>,
    here: (i32, i32, i32),
    map_members: bool,
    slot: Option<&str>,
) -> bool {
    if world
        .is_some_and(|world| observe_walk_arm_admission(snapshot, arm, world, map_members, slot))
    {
        return true;
    }

    let before = walk_destination(arm);
    let poison_hold_active = with_walk_arm_bot(arm, |bot| {
        if let Some(world) = world {
            crate::admission::publish(bot, world, snapshot, map_members, slot);
        }
        step_poison_hold_on_bot(driver, snapshot, bot, world, slot, true)
    });
    if finish_walk_arm_refusal(
        before,
        snapshot,
        arm,
        slot,
        world.is_none_or(|world| world.graph.zones.is_none()),
    ) {
        return true;
    }
    if poison_hold_active {
        return false;
    }
    if arm
        .admission
        .as_deref()
        .and_then(|admission| admission.escape)
        .is_some_and(crate::admission::escape_live)
    {
        return false;
    }
    if arm.bank_fetch.is_some() {
        let walking_stand_before = bank_stand_route_active(arm);
        let bank_destination = walk_destination(arm);
        step_walk_arm_bank_fetch(driver, snapshot, arm, world, Some(here), map_members);
        if walking_stand_before && !bank_stand_route_active(arm) {
            // BankBudget completed its private stand route before the
            // traveller polled that arrival. Discard that private follow
            // state before the operator's final route is resumed.
            arm.traveller.clear();
        }
        let hold_active = with_walk_arm_bot(arm, |bot| {
            step_poison_hold_on_bot(driver, snapshot, bot, world, slot, true)
        });
        if finish_walk_arm_refusal(
            bank_destination,
            snapshot,
            arm,
            slot,
            world.is_none_or(|world| world.graph.zones.is_none()),
        ) {
            return true;
        }
        if hold_active {
            return false;
        }
        if arm.bank_fetch.is_none() && arm.route.is_none() {
            arm.traveller.clear();
            if let Some(destination) = bank_destination {
                crate::walk_map::emit_walk_aborted(
                    slot,
                    destination,
                    Some(WorldTile {
                        x: here.0,
                        z: here.1,
                        level: here.2,
                    }),
                    arm.refusal
                        .as_ref()
                        .and(arm.last_assessment.as_ref())
                        .map_or("BankFetch", |assessment| assessment.reason.as_ref()),
                    world.is_some_and(|world| world.graph.zones.is_none()),
                );
            }
            clear_admission(arm);
            return true;
        }
        if walk_arm_bank_fetch_freezes_follow(arm) {
            return false;
        }
    }
    let Some(route) = arm.route.as_ref() else {
        return false;
    };
    let destination = walk_destination(arm).unwrap_or(route.dest);
    let walking_stand = bank_stand_route_active(arm);
    let mut options = TravelOptions {
        close_enough: 0,
        teleports: world.map(|world| world.graph.teleports.as_slice()),
        edges: world.map(|world| world.graph.edges.as_slice()),
        quest_evidence: arm
            .basis
            .as_deref()
            .and_then(|basis| basis.quest_evidence.as_ref()),
        route_index_limit: arm.poison_hold.as_ref().map(|hold| hold.until_index),
        ..TravelOptions::default()
    };
    if let Some(context) = arm.manual_prereq.as_deref() {
        options.refusal_texts = Arc::clone(&context.content.refusal_texts);
        options.crossing_passes = Arc::clone(&context.content.crossing_passes);
    }
    let outcome = arm
        .traveller
        .follow_shared(driver, snapshot, Arc::clone(route), &mut options);
    // Release the borrowed route evidence before clearing the arm.
    drop(options);
    if walking_stand && matches!(&outcome, Some(TravelOutcome::Arrived { .. })) {
        arm.route = None;
        arm.poison_hold = None;
        return false;
    }
    let Some(outcome) = outcome else {
        return false;
    };
    let leg = if walking_stand || matches!(&outcome, TravelOutcome::Arrived { .. }) {
        None
    } else {
        arm.traveller.terminal_leg_index()
    };
    let handed_over = matches!(&outcome, TravelOutcome::GateRefused { .. })
        && handover_manual_refusal(&mut arm.manual_prereq, &outcome);
    if !handed_over {
        crate::walk_map::emit_walk_terminal(
            slot,
            destination,
            &outcome,
            leg,
            route,
            world.is_some_and(|world| world.graph.zones.is_none()),
        );
    }
    if walking_stand {
        arm.bank_fetch = None;
    }
    arm.route = None;
    clear_admission(arm);
    true
}

fn handover_manual_refusal(
    pending: &mut Option<Box<ManualWalkPrereqContext>>,
    outcome: &TravelOutcome,
) -> bool {
    let Some(context) = pending.take().map(|context| *context) else {
        return false;
    };
    let TravelOutcome::GateRefused {
        at,
        leg,
        crossing,
        gates,
        matched,
        ..
    } = outcome
    else {
        return false;
    };
    let Some(observation) = context
        .content
        .refusal_observation(*leg, *crossing, gates, *matched)
    else {
        return false;
    };
    if !context
        .start
        .record_gate_refusal(context.slot.as_ref(), context.route_run, &observation)
    {
        return false;
    }
    api::host_log!(
        api::hostlog::Category::NavEvent,
        api::hostlog::Level::Info,
        slot = context.slot.as_ref(),
        "prereq refusal-record owner=manual run={:?} leg={} crossing={:?} tick={} sequence={}",
        context.route_run,
        observation.leg,
        observation.crossing,
        observation.fact.observed.tick,
        observation.fact.observed.sequence,
    );
    if !context.handover_enabled
        || (!context.outer.prereq_allow.dialogue && !context.outer.prereq_allow.quest_start)
    {
        return false;
    }
    let intent_seq = match context.outer.identity {
        OuterIdentity::Manual { intent_seq } => intent_seq,
        OuterIdentity::Compat { .. } => return false,
    };
    api::host_log!(
        api::hostlog::Category::NavEvent,
        api::hostlog::Level::Info,
        slot = context.slot.as_ref(),
        "prereq handover owner=manual entry=GateRefused run={:?} intent_seq={} leg={} crossing={:?}",
        context.route_run,
        intent_seq,
        leg,
        crossing,
    );
    let handover = WalkHandover {
        outer: context.outer,
        entry: HandoverEntry::GateRefused(observation),
        here: *at,
    };
    context
        .start
        .start_manual_walk_to(context.slot.as_ref(), handover, context.route_run)
        .is_ok()
}

fn clear_admission(arm: &mut WalkArm) {
    arm.admission = None;
    arm.assessment = None;
    arm.basis = None;
    arm.poison_hold = None;
    arm.admission_pending = false;
    arm.manual_prereq = None;
}

/// Cancel one active manual WalkTo at a frontend/session ownership boundary.
/// Returns whether an active request was closed.
pub fn cancel_walk_arm(
    slot: Option<&str>,
    arm: &mut WalkArm,
    at: Option<WorldTile>,
    reason: &'static str,
) -> bool {
    let Some(destination) = walk_destination(arm) else {
        return false;
    };
    crate::walk_map::emit_walk_cancelled(slot, destination, at, arm.route_generation, reason);
    arm.traveller.clear();
    arm.route_generation = arm.route_generation.wrapping_add(1);
    arm.route = None;
    arm.bank_fetch = None;
    clear_admission(arm);
    true
}

/// Both frontends call this before any scenario, local-player or hold return.
/// Cancelling one member never changes another slot or pauses an unrelated script.
pub fn cancel_walk_arm_on_manual_input(
    slot: &str,
    arms: &WalkArms,
    at: Option<WorldTile>,
    frame: crate::SlotFrameInput,
) -> bool {
    if frame.manual_move_count() == 0 {
        return false;
    }
    let Some(arm) = arms.lock().unwrap().get(slot).cloned() else {
        return false;
    };
    let mut owned = arm.lock().unwrap();
    cancel_walk_arm(Some(slot), &mut owned, at, "UserInput")
}

#[cfg(test)]
mod prereq_layout_tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn idle_manual_arm_keeps_prerequisite_context_boxed_and_unallocated() {
        let idle = allocation_counter::measure(|| {
            let arm = std::hint::black_box(WalkArm::default());
            assert!(arm.manual_prereq.is_none());
        });
        eprintln!(
            "prereq manual layout WalkArm={} ManualWalkPrereqContext={} idle_context={} SlotScript={} idle={idle:?}",
            size_of::<WalkArm>(),
            size_of::<ManualWalkPrereqContext>(),
            size_of::<Option<Box<ManualWalkPrereqContext>>>(),
            size_of::<script::SlotScript>(),
        );
        assert_eq!(
            size_of::<Option<Box<ManualWalkPrereqContext>>>(),
            size_of::<usize>()
        );
        assert!(size_of::<ManualWalkPrereqContext>() <= 512);
        assert!(size_of::<WalkArm>() <= 4096);
        assert!(size_of::<script::SlotScript>() <= 2048);
        assert_eq!(idle.count_total, 0);
        assert_eq!(idle.bytes_total, 0);
    }
}

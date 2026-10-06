use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::traveller::{TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;
use nav::WorldState;

use crate::script_runtime::{session_freezes_follow, step_bank_fetch_on_bot, NavBot};
use crate::walk_plan::{route_or_bank_fetch, PendingBankFetch, RouteOutcome};
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
    pub route: Option<Route>,
    /// Changes when a route is installed or cancelled, fencing the prior owner.
    pub route_generation: u64,
    pub bank_fetch: Option<PendingBankFetch>,
}

impl WalkArm {
    /// The armed route's dest as a tile, `None` when idle.
    pub fn queued_tile(&self) -> Option<Tile> {
        self.route.as_ref().map(|r| Tile {
            x: r.dest.x,
            z: r.dest.z,
            level: r.dest.level,
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

/// `arm_walk_on` no-path result: the picked dest has no route under the
/// caller's nav options (the caller keeps its picked dest and shows a
/// short error).
#[derive(Debug)]
pub struct NoPath;

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
/// ([`nav::bank_fetch::planning_rows`]): a `Session` or `Unknown`
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
    bank: &nav::bank_fetch::BankRows,
    travellers: &WalkArms,
    focused: Option<&str>,
) -> Result<Route, NoPath> {
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
            .unwrap()
            .get(name)
            .and_then(|arm| arm.lock().unwrap().traveller.essence())
    });
    let outcome = route_or_bank_fetch(
        world,
        from_w,
        dest_w,
        options,
        state,
        bank.origin,
        &bank.rows,
        &[],
    );
    match outcome {
        RouteOutcome::Routed(route) => {
            replace_walk_arm(travellers, focused, from_w, &route, None);
            Ok(route)
        }
        RouteOutcome::BankSession { pending, route } => {
            replace_walk_arm(travellers, focused, from_w, &route, Some(pending));
            Ok(route)
        }
        RouteOutcome::NoPath => Err(NoPath),
    }
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
    // Reuse NavBot stepping by moving the arm's session and route into
    // the same shape; every step result moves back.
    let mut bot = NavBot {
        route: arm.route.take(),
        bank_fetch: arm.bank_fetch.take(),
        ..Default::default()
    };
    let wrote = step_bank_fetch_on_bot(driver, snapshot, &mut bot, world, here, map_members);
    arm.bank_fetch = bot.bank_fetch.take();
    // The step may arm the access sub-route or the final route (the
    // follow's route generation moves with it); an abort clears both.
    arm.route = bot.route.take();
    if arm.route.is_some() && bot.map_route_generation != 0 {
        arm.route_generation = bot.map_route_generation;
    }
    wrote
}

/// Whether a WalkArm BankBudget session freezes follow this frame
/// (panel / TUI). Same rule as the script [`NavBot`] pump.
pub fn walk_arm_bank_fetch_freezes_follow(arm: &WalkArm) -> bool {
    session_freezes_follow(arm.bank_fetch.as_ref(), arm.route.as_ref())
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
            Some(nav::bank_fetch::BankStep::Walk { x, z, level })
                if route.dest.x == *x && route.dest.z == *z && route.dest.level == *level
        )
    })
}

/// Replace the focused slot's in-flight WalkTo state with a newly routed
/// request, preserving one consistent cancellation receipt for both direct
/// routes and BankBudget sessions.
fn replace_walk_arm(
    travellers: &WalkArms,
    focused: Option<&str>,
    from: WorldTile,
    route: &Route,
    bank_fetch: Option<PendingBankFetch>,
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
        arm.bank_fetch = bank_fetch;
        arm.route = Some(route.clone());
        arm.route_generation = crate::walk_map::next_map_route_generation();
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
                    "BankFetch",
                    world.is_some_and(|world| world.graph.zones.is_none()),
                );
            }
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
        ..TravelOptions::default()
    };
    let outcome = arm
        .traveller
        .follow(driver, snapshot, route.clone(), &mut options);
    if walking_stand && matches!(&outcome, Some(TravelOutcome::Arrived { .. })) {
        arm.route = None;
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
    crate::walk_map::emit_walk_terminal(
        slot,
        destination,
        &outcome,
        leg,
        route,
        world.is_some_and(|world| world.graph.zones.is_none()),
    );
    if walking_stand {
        arm.bank_fetch = None;
    }
    arm.route = None;
    true
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::traveller::Traveller;
use nav::world::NavWorld;
use nav::WorldState;

use crate::script_runtime::{bank_fetch_freezes_follow, step_bank_fetch_on_bot, NavBot};
use crate::walk_plan::{route_or_bank_fetch, PendingBankFetch, RouteOutcome};
/// Per-username WalkTo arm: the whole-world [`Traveller`] plus the
/// [`Route`] it is following. [`arm_walk_on`] stores the route (found
/// over the shared [`NavWorld`]); the slot hook polls
/// [`Traveller::follow`] with a clone of it one step per player-info
/// tick. `route` being set is the "armed" gate the status row and the
/// overlay read; any terminal outcome clears it (arrival and stall
/// alike). A pending [`PendingBankFetch`] freezes follow for non-Walk
/// steps (Open / deposit / withdraw / Wear / Close); Walk follows the
/// stand sub-route only (never `final_route` until the session clears).
/// Shared by the panel and the TUI so a walk armed from either view
/// drives the same follow path.
#[derive(Default)]
pub struct WalkArm {
    pub traveller: Traveller,
    pub route: Option<Route>,
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
/// none. `bank` is the open bank's rows (obj id, count) for the BankBudget
/// session — empty when the bank is closed (no closed-bank inventory).
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
    bank: &[(i32, i32)],
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
    let outcome = route_or_bank_fetch(world, from_w, dest_w, options, state, bank);
    match outcome {
        RouteOutcome::Routed(route) => {
            if let Some(name) = focused {
                let arm = travellers
                    .lock()
                    .unwrap()
                    .entry(name.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
                    .clone();
                let mut arm = arm.lock().unwrap();
                // A fresh arm replaces any in-flight follow run.
                arm.traveller.clear();
                arm.bank_fetch = None;
                arm.route = Some(route.clone());
            }
            Ok(route)
        }
        RouteOutcome::BankSession { pending, route } => {
            if let Some(name) = focused {
                let arm = travellers
                    .lock()
                    .unwrap()
                    .entry(name.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
                    .clone();
                let mut arm = arm.lock().unwrap();
                arm.traveller.clear();
                arm.bank_fetch = Some(pending);
                arm.route = Some(route.clone());
            }
            Ok(route)
        }
        RouteOutcome::NoPath => Err(NoPath),
    }
}
/// Public WalkArm BankBudget pump (panel / TUI follow path). Same step
/// semantics as the script [`NavBot`] pump.
pub fn step_walk_arm_bank_fetch<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
    map_members: bool,
) -> bool {
    // Reuse NavBot stepping by temporarily viewing the arm as the same
    // shape of pending session + route.
    let mut bot = NavBot {
        traveller: Traveller::default(),
        route: arm.route.clone(),
        bank_fetch: arm.bank_fetch.take(),
        allow_teleports: false,
        ..Default::default()
    };
    let wrote = step_bank_fetch_on_bot(driver, snapshot, &mut bot, world, here, map_members);
    arm.bank_fetch = bot.bank_fetch;
    // Walk-to-stand may have armed a temporary route on the bot; abort
    // clears both session and route.
    if arm.bank_fetch.is_none() && bot.route.is_none() {
        arm.route = None;
    } else if let Some(r) = bot.route {
        arm.route = Some(r);
    }
    wrote
}

/// Whether a WalkArm BankBudget session freezes follow this frame
/// (panel / TUI). Same rule as the script [`NavBot`] pump.
pub fn walk_arm_bank_fetch_freezes_follow(arm: &WalkArm) -> bool {
    bank_fetch_freezes_follow(&NavBot {
        traveller: Traveller::default(),
        route: arm.route.clone(),
        bank_fetch: arm.bank_fetch.clone(),
        allow_teleports: false,
        ..Default::default()
    })
}

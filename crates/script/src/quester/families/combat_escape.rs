//! Escape-goal planning for an aborted Quester fight.
//!
//! The adapter used to pick one tile at the chase-range distance and check
//! only world-coordinate bounds. A tile inside a tree footprint has no legal
//! Reach arrival goal, so the host refused the route and the bot stood still.
//!
//! This module does not decide what is reachable. It names exact candidate
//! tiles in three tiers and hands them to the host router as a [`WalkGoals`]
//! request. The router owns reachability: it searches the baked collision and
//! transports, applies the account's own restrictions, and takes the cheapest
//! reachable goal of the preferred tier before any goal of the fallback tier.
//! The Gatherer's observation walks use the same interface. A candidate the
//! router cannot reach costs nothing, and an unreachable pocket, a door or a
//! detour outside any local window needs no code here.
use super::tile_distance;
use crate::native::WalkGoals;
use crate::reach_route;
use api::query::ReachQueryView;
use api::WorldTile;
use nav_planner::contract::CollisionView;
use nav_planner::router::NODE_BUDGET;

/// The destination tiers, best first. Each is a fact of the content:
/// - `Stand`: the Path's authored stand, outside the attacker's chase envelope.
/// - `Beyond`: tiles outside the chase envelope. The engine drops a melee
///   target once the player is more than `maxrange + 1` from the NPC's spawn,
///   and the NPC itself may stand `maxrange` from it, so the envelope measured
///   from the NPC's tile is `2 * maxrange + attackrange` (`Npc.ts:652-667`).
/// - `Away`: tiles inside the envelope that are still farther from the
///   attacker than the player is, better than standing in its reach.
const STAND: usize = 0;
const BEYOND: usize = 1;
const AWAY: usize = 2;

/// Walk requests one escape pass may make: one per non-empty tier. A request
/// names its preferred tier and the next non-empty tier as the router's
/// fallback, and the next request of the pass starts one tier lower. The
/// tiers may share tiles (the stand's neighbours can also lie beyond the
/// envelope), so two requests can overlap, but the request count cannot pass
/// this. Each request is a fresh plan from the observed position. A request
/// ends with its walk receipt or, when none comes, with its observed-tick
/// budget ([`ESCAPE_SLACK_TICKS`], [`ESCAPE_MAX_WALK_STEPS`]). The walk
/// machine's ten-minute active-clock deadline stays as the outer backstop.
pub(super) const ESCAPE_PASS_ATTEMPTS: usize = 3;

/// Chebyshev margin around the authored stand that still counts as standing
/// at it. It is the radius the stand walk used before goal tiers.
const STAND_RADIUS: i32 = 1;

/// Largest ring radius of the beyond tier around the attacker. A ring of this
/// radius has 512 tiles, which is a whole tier. The away tier does not use
/// it: its rings lie inside the chase envelope.
const MAX_RING: i32 = 64;

/// Goals one tier may name. The router's cap applies to the union of both
/// tiers of a request, so each tier gets half of it and the union of any two
/// tiers always fits.
const TIER_CAP: usize = WalkGoals::MAX / 2;

/// Observed ticks, beyond the measured walking depth, that one escape walk
/// may stay unsettled before it is abandoned and the next tier starts. The
/// same 12 as `shop_open::OPEN_SLACK_TICKS`, split here as:
/// - 1 (engine tick order): the click is read in one cycle and the player
///   moves in a later one;
/// - 1 (engine input budget): a cycle can defer the click's packet;
/// - 10 (host recovery): the nav executor re-sends a click it saw dropped
///   after 5 idle ticks (`nav-executor/src/traveller/walk.rs:8`,
///   `WALK_STALL_RECOVER_IDLE_TICKS`), and this allows two such windows.
///
/// The slack does not model the chase. A walk that the attacker slows or
/// blocks is the stall this budget exists to end.
pub(super) const ESCAPE_SLACK_TICKS: u64 = 12;

/// Largest number of steps a route can have, and so the largest number of
/// observed ticks that a walk to a reachable goal can need at walk speed.
/// The reach flood has no depth limit of its own (`SceneQuery::flood_reach`,
/// `api/src/query/reach.rs:612-614`: "No 400-step cap"), so the bound is the
/// router's: it settles at most [`NODE_BUDGET`] nodes in one search
/// (`nav-planner/src/router.rs:4035-4038`, used for the script's walk
/// requests at `host-play/src/script_nav.rs:323`), and a route of `n` steps
/// passes through `n + 1` settled nodes, so `n` stays below it. A player
/// moves one tile per game tick at walk speed and two at run speed
/// (`PathingEntity.ts:144-149`, with `Player.ts:729-731` choosing walk or
/// run), so the walk takes at most one tick per step. A walk whose depth is
/// not measured (no live flood, or no goal in it) gets this bound plus
/// [`ESCAPE_SLACK_TICKS`]: no walk to a reachable goal needs more. The bound
/// covers walking only. A route that waits on a transport has no tick bound
/// from it.
pub(super) const ESCAPE_MAX_WALK_STEPS: u64 = NODE_BUDGET as u64;

/// Observed-tick clock of the escape walk in flight. It counts distinct game
/// ticks, one per `PLAYER_INFO` (`api/src/snapshot/decode.rs:101-104`), never
/// wall time.
struct WalkClock {
    last_tick: u64,
    remaining: u64,
}

/// Escape state of one abort: where the next request of the pass starts, the
/// clock of the walk in flight, and the last walk failure. A new pass keeps
/// the failure, so an exhausted recovery can report it.
pub(super) struct AbortEscape {
    next_tier: usize,
    walk: Option<WalkClock>,
    last_failure: Option<crate::native::ActionError>,
}

impl AbortEscape {
    pub(super) fn new() -> Self {
        Self {
            next_tier: 0,
            walk: None,
            last_failure: None,
        }
    }

    /// Start a fresh pass from a newly observed position.
    pub(super) fn begin_pass(&mut self) {
        self.next_tier = 0;
    }

    pub(super) fn record_failure(&mut self, error: crate::native::ActionError) {
        self.last_failure = Some(error);
    }

    pub(super) fn last_failure(&self) -> Option<&crate::native::ActionError> {
        self.last_failure.as_ref()
    }

    /// The next request of the pass, planned from the observed `geometry`.
    /// `None` when every tier from the current one down is empty. `reach` is
    /// the live posted flood, when there is one; it measures the walk.
    pub(super) fn next_goals(
        &mut self,
        geometry: &EscapeGeometry,
        collision: Option<CollisionView<'_>>,
        reach: Option<&ReachQueryView>,
    ) -> Option<EscapeGoals> {
        let tiers = tier_tiles(geometry, collision);
        let preferred =
            (self.next_tier..ESCAPE_PASS_ATTEMPTS).find(|&tier| !tiers[tier].is_empty())?;
        self.next_tier = preferred + 1;
        let fallback = (preferred + 1..ESCAPE_PASS_ATTEMPTS).find(|&tier| !tiers[tier].is_empty());
        let focus = if preferred == STAND {
            geometry.stand.unwrap_or(geometry.here)
        } else {
            geometry.here
        };
        let target = tiers[preferred]
            .iter()
            .copied()
            .min_by_key(|tile| (tile_distance(focus, *tile), squared_gap(focus, *tile)))?;
        // The router takes the nearest reachable goal of the preferred tier
        // and the fallback tier only when no preferred goal is reachable.
        let depth = reach.and_then(|view| {
            reach_route::nearest_depth(view, &tiers[preferred]).or_else(|| {
                fallback.and_then(|tier| reach_route::nearest_depth(view, &tiers[tier]))
            })
        });
        let fallback_tiles = fallback.map_or_else(Vec::new, |tier| tiers[tier].clone());
        // Each tier holds at most `TIER_CAP` tiles, so the union fits the cap
        // and construction cannot fail.
        let goals = if fallback_tiles.is_empty() {
            WalkGoals::new(tiers[preferred].clone())
        } else {
            WalkGoals::with_preferred(fallback_tiles, &tiers[preferred])
        }
        .ok()?;
        Some(EscapeGoals {
            target,
            goals,
            budget_ticks: walk_budget_ticks(depth),
        })
    }

    /// Start counting the observed ticks of a new escape walk.
    pub(super) fn start_walk(&mut self, tick: u64, budget_ticks: u64) {
        self.walk = Some(WalkClock {
            last_tick: tick,
            remaining: budget_ticks,
        });
    }

    /// Count the distinct ticks observed since the last call against the
    /// budget of the running walk. `true` once the budget is spent. The clock
    /// is then cleared, so one walk expires once.
    pub(super) fn walk_expired(&mut self, tick: u64) -> bool {
        let Some(clock) = self.walk.as_mut() else {
            return false;
        };
        let elapsed = tick.saturating_sub(clock.last_tick);
        clock.last_tick = clock.last_tick.max(tick);
        clock.remaining = clock.remaining.saturating_sub(elapsed);
        if clock.remaining == 0 {
            self.walk = None;
            return true;
        }
        false
    }
}

/// Walk speed is at least one tile per observed tick (see
/// [`ESCAPE_MAX_WALK_STEPS`]). The router's route is not visible to the
/// script, so a measured budget is the flood depth to the nearest goal of the
/// preferred tier (of the fallback tier when no preferred goal is in the
/// flood) plus [`ESCAPE_SLACK_TICKS`]. Without a measured depth it is
/// [`ESCAPE_MAX_WALK_STEPS`] plus the slack.
fn walk_budget_ticks(depth: Option<u32>) -> u64 {
    depth.map_or(ESCAPE_MAX_WALK_STEPS, u64::from) + ESCAPE_SLACK_TICKS
}

/// What the planner knows about the fight.
pub(super) struct EscapeGeometry {
    /// The player's packet-time tile.
    pub(super) here: WorldTile,
    /// The attacker's packet-time tile, when the report names one.
    pub(super) threat: Option<WorldTile>,
    /// The step's authored stand.
    pub(super) stand: Option<WorldTile>,
    /// Chase envelope of the attacker, in tiles.
    pub(super) chase_range: i32,
}

/// One escape request: the tile that stands for it in logs, the exact goals
/// the router chooses among, and the observed ticks its walk may stay
/// unsettled.
pub(super) struct EscapeGoals {
    pub(super) target: WorldTile,
    pub(super) goals: WalkGoals,
    pub(super) budget_ticks: u64,
}

fn squared_gap(a: WorldTile, b: WorldTile) -> i64 {
    let (dx, dz) = (i64::from(a.x - b.x), i64::from(a.z - b.z));
    dx * dx + dz * dz
}

/// The tiles of each tier. A tile is a candidate when the world has it and,
/// with a baked collision view, it is standable. Without a nav pack the host
/// cannot route either, so every in-bounds tile stays a candidate, as in the
/// Gatherer's hazard retreat.
fn tier_tiles(
    geometry: &EscapeGeometry,
    collision: Option<CollisionView<'_>>,
) -> [Vec<WorldTile>; ESCAPE_PASS_ATTEMPTS] {
    let usable = |tile: WorldTile| {
        (0..=16_383).contains(&tile.x)
            && (0..=16_383).contains(&tile.z)
            && collision.is_none_or(|collision| collision.standable(tile))
    };
    let mut tiers: [Vec<WorldTile>; ESCAPE_PASS_ATTEMPTS] = Default::default();
    let envelope = geometry.chase_range.max(0).saturating_add(1);

    if let Some(stand) = geometry.stand {
        let outside = geometry
            .threat
            .is_none_or(|threat| tile_distance(stand, threat) > envelope);
        if outside {
            for dx in -STAND_RADIUS..=STAND_RADIUS {
                for dz in -STAND_RADIUS..=STAND_RADIUS {
                    let tile = WorldTile {
                        x: stand.x.saturating_add(dx),
                        z: stand.z.saturating_add(dz),
                        level: stand.level,
                    };
                    if usable(tile) {
                        tiers[STAND].push(tile);
                    }
                }
            }
        }
    }

    if let Some(threat) = geometry.threat {
        let level = geometry.here.level;
        let here_gap = tile_distance(geometry.here, threat);
        // Beyond the envelope, and farther than the player already is.
        let first = envelope.max(here_gap).saturating_add(1);
        for radius in first..=MAX_RING {
            if !push_ring(&mut tiers[BEYOND], threat, level, radius, &usable) {
                break;
            }
        }
        // Inside the envelope, outermost ring first.
        for radius in (here_gap.saturating_add(1)..=envelope).rev() {
            if !push_ring(&mut tiers[AWAY], threat, level, radius, &usable) {
                break;
            }
        }
    }
    tiers
}

/// Append the usable tiles of the Chebyshev ring of `radius` around `center`
/// on `level`. `false` when the ring does not fit the tier cap, in which case
/// nothing of it is kept.
fn push_ring(
    tier: &mut Vec<WorldTile>,
    center: WorldTile,
    level: i32,
    radius: i32,
    usable: &impl Fn(WorldTile) -> bool,
) -> bool {
    if radius < 1 {
        return true;
    }
    let before = tier.len();
    let at = |dx: i32, dz: i32| WorldTile {
        x: center.x.saturating_add(dx),
        z: center.z.saturating_add(dz),
        level,
    };
    let edge = (-radius..=radius).flat_map(|d| [at(d, -radius), at(d, radius)]);
    let sides = (1 - radius..radius).flat_map(|d| [at(-radius, d), at(radius, d)]);
    for tile in edge.chain(sides) {
        if usable(tile) {
            tier.push(tile);
        }
    }
    if tier.len() > TIER_CAP {
        tier.truncate(before);
        return false;
    }
    true
}

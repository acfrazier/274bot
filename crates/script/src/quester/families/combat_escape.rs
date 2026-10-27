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
use api::WorldTile;
use nav_planner::contract::CollisionView;

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

/// Walk requests one escape pass may make: one per tier. A request names its
/// preferred tier and the next non-empty tier as the router's fallback, and
/// the next request of the pass starts one tier lower, so no request repeats
/// an earlier one. Each request is a fresh plan from the observed position.
/// A request ends with its walk receipt, which the walk machine's existing
/// deadline bounds. This module adds no timer.
pub(super) const ESCAPE_PASS_ATTEMPTS: usize = 3;

/// Chebyshev margin around the authored stand that still counts as standing
/// at it. It is the radius the stand walk used before goal tiers.
const STAND_RADIUS: i32 = 1;

/// Largest ring radius around the attacker. A ring of this radius has 512
/// tiles, which is a whole tier.
const MAX_RING: i32 = 64;

/// Goals one tier may name. The router's cap applies to the union of both
/// tiers of a request, so each tier gets half of it and the union of any two
/// tiers always fits.
const TIER_CAP: usize = WalkGoals::MAX / 2;

/// Escape state of one abort: where the next request of the pass starts, and
/// the last walk failure. A new pass keeps the failure, so an exhausted
/// recovery can report it.
pub(super) struct AbortEscape {
    next_tier: usize,
    last_failure: Option<crate::native::ActionError>,
}

impl AbortEscape {
    pub(super) fn new() -> Self {
        Self {
            next_tier: 0,
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
    /// `None` when every tier from the current one down is empty.
    pub(super) fn next_goals(
        &mut self,
        geometry: &EscapeGeometry,
        collision: Option<CollisionView<'_>>,
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
        let fallback_tiles = fallback.map_or_else(Vec::new, |tier| tiers[tier].clone());
        // Each tier holds at most `TIER_CAP` tiles, so the union fits the cap
        // and construction cannot fail.
        let goals = if fallback_tiles.is_empty() {
            WalkGoals::new(tiers[preferred].clone())
        } else {
            WalkGoals::with_preferred(fallback_tiles, &tiers[preferred])
        }
        .ok()?;
        Some(EscapeGoals { target, goals })
    }
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

/// One escape request: the tile that stands for it in logs, and the exact
/// goals the router chooses among.
pub(super) struct EscapeGoals {
    pub(super) target: WorldTile,
    pub(super) goals: WalkGoals,
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

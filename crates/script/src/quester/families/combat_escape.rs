//! Escape-goal planning for a quest fight that cannot attack its target.
//!
//! The planner sends exact, standable goals to the route planner. Baked
//! collision removes blocked tiles. The route planner decides reachability.
//! A live Reach flood measures the selected tier's walking depth but does not
//! filter goals.
use super::tile_distance;
use crate::native::WalkGoals;
use crate::reach_route;
use api::query::ReachQueryView;
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

/// An escape makes at most two passes over its three tiers. It keeps one
/// compact record per request. Later requests never send a tile from an
/// earlier failed request, including in the second pass after food runs out.
/// Six 32-byte records plus a one-byte count use 193 fixed bytes per escape.
/// The records do not grow on game ticks.
pub(super) const ESCAPE_PASS_ATTEMPTS: usize = 3;
const ESCAPE_PASS_COUNT: usize = 2;
const MAX_ESCAPE_REQUESTS: usize = ESCAPE_PASS_ATTEMPTS * ESCAPE_PASS_COUNT;

/// Chebyshev margin around the authored stand that still counts as standing
/// at it. It is the radius the stand walk used before goal tiers.
const STAND_RADIUS: i32 = 1;

/// Largest ring radius of the beyond tier around the attacker. A ring at this
/// radius has 512 tiles. The away tier does not use it because its rings are
/// inside the chase envelope.
const MAX_RING: i32 = 64;

/// Maximum number of candidate tiles in one tier. One request sends one tier.
const TIER_CAP: usize = WalkGoals::MAX;

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

/// Observed-tick clock of the escape walk in flight. It counts distinct game
/// ticks, one per `PLAYER_INFO` (`api/src/snapshot/decode.rs:101-104`), never
/// wall time.
struct WalkClock {
    last_tick: u64,
    remaining: u64,
}
#[repr(u8)]
#[derive(Clone, Copy)]
enum GoalRegionShape {
    Unused,
    Stand,
    Rings,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GoalRegion {
    center_x: i32,
    center_z: i32,
    level: i32,
    min_radius: u8,
    max_radius: u8,
    shape: GoalRegionShape,
    stand_x: i32,
    stand_z: i32,
    stand_level: i32,
    stand_mask: u16,
}

impl GoalRegion {
    const EMPTY: Self = Self {
        center_x: 0,
        center_z: 0,
        level: 0,
        min_radius: 0,
        max_radius: 0,
        shape: GoalRegionShape::Unused,
        stand_x: 0,
        stand_z: 0,
        stand_level: 0,
        stand_mask: 0,
    };

    fn for_request(
        tier: usize,
        geometry: &EscapeGeometry,
        goals: &[WorldTile],
        stand_goals: &[WorldTile],
    ) -> Option<Self> {
        let (center, min_radius, max_radius, shape, stand_center, stand_mask) = if tier == STAND {
            let center = geometry.stand?;
            (
                center,
                0,
                u8::try_from(STAND_RADIUS).ok()?,
                GoalRegionShape::Stand,
                None,
                stand_goal_mask(center, goals),
            )
        } else {
            let center = geometry.threat?;
            let mut min_radius = i32::MAX;
            let mut max_radius = 0;
            for goal in goals {
                let radius = tile_distance(center, *goal);
                min_radius = min_radius.min(radius);
                max_radius = max_radius.max(radius);
            }
            let (stand_center, stand_mask) = geometry.stand.map_or((None, 0), |stand| {
                let mask = stand_goal_mask(stand, stand_goals);
                ((mask != 0).then_some(stand), mask)
            });
            (
                center,
                u8::try_from(min_radius).ok()?,
                u8::try_from(max_radius).ok()?,
                GoalRegionShape::Rings,
                stand_center,
                stand_mask,
            )
        };
        Some(Self {
            center_x: center.x,
            center_z: center.z,
            level: center.level,
            min_radius,
            max_radius,
            shape,
            stand_x: stand_center.map_or(0, |stand| stand.x),
            stand_z: stand_center.map_or(0, |stand| stand.z),
            stand_level: stand_center.map_or(0, |stand| stand.level),
            stand_mask,
        })
    }

    fn contains(&self, tile: WorldTile) -> bool {
        let center = WorldTile {
            x: self.center_x,
            z: self.center_z,
            level: self.level,
        };
        if tile.level != center.level {
            return false;
        }
        let radius = tile_distance(center, tile);
        match self.shape {
            GoalRegionShape::Unused => false,
            GoalRegionShape::Stand => stand_goal_mask_contains(center, self.stand_mask, tile),
            GoalRegionShape::Rings => {
                if !(i32::from(self.min_radius)..=i32::from(self.max_radius)).contains(&radius) {
                    return false;
                }
                let stand = WorldTile {
                    x: self.stand_x,
                    z: self.stand_z,
                    level: self.stand_level,
                };
                !stand_goal_mask_contains(stand, self.stand_mask, tile)
            }
        }
    }
}

fn stand_goal_mask(center: WorldTile, goals: &[WorldTile]) -> u16 {
    goals.iter().fold(0, |mask, goal| {
        let dx = goal.x.saturating_sub(center.x);
        let dz = goal.z.saturating_sub(center.z);
        if goal.level == center.level && (-1..=1).contains(&dx) && (-1..=1).contains(&dz) {
            let bit = ((dx + 1) * 3 + (dz + 1)) as u32;
            mask | (1_u16 << bit)
        } else {
            mask
        }
    })
}

fn stand_goal_mask_contains(center: WorldTile, mask: u16, tile: WorldTile) -> bool {
    let dx = tile.x.saturating_sub(center.x);
    let dz = tile.z.saturating_sub(center.z);
    if mask == 0 || tile.level != center.level || !(-1..=1).contains(&dx) || !(-1..=1).contains(&dz)
    {
        return false;
    }
    let bit = ((dx + 1) * 3 + (dz + 1)) as u32;
    mask & (1_u16 << bit) != 0
}

const _: [(); 32] = [(); std::mem::size_of::<GoalRegion>()];

/// Escape state of one abort: the pass tier, the walk clock, the last failure
/// and the fixed list of goal regions already requested.
pub(super) struct AbortEscape {
    next_tier: usize,
    walk: Option<WalkClock>,
    last_failure: Option<crate::native::ActionError>,
    requested_regions: [GoalRegion; MAX_ESCAPE_REQUESTS],
    requested_count: u8,
}

impl AbortEscape {
    pub(super) fn new() -> Self {
        Self {
            next_tier: 0,
            walk: None,
            last_failure: None,
            requested_regions: [GoalRegion::EMPTY; MAX_ESCAPE_REQUESTS],
            requested_count: 0,
        }
    }

    /// Start the second pass. Keep the failed request regions from the first.
    pub(super) fn begin_pass(&mut self) {
        self.next_tier = 0;
    }

    pub(super) fn record_failure(&mut self, error: crate::native::ActionError) {
        self.last_failure = Some(error);
    }

    pub(super) fn last_failure(&self) -> Option<&crate::native::ActionError> {
        self.last_failure.as_ref()
    }

    /// Build the next exact goal request, or return `None` when no tier remains.
    /// `reach` is the live posted flood, when there is one. It measures walking
    /// depth only.
    pub(super) fn next_goals(
        &mut self,
        geometry: &EscapeGeometry,
        collision: Option<CollisionView<'_>>,
        reach: Option<&ReachQueryView>,
    ) -> Option<EscapeGoals> {
        let count = usize::from(self.requested_count);
        if count == MAX_ESCAPE_REQUESTS {
            return None;
        }

        let mut tiers = tier_tiles(geometry, collision);
        let requested = &self.requested_regions[..count];
        for tier in &mut tiers {
            tier.retain(|tile| requested.iter().all(|region| !region.contains(*tile)));
        }
        let preferred =
            (self.next_tier..ESCAPE_PASS_ATTEMPTS).find(|&tier| !tiers[tier].is_empty())?;
        self.next_tier = preferred + 1;
        let focus = if preferred == STAND {
            geometry.stand.unwrap_or(geometry.here)
        } else {
            geometry.here
        };
        let target = tiers[preferred]
            .iter()
            .copied()
            .min_by_key(|tile| (tile_distance(focus, *tile), squared_gap(focus, *tile)))?;
        let depth = reach.and_then(|view| reach_route::nearest_depth(view, &tiers[preferred]));
        let region =
            GoalRegion::for_request(preferred, geometry, &tiers[preferred], &tiers[STAND])?;
        let goals = WalkGoals::new(std::mem::take(&mut tiers[preferred])).ok()?;
        self.requested_regions[count] = region;
        self.requested_count += 1;
        Some(EscapeGoals {
            target,
            goals,
            budget_ticks: walk_budget_ticks(depth),
        })
    }

    /// Start counting the observed ticks of a new escape walk. A walk with no
    /// tick budget (`None`) has no clock, and any clock of an earlier walk is
    /// dropped.
    pub(super) fn start_walk(&mut self, tick: u64, budget_ticks: Option<u64>) {
        self.walk = budget_ticks.map(|remaining| WalkClock {
            last_tick: tick,
            remaining,
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

/// The observed-tick budget of an escape walk. A player moves one tile per
/// game tick at walk speed and two at run speed (`PathingEntity.ts:144-149`,
/// with `Player.ts:729-731` choosing walk or run). So a walk to a goal at
/// flood depth `d` needs at most `d` observed ticks. A measured budget is the
/// flood depth to the nearest goal of the selected tier plus
/// [`ESCAPE_SLACK_TICKS`]. The Reach flood has no depth limit of its own
/// (`SceneQuery::flood_reach`, `api/src/query/reach.rs:612-614`: "No 400-step
/// cap"), and the `script` crate reaches nav only through
/// `nav_planner::contract`, so it has no router search bound to use.
/// Without a measured depth (no live flood, or no goal in it), the walk has
/// no tick budget (`None`). It has no [`WalkClock`], so it ends only by its
/// receipt or by the existing ten-minute walk deadline (Main ruling
/// `FIX-ABORT-ESCAPE/r4/MAIN-RULING.md`, 2026-10-27).
fn walk_budget_ticks(depth: Option<u32>) -> Option<u64> {
    depth.map(|depth| u64::from(depth) + ESCAPE_SLACK_TICKS)
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
/// unsettled (`None`: no tick budget, because the depth is not measured).
pub(super) struct EscapeGoals {
    pub(super) target: WorldTile,
    pub(super) goals: WalkGoals,
    pub(super) budget_ticks: Option<u64>,
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
        let (stand_tier, other_tiers) = tiers.split_at_mut(BEYOND);
        let stand_goals = &stand_tier[STAND];
        let outside_stand = |tile| usable(tile) && !stand_goals.contains(&tile);

        // Beyond the envelope, and farther than the player already is.
        let first = envelope.max(here_gap).saturating_add(1);
        let beyond = &mut other_tiers[0];
        for radius in first..=MAX_RING {
            if !push_ring(beyond, threat, level, radius, &outside_stand) {
                break;
            }
        }
        // Inside the envelope, outermost ring first. The stand's neighbours
        // are reserved for its tier so a later retry never repeats them.
        let away = &mut other_tiers[AWAY - BEYOND];
        for radius in (here_gap.saturating_add(1)..=envelope).rev() {
            if !push_ring(away, threat, level, radius, &outside_stand) {
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

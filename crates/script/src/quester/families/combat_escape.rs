//! Escape-destination planning for an aborted Quester fight.
//!
//! The adapter used to pick one tile at the chase-range distance and check
//! only world-coordinate bounds. A tile inside a tree footprint has no legal
//! Reach arrival goal, so the host refused the route and the bot stood still.
//! This module chooses among destinations that the baked collision proves
//! reachable from the observed tile, and it never repeats a destination the
//! caller already tried.
use super::tile_distance;
use crate::native::ActionError;
use api::WorldTile;
use nav_planner::contract::{ArrivalSpec, CollisionView};

/// The eight movement directions of the engine
/// (`engine/src/engine/routefinder/flags.ts:54-63`: `DirectionFlag`).
const ESCAPE_COMPASS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
];

/// Destinations one escape pass may request. A pass tries each of the eight
/// engine directions at most once, so the retry budget has the same bound as
/// the engine's own movement choices. Each failed route is observed as a walk
/// receipt, in the same game tick or the next one. A retry therefore needs no
/// wait and spends no more game ticks than the receipts take to arrive.
pub(super) const ESCAPE_PASS_ATTEMPTS: usize = ESCAPE_COMPASS.len();

/// Chebyshev arrival margin of every escape walk. The legality check below
/// uses the same value as the walk request.
pub(super) const ESCAPE_ARRIVAL_RADIUS: u16 = 1;

/// Largest retreat distance, and so the radius of the reachability window.
const WINDOW_RADIUS: i32 = 32;
const WINDOW_SPAN: usize = (WINDOW_RADIUS as usize) * 2 + 1;

const STEPS: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// Escape state of one abort: the destinations of the current pass and the
/// last walk failure. A new pass keeps the failure, so an exhausted recovery
/// can report it.
pub(super) struct AbortEscape {
    tried: [WorldTile; ESCAPE_PASS_ATTEMPTS],
    len: usize,
    last_failure: Option<ActionError>,
}

impl AbortEscape {
    pub(super) fn new() -> Self {
        Self {
            tried: [WorldTile {
                x: 0,
                z: 0,
                level: 0,
            }; ESCAPE_PASS_ATTEMPTS],
            len: 0,
            last_failure: None,
        }
    }

    /// Start a fresh pass from a newly observed position.
    pub(super) fn begin_pass(&mut self) {
        self.len = 0;
    }

    pub(super) fn tried(&self) -> &[WorldTile] {
        &self.tried[..self.len]
    }

    pub(super) fn spent(&self) -> bool {
        self.len >= ESCAPE_PASS_ATTEMPTS
    }

    pub(super) fn record_attempt(&mut self, destination: WorldTile) {
        if self.len < ESCAPE_PASS_ATTEMPTS {
            self.tried[self.len] = destination;
            self.len += 1;
        }
    }

    pub(super) fn record_failure(&mut self, error: ActionError) {
        self.last_failure = Some(error);
    }

    pub(super) fn last_failure(&self) -> Option<&ActionError> {
        self.last_failure.as_ref()
    }
}

/// What the planner knows about the fight.
pub(super) struct EscapeGeometry {
    /// The player's packet-time tile.
    pub(super) here: WorldTile,
    /// The attacker's packet-time tile.
    pub(super) threat: WorldTile,
    /// The step's authored stand, when it is out of the attacker's reach.
    pub(super) stand: Option<WorldTile>,
    /// Chase envelope of the attacker, in tiles.
    pub(super) chase_range: i32,
}

/// Tiles reachable from the player over the baked collision, inside a
/// Chebyshev window. The flood uses the bake's own step rule, so it holds
/// the reachability the router starts from. It adds no second rule.
pub(super) struct ReachWindow {
    here: WorldTile,
    radius: i32,
    cells: [[bool; WINDOW_SPAN]; WINDOW_SPAN],
}

impl ReachWindow {
    fn flood(collision: CollisionView<'_>, here: WorldTile, radius: i32) -> Self {
        let mut window = Self {
            here,
            radius: radius.clamp(0, WINDOW_RADIUS),
            cells: [[false; WINDOW_SPAN]; WINDOW_SPAN],
        };
        let mut queue = [(0i8, 0i8); WINDOW_SPAN * WINDOW_SPAN];
        let (mut head, mut tail) = (0usize, 1usize);
        window.mark(0, 0);
        while head < tail {
            let (ox, oz) = queue[head];
            let (ox, oz) = (i32::from(ox), i32::from(oz));
            head += 1;
            let current = window.tile(ox, oz);
            for (step_x, step_z) in STEPS {
                let (nx, nz) = (ox + step_x, oz + step_z);
                if nx.abs() > window.radius || nz.abs() > window.radius {
                    continue;
                }
                if window.cell(nx, nz) || !collision.step_ok(current, window.tile(nx, nz)) {
                    continue;
                }
                window.mark(nx, nz);
                // The offsets are at most `WINDOW_RADIUS` (32), so they fit.
                queue[tail] = (nx as i8, nz as i8);
                tail += 1;
            }
        }
        window
    }

    fn tile(&self, ox: i32, oz: i32) -> WorldTile {
        WorldTile {
            x: self.here.x.saturating_add(ox),
            z: self.here.z.saturating_add(oz),
            level: self.here.level,
        }
    }

    fn slot(ox: i32, oz: i32) -> Option<(usize, usize)> {
        Some((
            usize::try_from(ox.checked_add(WINDOW_RADIUS)?).ok()?,
            usize::try_from(oz.checked_add(WINDOW_RADIUS)?).ok()?,
        ))
    }

    fn mark(&mut self, ox: i32, oz: i32) {
        if let Some((ix, iz)) = Self::slot(ox, oz) {
            if let Some(cell) = self.cells.get_mut(ix).and_then(|row| row.get_mut(iz)) {
                *cell = true;
            }
        }
    }

    fn cell(&self, ox: i32, oz: i32) -> bool {
        Self::slot(ox, oz)
            .and_then(|(ix, iz)| self.cells.get(ix).and_then(|row| row.get(iz)))
            .copied()
            .unwrap_or(false)
    }

    fn contains(&self, tile: WorldTile) -> bool {
        tile.level == self.here.level
            && (tile.x - self.here.x).abs() <= self.radius
            && (tile.z - self.here.z).abs() <= self.radius
            && self.cell(tile.x - self.here.x, tile.z - self.here.z)
    }
}

/// Whether `destination` has a legal Reach arrival goal at the walk's radius:
/// a standable tile that production `is_arrived` accepts for it, and that the
/// player can reach when a window is known. This is the filter the host applies
/// to packed route goals, so a destination that fails it cannot route.
pub(super) fn has_legal_reach_goal(
    collision: Option<CollisionView<'_>>,
    window: Option<&ReachWindow>,
    destination: WorldTile,
) -> bool {
    let Some(collision) = collision else {
        // Without a nav pack the host cannot route either; keep the in-bounds
        // geometric tile, as the Gatherer's hazard retreat does.
        return (0..=16_383).contains(&destination.x) && (0..=16_383).contains(&destination.z);
    };
    let radius = i32::from(ESCAPE_ARRIVAL_RADIUS);
    let spec = ArrivalSpec::reach(ESCAPE_ARRIVAL_RADIUS);
    (-radius..=radius).any(|dx| {
        (-radius..=radius).any(|dz| {
            let goal = WorldTile {
                x: destination.x.saturating_add(dx),
                z: destination.z.saturating_add(dz),
                level: destination.level,
            };
            collision.standable(goal)
                && window.is_none_or(|window| window.contains(goal))
                && collision.arrived(goal, destination, spec) == Ok(true)
        })
    })
}

/// Directions to try, those that lead away from the attacker first. The score
/// of a direction is its dot product with the direction from the attacker to
/// the player, so a ray that runs past the attacker comes last. Equal scores
/// keep the fixed compass order.
fn direction_order(here: WorldTile, threat: WorldTile) -> [(i32, i32); 8] {
    let away_x = (here.x - threat.x).signum();
    let away_z = (here.z - threat.z).signum();
    let mut order = ESCAPE_COMPASS;
    order.sort_by_key(|&(dx, dz)| std::cmp::Reverse(dx * away_x + dz * away_z));
    order
}

/// The next destination to try: reachable from `here`, with a legal Reach
/// arrival goal, and not in `tried`. `None` when no such destination is left.
pub(super) fn next_destination(
    geometry: &EscapeGeometry,
    collision: Option<CollisionView<'_>>,
    tried: &[WorldTile],
) -> Option<WorldTile> {
    let envelope = geometry.chase_range.saturating_add(1);
    if let Some(stand) = geometry.stand {
        if tile_distance(stand, geometry.threat) > envelope
            && !tried.contains(&stand)
            && has_legal_reach_goal(collision, None, stand)
        {
            return Some(stand);
        }
    }

    let retreat = geometry
        .chase_range
        .saturating_add(3)
        .clamp(2, WINDOW_RADIUS);
    let window = collision.map(|collision| ReachWindow::flood(collision, geometry.here, retreat));
    let here_distance = tile_distance(geometry.here, geometry.threat);
    let order = direction_order(geometry.here, geometry.threat);
    // A direction whose ray already carries a tried destination likely fails
    // for the same reason again, so untried directions come first.
    let on_ray = |tile: WorldTile, (dx, dz): (i32, i32)| {
        let (ox, oz) = (tile.x - geometry.here.x, tile.z - geometry.here.z);
        tile.level == geometry.here.level && ox * dz == oz * dx && ox * dx + oz * dz > 0
    };
    // First choose a tile outside the attacker's chase envelope. When none is
    // reachable, any reachable tile that is farther from the attacker beats
    // standing in its reach.
    [(false, true), (true, true), (false, false), (true, false)]
        .into_iter()
        .find_map(|(relaxed, untried_only)| {
            order.iter().find_map(|&direction| {
                if untried_only && tried.iter().any(|&tile| on_ray(tile, direction)) {
                    return None;
                }
                (2..=retreat).rev().find_map(|distance| {
                    let destination = WorldTile {
                        x: geometry
                            .here
                            .x
                            .saturating_add(direction.0.saturating_mul(distance)),
                        z: geometry
                            .here
                            .z
                            .saturating_add(direction.1.saturating_mul(distance)),
                        level: geometry.here.level,
                    };
                    let away = tile_distance(destination, geometry.threat);
                    let outside = if relaxed {
                        away > here_distance
                    } else {
                        away > envelope
                    };
                    (outside
                        && !tried.contains(&destination)
                        && collision.is_none_or(|collision| collision.standable(destination))
                        && has_legal_reach_goal(collision, window.as_ref(), destination))
                    .then_some(destination)
                })
            })
        })
}

//! Maze behavioural port of rs2b0t `randomevents/maze` (mazeLayout,
//! mazeGraph, selectRoute, solveMaze). The layout quintuples are copied
//! static game data (their `tools/maze-derive.ts` is not executed); the
//! graph / route logic is a Rust port of their TS, solved from the
//! observed tile — never a corner guess, so a wrong-spawn route cannot
//! land. The act state machine ([`MazeSolve`]) is their `solveMaze`
//! control flow driven one tick at a time by the guardian.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
mod layout;
use layout::MAZE_LAYOUT;

/// Maze region: `x>>6 == 45 && z>>6 == 71` at level 0.
pub const MAZE_SQUARE: (i32, i32) = (45, 71);
/// SW origin of the maze region (`45*64`, `71*64`).
const MAZE_ORIGIN: (i32, i32) = (2880, 4544);
/// SW corner of the 3x3 Strange shrine (loc 3634 `macro_maze_complete`).
pub const MAZE_SHRINE: (i32, i32) = (2911, 4575);
/// Content pack: length=3 width=3 on `macro_maze_complete`.
const MAZE_SHRINE_SIZE: i32 = 3;
/// West door into the shrine chamber (local 30,32): routes must include
/// it, the south face of the shrine SW is a solid wall.
pub const MAZE_SHRINE_DOOR: (i32, i32) = (2910, 4576);
/// The spawn corners (rs2b0t `MAZE_SPAWNS`); consumed by the spawn
/// route tests (each must solve to the chamber door).
#[allow(dead_code)]
pub const MAZE_SPAWNS: [(i32, i32); 4] = [
    (2891, 4597), // NW local (11,53)
    (2933, 4597), // NE local (53,53)
    (2933, 4555), // SE local (53,11)
    (2891, 4555), // SW local (11,11)
];
/// The maze wall loc id (3626).
const WALL_ID: i32 = 3626;
/// The maze door loc ids (`macro_maze_walllow*`, op Open).
pub const MAZE_DOOR_IDS: [i32; 5] = [3628, 3629, 3630, 3631, 3632];
/// The shrine loc id (`macro_maze_complete`, op Touch).
pub const MAZE_SHRINE_LOC: i32 = 3634;
/// Step-backs allowed before a pass gives up (rs2b0t `MAX_RESYNCS`).
pub const MAX_RESYNCS: u32 = 3;
/// Consecutive `try_move` sends without a tile change before a door is
/// declared walled off (the rs2b0t 12-attempt walk cap).
pub const WALK_LIMIT: u32 = 12;
/// Chat continues per mesbox/briefing opening (rs2b0t `clearMesbox`, 6).
pub const MESBOX_LIMIT: u32 = 6;
/// Ticks spent waiting for an open to resolve before the route advances
/// (rs2b0t waits ~3 s, then continues).
pub const OPEN_WAIT: u32 = 12;
/// Ticks spent waiting for a Touch to move the player off the square
/// before the next touch pass (rs2b0t waits 12 s).
pub const TOUCH_WAIT: u32 = 20;
/// Touch passes before the pass gives up (rs2b0t 6).
pub const TOUCH_LIMIT: u32 = 6;
/// Touch stands after the chamber door (rs2b0t `touchStands`).
pub const TOUCH_STANDS: [(i32, i32); 3] = [
    MAZE_SHRINE_DOOR,
    (MAZE_SHRINE.0, MAZE_SHRINE.1 + 1),
    (MAZE_SHRINE.0 - 1, MAZE_SHRINE.1 + 2),
];

/// Door id → approach side (rs2b0t `DOOR_DIRS`): 0 always opens, 1 only
/// from the same axis coordinate, 2 only from the opposite.
fn door_dir(id: i32) -> Option<i32> {
    match id {
        3628 => Some(0),
        3629 => Some(1),
        3630 | 3631 => Some(2),
        3632 => Some(1),
        _ => None,
    }
}

/// `WALL_L_ANGLES` (rs2b0t): the two straight edges of a 2-1 wall by angle.
const WALL_L_ANGLES: [[i32; 2]; 4] = [[1, 0], [1, 2], [3, 2], [3, 0]];

/// The four cardinal neighbours.
const CARDINAL: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// Canonical (unordered) edge key for the tile pair, like rs2b0t
/// `edgeKey`.
pub fn edge_key(ax: i32, az: i32, bx: i32, bz: i32) -> (i32, i32, i32, i32) {
    if ax < bx || az < bz {
        (ax, az, bx, bz)
    } else {
        (bx, bz, ax, az)
    }
}

/// The straight wall edge a wall loc of `angle` blocks (rs2b0t
/// `straightEdge`).
fn straight_edge(wx: i32, wz: i32, angle: i32) -> (i32, i32, i32, i32) {
    match angle {
        0 => (wx, wz, wx - 1, wz), // WEST
        1 => (wx, wz, wx, wz + 1), // NORTH
        2 => (wx, wz, wx + 1, wz), // EAST
        _ => (wx, wz, wx, wz - 1), // SOUTH
    }
}

/// One maze door: the loc tile, id and angle (rs2b0t `DoorInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorInfo {
    pub tile: (i32, i32),
    pub id: i32,
    pub angle: i32,
}

/// The maze collision graph (rs2b0t `MazeGraph`): wall edges, door
/// edges and the door loc id per door tile, built once from the layout.
#[derive(Debug, Clone, Default)]
pub struct MazeGraph {
    pub wall_edge: HashSet<(i32, i32, i32, i32)>,
    pub door: HashMap<(i32, i32, i32, i32), DoorInfo>,
    /// Door loc id by door tile (the `oploc` target at a route tile).
    pub door_id: HashMap<(i32, i32), i32>,
    pub minx: Option<i32>,
    pub maxx: Option<i32>,
    pub minz: Option<i32>,
    pub maxz: Option<i32>,
}

/// `buildMaze`: walls block their straight/L edges; doors record their
/// edge and tile. The layout rows are local `(lx, lz)` within the region.
pub fn build_maze(locs: &[(i32, i32, i32, i32, i32)]) -> MazeGraph {
    let mut g = MazeGraph::default();
    for &(lx, lz, id, shape, angle) in locs {
        let wx = MAZE_ORIGIN.0 + lx;
        let wz = MAZE_ORIGIN.1 + lz;
        if id == WALL_ID {
            g.minx = Some(g.minx.map_or(wx, |m| m.min(wx)));
            g.maxx = Some(g.maxx.map_or(wx, |m| m.max(wx)));
            g.minz = Some(g.minz.map_or(wz, |m| m.min(wz)));
            g.maxz = Some(g.maxz.map_or(wz, |m| m.max(wz)));
            if shape == 0 {
                g.wall_edge.insert(edge_key_t(straight_edge(wx, wz, angle)));
            } else if shape == 2 {
                for a in WALL_L_ANGLES[angle as usize] {
                    g.wall_edge.insert(edge_key_t(straight_edge(wx, wz, a)));
                }
            }
        } else if door_dir(id).is_some() {
            g.door_id.insert((wx, wz), id);
            g.door.insert(
                edge_key_t(straight_edge(wx, wz, angle)),
                DoorInfo {
                    tile: (wx, wz),
                    id,
                    angle,
                },
            );
        }
    }
    g
}

/// `edgeKey` applied to a straight edge tuple.
fn edge_key_t((ax, az, bx, bz): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    edge_key(ax, az, bx, bz)
}

/// `doorPassable`: whether a door opens when approached from `from`
/// (the door's `approach_direction`).
pub fn door_passable(door: &DoorInfo, from: (i32, i32)) -> bool {
    let dir = door_dir(door.id).expect("door map only holds door ids");
    if dir == 0 {
        return true;
    }
    let axis_true = if door.angle == 1 || door.angle == 3 {
        from.1 == door.tile.1
    } else {
        from.0 == door.tile.0
    };
    if dir == 1 {
        axis_true
    } else {
        !axis_true
    }
}

/// `isShrineTouchStand`: true when (x,z) is outside the shrine footprint
/// and shares an open (non-wall) edge with it. The door edge into the
/// 3x3 is counted as passable once the route opens it.
fn is_shrine_touch_stand(g: &MazeGraph, x: i32, z: i32) -> bool {
    let x1 = MAZE_SHRINE.0 + MAZE_SHRINE_SIZE - 1;
    let z1 = MAZE_SHRINE.1 + MAZE_SHRINE_SIZE - 1;
    if x >= MAZE_SHRINE.0 && x <= x1 && z >= MAZE_SHRINE.1 && z <= z1 {
        return false;
    }
    for (dx, dz) in CARDINAL {
        let nx = x + dx;
        let nz = z + dz;
        if nx < MAZE_SHRINE.0 || nx > x1 || nz < MAZE_SHRINE.1 || nz > z1 {
            continue;
        }
        if g.wall_edge.contains(&edge_key(x, z, nx, nz)) {
            continue;
        }
        return true;
    }
    false
}

/// A maze BFS back-edge: the previous tile plus the door crossed to reach
/// the current one (rs2b0t `solveRoute` prev map).
type MazePrev = HashMap<(i32, i32), Option<((i32, i32), Option<DoorInfo>)>>;

/// `solveRoute`: BFS from the spawn to the first shrine touch stand,
/// crossing passable door edges, never the shrine footprint. Returns the
/// door tiles in route order (the last is the chamber door).
pub fn solve_route(g: &MazeGraph, spawn: (i32, i32)) -> Vec<(i32, i32)> {
    let lo = (
        g.minx.unwrap_or(spawn.0.min(MAZE_SHRINE.0)) - 2,
        g.minz.unwrap_or(spawn.1.min(MAZE_SHRINE.1)) - 2,
    );
    let hi = (
        g.maxx.unwrap_or(spawn.0.max(MAZE_SHRINE.0)) + 2,
        g.maxz.unwrap_or(spawn.1.max(MAZE_SHRINE.1)) + 2,
    );
    let mut prev: MazePrev = HashMap::new();
    prev.insert(spawn, None);
    let mut queue: VecDeque<(i32, i32)> = VecDeque::from([spawn]);
    while let Some(cur) = queue.pop_front() {
        if is_shrine_touch_stand(g, cur.0, cur.1) {
            let mut doors: Vec<(i32, i32)> = Vec::new();
            let mut node = prev.get(&cur).copied().flatten();
            while let Some((prev_tile, door)) = node {
                if let Some(d) = door {
                    doors.push(d.tile);
                }
                node = prev.get(&prev_tile).copied().flatten();
            }
            doors.reverse();
            // Standing on the west door tile is a valid touch stand, but
            // the door edge into the 3x3 is never *crossed* by the BFS,
            // so the chamber door is appended here.
            for (dx, dz) in CARDINAL {
                let nx = cur.0 + dx;
                let nz = cur.1 + dz;
                if nx < MAZE_SHRINE.0
                    || nx >= MAZE_SHRINE.0 + MAZE_SHRINE_SIZE
                    || nz < MAZE_SHRINE.1
                    || nz >= MAZE_SHRINE.1 + MAZE_SHRINE_SIZE
                {
                    continue;
                }
                if let Some(edge_door) = g.door.get(&edge_key(cur.0, cur.1, nx, nz)) {
                    if !doors.contains(&edge_door.tile) {
                        doors.push(edge_door.tile);
                    }
                }
            }
            return doors;
        }
        for (dx, dz) in CARDINAL {
            let nx = cur.0 + dx;
            let nz = cur.1 + dz;
            if nx < lo.0 || nx > hi.0 || nz < lo.1 || nz > hi.1 {
                continue;
            }
            // The shrine footprint is solid: never path through it.
            if nx >= MAZE_SHRINE.0
                && nx < MAZE_SHRINE.0 + MAZE_SHRINE_SIZE
                && nz >= MAZE_SHRINE.1
                && nz < MAZE_SHRINE.1 + MAZE_SHRINE_SIZE
            {
                continue;
            }
            if prev.contains_key(&(nx, nz)) {
                continue;
            }
            let ek = edge_key(cur.0, cur.1, nx, nz);
            let door = g.door.get(&ek);
            if let Some(d) = door {
                if !door_passable(d, cur) {
                    continue;
                }
            } else if g.wall_edge.contains(&ek) {
                continue;
            }
            prev.insert((nx, nz), Some((cur, door.copied())));
            queue.push_back((nx, nz));
        }
    }
    Vec::new()
}

/// `selectRoute`: solve from the observed tile; `None` when the layout
/// cannot reach the shrine from here (never replay another spawn's route).
pub fn select_route(g: &MazeGraph, me: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    let doors = solve_route(g, me);
    if doors.is_empty() {
        None
    } else {
        Some(doors)
    }
}

/// The static graph, built once from the copied layout (rs2b0t caches
/// `graph()` the same way).
pub fn graph() -> &'static MazeGraph {
    static GRAPH: OnceLock<MazeGraph> = OnceLock::new();
    GRAPH.get_or_init(|| build_maze(MAZE_LAYOUT))
}

/// One phase of the maze act machine (rs2b0t `solveMaze` control flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MazePhase {
    /// Walking to `doors[next]`.
    WalkDoor,
    /// Open sent for `doors[next]` from `from`; waiting for a >= 2-tile
    /// move (through) or a wrong-door mesbox (refused).
    OpenDoor { from: (i32, i32) },
    /// Walking back to `doors[next - 1]` after a walled-off door.
    Resync,
    /// Re-open sent for `doors[next - 1]` from `from`.
    OpenResync { from: (i32, i32) },
    /// Walking to the chamber door.
    ShrineDoor,
    /// Open sent for the chamber door from `from`.
    OpenShrine { from: (i32, i32) },
    /// Touch pass `pass`: walk to a stand, then Touch the shrine.
    Touch { pass: u32 },
    /// Touch sent; waiting to leave the maze square.
    TouchWait,
}

/// One active maze solve (rs2b0t `solveMaze` state), tick-driven by the
/// guardian.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MazeSolve {
    /// Door tiles in route order.
    pub doors: Vec<(i32, i32)>,
    /// The next route door index.
    pub next: usize,
    /// Step-backs used against walled-off doors.
    pub resyncs: u32,
    /// The current phase.
    pub phase: MazePhase,
    /// The player tile when the current walk started; a walled-off door
    /// is no tile change for [`WALK_LIMIT`] sends.
    pub walk_from: Option<(i32, i32)>,
    pub walk_sends: u32,
    pub wait_ticks: u32,
    pub continues: u32,
    /// A wrong-door mesbox was drained while an open was in flight.
    pub refused: bool,
    /// The touch pass in flight (re-opened-chamber-door target).
    pub touch_pass: u32,
}

impl MazeSolve {
    pub fn new(doors: Vec<(i32, i32)>) -> Self {
        Self {
            doors,
            next: 0,
            resyncs: 0,
            phase: MazePhase::WalkDoor,
            walk_from: None,
            walk_sends: 0,
            wait_ticks: 0,
            continues: 0,
            refused: false,
            touch_pass: 0,
        }
    }

    /// The door tile the current phase targets.
    pub fn target(&self) -> Option<(i32, i32)> {
        match self.phase {
            MazePhase::WalkDoor | MazePhase::OpenDoor { .. } => self.doors.get(self.next).copied(),
            MazePhase::Resync | MazePhase::OpenResync { .. } => self
                .next
                .checked_sub(1)
                .and_then(|i| self.doors.get(i))
                .copied(),
            MazePhase::ShrineDoor | MazePhase::OpenShrine { .. } => Some(MAZE_SHRINE_DOOR),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "maze_tests.rs"]
mod tests;

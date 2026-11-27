//! Underground Pass floor grid (`x:upass:cross_grid`).
//!
//! Content stores one safe route per player in `ibanmulti` bits 22 to 31, three
//! decimal digits: the safe lane (1 to 5) in each of three columns
//! (`upass_grid.rs2:47-58`). Each next digit is the previous one or a
//! neighbour of it (`:53-56`). While the player stands on the grid, a one-tick
//! timer sends them to the spike pit when no column's safe block holds their
//! tile (`:61-83`, `quest_upass.rs2:71-78`). The fall teleports to the pit,
//! then `damage_self(15)` (`:85-99`). A climb on the pit's hand holds returns
//! the player to the entrance tile (`:101-106`).
//!
//! `ibanmulti` has no `transmit` flag (`quest_upass.varp:4-5`; the engine sends
//! only `transmit=yes` varps, `Player.ts:514-521` and `VarPlayerType.ts:98-99`),
//! so the client cannot read the route. The step learns it by trying lanes:
//!
//! 1. Per column it tries the lanes content allows (all five for column 1, the
//!    previous lane and its neighbours for the next two). It stands on a tile
//!    that only this column's block covers and waits for the trap's own signal
//!    (the line "It's a trap!" or the pit teleport), counted in observed ticks.
//! 2. A trap rules that lane out for this player. The step climbs out,
//!    walks back along the lanes it already knows and tries the next lane.
//! 3. A quiet wait confirms the lane. The next column starts from there.
//!
//! Every walk between probes goes through tiles that a confirmed lane
//! proves safe (a breadth-first search over a 13 by 10 tile window), and only
//! the last one or two tiles of a probe are unproven. What it learned is kept
//! twice. The run's retained cell (`QuesterRetained::upass_grid`, 8 bytes)
//! holds the route and the trap count, so a step the runner builds again in
//! the same run (survival drops it) does not trap twice on a lane, and the
//! bound of 8 traps holds for the run. The account's saved route
//! (`upass_store.rs`: one SQLite `knowledge` fact of the local-state service) is the
//! cross-run copy: a new run, a relog or a new process starts from it with no
//! traps taken, and a later crossing (Regicide) starts from the learned route.
//!
//! Each probe needs more than 15 Hitpoints, the fall's damage; the step
//! stops with that reason otherwise, and the runner's survival hold feeds or
//! rests the bot before the step restarts (`hazard` on the guide step).
use crate::interact::InteractReq;
use crate::native::{
    ActionContext, ActionError, ActionHandle, InteractionDispatch, NativeActions, NativeMachine,
    WalkRetryState,
};
use crate::quester::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use crate::quester::families::reach;
use crate::quester::prereq::walk::PrereqWalk;
use api::WorldTile;
use super::upass_store::{self, RouteStore};
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;

/// Lanes per column (`upass_grid.rs2:51` `~random_range(1, 5)`).
const LANES: u8 = 5;
/// Columns of the route: three digits (`:51-57`, the loop at `:53` runs twice).
const COLUMNS: usize = 3;
/// Most lanes one column can hold after the first: the previous lane and its two
/// neighbours (`:54` `random_range(max(d-1,1), min(d+1,5))`).
const NEIGHBOUR_LANES: u8 = 3;
/// Traps one fixed route can cost: all lanes but the last in column 1, all but
/// the last of three in columns 2 and 3. The last candidate cannot trap.
pub(super) const MAX_TRAPS: u8 = (LANES - 1) + 2 * (NEIGHBOUR_LANES - 1);
/// The fall's damage, always (`upass_grid.rs2:99` `~damage_self(15)`).
const TRAP_DAMAGE: i32 = 15;
/// Hitpoints stat index (`api` `StatView::index` 3).
const HITPOINTS: i32 = 3;

/// x of `^upass_grid_col1`, `col3`, `col5` (`quest_upass.constant:36-38`:
/// `0_38_151_43_9`, `0_38_151_39_9`, `0_38_151_35_9`; 38 * 64 + local x).
const COL_X: [i32; COLUMNS] = [2475, 2471, 2467];
/// Row of lane 1 (`0_38_151_*_9`: 151 * 64 + 9). Lane `k` spans two rows,
/// `GRID_Z + 2(k-1)` and the next (`upass_grid.rs2:67-81`, offsets `2k-2`, `2k-1`).
const GRID_Z: i32 = 9673;
const LANE_HEIGHT: i32 = 2;
/// Inclusive x range of each column's safe block (`upass_grid.rs2:67-81`): the
/// first two columns span `col-2 ..= col+3`, the third `col ..= col+3`.
const BLOCK_X: [(i32, i32); COLUMNS] = [
    (COL_X[0] - 2, COL_X[0] + 3),
    (COL_X[1] - 2, COL_X[1] + 3),
    (COL_X[2], COL_X[2] + 3),
];
/// The timer's area (`:66` `inzone(col5, col1 + (1, 0, 9))`): x range; its z
/// range is the five lanes.
const AREA_X: (i32, i32) = (COL_X[2], COL_X[0] + 1);
/// x of the tile only column `k`'s block covers: the tile a probe stands on.
const PROBE_X: [i32; COLUMNS] = [AREA_X.1, COL_X[1] + 1, COL_X[2] + 1];
/// x of the tile that columns `k-1` and `k` both cover (column 1: east of the
/// area, where nothing traps). A probe starts its unproven part from here.
const LAUNCH_X: [i32; COLUMNS] = [AREA_X.1 + 1, BLOCK_X[1].1, BLOCK_X[2].1];
/// Walk window: one tile past the area on the west (the far side) and two on
/// the east (the entrance and its teleport tile).
const WIN_X: (i32, i32) = (AREA_X.0 - 1, AREA_X.1 + 2);
const WIN_Z: (i32, i32) = (GRID_Z, GRID_Z + LANES as i32 * LANE_HEIGHT - 1);
const WIDTH: usize = (WIN_X.1 - WIN_X.0 + 1) as usize;
const HEIGHT: usize = (WIN_Z.1 - WIN_Z.0 + 1) as usize;
const CELLS: usize = WIDTH * HEIGHT;
/// Lane order for column 1, nearest the entrance row first. The climb returns
/// the player to `0_38_151_45_13` (z 9677, lane 3; `upass_grid.rs2:106`).
const FIRST_COLUMN_ORDER: [u8; LANES as usize] = [3, 2, 4, 1, 5];
/// Where the climb leaves the player (`upass_grid.rs2:106`, `0_38_151_45_13`).
const ENTRY: WorldTile = WorldTile {
    x: 2477,
    z: 9677,
    level: 0,
};
/// The pit: `0_37_149_27_23` plus `random(2)` on x and z (`:94`).
const PIT: WorldTile = WorldTile {
    x: 2395,
    z: 9559,
    level: 0,
};
/// The line the trap sends first (`upass_grid.rs2:88`).
const TRAP_MESSAGE: &str = "It's a trap!";
/// Observed ticks a probe stands before its lane counts as safe. The timer runs
/// one tick after the player stands there (`quest_upass.rs2:71` `settimer(..., 1)`,
/// timers run before movement, `World.ts:701-717`), then `p_stopaction;
/// p_delay(0)` (`:86-87`) puts "It's a trap!" one tick later (`:88`), and one
/// more tick covers a late frame.
const OBSERVE_TICKS: u64 = 4;
/// Observed ticks from the trap line to the pit teleport: `anim; sound;
/// p_delay(0)` (`:89-91`), then `mes; p_teleport` (`:92-94`) is one tick, and
/// one more tick covers a late frame; doubled.
const FALL_TICKS: u64 = 4;
/// Ticks to wait after the pit shows before the climb click: the fall script
/// keeps the player delayed until `anim(null); say; damage_self` (`:95-99`),
/// one `p_delay(0)` after the teleport (`:96`), and one more covers a late frame.
const POST_FALL_TICKS: u64 = 2;
/// Observed ticks from the climb click to the teleport: the op runs the next
/// tick, then two `p_delay(0)` (`:102`, `:104`) pass before `p_teleport`
/// (`:106`), and one for a late frame; doubled.
const CLIMB_TICKS: u64 = 8;
/// Climb clicks before the step fails. The climb has no check in content; a
/// click lost to the fall script's delay is the only reason to repeat it.
const CLIMB_ATTEMPTS: u8 = 3;
/// Observed ticks to wait for the stats row before the step stops.
const STATS_TICKS: u8 = 10;
/// The far side counts as reached within this many tiles of the authored tile,
/// once outside the timer's area (x below `AREA_X.0`): a tile inside the area
/// can still trap.
const ARRIVAL_RADIUS: i32 = 1;

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    /// The far side of the grid, west of its area. The step ends there.
    arrival: ArrivalArg,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct ArrivalArg {
    /// x, z, level; west of the grid area on level 0.
    tile: [i32; 3],
    /// Source citation for this authored tile.
    source: String,
}

pub(super) const STEPS: &[crate::quester::compile::StepHandler] =
    &[crate::quester::compile::step!(
        "x:upass:cross_grid",
        1,
        Explicit,
        Args,
        compile
    )];

pub(super) const FACTS: &[crate::quester::compile::PredicateHandler] = &[];

fn compile(args: Args, cx: &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.arrival.source.trim().is_empty() {
        return Err(CompileError::code("tile-source-required"));
    }
    let [x, z, level] = args.arrival.tile;
    if !(0..=16383).contains(&x) || !(0..=16383).contains(&z) || !(0..=3).contains(&level) {
        return Err(CompileError::code("invalid-tile"));
    }
    if level != 0 || x >= AREA_X.0 || !(WIN_Z.0..=WIN_Z.1).contains(&z) {
        return Err(CompileError::code("invalid-tile").with_detail(
            "x:upass:cross_grid: arrival must lie west of the grid on level 0, in its lanes",
        ));
    }
    let hand_holds = cx
        .selected
        .loc_by_config("upass_grilltrap_hand_holds")
        .ok_or_else(|| CompileError::code("unresolved-loc"))?;
    if !hand_holds
        .ops
        .iter()
        .any(|op| op.eq_ignore_ascii_case("Climb"))
    {
        return Err(CompileError::code("unavailable-op"));
    }
    Ok(Arc::new(Plan {
        arrival: WorldTile { x, z, level },
        hand_holds_id: hand_holds.id,
    }))
}

// ---------------------------------------------------------------------------
// The route model: what the step knows, and which tiles that proves safe.

/// What the step has learned about this player's route. Three digits (0 =
/// unknown), the lanes ruled out per column and the traps taken. Seven bytes.
/// The route and the lanes ruled out are also saved per account
/// (`upass_store.rs`, the cross-run copy); the trap count belongs to one run,
/// so that its bound holds per run and not per account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GridMemory {
    lane: [u8; COLUMNS],
    /// Bit `lane - 1` set: that lane trapped in that column.
    ruled_out: [u8; COLUMNS],
    traps: u8,
}

/// The run's copy of the route (`QuesterRetained::upass_grid`): the memory and
/// the run's saved-route state, boxed once per run so the slot's retained cell
/// grows by one pointer (8 bytes). The runner drops a step for survival and
/// builds it again in the same run, so the step takes this copy first and does
/// not read the account's fact again:
///
/// - the route it learned and the traps it took survive the recreation (the
///   bound of [`MAX_TRAPS`] holds across the run), with no state capability and
///   with a store the service refuses;
/// - the write in flight is followed by the new step, not sent again against
///   an older version;
/// - an unsaved change keeps the version it replaces and its observed stamp,
///   so a store that changed elsewhere refuses it;
/// - a version conflict stops saving for the rest of the run.
///
/// A new run starts from the default value: no traps, and the route comes from
/// the account's saved fact.
#[derive(Debug, Default)]
pub struct GridRun(Option<Box<RunRoute>>);

/// What one run knows of its route and of the route's saved fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RunRoute {
    pub(super) memory: GridMemory,
    pub(super) store: RouteStore,
}

impl GridRun {
    /// The run's route and saved-route state, once a step of the run has read
    /// the route.
    pub(super) fn kept(&self) -> Option<RunRoute> {
        self.0.as_deref().copied()
    }

    /// Keep `route` for the run. The first call of a run allocates the box;
    /// later calls overwrite it in place, and an unchanged route writes nothing.
    pub(super) fn keep(&mut self, route: RunRoute) {
        match self.0.as_deref_mut() {
            Some(kept) if *kept == route => {}
            Some(kept) => *kept = route,
            None => self.0 = Some(Box::new(route)),
        }
    }
}

/// One lane to try in one column (0-based column, lane 1 to 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Probe {
    pub(super) col: u8,
    pub(super) lane: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Next {
    Probe(Probe),
    /// All three columns are confirmed.
    Known,
    /// Every lane content allows here has trapped: the stored route is not the
    /// route being tested (the route was drawn again), so learning starts over.
    Inconsistent,
}

/// Bytes of a saved route: three lanes, then three ruled-out lane masks.
pub(super) const PAYLOAD_LEN: usize = COLUMNS * 2;

impl GridMemory {
    pub(super) fn traps(&self) -> u8 {
        self.traps
    }

    /// Columns whose lane is confirmed.
    pub(super) fn confirmed(&self) -> usize {
        self.confirmed_columns()
    }

    /// Nothing learned: no lane confirmed and no lane ruled out.
    pub(super) fn is_empty(&self) -> bool {
        self.lane == [0; COLUMNS] && self.ruled_out == [0; COLUMNS]
    }

    /// What is saved: the lanes and the lanes ruled out (not the trap count).
    pub(super) fn to_payload(self) -> [u8; PAYLOAD_LEN] {
        let mut out = [0; PAYLOAD_LEN];
        out[..COLUMNS].copy_from_slice(&self.lane);
        out[COLUMNS..].copy_from_slice(&self.ruled_out);
        out
    }

    /// The memory a saved payload holds, or `None` when the step could not
    /// have written it. The step tries the lanes of a column in the order of
    /// [`Self::candidates`], so what a column holds is fixed by that order:
    /// a confirmed column's lane is allowed by the previous lane
    /// (`upass_grid.rs2:53-56`) and every lane before it was ruled out; the
    /// next column holds a leading run of its candidates ruled out; a column
    /// after it holds nothing. A wrong length, a lane outside 1 to 5, a gap in
    /// the confirmed columns or any other mask is refused.
    pub(super) fn from_payload(bytes: &[u8]) -> Option<Self> {
        let bytes: &[u8; PAYLOAD_LEN] = bytes.try_into().ok()?;
        let memory = Self {
            lane: bytes[..COLUMNS].try_into().ok()?,
            ruled_out: bytes[COLUMNS..].try_into().ok()?,
            traps: 0,
        };
        if memory.lane.iter().any(|lane| *lane > LANES) {
            return None;
        }
        let confirmed = memory.confirmed_columns();
        if memory.lane[confirmed..].iter().any(|lane| *lane != 0) {
            return None;
        }
        for col in 0..COLUMNS {
            let mask = memory.ruled_out[col];
            if col > confirmed {
                if mask != 0 {
                    return None;
                }
                continue;
            }
            let (allowed, len) = memory.candidates(col);
            let allowed = &allowed[..len];
            let ruled_out_before = |count: usize| {
                allowed[..count]
                    .iter()
                    .fold(0u8, |bits, lane| bits | 1 << (lane - 1))
            };
            let consistent = if col < confirmed {
                allowed
                    .iter()
                    .position(|lane| *lane == memory.lane[col])
                    .is_some_and(|at| mask == ruled_out_before(at))
            } else {
                (0..=len).any(|count| mask == ruled_out_before(count))
            };
            if !consistent {
                return None;
            }
        }
        Some(memory)
    }

    fn confirmed_columns(&self) -> usize {
        self.lane.iter().take_while(|lane| **lane != 0).count()
    }

    /// The lanes content allows in `col`, in the order they are tried: column 1
    /// takes all five, nearest the entrance first; a later column takes the
    /// previous lane first, then its neighbours (`upass_grid.rs2:51`).
    fn candidates(&self, col: usize) -> ([u8; LANES as usize], usize) {
        let mut out = [0; LANES as usize];
        if col == 0 {
            return (FIRST_COLUMN_ORDER, FIRST_COLUMN_ORDER.len());
        }
        let prev = self.lane[col - 1];
        let mut len = 0;
        for lane in [prev, prev.wrapping_sub(1), prev + 1] {
            if (1..=LANES).contains(&lane) {
                out[len] = lane;
                len += 1;
            }
        }
        (out, len)
    }

    pub(super) fn next(&self) -> Next {
        let col = self.confirmed_columns();
        if col == COLUMNS {
            return Next::Known;
        }
        let (lanes, len) = self.candidates(col);
        lanes[..len]
            .iter()
            .copied()
            .find(|lane| self.ruled_out[col] & (1 << (lane - 1)) == 0)
            .map_or(Next::Inconsistent, |lane| {
                Next::Probe(Probe {
                    col: col as u8,
                    lane,
                })
            })
    }

    pub(super) fn confirm(&mut self, probe: Probe) {
        self.lane[usize::from(probe.col)] = probe.lane;
    }

    pub(super) fn rule_out(&mut self, probe: Probe) {
        self.ruled_out[usize::from(probe.col)] |= 1 << (probe.lane - 1);
    }

    pub(super) fn count_trap(&mut self) {
        self.traps = self.traps.saturating_add(1);
    }

    /// Forget the route, keep the trap count so the bound still holds.
    pub(super) fn clear_route(&mut self) {
        self.lane = [0; COLUMNS];
        self.ruled_out = [0; COLUMNS];
    }

    /// A tile the confirmed lanes prove safe: outside the timer's area, or
    /// inside a confirmed column's block on that column's lane
    /// (`upass_grid.rs2:63-81`: the trap needs every column's block to miss).
    pub(super) fn proves_safe(&self, x: i32, z: i32) -> bool {
        if !(WIN_X.0..=WIN_X.1).contains(&x) || !(WIN_Z.0..=WIN_Z.1).contains(&z) {
            return false;
        }
        if x < AREA_X.0 || x > AREA_X.1 {
            return true;
        }
        let lane = lane_of(z);
        (0..COLUMNS)
            .any(|col| self.lane[col] == lane && (BLOCK_X[col].0..=BLOCK_X[col].1).contains(&x))
    }
}

/// The lane of row `z` (1 to 5); 0 outside the five lanes.
fn lane_of(z: i32) -> u8 {
    if !(WIN_Z.0..=WIN_Z.1).contains(&z) {
        return 0;
    }
    ((z - GRID_Z) / LANE_HEIGHT + 1) as u8
}

/// The first row of a lane.
fn lane_z(lane: u8) -> i32 {
    GRID_Z + (i32::from(lane) - 1) * LANE_HEIGHT
}

type Pt = (i16, i16);

fn pt(x: i32, z: i32) -> Pt {
    (x as i16, z as i16)
}

/// Straight segments, each end a tile to walk to exactly. A straight segment
/// between two tiles has one shortest path, so the walker cannot cut a corner
/// through a tile the route did not prove.
const MAX_CORNERS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Route {
    len: u8,
    pts: [Pt; MAX_CORNERS],
}

impl Route {
    fn empty() -> Self {
        Self {
            len: 0,
            pts: [(0, 0); MAX_CORNERS],
        }
    }

    fn push(&mut self, point: Pt) -> bool {
        if usize::from(self.len) == MAX_CORNERS {
            return false;
        }
        self.pts[usize::from(self.len)] = point;
        self.len += 1;
        true
    }

    pub(super) fn corners(&self) -> &[Pt] {
        &self.pts[..usize::from(self.len)]
    }
}

fn cell(x: i32, z: i32) -> usize {
    (z - WIN_Z.0) as usize * WIDTH + (x - WIN_X.0) as usize
}

/// The shortest 4-neighbour route from `from` to `to` over tiles `memory`
/// proves safe (`from` itself may be unproven), as straight segments. `None`
/// when `to` is not proven, or no proven route reaches it.
pub(super) fn safe_route(memory: &GridMemory, from: (i32, i32), to: (i32, i32)) -> Option<Route> {
    let inside =
        |(x, z): (i32, i32)| (WIN_X.0..=WIN_X.1).contains(&x) && (WIN_Z.0..=WIN_Z.1).contains(&z);
    if !inside(from) || !inside(to) || !memory.proves_safe(to.0, to.1) {
        return None;
    }
    if from == to {
        return Some(Route::empty());
    }
    let mut parent = [u8::MAX; CELLS];
    let mut seen = [false; CELLS];
    let mut queue = [0u8; CELLS];
    let (mut head, mut tail) = (0, 0);
    let start = cell(from.0, from.1);
    let goal = cell(to.0, to.1);
    seen[start] = true;
    queue[tail] = start as u8;
    tail += 1;
    while head < tail {
        let at = usize::from(queue[head]);
        head += 1;
        if at == goal {
            break;
        }
        let (x, z) = (WIN_X.0 + (at % WIDTH) as i32, WIN_Z.0 + (at / WIDTH) as i32);
        for next in [(x - 1, z), (x + 1, z), (x, z + 1), (x, z - 1)] {
            if !inside(next) || !memory.proves_safe(next.0, next.1) {
                continue;
            }
            let id = cell(next.0, next.1);
            if seen[id] {
                continue;
            }
            seen[id] = true;
            parent[id] = at as u8;
            queue[tail] = id as u8;
            tail += 1;
        }
    }
    if !seen[goal] {
        return None;
    }
    let mut reversed = [0u8; CELLS];
    let mut count = 0;
    let mut at = goal;
    while at != start {
        reversed[count] = at as u8;
        count += 1;
        at = usize::from(parent[at]);
    }
    let mut route = Route::empty();
    let mut previous = from;
    let mut direction = (0, 0);
    for index in (0..count).rev() {
        let id = usize::from(reversed[index]);
        let tile = (WIN_X.0 + (id % WIDTH) as i32, WIN_Z.0 + (id / WIDTH) as i32);
        let step = (tile.0 - previous.0, tile.1 - previous.1);
        if direction != (0, 0) && step != direction && !route.push(pt(previous.0, previous.1)) {
            return None;
        }
        direction = step;
        previous = tile;
    }
    route.push(pt(previous.0, previous.1)).then_some(route)
}

/// Where a probe starts its unproven part: east of the area for column 1, else
/// the tile both neighbouring blocks cover on the previous column's lane.
pub(super) fn launch_tile(memory: &GridMemory, probe: Probe) -> (i32, i32) {
    let col = usize::from(probe.col);
    if col == 0 {
        (LAUNCH_X[0], lane_z(probe.lane))
    } else {
        (LAUNCH_X[col], lane_z(memory.lane[col - 1]))
    }
}

/// The unproven tiles of a probe, in walking order: a side step to the probed
/// lane when it differs from the previous column's lane, then the probe tile.
pub(super) fn explore_tiles(memory: &GridMemory, probe: Probe) -> ([Pt; 2], usize) {
    let col = usize::from(probe.col);
    let z = lane_z(probe.lane);
    let side_step = col > 0 && memory.lane[col - 1] != probe.lane;
    if side_step {
        ([pt(LAUNCH_X[col], z), pt(PROBE_X[col], z)], 2)
    } else {
        ([pt(PROBE_X[col], z), (0, 0)], 1)
    }
}

/// Whether the next trap cannot kill: the fall deals 15 (`upass_grid.rs2:99`).
pub(super) fn survives_trap(hitpoints: i32) -> bool {
    hitpoints > TRAP_DAMAGE
}

fn in_pit(tile: WorldTile) -> bool {
    tile.level == PIT.level
        && (PIT.x..=PIT.x + 1).contains(&tile.x)
        && (PIT.z..=PIT.z + 1).contains(&tile.z)
}

fn in_window(tile: WorldTile) -> bool {
    tile.level == 0
        && (WIN_X.0..=WIN_X.1).contains(&tile.x)
        && (WIN_Z.0..=WIN_Z.1).contains(&tile.z)
}

fn tile(point: Pt) -> WorldTile {
    WorldTile {
        x: i32::from(point.0),
        z: i32::from(point.1),
        level: 0,
    }
}

// ---------------------------------------------------------------------------
// The step.

struct Plan {
    arrival: WorldTile,
    hand_holds_id: i32,
}

impl StepPlan for Plan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let kept = cx.tick.cx.retained().quester().upass_grid.kept();
        if let Some(kept) = kept {
            upass_store::Loaded::Kept {
                columns: kept.memory.confirmed(),
                traps: kept.memory.traps(),
                unsaved: kept.store.unsaved(),
            }
            .log(cx.tick.state.map_or("", |state| state.account().account()));
        }
        Ok(Box::new(Run::new(self.arrival, self.hand_holds_id, kept)))
    }
}

/// The step plan for a runner test: a far side and the hand holds' id.
#[cfg(test)]
pub(in crate::quester) fn plan_for_test(arrival: WorldTile, hand_holds_id: i32) -> Arc<dyn StepPlan> {
    Arc::new(Plan {
        arrival,
        hand_holds_id,
    })
}

#[derive(Clone, Copy)]
enum After {
    /// Start the unproven part of this probe.
    Probe(Probe),
    /// The far side is reached.
    Finish,
    /// Read position and memory again.
    Decide,
}

#[derive(Clone, Copy)]
enum Phase {
    /// Read the account's saved route, then decide.
    Load,
    /// Read the position and memory, choose what to do next.
    Decide,
    /// Walk proven tiles: one corner at a time, the last with `last_radius`,
    /// then `after`.
    Spine {
        route: Route,
        next: u8,
        last_radius: u16,
        after: After,
    },
    /// Walk the unproven tiles of a probe (`probe` is `None` for a retreat
    /// from an unproven tile that no proven route leaves).
    Explore {
        tiles: [Pt; 2],
        len: u8,
        next: u8,
        probe: Option<Probe>,
    },
    /// Stand on the probe tile until the timer would have fired.
    Observe { probe: Probe, until: u64 },
    /// The trap line came; wait for the pit teleport.
    Fallen { deadline: u64 },
    /// In the pit: one click on the hand holds per attempt, until the
    /// teleport back.
    Climb {
        pit_seen: u64,
        attempts: u8,
        click: Click,
    },
}

/// Where the current climb click is.
#[derive(Clone, Copy)]
enum Click {
    /// None sent in this attempt.
    Ready,
    /// Emitted; the host has not settled it.
    Sent,
    /// The host sent it; wait for the teleport until this tick.
    Accepted { until: u64 },
}

impl Phase {
    fn climb(pit_seen: u64) -> Self {
        Phase::Climb {
            pit_seen,
            attempts: 0,
            click: Click::Ready,
        }
    }
}

struct Run {
    arrival: WorldTile,
    hand_holds_id: i32,
    phase: Phase,
    hop: Hop,
    /// The climb click's machine while one is in flight.
    click: Option<ActionHandle<ClimbClick>>,
    /// Chat sequence before the unproven part began; the trap line is newer.
    chat_mark: i32,
    stats_wait: u8,
    /// What this account's route is known to be (saved by `store`, and kept
    /// in the run's retained cell by [`Run::keep`]).
    memory: GridMemory,
    store: RouteStore,
    /// The step's end, held until its last write settled: the result, and the
    /// tick the wait began.
    hold: Option<(Result<(), ActionError>, u64)>,
}

/// One exact-tile walk with the shared bounded retry of the `walk` step.
#[derive(Default)]
struct Hop {
    walk: Option<PrereqWalk>,
    retry: WalkRetryState,
}

impl Hop {
    fn poll(
        &mut self,
        target: WorldTile,
        radius: u16,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<(), ActionError>> {
        let now = cx.tick.cx.evidence().tick;
        if let Some(walk) = self.walk.as_mut() {
            match walk.poll(cx.tick.actions, &mut cx.tick.cx, cx.tick.state) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.walk = None;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(receipt)) => {
                    self.walk = None;
                    if self.retry.observe_failure(&receipt, now) {
                        return Poll::Pending;
                    }
                    let arrived = receipt.into_arrival().map(|_| ());
                    if arrived.is_ok() {
                        self.retry.reset();
                    }
                    return Poll::Ready(arrived);
                }
            }
        }
        if self.retry.waiting_for_next_tick(now) {
            return Poll::Pending;
        }
        let request = reach::walk_request(target, radius, None, cx.required_after);
        match crate::native::defer_budget(PrereqWalk::begin(
            request,
            cx.tick.actions,
            &mut cx.tick.cx,
        )) {
            Poll::Pending => {
                self.retry.defer_until_next_tick(now);
                Poll::Pending
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(handle)) => {
                self.retry.action_submitted();
                self.walk = Some(handle);
                Poll::Pending
            }
        }
    }

    fn cancel(&mut self) {
        if let Some(walk) = self.walk.as_mut() {
            walk.cancel();
        }
        self.walk = None;
        self.retry.reset();
    }
}

fn failed(detail: impl Into<String>) -> ActionError {
    ActionError::Failed(Arc::from(detail.into()))
}

impl Run {
    /// A step that reads the account's route before it decides anything, or
    /// takes the run's route and saved-route state `kept` when an earlier step
    /// of the run read it.
    fn new(arrival: WorldTile, hand_holds_id: i32, kept: Option<RunRoute>) -> Self {
        let (memory, store, phase) = match kept {
            Some(RunRoute { memory, mut store }) => {
                store.resume();
                (memory, store, Phase::Decide)
            }
            None => (GridMemory::default(), RouteStore::loading(), Phase::Load),
        };
        Self {
            arrival,
            hand_holds_id,
            phase,
            hop: Hop::default(),
            click: None,
            chat_mark: 0,
            stats_wait: 0,
            memory,
            store,
            hold: None,
        }
    }

    /// Keep `memory` and save it, when it differs from what is kept.
    fn learn(&mut self, memory: GridMemory) {
        if self.memory.to_payload() != memory.to_payload() {
            self.store.changed();
        }
        self.memory = memory;
    }

    fn done(cx: &StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
    }

    /// The trap's own signals: its first line, or the player in the pit.
    fn trap_signal(&self, cx: &StepContext<'_, '_>, here: WorldTile) -> bool {
        in_pit(here) || reach::saw_game_message(&cx.tick.cx, self.chat_mark, TRAP_MESSAGE)
    }

    /// Current Hitpoints, `Pending` until the stats row is observed.
    fn hitpoints(&mut self, cx: &StepContext<'_, '_>) -> Result<Option<i32>, ActionError> {
        let row = cx.tick.cx.snapshot().stats().and_then(|stats| {
            stats
                .value
                .iter()
                .find(|row| row.index == HITPOINTS && row.used)
                .map(|row| row.effective)
        });
        if row.is_some() {
            self.stats_wait = 0;
            return Ok(row);
        }
        self.stats_wait += 1;
        if self.stats_wait > STATS_TICKS {
            return Err(failed(
                "Underground Pass grid: Hitpoints were not observed, so a trap's 15 damage cannot be ruled out",
            ));
        }
        Ok(None)
    }

    /// Start the unproven part of `probe` (or a retreat when `probe` is None).
    fn begin_explore(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        tiles: [Pt; 2],
        len: usize,
        probe: Option<Probe>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let memory = self.memory;
        if memory.traps() >= MAX_TRAPS {
            return Poll::Ready(Err(failed(format!(
                "Underground Pass grid: {} traps taken and the route is still unknown; a fixed route needs at most {MAX_TRAPS}",
                memory.traps()
            ))));
        }
        let hitpoints = match self.hitpoints(cx) {
            Ok(Some(hitpoints)) => hitpoints,
            Ok(None) => return Poll::Pending,
            Err(error) => return Poll::Ready(Err(error)),
        };
        if !survives_trap(hitpoints) {
            return Poll::Ready(Err(failed(format!(
                "Underground Pass grid: {hitpoints} Hitpoints would not survive a trap's {TRAP_DAMAGE} damage; {} traps so far",
                memory.traps()
            ))));
        }
        self.chat_mark = reach::last_chat_seq(&cx.tick.cx);
        self.phase = Phase::Explore {
            tiles,
            len: len as u8,
            next: 0,
            probe,
        };
        Poll::Pending
    }

    fn decide(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        here: WorldTile,
        tick: u64,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        if in_pit(here) {
            self.phase = Phase::climb(tick);
            return Poll::Pending;
        }
        if here.x < AREA_X.0 && reach::within(here, self.arrival, ARRIVAL_RADIUS) {
            return Self::done(cx);
        }
        if !in_window(here) {
            // West of the window the grid is behind us; anywhere else the
            // entrance is the way in. Neither crosses the area.
            let mut route = Route::empty();
            if here.level == 0 && here.x < WIN_X.0 {
                route.push(pt(self.arrival.x, self.arrival.z));
                return self.start_spine(route, After::Finish, 0);
            }
            route.push(pt(ENTRY.x, ENTRY.z));
            return self.start_spine(route, After::Decide, 0);
        }
        let mut memory = self.memory;
        let here_xz = (here.x, here.z);
        match memory.next() {
            Next::Inconsistent => {
                api::host_log!(
                    api::hostlog::Category::ScriptTrace,
                    api::hostlog::Level::Info,
                    "quester upass grid: every allowed lane trapped; the stored route is stale, learning again ({} traps so far)",
                    memory.traps()
                );
                memory.clear_route();
                self.learn(memory);
                Poll::Pending
            }
            Next::Known => {
                let arrival = (self.arrival.x, self.arrival.z);
                match safe_route(&memory, here_xz, arrival) {
                    Some(route) => self.start_spine(route, After::Finish, 0),
                    None => self.retreat(cx, here),
                }
            }
            Next::Probe(probe) => {
                let launch = launch_tile(&memory, probe);
                match safe_route(&memory, here_xz, launch) {
                    Some(route) => self.start_spine(route, After::Probe(probe), 0),
                    None => self.retreat(cx, here),
                }
            }
        }
    }

    /// From a tile no proven route leaves, step east along the row to the
    /// entrance side. A trap on the way only takes the player where the climb
    /// ends anyway, and counts; Hitpoints are checked first.
    fn retreat(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        here: WorldTile,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.begin_explore(cx, [pt(LAUNCH_X[0], here.z), (0, 0)], 1, None)
    }

    fn start_spine(
        &mut self,
        route: Route,
        after: After,
        last_radius: u16,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        self.hop.cancel();
        self.phase = Phase::Spine {
            route,
            next: 0,
            last_radius,
            after,
        };
        Poll::Pending
    }
}

impl Run {
    /// A trap fired: count it, learn from where it fired, wait for the pit.
    fn fall(&mut self, here: WorldTile, tick: u64) -> Poll<Result<StepOutcome, ActionError>> {
        self.hop.cancel();
        let mut memory = self.memory;
        memory.count_trap();
        match self.phase {
            Phase::Explore {
                probe: Some(probe), ..
            }
            | Phase::Observe { probe, .. } => {
                memory.rule_out(probe);
                api::host_log!(
                    api::hostlog::Category::ScriptTrace,
                    api::hostlog::Level::Info,
                    "quester upass grid: trap in column {} lane {} (trap {})",
                    probe.col + 1,
                    probe.lane,
                    memory.traps()
                );
            }
            Phase::Explore { probe: None, .. } => {
                api::host_log!(
                    api::hostlog::Category::ScriptTrace,
                    api::hostlog::Level::Info,
                    "quester upass grid: trap while leaving an unproven tile (trap {})",
                    memory.traps()
                );
            }
            _ => {
                // A tile the route proved safe trapped: the stored route is
                // not this player's route any more.
                memory.clear_route();
                api::host_log!(
                    api::hostlog::Category::ScriptTrace,
                    api::hostlog::Level::Info,
                    "quester upass grid: trap on a tile the learned route proved safe; learning again (trap {})",
                    memory.traps()
                );
            }
        }
        self.learn(memory);
        self.phase = if in_pit(here) {
            Phase::climb(tick)
        } else {
            Phase::Fallen {
                deadline: tick + FALL_TICKS,
            }
        };
        Poll::Pending
    }

    fn spine(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        route: Route,
        next: u8,
        last_radius: u16,
        after: After,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let corners = route.corners();
        if let Some(&point) = corners.get(usize::from(next)) {
            let radius = if usize::from(next) + 1 == corners.len() {
                last_radius
            } else {
                0
            };
            return match self.hop.poll(tile(point), radius, cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => {
                    self.phase = Phase::Spine {
                        route,
                        next: next + 1,
                        last_radius,
                        after,
                    };
                    Poll::Pending
                }
            };
        }
        match after {
            After::Finish => Self::done(cx),
            After::Decide => {
                self.phase = Phase::Decide;
                Poll::Pending
            }
            After::Probe(probe) => {
                let memory = self.memory;
                let (tiles, len) = explore_tiles(&memory, probe);
                self.begin_explore(cx, tiles, len, Some(probe))
            }
        }
    }

    fn explore(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        tiles: [Pt; 2],
        len: u8,
        next: u8,
        probe: Option<Probe>,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(&point) = tiles[..usize::from(len)].get(usize::from(next)) else {
            self.phase = Phase::Decide;
            return Poll::Pending;
        };
        match self.hop.poll(tile(point), 0, cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {
                let arrived = cx.tick.cx.evidence().tick;
                self.phase = match (next + 1 < len, probe) {
                    (true, _) => Phase::Explore {
                        tiles,
                        len,
                        next: next + 1,
                        probe,
                    },
                    (false, Some(probe)) => Phase::Observe {
                        probe,
                        until: arrived + OBSERVE_TICKS,
                    },
                    (false, None) => Phase::Decide,
                };
                Poll::Pending
            }
        }
    }

    fn observe(
        &mut self,
        here: WorldTile,
        tick: u64,
        probe: Probe,
        until: u64,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        if tick < until {
            return Poll::Pending;
        }
        let probe_tile = WorldTile {
            x: PROBE_X[usize::from(probe.col)],
            z: lane_z(probe.lane),
            level: 0,
        };
        if here == probe_tile {
            let mut memory = self.memory;
            memory.confirm(probe);
            self.learn(memory);
            api::host_log!(
                api::hostlog::Category::ScriptTrace,
                api::hostlog::Level::Info,
                "quester upass grid: column {} is lane {} ({} traps so far)",
                probe.col + 1,
                probe.lane,
                memory.traps()
            );
        }
        self.phase = Phase::Decide;
        Poll::Pending
    }

    fn climbing(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        here: WorldTile,
        tick: u64,
        pit_seen: u64,
        attempts: u8,
        click: Click,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        if !in_pit(here) {
            if let Some(handle) = self.click.take() {
                cx.tick.actions.cancel(handle);
            }
            self.phase = Phase::Decide;
            return Poll::Pending;
        }
        if tick < pit_seen + POST_FALL_TICKS {
            return Poll::Pending;
        }
        let next = |click| Phase::Climb {
            pit_seen,
            attempts,
            click,
        };
        match click {
            Click::Ready => {
                if attempts >= CLIMB_ATTEMPTS {
                    return Poll::Ready(Err(failed(format!(
                        "Underground Pass grid: {CLIMB_ATTEMPTS} climbs did not leave the pit"
                    ))));
                }
                // The player stands on one of the four hand holds
                // (`0_37_149_27_23` plus 0 or 1 on x and z, `:94`).
                let args = ClimbClickArgs {
                    x: here.x,
                    z: here.z,
                    level: here.level,
                    id: self.hand_holds_id,
                };
                match cx.tick.actions.begin::<ClimbClick>(args, &mut cx.tick.cx) {
                    Ok(handle) => {
                        self.click = Some(handle);
                        self.phase = Phase::Climb {
                            pit_seen,
                            attempts: attempts + 1,
                            click: Click::Sent,
                        };
                    }
                    // Another action or this tick's event allowance holds
                    // the slot: try again next tick.
                    Err(ActionError::Busy | ActionError::BudgetExhausted) => {}
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            Click::Sent => {
                let Some(handle) = self.click.as_ref() else {
                    self.phase = next(Click::Ready);
                    return Poll::Pending;
                };
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Pending => {}
                    Poll::Ready(Ok(())) => {
                        self.click = None;
                        self.phase = next(Click::Accepted {
                            until: tick + CLIMB_TICKS,
                        });
                    }
                    // The host dropped or refused the click (the scene may
                    // still be loading after the fall): the same click is
                    // tried again next tick, within the attempt bound.
                    Poll::Ready(Err(ActionError::Failed(_))) => {
                        self.click = None;
                        self.phase = next(Click::Ready);
                    }
                    Poll::Ready(Err(error)) => {
                        self.click = None;
                        return Poll::Ready(Err(error));
                    }
                }
            }
            Click::Accepted { until } => {
                if tick >= until {
                    self.phase = next(Click::Ready);
                }
            }
        }
        Poll::Pending
    }
}

struct ClimbClickArgs {
    x: i32,
    z: i32,
    level: i32,
    id: i32,
}

/// One `Climb` click on the hand hold under the player, sent once and
/// settled on the host's receipt. A step cannot queue an interaction itself:
/// only a machine that owns the action can.
struct ClimbClick {
    args: ClimbClickArgs,
    request: Option<u64>,
}

impl NativeMachine for ClimbClick {
    type Args = ClimbClickArgs;
    type Output = ();

    fn begin(args: ClimbClickArgs, _cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            args,
            request: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        let Some(request) = self.request else {
            let ClimbClickArgs { x, z, level, id } = self.args;
            let request = InteractReq::Loc {
                x,
                z,
                level,
                action: "Climb".into(),
                id: Some(id),
            };
            return match crate::native::defer_budget(cx.emit(request)) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(id)) => {
                    self.request = Some(id);
                    Poll::Pending
                }
            };
        };
        match cx.interaction_dispatch(request) {
            InteractionDispatch::Queued => Poll::Pending,
            InteractionDispatch::Unsent => Poll::Ready(Err(failed(
                "Underground Pass grid: the host dropped the climb click before it was sent",
            ))),
            InteractionDispatch::Settled(receipt) if !receipt.accepted => Poll::Ready(Err(failed(
                "Underground Pass grid: the host refused the climb click",
            ))),
            InteractionDispatch::Settled(_) => Poll::Ready(Ok(())),
        }
    }

    fn cancel(&mut self) {}
}

impl StepRun for Run {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let result = self.step(cx);
        self.keep(cx);
        result
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        self.hop.cancel();
        if let Some(handle) = self.click.take() {
            actions.cancel(handle);
        }
    }
}

impl Run {
    /// Keep the route and its saved-route state in the run's retained cell, so
    /// that a step the runner builds again in this run (survival drops it)
    /// continues from them. Once per poll, and only when they changed; nothing
    /// is kept while the route is still being read.
    fn keep(&self, cx: &mut StepContext<'_, '_>) {
        if self.store.is_loading() {
            return;
        }
        cx.tick.cx.retained().quester().upass_grid.keep(RunRoute {
            memory: self.memory,
            store: self.store,
        });
    }

    /// One poll of the step: the route's read and writes, then the crossing.
    fn step(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let tick = cx.tick.cx.evidence().tick;
        let state = cx.tick.state;
        if self.store.is_loading() {
            return match self.store.load(state, tick) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok((memory, loaded))) => {
                    loaded.log(state.map_or("", |state| state.account().account()));
                    self.memory = memory;
                    self.phase = Phase::Decide;
                    Poll::Pending
                }
            };
        }
        self.store.pump(state, tick, &self.memory);
        if let Some((result, since)) = self.hold.take() {
            if !self.store.settled() {
                if tick < since + upass_store::CONFIRM_TICKS {
                    self.hold = Some((result, since));
                    return Poll::Pending;
                }
                cx.tick.output.log(
                    api::hostlog::Level::Warn,
                    &RouteStore::unconfirmed_line(state, tick - since),
                );
            }
            return match result {
                Ok(()) => Self::done(cx),
                Err(error) => Poll::Ready(Err(error)),
            };
        }
        let advanced = self.advance(cx, tick);
        // A route learned this tick is sent this tick.
        self.store.pump(state, tick, &self.memory);
        match advanced {
            Poll::Ready(result) if !self.store.settled() => {
                self.hold = Some((result.map(|_| ()), tick));
                Poll::Pending
            }
            other => other,
        }
    }

    /// One tick of the crossing itself.
    fn advance(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        tick: u64,
    ) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(here) = cx.tick.cx.snapshot().here().map(|observed| observed.value) else {
            return Poll::Pending;
        };
        let exposed = matches!(
            self.phase,
            Phase::Spine { .. } | Phase::Explore { .. } | Phase::Observe { .. }
        );
        if exposed && self.trap_signal(cx, here) {
            return self.fall(here, tick);
        }
        match self.phase {
            Phase::Load => Poll::Pending,
            Phase::Decide => {
                self.chat_mark = reach::last_chat_seq(&cx.tick.cx);
                self.decide(cx, here, tick)
            }
            Phase::Spine {
                route,
                next,
                last_radius,
                after,
            } => self.spine(cx, route, next, last_radius, after),
            Phase::Explore {
                tiles,
                len,
                next,
                probe,
            } => self.explore(cx, tiles, len, next, probe),
            Phase::Observe { probe, until } => self.observe(here, tick, probe, until),
            Phase::Fallen { deadline } => {
                if in_pit(here) {
                    self.phase = Phase::climb(tick);
                    Poll::Pending
                } else if tick >= deadline {
                    Poll::Ready(Err(failed(format!(
                        "Underground Pass grid: the trap line came but no fall to the pit followed within {FALL_TICKS} ticks"
                    ))))
                } else {
                    Poll::Pending
                }
            }
            Phase::Climb {
                pit_seen,
                attempts,
                click,
            } => self.climbing(cx, here, tick, pit_seen, attempts, click),
        }
    }
}

#[cfg(test)]
#[path = "upass_tests.rs"]
pub(super) mod tests;

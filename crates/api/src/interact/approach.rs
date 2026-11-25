//! Reach gate and stall watch for owned world operations.
//!
//! The client menu routes to a target and then sends the operation. When its
//! route search fails it still sends the operation, alone
//! (`vendor/fr-client-rust/crates/client/src/client/client.rs:4300-4310`, the
//! NPC arm ignores `tryMove`'s result). The Lost City engine then keeps the
//! interaction and paths greedily. A greedy step that collision blocks takes
//! no step, but the queued waypoint stays, so the engine's `I can't reach
//! that!` branch (`Player.ts:1300-1304`, 289 engine; `Player.ts:1291-1295`,
//! 274 engine) never runs: `!hasWaypoints()` is false. The bot then waits
//! with no refusal and no outcome.
//!
//! Two owned-path defences live here:
//! - [`op_reach`] classifies a target before dispatch. [`OpReach::Unreachable`]
//!   means no operable stand is reachable by collision and the engine's
//!   approach-range escape cannot fire either, so the owned op is refused.
//! - [`ApproachWatch`] follows an accepted op that needed an approach and
//!   reports [`ApproachVerdict::Stalled`] after [`APPROACH_STALL_TICKS`]
//!   observed server ticks without movement or any outcome.
//!
//! Manual play and sealed advanced packets do not pass through here.

use super::OpTarget;
use crate::line_of_sight::{has_line_of_sight_local, Footprint};
use crate::query::loc_approach::{
    loc_can_operate, loc_candidate_stands, LocApproach, LocOperateQuery,
};
use crate::query::{ReachFlood, SceneQuery};
use crate::snapshot::{GameSnapshot, LocalTile, WorldTile};

/// The engine's default approach range: `Player.apRange` is set to 10 when an
/// interaction starts (`PathingEntity.ts:552-579`, 289 engine). An approach
/// script (`[apnpc*]`, `[aploc*]`, `[apobj*]`) runs without a step when the
/// target is within this distance and in line of sight
/// (`PathingEntity.ts:422-436`, `inApproachDistance`; `Player.ts:1194`). A
/// target outside that window is never served by an approach script.
pub const APPROACH_RANGE: i32 = 10;

/// Consecutive observed server ticks without movement that end an approach.
///
/// The engine gives no deadline: a blocked step takes no step, leaves the
/// waypoint and `stepsTaken` at 0 (`PathingEntity.ts:206-251,651-696`), so
/// the `I can't reach that!` branch (`!hasWaypoints()`, 289 `Player.ts:1302`,
/// 274 `Player.ts:1293`) never runs, and the greedy endpoint is kept until
/// the target leaves a one-tile window (`Player.ts:1086-1102`, 274 `:1085-
/// 1101`). A watcher cannot tell that from a transient wait, so N is an
/// upper bound on the transient causes the engine source shows, chosen so a
/// transient cause outlasts it with probability at most 1%:
/// - Latency, 2 ticks: the server reads the op in the next cycle's
///   `processClientsIn` and moves the player in the same cycle (`World.ts:356`
///   before `:379`, 289); one more cycle covers a packet that misses that
///   read.
/// - An NPC blocker, or an NPC target whose move frees a stale endpoint,
///   starts a wander with probability 1/8 per tick (289 `Npc.ts:721-722`,
///   274 `Npc.ts:714-715`, `Math.random() < 0.125`). `wander` picks a random
///   destination within `wanderrange` of the spawn and may pick the tile the
///   NPC already stands on (289 `Npc.ts:702-710`, 274 `:695-703`), so a
///   wander is not always a move. For an idle NPC at spawn with
///   `wanderrange=1` (the narrowest range content uses, e.g. Aggie, 289
///   `area_draynor/configs/draynor.npc:76-100`) each axis picks zero with
///   probability 1/2, so a quarter of wanders pick the current tile and the
///   per-tick probability of no displacement is `p = 1 - (1/8)(3/4) =
///   29/32 = 0.906` (the reviewer's measurement of 35 idle ticks, 3.19%,
///   gives the same `p`). A wider range only lowers `p`. The NPC is still in
///   place after `k` ticks with probability about `p^k`. Solving `p^k < 1%`:
///   `k > ln(0.01) / ln(29/32) = 46.8`, so 47 ticks; `(29/32)^47 = 0.0098`.
///   This does not bound collision, which can keep a moved NPC in the way.
///
/// N = 2 + 47 = 49 observed ticks (about 29 s at 600 ms). This bounds the
/// NPC wander cause only, and not collision. A player blocker, a delayed or
/// frozen player and any wait with no source bound are not covered: the watch
/// can flag a legitimate wait of that kind, and its consumers re-route or
/// fail instead of trusting it. Observed ticks come from the snapshot's
/// server-tick count ([`server_ticks`]) and only one-packet observations
/// count ([`ApproachWatch::observe_sample`]); never polls or wall time.
pub const APPROACH_STALL_TICKS: u32 = 49;

/// The player's observed server-tick count. One place to switch to the shared
/// tick-health receipt when it lands: `snapshot.tick()` advances by the
/// `PLAYER_INFO` packets the host applied since the previous observation.
pub fn server_ticks(snapshot: &GameSnapshot) -> u32 {
    snapshot.tick()
}

/// How far an owned world operation's target is from the player's reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpReach {
    /// The player already stands where the engine operates the target. The
    /// operation alone is valid and runs on its first cycle.
    InReach,
    /// Some operable stand is reachable by collision: the client's route
    /// search finds a route (or the engine's greedy approach can follow one).
    Routable,
    /// An approach script may serve the target from a tile the player can
    /// walk to: within [`APPROACH_RANGE`] and in line of sight of it, with no
    /// operable stand reachable. The operation is sent as before and watched:
    /// a script serves it at once or after the approach, so standing still
    /// with no outcome is still a stall.
    ApproachScript,
    /// No verdict: the scene, the player or the loc's shape has no model
    /// here. The operation is sent as before and watched.
    Unknown,
    /// No operable stand is reachable and no approach script can run (the
    /// target is beyond [`APPROACH_RANGE`] or out of line of sight). The
    /// engine would path greedily and stall silently.
    Unreachable,
}

/// The body an operation addresses, in the terms the engine's reach uses.
enum Geometry<'a> {
    /// An NPC, player or ground object: a size-by-size rectangle at `origin`.
    Rect {
        origin: WorldTile,
        width: i32,
        length: i32,
    },
    Loc(&'a crate::snapshot::LocView),
}

impl<'a> Geometry<'a> {
    fn of(target: &OpTarget<'a>) -> Option<Self> {
        match target {
            OpTarget::Npc(npc) => {
                let size = npc.size.max(1);
                Some(Self::Rect {
                    origin: npc.network,
                    width: size,
                    length: size,
                })
            }
            OpTarget::Player(player) => Some(Self::Rect {
                origin: player.network,
                width: 1,
                length: 1,
            }),
            OpTarget::GroundItem(ground) => Some(Self::Rect {
                origin: ground.tile,
                width: 1,
                length: 1,
            }),
            OpTarget::Loc(loc) => Some(Self::Loc(loc)),
            OpTarget::Item(_) => None,
        }
    }

    fn origin(&self) -> WorldTile {
        match self {
            Self::Rect { origin, .. } => *origin,
            Self::Loc(loc) => loc.tile,
        }
    }

    /// The rectangle's side lengths for the range and sight test.
    fn span(&self) -> (i32, i32) {
        match self {
            Self::Rect { width, length, .. } => (*width, *length),
            Self::Loc(loc) => (loc.footprint_width.max(1), loc.footprint_length.max(1)),
        }
    }

    /// Whether the engine operates the target from `from`. `None` when the
    /// loc's shape has no reach model.
    fn can_operate(&self, query: &SceneQuery<'_>, from: WorldTile) -> Option<bool> {
        let flags = query.collision_at(from)?;
        match self {
            Self::Rect {
                origin,
                width,
                length,
            } => Some(
                LocApproach {
                    width: u8::try_from(*width).ok()?,
                    length: u8::try_from(*length).ok()?,
                    blocked_sides: 0,
                }
                .can_operate(*origin, from, flags),
            ),
            Self::Loc(loc) => loc_can_operate(LocOperateQuery {
                origin: loc.tile,
                from,
                shape: u8::try_from(loc.shape).ok()?,
                angle: u8::try_from(loc.angle).ok()?,
                width: loc.width,
                length: loc.length,
                force_approach: loc.force_approach,
                flags,
            }),
        }
    }

    /// Every tile that may be an operable stand. `None` for an unmodelled loc.
    fn candidates(&self) -> Option<Vec<WorldTile>> {
        match self {
            Self::Rect {
                origin,
                width,
                length,
            } => {
                let mut tiles = Vec::with_capacity(((width + 2) * (length + 2)) as usize);
                for x in (origin.x - 1)..=(origin.x + width) {
                    for z in (origin.z - 1)..=(origin.z + length) {
                        tiles.push(WorldTile {
                            x,
                            z,
                            level: origin.level,
                        });
                    }
                }
                Some(tiles)
            }
            Self::Loc(loc) => loc_candidate_stands(
                loc.tile,
                u8::try_from(loc.shape).ok()?,
                u8::try_from(loc.angle).ok()?,
                loc.width,
                loc.length,
            ),
        }
    }
}

/// Whether an approach script could serve the target from some tile the
/// player can walk to: within [`APPROACH_RANGE`] of the target and in line of
/// sight. The engine walks and tests the approach again each cycle
/// (`Player.ts:1194` `tryInteract`; called before and after the step at
/// `:1277,1298`), so a tile that is out of range or sight from
/// where the player stands now still counts when the player can reach it.
/// Only a target no reachable tile sees within range is out of every
/// approach script's window.
fn approach_script_reachable(
    query: &SceneQuery<'_>,
    flood: &ReachFlood,
    geometry: &Geometry<'_>,
) -> bool {
    let origin = geometry.origin();
    let (width, length) = geometry.span();
    let size = width.max(length);
    let flags = |lx: i32, lz: i32| query.collision_at_local(LocalTile { lx, lz });
    let Some(to) = query.to_local(origin) else {
        return false;
    };
    let target = Footprint {
        lx: to.lx,
        lz: to.lz,
        size,
    };
    for x in (origin.x - APPROACH_RANGE)..(origin.x + width + APPROACH_RANGE) {
        for z in (origin.z - APPROACH_RANGE)..(origin.z + length + APPROACH_RANGE) {
            let tile = WorldTile {
                x,
                z,
                level: origin.level,
            };
            if !flood.at(&tile).0 {
                continue;
            }
            let Some(from) = query.to_local(tile) else {
                continue;
            };
            if has_line_of_sight_local(
                &flags,
                Footprint {
                    lx: from.lx,
                    lz: from.lz,
                    size: 1,
                },
                target,
            ) {
                return true;
            }
        }
    }
    false
}

/// Whether the player already stands where the engine operates the target.
/// `None` when the scene, the player or the geometry gives no answer.
pub fn in_reach(snapshot: &GameSnapshot, target: &OpTarget<'_>) -> Option<bool> {
    let geometry = Geometry::of(target)?;
    let player = snapshot.local_player()?.player.network;
    let query = SceneQuery::new(snapshot.scene(), Some(player));
    if !query.contains(player) || !query.contains(geometry.origin()) {
        return None;
    }
    geometry.can_operate(&query, player)
}

/// Classify `target` for an owned operation sent from the snapshot's local
/// player. Only a positive proof of unreachability returns
/// [`OpReach::Unreachable`]; an unavailable scene, an unmodelled loc shape reads as
/// [`OpReach::Unknown`], a possible approach script as [`OpReach::ApproachScript`].
pub fn op_reach(snapshot: &GameSnapshot, target: &OpTarget<'_>) -> OpReach {
    let Some(geometry) = Geometry::of(target) else {
        return OpReach::Unknown;
    };
    let Some(player) = snapshot.local_player().map(|p| p.player.network) else {
        return OpReach::Unknown;
    };
    let query = SceneQuery::new(snapshot.scene(), Some(player));
    if !query.contains(player) || !query.contains(geometry.origin()) {
        return OpReach::Unknown;
    }
    match geometry.can_operate(&query, player) {
        None => return OpReach::Unknown,
        Some(true) => return OpReach::InReach,
        Some(false) => {}
    }
    // The collision map calls the player's own tile blocked: map and position
    // disagree, so no flood from it proves anything.
    if !query.walkable(player) {
        return OpReach::Unknown;
    }
    let Some(candidates) = geometry.candidates() else {
        return OpReach::Unknown;
    };
    let Some(flood) = query.flood_reach() else {
        return OpReach::Unknown;
    };
    if reachable_stand(&query, &flood, &geometry, &candidates) {
        OpReach::Routable
    } else if approach_script_reachable(&query, &flood, &geometry) {
        OpReach::ApproachScript
    } else if candidates
        .iter()
        .any(|&stand| on_boundary_ring(snapshot, stand))
    {
        // The client blocks the scene's outermost ring of tiles whatever the
        // server holds there (`CollisionMap::reset`): a stand on it proves
        // nothing about reach.
        OpReach::Unknown
    } else {
        OpReach::Unreachable
    }
}

/// Whether `tile` is on the outermost ring of the built scene.
fn on_boundary_ring(snapshot: &GameSnapshot, tile: WorldTile) -> bool {
    let scene = snapshot.scene();
    let (lx, lz) = (tile.x - scene.base_x, tile.z - scene.base_z);
    lx <= 0 || lz <= 0 || lx >= scene.width - 1 || lz >= scene.height - 1
}

fn reachable_stand(
    query: &SceneQuery<'_>,
    flood: &ReachFlood,
    geometry: &Geometry<'_>,
    candidates: &[WorldTile],
) -> bool {
    candidates.iter().any(|&stand| {
        query.walkable(stand)
            && flood.at(&stand).0
            && geometry.can_operate(query, stand) == Some(true)
    })
}

/// The identity of the target a watch follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchTarget {
    Npc { index: usize },
    Player { index: usize },
    Loc { tile: WorldTile, id: i32 },
    Obj { tile: WorldTile, id: i32 },
}

impl WatchTarget {
    fn of(target: &OpTarget<'_>) -> Option<Self> {
        match target {
            OpTarget::Npc(npc) => Some(Self::Npc { index: npc.index }),
            OpTarget::Player(player) => Some(Self::Player {
                index: player.index,
            }),
            OpTarget::Loc(loc) => Some(Self::Loc {
                tile: loc.tile,
                id: loc.id,
            }),
            OpTarget::GroundItem(ground) => Some(Self::Obj {
                tile: ground.tile,
                id: ground.def.id,
            }),
            OpTarget::Item(_) => None,
        }
    }

    /// Whether the player stands where the engine operates the current
    /// instance of this target. `None` when the target is no longer present.
    fn in_reach(self, snapshot: &GameSnapshot) -> Option<Option<bool>> {
        match self {
            Self::Npc { index } => {
                let npc = snapshot.npcs().iter().find(|npc| npc.index == index)?;
                Some(in_reach(snapshot, &OpTarget::Npc(npc)))
            }
            Self::Player { index } => {
                let player = snapshot.players().iter().find(|p| p.index == index)?;
                Some(in_reach(snapshot, &OpTarget::Player(player)))
            }
            Self::Loc { tile, id } => {
                let loc = snapshot
                    .locs()
                    .iter()
                    .find(|loc| loc.tile == tile && loc.id == id)?;
                Some(in_reach(snapshot, &OpTarget::Loc(loc)))
            }
            Self::Obj { tile, id } => {
                let ground = snapshot
                    .ground_items()
                    .iter()
                    .find(|item| item.tile == tile && item.def.id == id)?;
                Some(in_reach(snapshot, &OpTarget::GroundItem(ground)))
            }
        }
    }
}

/// Why a watch ended without a stall: the approach no longer needs movement,
/// or an outcome reached the owner's own receipt logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The player stands where the engine operates the target.
    InReach,
    /// A game message, an open modal or an animation arrived since the
    /// dispatch: the operation ran or was answered.
    Output,
    /// The engine's own `I can't reach that!` arrived.
    Refused,
    /// The target left the snapshot.
    TargetGone,
}

/// One observation of a watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproachVerdict {
    Pending,
    Resolved(Resolution),
    /// `ticks` consecutive observed server ticks without movement or outcome.
    Stalled {
        ticks: u32,
    },
}

/// The facts of one observation. Plain data so the state machine is tested
/// without a snapshot; [`ApproachWatch::observe`] builds it from one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApproachSample {
    /// Observed server ticks ([`server_ticks`]).
    pub tick: u32,
    pub player: WorldTile,
    /// Newest chat sequence; larger than the baseline means a new line.
    pub chat_sequence: i32,
    /// A line newer than the baseline reads `I can't reach that!`.
    pub refused: bool,
    /// The main or chat modal is open.
    pub modal_open: bool,
    /// The local player's primary animation id (`-1` for none).
    pub animation: i32,
    /// `None`: the target left the snapshot. `Some(None)`: reach unknown.
    pub in_reach: Option<Option<bool>>,
}

/// The engine's refusal text (`Player.ts:1300-1304`).
pub const CANT_REACH: &str = "I can't reach that!";

/// Follows one accepted operation that needed an approach.
#[derive(Debug, Clone, Copy)]
pub struct ApproachWatch {
    target: WatchTarget,
    last_tile: WorldTile,
    last_tick: u32,
    chat_baseline: i32,
    modal_baseline: bool,
    animation_baseline: i32,
    still_ticks: u32,
}

impl ApproachWatch {
    /// Arm a watch from the snapshot the operation was dispatched on.
    /// `None` when the target is not a world body or the player is unknown.
    pub fn arm(snapshot: &GameSnapshot, target: &OpTarget<'_>) -> Option<Self> {
        let target = WatchTarget::of(target)?;
        let player = &snapshot.local_player()?.player;
        Some(Self {
            target,
            last_tile: player.network,
            last_tick: server_ticks(snapshot),
            chat_baseline: newest_chat(snapshot),
            modal_baseline: modal_open(snapshot),
            animation_baseline: player.actor.animation,
            still_ticks: 0,
        })
    }

    /// The request's target identity.
    pub fn target(&self) -> WatchTarget {
        self.target
    }

    /// Consecutive still ticks counted so far.
    pub fn still_ticks(&self) -> u32 {
        self.still_ticks
    }

    /// Observe the snapshot. A call without a new server tick changes nothing.
    pub fn observe(&mut self, snapshot: &GameSnapshot) -> ApproachVerdict {
        let Some(player) = snapshot.local_player().map(|p| &p.player) else {
            return ApproachVerdict::Pending;
        };
        let refused = snapshot
            .chat_lines()
            .iter()
            .take_while(|line| line.sequence > self.chat_baseline)
            .any(|line| line.text.trim().eq_ignore_ascii_case(CANT_REACH));
        self.observe_sample(ApproachSample {
            tick: server_ticks(snapshot),
            player: player.network,
            chat_sequence: newest_chat(snapshot),
            refused,
            modal_open: modal_open(snapshot),
            animation: player.actor.animation,
            in_reach: self.target.in_reach(snapshot),
        })
    }

    /// The state machine. A tick counts as stationary only when the host
    /// observed exactly one new server tick since the last sample (one
    /// `PLAYER_INFO`) and the player's tile is unchanged. A coalesced drain
    /// (more than one new tick) is a coverage gap: the intermediate tiles,
    /// modals and animations were never observed, so the streak restarts and
    /// the gap counts for nothing. This follows the tick-health contract
    /// (`coalesced_observations`): elapsed packets are not stationary
    /// observations.
    pub fn observe_sample(&mut self, sample: ApproachSample) -> ApproachVerdict {
        let delta = sample.tick.wrapping_sub(self.last_tick);
        // No new server tick (a poll between packets), or a counter that went
        // backwards (a session change): nothing to count.
        if delta == 0 || delta > u32::MAX / 2 {
            if delta != 0 {
                self.last_tick = sample.tick;
                self.last_tile = sample.player;
                self.still_ticks = 0;
            }
            return ApproachVerdict::Pending;
        }
        self.last_tick = sample.tick;
        if sample.refused {
            return ApproachVerdict::Resolved(Resolution::Refused);
        }
        let Some(in_reach) = sample.in_reach else {
            return ApproachVerdict::Resolved(Resolution::TargetGone);
        };
        let moved = sample.player != self.last_tile;
        let gap = delta != 1;
        if moved || gap {
            // Walking toward the target is the healthy case; a gap proves
            // nothing either way. Either restarts the streak.
            self.last_tile = sample.player;
            self.still_ticks = 0;
        }
        if in_reach == Some(true) {
            return ApproachVerdict::Resolved(Resolution::InReach);
        }
        if !moved
            && (sample.chat_sequence > self.chat_baseline
                || (sample.modal_open && !self.modal_baseline)
                || sample.animation != self.animation_baseline)
        {
            return ApproachVerdict::Resolved(Resolution::Output);
        }
        if moved || gap || sample.modal_open {
            // A modal blocks interaction (`canAccess`, `Player.ts:873-883`):
            // not a stall, and not counted.
            return ApproachVerdict::Pending;
        }
        self.still_ticks += 1;
        if self.still_ticks >= APPROACH_STALL_TICKS {
            ApproachVerdict::Stalled {
                ticks: self.still_ticks,
            }
        } else {
            ApproachVerdict::Pending
        }
    }
}

fn newest_chat(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .chat_lines()
        .first()
        .map_or(0, |line| line.sequence)
}

fn modal_open(snapshot: &GameSnapshot) -> bool {
    let modals = snapshot.modals();
    modals.main != -1 || modals.chat != -1
}

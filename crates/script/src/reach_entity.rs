//! `Reach.entityOp` and the quest `walkWithHops`, ported from frozen
//! `Reach.ts` (`reachThroughDoors`, `clearBlockingDoor`) and
//! `primitives.ts` (`crossHops`, `hopLadder`) as [`crate::machine`]
//! families: `reach-entity-op` and `walk-hops`.
//!
//! JavaScript passes the caller's `expect`, the frozen attempt's
//! `find` → `interact(op)` as one `interact` hook, `find()?.tile()` as
//! `target`, and `log`. Rust owns the eight-round loop, the scene probe,
//! the "I can't reach that" check, door and gate clearing (close a swung
//! leaf, else walk to and open the blocking door), ladder hops and every
//! wait. Game actions are the native loc and walk ops.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, SceneRow};
use crate::shim::InteractReq;
use crate::walk::{Resilient, BAKED_TIMEOUT_MS, HOP_ATTEMPTS};

use api::query::SceneReachOptions;
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;

/// Frozen `REACH_DOOR_ATTEMPTS`.
const DOOR_ATTEMPTS: u32 = 8;
/// Frozen `PROBE_RADIUS`.
const PROBE_RADIUS: i32 = 10;
/// Frozen `REACH_BFS_STEPS`.
const BFS_STEPS: u32 = 400;
/// Frozen `LEAF_CLOSE_RADIUS`.
const LEAF_CLOSE_RADIUS: i32 = 3;
/// Frozen `towardDest` slack.
const TOWARD_SLACK: i32 = 4;
/// Frozen door open / leaf close settle wait.
const DOOR_WAIT_MS: u64 = 5_000;
/// Frozen `openBlockingDoor` walk: `walkResilient(t, { radius: 1,
/// attempts: 3, timeoutMs: 30_000 })` (`Reach.ts:112–114`).
const DOOR_WALK_MS: u64 = 30_000;
const DOOR_WALK_ATTEMPTS: u32 = 3;
/// Frozen `Traversal.walkResilient` default baked timeout (`Traversal.ts:107`).
const WALK_MS: u64 = BAKED_TIMEOUT_MS;

/// Frozen `hopLadder` arrival wait.
const CLIMB_MS: u64 = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
struct Tile {
    x: i32,
    z: i32,
    #[serde(default)]
    level: i32,
}

impl Tile {
    fn world(self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }

    fn cheb(self, other: Tile) -> i32 {
        (self.x - other.x).abs().max((self.z - other.z).abs())
    }

    /// Frozen `Tile.distanceTo`: Chebyshev, far off another level.
    fn distance_to(self, other: Tile) -> i32 {
        let xz = self.cheb(other);
        if self.level != other.level {
            1_000_000 + xz
        } else {
            xz
        }
    }

    /// Frozen `isUnderground`.
    fn underground(self) -> bool {
        self.z >= 5000
    }
}

fn here() -> Option<Tile> {
    observed::with(|scene| {
        scene.since_login().here().map(|t| Tile {
            x: t.x,
            z: t.z,
            level: t.level,
        })
    })
}

fn locs() -> Vec<SceneRow> {
    observed::with(|scene| scene.since_login().locs().cloned().unwrap_or_default())
}

fn loc_tile(loc: &SceneRow) -> Tile {
    Tile {
        x: loc.x,
        z: loc.z,
        level: loc.level,
    }
}

/// The posted ops, without empty and `hidden` slots (frozen `actions()`).
fn ops(loc: &SceneRow) -> impl Iterator<Item = &str> {
    loc.actions
        .iter()
        .map(|op| &**op)
        .filter(|op| !op.is_empty() && *op != "hidden")
}

fn op_starting(loc: &SceneRow, prefix: &str) -> Option<String> {
    ops(loc)
        .find(|op| op.len() >= prefix.len() && op[..prefix.len()].eq_ignore_ascii_case(prefix))
        .map(str::to_string)
}

fn door_named(loc: &SceneRow) -> bool {
    let name = loc.name_or_empty().to_ascii_lowercase();
    name.contains("door") || name.contains("gate")
}

/// Frozen `isOpenableBarrier`.
fn openable_barrier(loc: &SceneRow) -> bool {
    door_named(loc) && op_starting(loc, "open").is_some()
}

/// Frozen `isOpenBarrierLeaf`.
fn open_leaf(loc: &SceneRow) -> bool {
    door_named(loc) && op_starting(loc, "close").is_some()
}

/// The nearest posted loc passing `keep` (frozen `.nearest()`).
fn nearest(keep: impl Fn(&SceneRow) -> bool) -> Option<SceneRow> {
    locs()
        .into_iter()
        .filter(|loc| keep(loc))
        .min_by_key(|loc| loc.distance)
}

/// Frozen `Loc.interact(op)`: the loc op on this exact row.
fn interact(loc: &SceneRow, op: &str, cx: &mut Cx<'_>) -> bool {
    if !ops(loc).any(|have| have.eq_ignore_ascii_case(op)) {
        return false;
    }
    cx.emit(InteractReq::Loc {
        x: loc.x,
        z: loc.z,
        level: loc.level,
        action: op.to_string(),
        id: Some(loc.id),
    });
    true
}

/// Frozen `Reachability.canReach(tile, { maxSteps: 400, adjacentOk })`.
fn can_reach(tile: Tile, adjacent_ok: bool) -> bool {
    let options = SceneReachOptions {
        max_steps: Some(BFS_STEPS),
        adjacent_ok,
    };
    crate::load::reach_query::with_view(|view| view.can_reach(tile.world(), &options))
}

fn can_step(from: Tile, to: Tile) -> bool {
    crate::load::reach_query::with_view(|view| view.can_step(from.world(), to.world()))
}

/// Frozen `doorApproachable`: some tile on or beside the door is reachable.
fn door_approachable(door: Tile) -> bool {
    (-1..=1).any(|dx| {
        (-1..=1).any(|dz| {
            can_reach(
                Tile {
                    x: door.x + dx,
                    z: door.z + dz,
                    level: door.level,
                },
                false,
            )
        })
    })
}

/// Frozen `towardDest`.
fn toward_dest(door: Tile, here: Tile, dest: Tile) -> bool {
    door.cheb(dest) <= here.cheb(dest) + TOWARD_SLACK
}

/// Frozen `clearBlockingDoor(toward)`: close a swung leaf that blocks the
/// last step, else walk to and open the nearest blocking door.
struct Clear {
    toward: Tile,
    phase: ClearPhase,
}

enum ClearPhase {
    Start,
    LeafWait,
    DoorWalk { door: Tile, walk: Resilient },
    DoorOpen { door: Tile },
    DoorWait { door: Tile },
}

impl Clear {
    fn new(toward: Tile) -> Self {
        Self {
            toward,
            phase: ClearPhase::Start,
        }
    }

    /// `Some(cleared)` once done; log lines go to `says`.
    fn step(&mut self, cx: &mut Cx<'_>, says: &mut VecDeque<String>) -> Option<bool> {
        let toward = self.toward;
        loop {
            match std::mem::replace(&mut self.phase, ClearPhase::Start) {
                ClearPhase::Start => {
                    if self.close_leaf(cx, says) {
                        cx.clock().arm(DOOR_WAIT_MS);
                        self.phase = ClearPhase::LeafWait;
                        return None;
                    }
                    if let Some(done) = self.find_door(cx) {
                        return done;
                    }
                }
                ClearPhase::LeafWait => {
                    if can_reach(toward, true) {
                        return Some(true);
                    }
                    if !cx.clock().bound_reached() {
                        self.phase = ClearPhase::LeafWait;
                        return None;
                    }
                    // `closeSwungLeaf(…) || openBlockingDoor(…)`
                    if let Some(done) = self.find_door(cx) {
                        return done;
                    }
                }
                ClearPhase::DoorWalk { door, mut walk } => {
                    let out = walk.step(cx);
                    while let Some(line) = walk.pop_log() {
                        says.push_back(line);
                    }
                    match out {
                        None => {
                            self.phase = ClearPhase::DoorWalk { door, walk };
                            return None;
                        }
                        // Frozen ignores the walk's result and reads the
                        // door where the player stopped (`Reach.ts:112–118`).
                        Some(_) => self.phase = ClearPhase::DoorOpen { door },
                    }
                }
                ClearPhase::DoorOpen { door } => {
                    let Some(shut) = shut_at(door) else {
                        return Some(true);
                    };
                    let Some(op) = op_starting(&shut, "open") else {
                        return Some(false);
                    };
                    says.push_back(format!(
                        "reach: opening blocking '{}' at ({},{})",
                        shut.name_or_empty(),
                        door.x,
                        door.z
                    ));
                    if !interact(&shut, &op, cx) {
                        return Some(false);
                    }
                    cx.clock().arm(DOOR_WAIT_MS);
                    self.phase = ClearPhase::DoorWait { door };
                    return None;
                }
                ClearPhase::DoorWait { door } => {
                    if shut_at(door).is_none() {
                        return Some(true);
                    }
                    if cx.clock().bound_reached() {
                        return Some(false);
                    }
                    self.phase = ClearPhase::DoorWait { door };
                    return None;
                }
            }
        }
    }

    /// The door-approach walk this clear has armed ([`Resilient::release`]).
    fn release(&self) -> Option<InteractReq> {
        match &self.phase {
            ClearPhase::DoorWalk { walk, .. } => walk.release(),
            _ => None,
        }
    }

    /// Frozen `closeSwungLeaf`, up to its close click.
    fn close_leaf(&self, cx: &mut Cx<'_>, says: &mut VecDeque<String>) -> bool {
        let toward = self.toward;
        let Some(here) = here() else {
            return false;
        };
        if here.level != toward.level || can_reach(toward, true) {
            return false;
        }
        let Some(leaf) = nearest(|loc| {
            open_leaf(loc)
                && loc.distance <= LEAF_CLOSE_RADIUS
                && loc_tile(loc).cheb(toward) <= 1
                && !can_step(loc_tile(loc), toward)
        }) else {
            return false;
        };
        let Some(op) = op_starting(&leaf, "close") else {
            return false;
        };
        says.push_back(format!(
            "reach: closing '{}' at ({},{}) to reach ({},{})",
            leaf.name_or_empty(),
            leaf.x,
            leaf.z,
            toward.x,
            toward.z
        ));
        interact(&leaf, &op, cx)
    }

    /// Frozen `openBlockingDoor` up to its walk. `Some` is the step's
    /// answer (no door: `Some(Some(false))`; walking: `Some(None)`); `None`
    /// goes straight on to the open click.
    fn find_door(&mut self, cx: &mut Cx<'_>) -> Option<Option<bool>> {
        let Some(here) = here() else {
            return Some(Some(false));
        };
        let toward = self.toward;
        let Some(door) = nearest(|loc| {
            openable_barrier(loc)
                && loc.distance <= 6
                && toward_dest(loc_tile(loc), here, toward)
                && door_approachable(loc_tile(loc))
        }) else {
            return Some(Some(false));
        };
        let door = loc_tile(&door);
        if here.cheb(door) > 1 {
            // Frozen ignores the walk's result.
            let walk = Resilient::new(
                door.world(),
                1,
                DOOR_WALK_MS,
                Some(DOOR_WALK_ATTEMPTS),
                false,
            );
            if let Ok(walk) = walk.start(cx) {
                self.phase = ClearPhase::DoorWalk { door, walk };
                return Some(None);
            }
        }

        self.phase = ClearPhase::DoorOpen { door };
        None
    }
}

/// The shut barrier on `door`'s x/z (frozen compares x and z only).
fn shut_at(door: Tile) -> Option<SceneRow> {
    nearest(|loc| loc.x == door.x && loc.z == door.z && openable_barrier(loc))
}

/// A `{x, z, level}` a callback returned.
fn tile_of(value: &Value) -> Option<Tile> {
    serde_json::from_value(value.clone()).ok()
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// The last posted chat line's seq (frozen `GameMessages.mark()`).
fn chat_mark() -> i32 {
    observed::with(|scene| {
        scene
            .since_login()
            .chat_lines()
            .and_then(|lines| lines.iter().map(|line| line.seq).max())
            .unwrap_or(0)
    })
}

/// Frozen `sawSince(mark, CANT_REACH)` (`/^i can't reach that/i`).
fn saw_cant_reach(mark: i32) -> bool {
    const PREFIX: &[u8] = b"i can't reach that";
    observed::with(|scene| {
        scene.since_login().chat_lines().is_some_and(|lines| {
            lines.iter().any(|line| {
                let bytes = line.text.as_bytes();
                line.seq > mark
                    && bytes.len() >= PREFIX.len()
                    && bytes[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
            })
        })
    })
}

/// What one advance decided (the same shape as partner-trade's).
enum Next {
    Continue,
    Wait,
    Done(&'static str),
    Call(usize, Vec<Value>),
}

/// Queued `log` lines, written in order before the state goes on.
#[derive(Default)]
struct Logger {
    lines: VecDeque<String>,
    writing: bool,
}

const EXPECT: usize = 0;
const INTERACT: usize = 1;
const TARGET: usize = 2;
const LOG: usize = 3;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EntityOpArgs {
    #[serde(default)]
    expect_ms: Option<u64>,
    #[serde(default)]
    what: String,
    #[serde(default)]
    open_when_unreachable: bool,
}

enum Phase {
    Top,
    ProbeExpect,
    ProbeTarget,
    AttemptExpect,
    AttemptInteract,
    Watch,
    WatchExpect,
    AfterExpect,
    CantReachTarget,
    Clearing { clear: Clear, cant_reach: bool },
    RetryTick,
}

/// One frozen `Reach.entityOp` (`reachThroughDoors` with
/// `retryAfterTimeout` false).
pub(crate) struct EntityOp {
    phase: Phase,
    round: u32,
    mark: i32,
    expect_ms: u64,
    what: String,
    probe: bool,
    logger: Logger,
    after: Option<Next>,
}

impl Family for EntityOp {
    const NAME: &'static str = "reach-entity-op";
    const CALLBACKS: &'static [&'static str] = &["expect", "interact", "target", "log"];
    /// Frozen calls `expect()`, the target closure and `log` without
    /// `await` (a promise from `expect` is truthy); only
    /// `await entity.interact(op)` is awaited.
    const SYNC_HOOKS: &'static [usize] = &[EXPECT, TARGET, LOG];
    /// The first `expect()` and click run in the caller's turn.
    const KICK_ON_START: bool = true;
    type Args = EntityOpArgs;
    /// `done`, `retry` or `unreachable`.
    type Output = &'static str;

    fn begin(args: EntityOpArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        Begin::Run(Self {
            phase: Phase::Top,
            round: 0,
            mark: 0,
            expect_ms: args.expect_ms.unwrap_or(5_000),
            what: args.what,
            probe: args.open_when_unreachable,
            logger: Logger::default(),
            after: None,
        })
    }

    /// A superseded reach stops the door approach it armed.
    fn release(&self) -> Option<InteractReq> {
        match &self.phase {
            Phase::Clearing { clear, .. } => clear.release(),
            _ => None,
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<&'static str> {
        let mut reply = match cx.reply() {
            Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
            Some(Reply::Value(value)) => Some(value),
            None => None,
        };
        if std::mem::take(&mut self.logger.writing) {
            reply = None;
        }
        loop {
            if let Some(line) = self.logger.lines.pop_front() {
                if !cx.has(LOG) {
                    continue;
                }
                self.logger.writing = true;
                return Step::Call(Call {
                    hook: LOG,
                    args: vec![json!(line)],
                });
            }
            let next = match self.after.take() {
                Some(next) => next,
                None => self.advance(reply.take(), cx),
            };
            if !self.logger.lines.is_empty() && !matches!(next, Next::Continue) {
                self.after = Some(next);
                continue;
            }
            match next {
                Next::Continue => {}
                Next::Wait => return Step::Wait,
                Next::Done(status) => return Step::Done(status),
                Next::Call(hook, args) => return Step::Call(Call { hook, args }),
            }
        }
    }
}

impl EntityOp {
    fn call(&mut self, phase: Phase, hook: usize) -> Next {
        self.phase = phase;
        Next::Call(hook, Vec::new())
    }

    fn advance(&mut self, reply: Option<Value>, cx: &mut Cx<'_>) -> Next {
        let value = reply.unwrap_or(Value::Null);
        match std::mem::replace(&mut self.phase, Phase::Top) {
            Phase::Top => {
                if self.round >= DOOR_ATTEMPTS {
                    return Next::Done("retry");
                }
                if self.probe {
                    return self.call(Phase::ProbeExpect, EXPECT);
                }
                self.attempt()
            }
            Phase::ProbeExpect => {
                if truthy(&value) {
                    return self.attempt();
                }
                self.call(Phase::ProbeTarget, TARGET)
            }
            Phase::ProbeTarget => {
                let blocked = tile_of(&value).filter(|blocked| {
                    here().is_some_and(|here| {
                        blocked.level == here.level
                            && here.cheb(*blocked) <= PROBE_RADIUS
                            && !can_reach(*blocked, true)
                    })
                });
                match blocked {
                    Some(blocked) => {
                        self.phase = Phase::Clearing {
                            clear: Clear::new(blocked),
                            cant_reach: false,
                        };
                        Next::Continue
                    }
                    None => self.attempt(),
                }
            }
            Phase::AttemptExpect => {
                if truthy(&value) {
                    return self.dispatched(cx);
                }
                self.call(Phase::AttemptInteract, INTERACT)
            }
            Phase::AttemptInteract => {
                if truthy(&value) {
                    return self.dispatched(cx);
                }
                // Not dispatched: one tick, then the caller decides.
                self.phase = Phase::RetryTick;
                Next::Wait
            }
            Phase::Watch => self.call(Phase::WatchExpect, EXPECT),
            Phase::WatchExpect => {
                if truthy(&value) || saw_cant_reach(self.mark) || cx.clock().bound_reached() {
                    return self.call(Phase::AfterExpect, EXPECT);
                }
                self.phase = Phase::Watch;
                Next::Wait
            }
            Phase::AfterExpect => {
                if truthy(&value) {
                    return Next::Done("done");
                }
                if saw_cant_reach(self.mark) {
                    return self.call(Phase::CantReachTarget, TARGET);
                }
                Next::Done("retry")
            }
            Phase::CantReachTarget => match tile_of(&value) {
                Some(toward) => {
                    self.phase = Phase::Clearing {
                        clear: Clear::new(toward),
                        cant_reach: true,
                    };
                    Next::Continue
                }
                None => self.unreachable(None),
            },
            Phase::Clearing {
                mut clear,
                cant_reach,
            } => match clear.step(cx, &mut self.logger.lines) {
                None => {
                    self.phase = Phase::Clearing { clear, cant_reach };
                    Next::Wait
                }
                Some(true) => {
                    self.round += 1;
                    Next::Continue
                }
                Some(false) if cant_reach => self.unreachable(Some(clear.toward)),
                Some(false) => self.attempt(),
            },
            Phase::RetryTick => Next::Done("retry"),
        }
    }

    fn attempt(&mut self) -> Next {
        self.mark = chat_mark();
        self.call(Phase::AttemptExpect, EXPECT)
    }

    fn dispatched(&mut self, cx: &mut Cx<'_>) -> Next {
        cx.clock().arm(self.expect_ms);
        self.phase = Phase::Watch;
        Next::Wait
    }

    fn unreachable(&mut self, toward: Option<Tile>) -> Next {
        self.logger
            .lines
            .push_back(unreachable_line(&self.what, toward));
        Next::Done("unreachable")
    }
}

/// Frozen `reachThroughDoors`' unreachable log (`Reach.ts:195`).
fn unreachable_line(what: &str, toward: Option<Tile>) -> String {
    let (x, z) = toward.map_or(("undefined".into(), "undefined".into()), |t| {
        (t.x.to_string(), t.z.to_string())
    });
    format!(
        "reach: '{what}' at ({x},{z}): server can't reach it and no door in front to open or close (unreachable)"
    )
}

/// What an [`NpcReach`] waits for after its Talk click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TalkExpect {
    /// Frozen `ChatDialog.isOpen() || ChatDialog.canContinue()`
    /// (`primitives.ts:228`, `Reach.ts:295`).
    #[cfg_attr(not(test), allow(dead_code))] // boat fare's openDialogue
    DialogReady,
    /// A dialogue the click produced: ready, and the chat modal differs from
    /// the one posted at the click, or opened from none, or Continue
    /// appeared. Armed at every click (reach-npc-dialog's `fresh_ready`).
    FreshDialog,
}

/// Frozen `reachThroughDoors` parameters (`Reach.ts:156–166`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct NpcReachOpts {
    pub(crate) expect: TalkExpect,
    pub(crate) expect_ms: u64,
    /// Frozen `retryAfterTimeout`: `Reach.entityOp` passes false,
    /// `Reach.npcDialog` true.
    pub(crate) retry_after_timeout: bool,
    /// Frozen `probeUnreachable` (`openWhenUnreachable`).
    pub(crate) probe_unreachable: bool,
    /// Frozen `Reach.entityOp`'s attempt answers dispatched without a click
    /// when `expect()` already holds (`Reach.ts:218–220`); `npcDialog`'s
    /// attempt always clicks (`Reach.ts:289–292`).
    pub(crate) skip_click_when_expected: bool,
}

enum NpcReachPhase {
    Top,
    Watch,
    Clearing {
        clear: Clear,
        cant_reach: bool,
    },
    /// Frozen `delayTicks(1)` before the next round (`retryAfterTimeout`).
    NextRound,
    /// Frozen `delayTicks(1)` after an undispatched attempt, then `retry`.
    RetryTick,
}

/// One NPC's Talk op through frozen `reachThroughDoors` (`Reach.ts:156–211`):
/// up to [`DOOR_ATTEMPTS`] rounds of the optional scene probe (a target
/// within [`PROBE_RADIUS`] the scene cannot reach clears a blocking door
/// first), the click on the nearest `npc` row with a talk op, the expect /
/// "I can't reach that" wait, and the door clear after a can't-reach.
/// Embeddable: `step` once per tick until it answers `done`, `retry` or
/// `unreachable`; log lines queue for [`NpcReach::pop_log`].
pub(crate) struct NpcReach {
    npc: String,
    opts: NpcReachOpts,
    phase: NpcReachPhase,
    round: u32,
    mark: i32,
    armed_modal: i32,
    armed_continue: bool,
    logs: VecDeque<String>,
}

/// The nearest posted `npc` row with a talk op (frozen
/// `Npcs.query().name(n).where(talkOp !== null).nearest()`).
struct TalkRow {
    name: String,
    action: String,
    index: i32,
    tile: Tile,
}

fn talk_row(npc: &str) -> Option<TalkRow> {
    let want = npc.trim();
    if want.is_empty() {
        return None;
    }
    observed::with(|scene| {
        let rows = scene.since_login().npcs()?;
        rows.iter()
            .filter(|row| {
                row.name
                    .as_deref()
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case(want))
            })
            .filter_map(|row| crate::dialog::talk_op(&row.actions).map(|op| (row, op)))
            .min_by_key(|(row, _)| row.distance)
            .map(|(row, op)| TalkRow {
                name: row.name.as_deref().unwrap_or_default().to_string(),
                action: op.to_string(),
                index: row.index,
                tile: Tile {
                    x: row.x,
                    z: row.z,
                    level: row.level,
                },
            })
    })
}

/// The posted chat modal id and Continue flag.
fn chat_state() -> (i32, bool) {
    observed::with(|scene| {
        let session = scene.since_login();
        (
            session.chat_modal_id().unwrap_or(-1),
            session.chat_continue().unwrap_or(false),
        )
    })
}

impl NpcReach {
    pub(crate) fn new(npc: &str, opts: NpcReachOpts) -> Self {
        Self {
            npc: npc.to_string(),
            opts,
            phase: NpcReachPhase::Top,
            round: 0,
            mark: 0,
            armed_modal: -1,
            armed_continue: false,
            logs: VecDeque::new(),
        }
    }

    pub(crate) fn pop_log(&mut self) -> Option<String> {
        self.logs.pop_front()
    }

    fn expect(&self) -> bool {
        let (modal, cont) = chat_state();
        let ready = modal != -1 || cont;
        match self.opts.expect {
            TalkExpect::DialogReady => ready,
            TalkExpect::FreshDialog => {
                ready
                    && (modal != self.armed_modal
                        || (modal != -1 && self.armed_modal == -1)
                        || (cont && !self.armed_continue))
            }
        }
    }

    /// `Some(status)` once done: `done`, `retry` or `unreachable`.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Option<&'static str> {
        loop {
            match std::mem::replace(&mut self.phase, NpcReachPhase::Top) {
                NpcReachPhase::Top => {
                    if self.round >= DOOR_ATTEMPTS {
                        return Some("retry");
                    }
                    if self.opts.probe_unreachable && !self.expect() {
                        let blocked = talk_row(&self.npc).map(|row| row.tile).filter(|blocked| {
                            here().is_some_and(|here| {
                                blocked.level == here.level
                                    && here.cheb(*blocked) <= PROBE_RADIUS
                                    && !can_reach(*blocked, true)
                            })
                        });
                        if let Some(blocked) = blocked {
                            self.phase = NpcReachPhase::Clearing {
                                clear: Clear::new(blocked),
                                cant_reach: false,
                            };
                            continue;
                        }
                    }
                    return self.attempt(cx);
                }
                NpcReachPhase::Clearing {
                    mut clear,
                    cant_reach,
                } => match clear.step(cx, &mut self.logs) {
                    None => {
                        self.phase = NpcReachPhase::Clearing { clear, cant_reach };
                        return None;
                    }
                    Some(true) => self.round += 1,
                    Some(false) if cant_reach => {
                        self.logs
                            .push_back(unreachable_line(&self.npc, Some(clear.toward)));
                        return Some("unreachable");
                    }
                    Some(false) => return self.attempt(cx),
                },
                NpcReachPhase::Watch => {
                    let cant_reach = saw_cant_reach(self.mark);
                    if !(self.expect() || cant_reach || cx.clock().bound_reached()) {
                        self.phase = NpcReachPhase::Watch;
                        return None;
                    }
                    if self.expect() {
                        return Some("done");
                    }
                    if cant_reach {
                        let Some(toward) = talk_row(&self.npc).map(|row| row.tile) else {
                            self.logs.push_back(unreachable_line(&self.npc, None));
                            return Some("unreachable");
                        };
                        self.phase = NpcReachPhase::Clearing {
                            clear: Clear::new(toward),
                            cant_reach: true,
                        };
                        continue;
                    }
                    return self.after_round(true);
                }
                NpcReachPhase::NextRound => self.round += 1,
                NpcReachPhase::RetryTick => return Some("retry"),
            }
        }
    }

    /// Frozen `mark` + `attempt()` + the dispatched wait's arm.
    fn attempt(&mut self, cx: &mut Cx<'_>) -> Option<&'static str> {
        self.mark = chat_mark();
        if self.opts.skip_click_when_expected && self.expect() {
            return Some("done");
        }
        let Some(row) = talk_row(&self.npc) else {
            return self.after_round(false);
        };
        (self.armed_modal, self.armed_continue) = chat_state();
        cx.emit(InteractReq::Npc {
            name: row.name,
            action: row.action,
            index: Some(row.index),
        });
        cx.clock().arm(self.opts.expect_ms);
        self.phase = NpcReachPhase::Watch;
        None
    }

    /// The end of a round that neither answered nor hit a can't-reach.
    fn after_round(&mut self, dispatched: bool) -> Option<&'static str> {
        if !self.opts.retry_after_timeout {
            if dispatched {
                return Some("retry");
            }
            self.phase = NpcReachPhase::RetryTick;
            return None;
        }
        self.phase = NpcReachPhase::NextRound;
        None
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hop {
    stand: Tile,
    loc_name: String,
    op: String,
    arrive: Tile,
    #[serde(default)]
    open: Option<String>,
    #[serde(default)]
    walk: Option<Tile>,
}

#[derive(Deserialize)]
pub(crate) struct WalkHopsArgs {
    dest: Tile,
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    hops: Vec<Hop>,
}

enum HopPhase {
    Start,
    HopWalk {
        hop: usize,
        walk: Resilient,
    },
    Opened {
        hop: usize,
        ticks: u32,
        retried: bool,
    },
    Climb {
        hop: usize,
    },
    FinalWalk {
        walk: Resilient,
    },
}

/// One frozen quest `walkWithHops(dest, radius, hops, log)`.
pub(crate) struct WalkHops {
    dest: Tile,
    radius: i32,
    hops: Vec<Hop>,
    phase: HopPhase,
    logger: Logger,
    result: Option<bool>,
    /// This tick's advance already ended waiting (a log was written after).
    waiting: bool,
    pumped: bool,
}

impl Family for WalkHops {
    const NAME: &'static str = "walk-hops";
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `log(...)` is not awaited; `await Sustain.run()` is.
    const SYNC_HOOKS: &'static [usize] = &[LOG_HOP];
    /// The first walk or ladder click goes out in the caller's turn.
    const KICK_ON_START: bool = true;
    type Args = WalkHopsArgs;
    type Output = bool;

    fn begin(args: WalkHopsArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        Begin::Run(Self {
            dest: args.dest,
            radius: args.radius,
            hops: args.hops,
            phase: HopPhase::Start,
            logger: Logger::default(),
            result: None,
            waiting: false,
            pumped: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.logger.lines.pop_front() {
                if cx.has(LOG_HOP) {
                    return Step::Call(Call {
                        hook: LOG_HOP,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if let Some(result) = self.result {
                return Step::Done(result);
            }
            if std::mem::take(&mut self.waiting) {
                self.pumped = false;
                return Step::Wait;
            }
            if !self.pumped && cx.has(SUSTAIN_HOP) {
                self.pumped = true;
                return Step::Call(Call {
                    hook: SUSTAIN_HOP,
                    args: Vec::new(),
                });
            }
            match self.advance(cx) {
                Some(result) => self.result = Some(result),
                None if self.logger.lines.is_empty() => {
                    self.pumped = false;
                    return Step::Wait;
                }
                None => self.waiting = true,
            }
        }
    }
}

const LOG_HOP: usize = 0;
const SUSTAIN_HOP: usize = 1;

impl WalkHops {
    /// `Some(result)` once the walk ended; `None` waits (or writes a log).
    fn advance(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        match std::mem::replace(&mut self.phase, HopPhase::Start) {
            HopPhase::Start => match here() {
                Some(here) => self.cross(here, cx),
                None => Some(false),
            },
            HopPhase::HopWalk { hop, mut walk } => {
                let out = walk.step(cx);
                while let Some(line) = walk.pop_log() {
                    self.logger.lines.push_back(line);
                }
                match out {
                    None => {
                        self.phase = HopPhase::HopWalk { hop, walk };
                        None
                    }
                    Some(false) => Some(false),
                    Some(true) => self.ladder(hop, cx),
                }
            }

            HopPhase::Opened {
                hop,
                ticks,
                retried,
            } => {
                if ticks > 1 {
                    self.phase = HopPhase::Opened {
                        hop,
                        ticks: ticks - 1,
                        retried,
                    };
                    return None;
                }
                if let Some(ladder) = self.find(hop, &self.hops[hop].op) {
                    return self.climb(hop, &ladder, cx);
                }
                if !retried {
                    self.phase = HopPhase::Opened {
                        hop,
                        ticks: 2,
                        retried: true,
                    };
                    return None;
                }
                self.no_ladder(hop)
            }
            HopPhase::Climb { hop } => {
                let arrive = self.hops[hop].arrive;
                let arrived =
                    here().filter(|t| t.level == arrive.level && arrive.distance_to(*t) <= 5);
                if let Some(here) = arrived {
                    // `return Game.tile()`, then the final leg.
                    return self.finish(here, cx);
                }
                if cx.clock().bound_reached() {
                    return Some(false);
                }
                self.phase = HopPhase::Climb { hop };
                None
            }
            HopPhase::FinalWalk { mut walk } => {
                let out = walk.step(cx);
                while let Some(line) = walk.pop_log() {
                    self.logger.lines.push_back(line);
                }
                match out {
                    None => {
                        self.phase = HopPhase::FinalWalk { walk };
                        None
                    }
                    done => done,
                }
            }
        }
    }

    /// Frozen `crossHops`.
    fn cross(&mut self, here: Tile, cx: &mut Cx<'_>) -> Option<bool> {
        if here.underground() == self.dest.underground() {
            return self.finish(here, cx);
        }
        let hop = self
            .hops
            .iter()
            .enumerate()
            .filter(|(_, hop)| hop.stand.underground() == here.underground())
            .min_by_key(|(_, hop)| hop.stand.distance_to(here))
            .map(|(i, _)| i);
        let Some(hop) = hop else {
            self.logger.lines.push_back(format!(
                "no hop from ({},{}) toward z {} — trying the baked graph",
                here.x, here.z, self.dest.z
            ));
            return self.finish(here, cx);
        };
        let stand = self.hops[hop].stand;
        if stand.distance_to(here) > 2 {
            let to = self.hops[hop].walk.unwrap_or(stand);
            match Resilient::new(to.world(), 2, WALK_MS, Some(HOP_ATTEMPTS), false).start(cx) {
                Ok(walk) => {
                    self.phase = HopPhase::HopWalk { hop, walk };
                    return None;
                }
                Err(false) => return Some(false),
                Err(true) => {}
            }
        }
        self.ladder(hop, cx)
    }

    /// `Locs.query().name(locName).action(op)` within 3 of the stand.
    fn find(&self, hop: usize, op: &str) -> Option<SceneRow> {
        let hop = &self.hops[hop];
        let name = hop.loc_name.trim().to_lowercase();
        nearest(|loc| {
            loc.name_or_empty().trim().to_lowercase() == name
                && ops(loc).any(|have| have.eq_ignore_ascii_case(op))
                && loc_tile(loc).distance_to(hop.stand) <= 3
        })
    }

    /// Frozen `hopLadder` up to its climb click.
    fn ladder(&mut self, hop: usize, cx: &mut Cx<'_>) -> Option<bool> {
        let op = self.hops[hop].op.clone();
        if let Some(ladder) = self.find(hop, &op) {
            return self.climb(hop, &ladder, cx);
        }
        if let Some(open) = self.hops[hop].open.clone() {
            if let Some(closed) = self.find(hop, &open) {
                if interact(&closed, &open, cx) {
                    self.phase = HopPhase::Opened {
                        hop,
                        ticks: 2,
                        retried: false,
                    };
                    return None;
                }
            }
        }
        self.no_ladder(hop)
    }

    fn no_ladder(&mut self, hop: usize) -> Option<bool> {
        let hop = &self.hops[hop];
        self.logger.lines.push_back(format!(
            "no '{}' offering '{}' near ({},{})",
            hop.loc_name, hop.op, hop.stand.x, hop.stand.z
        ));
        Some(false)
    }

    fn climb(&mut self, hop: usize, ladder: &SceneRow, cx: &mut Cx<'_>) -> Option<bool> {
        let op = self.hops[hop].op.clone();
        if !interact(ladder, &op, cx) {
            return Some(false);
        }
        cx.clock().arm(CLIMB_MS);
        self.phase = HopPhase::Climb { hop };
        None
    }

    /// Frozen `walkWithHops` after the hop: walk unless within `radius`.
    fn finish(&mut self, here: Tile, cx: &mut Cx<'_>) -> Option<bool> {
        if here.level == self.dest.level && self.dest.distance_to(here) <= self.radius {
            return Some(true);
        }
        match Resilient::new(
            self.dest.world(),
            self.radius,
            WALK_MS,
            Some(HOP_ATTEMPTS),
            false,
        )
        .start(cx)
        {
            Ok(walk) => {
                self.phase = HopPhase::FinalWalk { walk };
                None
            }
            Err(arrived) => Some(arrived),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Pending, Started, Take};
    use crate::walk_wait;

    /// No script callbacks are held here.
    struct NoJs;

    impl machine::Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no hook is held");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no hook is held");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    #[test]
    fn a_timed_out_walk_stops_the_host_follow_and_fails() {
        observed::on_reset();
        machine::on_reset();
        walk_wait::on_reset();
        observed::post(1, |post| {
            post.session(true).here(observed::Tile {
                x: 0,
                z: 0,
                level: 0,
            });
        });
        let args = json!({ "dest": { "x": 10, "z": 0, "level": 0 }, "radius": 0, "hops": [] });
        let Started::Running(h) = machine::start("walk-hops", args, Vec::new(), 0) else {
            panic!("walk-hops runs");
        };
        machine::step(&mut NoJs);
        let walk_token = match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x: 10,
                z: 0,
                request_id,
                ..
            }] => *request_id,
            other => panic!("expected the walk, got {other:?}"),
        };
        assert_ne!(walk_token, 0);
        machine::step(&mut NoJs);
        assert!(machine::merge_ops(Vec::new()).is_empty(), "still walking");
        // A script walk starts while the machine waits: it takes the wait
        // slot and the host's armed walk, under its own token.
        let script_token = walk_wait::dispatch(&json!({
            "op": "begin", "x": 3, "z": 3, "level": 0, "radius": 0,
        }))
        .as_u64()
        .unwrap();
        assert_ne!(script_token, walk_token);
        machine::tests::expire_deadlines();
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::AbortWalk {
                request_id: walk_token
            }],
            "a timed-out wait stops only its own follow (the host keeps the \
             script's walk, whose token differs)"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    fn loc(name: &str, actions: &[&str]) -> SceneRow {
        SceneRow {
            name: Some(name.into()),
            actions: actions.iter().map(|op| (*op).into()).collect(),
            ..SceneRow::default()
        }
    }

    #[test]
    fn barrier_predicates_are_the_frozen_door_crossing_rules() {
        assert!(openable_barrier(&loc("Gate", &["Open", "Examine"])));
        assert!(openable_barrier(&loc("Large door", &["open"])));
        assert!(!openable_barrier(&loc("Gate", &["Close"])));
        assert!(!openable_barrier(&loc("Ladder", &["Open"])));
        assert!(open_leaf(&loc("Door", &["Close"])));
        assert!(!open_leaf(&loc("Door", &["hidden", "Open"])));
    }

    #[test]
    fn toward_dest_allows_the_frozen_slack() {
        let t = |x| Tile { x, z: 0, level: 0 };
        assert!(toward_dest(t(10), t(0), t(10)));
        assert!(toward_dest(t(4), t(0), t(0)), "within 4 of here's distance");
        assert!(!toward_dest(t(5), t(0), t(0)));
    }

    #[test]
    fn distance_to_and_underground_are_frozen_tile_rules() {
        let a = Tile {
            x: 0,
            z: 4999,
            level: 0,
        };
        let b = Tile {
            x: 3,
            z: 5000,
            level: 1,
        };
        assert!(!a.underground());
        assert!(b.underground());
        assert_eq!(
            a.distance_to(b),
            1_000_003,
            "Chebyshev 3 plus the level penalty"
        );
    }

    fn reach_opts(
        expect: TalkExpect,
        retry_after_timeout: bool,
        skip_click_when_expected: bool,
    ) -> NpcReachOpts {
        NpcReachOpts {
            expect,
            expect_ms: 8_000,
            retry_after_timeout,
            probe_unreachable: true,
            skip_click_when_expected,
        }
    }

    /// A fresh session at (0,0) with Luthas (Talk-to) one tile away, chat as
    /// given. No reach view: the probe finds no door to clear and clicks.
    fn post_talk(tick: u64, modal: i32, cont: bool, lines: &[(i32, &str)]) {
        observed::post(tick, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                })
                .chat_modal_id(modal)
                .chat_continue(cont)
                .chat_lines(
                    lines
                        .iter()
                        .map(|(seq, text)| observed::ChatLine {
                            seq: *seq,
                            text: (*text).into(),
                        })
                        .collect(),
                )
                .npcs(vec![observed::EntityRow {
                    index: 7,
                    name: Some("Luthas".into()),
                    x: 1,
                    z: 0,
                    level: 0,
                    distance: 1,
                    actions: vec!["Talk-to".into(), "Examine".into()].into(),
                    ..observed::EntityRow::default()
                }]);
        });
    }

    fn fresh() {
        observed::on_reset();
        crate::load::reach_query::on_reset();
    }

    struct Driver {
        reach: NpcReach,
        clock: crate::task_clock::InstantTaskClock,
    }

    impl Driver {
        fn new(opts: NpcReachOpts) -> Self {
            Self {
                reach: NpcReach::new("Luthas", opts),
                clock: crate::task_clock::InstantTaskClock::new(),
            }
        }

        fn step(&mut self) -> (Option<&'static str>, Vec<InteractReq>) {
            let mut ops = Vec::new();
            let out = {
                let mut cx = Cx::test(&mut ops, &mut self.clock, None);
                self.reach.step(&mut cx)
            };
            (out, ops)
        }

        fn expire(&mut self) {
            self.clock.deadline =
                Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
        }
    }

    fn talk_click() -> InteractReq {
        InteractReq::Npc {
            name: "Luthas".into(),
            action: "Talk-to".into(),
            index: Some(7),
        }
    }

    #[test]
    fn entity_op_reach_clicks_once_and_is_done_when_the_dialogue_opens() {
        fresh();
        post_talk(1, -1, false, &[]);
        let mut d = Driver::new(reach_opts(TalkExpect::DialogReady, false, true));
        assert_eq!(d.step(), (None, vec![talk_click()]));
        assert_eq!(d.step(), (None, vec![]), "no second click while waiting");
        post_talk(2, 4882, true, &[]);
        assert_eq!(d.step(), (Some("done"), vec![]));
    }

    #[test]
    fn entity_op_reach_is_retry_after_one_timed_out_click() {
        fresh();
        post_talk(1, -1, false, &[]);
        let mut d = Driver::new(reach_opts(TalkExpect::DialogReady, false, true));
        assert_eq!(d.step(), (None, vec![talk_click()]));
        d.expire();
        assert_eq!(
            d.step(),
            (Some("retry"), vec![]),
            "retryAfterTimeout false: the caller decides after one click"
        );
    }

    #[test]
    fn npc_dialog_reach_reclicks_each_tick_after_a_timeout_for_eight_rounds() {
        fresh();
        post_talk(1, -1, false, &[]);
        let mut d = Driver::new(reach_opts(TalkExpect::FreshDialog, true, false));
        for round in 0..DOOR_ATTEMPTS {
            assert_eq!(d.step(), (None, vec![talk_click()]), "round {round} clicks");
            d.expire();
            assert_eq!(
                d.step(),
                (None, vec![]),
                "frozen delayTicks(1) after a timeout"
            );
        }
        assert_eq!(d.step(), (Some("retry"), vec![]));
    }

    #[test]
    fn fresh_dialog_expect_ignores_the_dialogue_already_up_at_the_click() {
        fresh();
        post_talk(1, 100, false, &[]);
        let mut d = Driver::new(reach_opts(TalkExpect::FreshDialog, true, false));
        assert_eq!(
            d.step(),
            (None, vec![talk_click()]),
            "npcDialog's attempt clicks even with a dialogue up"
        );
        assert_eq!(d.step(), (None, vec![]), "the armed modal is not fresh");
        post_talk(2, 200, false, &[]);
        assert_eq!(d.step(), (Some("done"), vec![]));

        fresh();
        post_talk(1, 100, false, &[]);
        let mut ready = Driver::new(reach_opts(TalkExpect::DialogReady, false, true));
        assert_eq!(
            ready.step(),
            (Some("done"), vec![]),
            "entityOp answers an expect that already holds without a click"
        );
    }

    #[test]
    fn cant_reach_with_no_door_to_clear_is_unreachable_with_the_frozen_line() {
        fresh();
        post_talk(1, -1, false, &[(3, "Welcome")]);
        let mut d = Driver::new(reach_opts(TalkExpect::DialogReady, true, false));
        assert_eq!(d.step(), (None, vec![talk_click()]));
        post_talk(2, -1, false, &[(3, "Welcome"), (4, "I can't reach that!")]);
        assert_eq!(d.step(), (Some("unreachable"), vec![]));
        assert_eq!(
            d.reach.pop_log().as_deref(),
            Some(
                "reach: 'Luthas' at (1,0): server can't reach it and no door in front to open or close (unreachable)"
            )
        );
    }

    #[test]
    fn an_absent_npc_waits_one_tick_then_retries_without_a_click() {
        fresh();
        observed::post(1, |post| {
            post.session(true).here(observed::Tile {
                x: 0,
                z: 0,
                level: 0,
            });
        });
        let mut d = Driver::new(reach_opts(TalkExpect::DialogReady, false, true));
        assert_eq!(d.step(), (None, vec![]));
        assert_eq!(d.step(), (Some("retry"), vec![]));
    }
}

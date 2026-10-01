//! Native reach/door loop extracted from isolate `reach_entity` / `reach`.
//! Policy constants match those modules; sequencing observes `SnapshotView`.

use crate::native::{ActionContext, ActionError, NativeMachine, WalkRequest};
use crate::shim::InteractReq;
use api::quest_progress::EvidenceStamp;
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

pub const DOOR_ATTEMPTS: u32 = 8;
pub const PROBE_RADIUS: i32 = 10;
pub const LEAF_CLOSE_RADIUS: i32 = 3;
pub const DOOR_WAIT_MS: u64 = 5_000;
pub const CANT_REACH: &str = "i can't reach that";

#[derive(Clone)]
pub struct ReachArgs {
    pub kind: ReachKind,
    pub op: Arc<str>,
    pub anchor: Option<WorldTile>,
    pub radius: i32,
    pub wait_if_missing: bool,
}

#[derive(Clone)]
pub enum ReachKind {
    Npc {
        id: i32,
        name: Arc<str>,
    },
    Loc {
        id: Option<i32>,
        name: Option<Arc<str>>,
    },
    Ground {
        id: i32,
        obj: Arc<str>,
    },
    Name {
        name: Arc<str>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Seek,
    Click,
    WaitDoor,
}

pub struct Reach {
    args: ReachArgs,
    phase: Phase,
    attempts: u32,
    chat_mark: i32,
    deadline_ms: u64,
    before_count: i32,
    clicked_loc: Option<(i32, WorldTile)>,
}

impl NativeMachine for Reach {
    type Args = ReachArgs;
    type Output = bool;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let mut reach = Self {
            args,
            phase: Phase::Seek,
            attempts: 0,
            chat_mark: last_chat_seq(cx),
            deadline_ms: cx.active_now().as_millis() as u64 + DOOR_WAIT_MS,
            before_count: 0,
            clicked_loc: None,
        };
        reach.click(cx)?;
        Ok(reach)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.attempts > DOOR_ATTEMPTS {
            return Poll::Ready(Ok(false));
        }
        match self.phase {
            Phase::Seek => {
                if self.click(cx)? {
                    return Poll::Pending;
                }
                if self.args.wait_if_missing || !matches!(self.args.kind, ReachKind::Ground { .. })
                {
                    Poll::Pending
                } else {
                    Poll::Ready(Ok(false))
                }
            }
            Phase::Click => {
                if saw_cant_reach(cx, self.chat_mark) {
                    if self.clear_door(cx).is_ok() {
                        self.phase = Phase::WaitDoor;
                        self.deadline_ms = cx.active_now().as_millis() as u64 + DOOR_WAIT_MS;
                        return Poll::Pending;
                    }
                    return Poll::Ready(Ok(false));
                }
                if let Some((id, tile)) = self.clicked_loc {
                    let Some(locs) = cx.snapshot().locs() else {
                        return Poll::Pending;
                    };
                    if !locs
                        .value
                        .iter()
                        .any(|loc| loc.id == id && loc.tile == tile)
                    {
                        return Poll::Ready(Ok(false));
                    }
                }
                if let ReachKind::Ground { id, .. } = self.args.kind {
                    return if held_count(cx, id).is_some_and(|count| count > self.before_count) {
                        Poll::Ready(Ok(true))
                    } else {
                        Poll::Pending
                    };
                }
                Poll::Ready(Ok(true))
            }
            Phase::WaitDoor => {
                if cx.active_now().as_millis() as u64 >= self.deadline_ms {
                    self.chat_mark = last_chat_seq(cx);
                    if self.click(cx).is_err() {
                        return Poll::Ready(Ok(false));
                    }
                    // A scene rebuild may hide the target again; click retains Seek.
                }
                Poll::Pending
            }
        }
    }

    fn cancel(&mut self) {}
}

impl Reach {
    fn click(&mut self, cx: &mut ActionContext<'_>) -> Result<bool, ActionError> {
        let request = match &self.args.kind {
            ReachKind::Npc { id, name } => {
                let Some(npc) = nearest_npc(cx, *id, &self.args.op, self.args.radius) else {
                    self.phase = Phase::Seek;
                    return Ok(false);
                };
                InteractReq::Npc {
                    name: name.to_string(),
                    action: self.args.op.to_string(),
                    index: Some(npc.index as i32),
                }
            }
            ReachKind::Loc { id, name } => {
                let Some(loc) =
                    nearest_loc(cx, *id, name.as_deref(), Some(&self.args.op), PROBE_RADIUS)
                else {
                    self.phase = Phase::Seek;
                    return Ok(false);
                };
                InteractReq::Loc {
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    action: self.args.op.to_string(),
                    id: Some(loc.id),
                }
            }
            ReachKind::Ground { id, obj } => {
                let Some(item) = nearest_ground(cx, *id) else {
                    self.phase = Phase::Seek;
                    return Ok(false);
                };
                let tile = item.tile;
                let Some(before) = held_count(cx, *id) else {
                    return Ok(false);
                };
                self.before_count = before;
                InteractReq::Obj {
                    x: tile.x,
                    z: tile.z,
                    level: tile.level,
                    name: Some(obj.to_string()),
                    action: self.args.op.to_string(),
                }
            }
            ReachKind::Name { name } => {
                let Some(loc) =
                    nearest_loc(cx, None, Some(name), Some(&self.args.op), self.args.radius)
                else {
                    self.phase = Phase::Seek;
                    return Ok(false);
                };
                InteractReq::Loc {
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    action: self.args.op.to_string(),
                    id: Some(loc.id),
                }
            }
        };
        let clicked_loc = match &request {
            InteractReq::Loc {
                id: Some(id),
                x,
                z,
                level,
                ..
            } => Some((
                *id,
                WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                },
            )),
            _ => None,
        };
        cx.emit(request)?;
        self.clicked_loc = clicked_loc;
        self.phase = Phase::Click;
        self.attempts += 1;
        self.chat_mark = last_chat_seq(cx);
        Ok(true)
    }

    fn clear_door(&self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let here = cx.snapshot().here().map(|obs| obs.value);
        let Some(locs) = cx.snapshot().locs() else {
            return Err(ActionError::Failed(Arc::from("no locs")));
        };
        let door =
            locs.value
                .iter()
                .filter(|loc| {
                    loc.actions.iter().flatten().any(|op| {
                        op.eq_ignore_ascii_case("open") || op.eq_ignore_ascii_case("close")
                    }) && loc.name.as_deref().is_some_and(|name| {
                        name.to_ascii_lowercase().contains("door")
                            || name.to_ascii_lowercase().contains("gate")
                    })
                })
                .min_by_key(|loc| loc.distance);
        let Some(door) = door else {
            return Err(ActionError::Failed(Arc::from("no door")));
        };
        let op = door
            .actions
            .iter()
            .flatten()
            .find(|op| {
                let lower = op.to_ascii_lowercase();
                lower.starts_with("open")
                    || (here.is_some_and(|tile| chebyshev(tile, door.tile) <= LEAF_CLOSE_RADIUS)
                        && lower.starts_with("close"))
            })
            .cloned()
            .unwrap_or_else(|| "Open".into());
        cx.emit(InteractReq::Loc {
            x: door.tile.x,
            z: door.tile.z,
            level: door.tile.level,
            action: op,
            id: Some(door.id),
        })?;
        Ok(())
    }
}

pub fn target_available(cx: &ActionContext<'_>, kind: &ReachKind, op: &str, radius: i32) -> bool {
    match kind {
        ReachKind::Ground { id, .. } => nearest_ground(cx, *id).is_some(),
        ReachKind::Loc { id, name } => {
            nearest_loc(cx, *id, name.as_deref(), Some(op), PROBE_RADIUS).is_some()
        }
        ReachKind::Name { name } => nearest_loc(cx, None, Some(name), Some(op), radius).is_some(),
        ReachKind::Npc { id, .. } => nearest_npc(cx, *id, op, radius).is_some(),
    }
}

pub fn nearest_npc<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
    op: &str,
    radius: i32,
) -> Option<&'a api::snapshot::NpcView> {
    cx.snapshot()
        .npcs()?
        .value
        .iter()
        .filter(|npc| {
            npc.r#type == Some(id as usize)
                && npc.distance <= radius
                && npc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case(op))
        })
        .min_by_key(|npc| npc.distance)
}

fn nearest_ground<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
) -> Option<&'a api::snapshot::GroundItemView> {
    cx.snapshot()
        .ground_items()?
        .value
        .iter()
        .filter(|item| item.def.id == id && item.distance <= 12)
        .min_by_key(|item| item.distance)
}

fn held_count(cx: &ActionContext<'_>, id: i32) -> Option<i32> {
    Some(
        cx.snapshot()
            .inventory()?
            .value
            .iter()
            .filter(|item| item.def.id == id)
            .map(|item| item.count)
            .sum(),
    )
}

pub fn nearest_loc<'a>(
    cx: &'a ActionContext<'_>,
    id: Option<i32>,
    name: Option<&str>,
    op: Option<&str>,
    radius: i32,
) -> Option<&'a api::snapshot::LocView> {
    cx.snapshot()
        .locs()?
        .value
        .iter()
        .filter(|loc| {
            id.map_or_else(
                || {
                    name.is_some_and(|want| {
                        loc.name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(want))
                    })
                },
                |id| loc.id == id,
            ) && op.is_none_or(|op| {
                loc.actions
                    .iter()
                    .flatten()
                    .any(|a| a.eq_ignore_ascii_case(op))
            }) && (radius <= 0 || loc.distance <= radius)
        })
        .min_by_key(|loc| loc.distance)
}

pub fn last_chat_seq(cx: &ActionContext<'_>) -> i32 {
    cx.snapshot()
        .chat_lines(0)
        .and_then(|lines| lines.value.iter().map(|line| line.sequence).max())
        .unwrap_or(0)
}

fn saw_cant_reach(cx: &ActionContext<'_>, since: i32) -> bool {
    saw_game_message(cx, since, CANT_REACH)
}

pub(super) fn saw_game_message(cx: &ActionContext<'_>, since: i32, message: &str) -> bool {
    cx.snapshot().chat_lines(since).is_some_and(|lines| {
        lines.value.iter().any(|line| {
            line.username.is_none()
                && line.type_ == 0
                && line
                    .text
                    .get(..message.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(message))
        })
    })
}

fn chebyshev(a: WorldTile, b: WorldTile) -> i32 {
    if a.level != b.level {
        return i32::MAX;
    }
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

pub fn walk_request(tile: WorldTile, radius: u16, required_after: EvidenceStamp) -> WalkRequest {
    WalkRequest {
        target: tile,
        radius,
        options: crate::FindOptions::default(),
        required_after,
        evidence: None,
    }
}

pub fn within(here: WorldTile, dest: WorldTile, radius: i32) -> bool {
    chebyshev(here, dest) <= radius
}

pub fn duration_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

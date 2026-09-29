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
        name: Arc<str>,
        index: Option<i32>,
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
    seen_target: bool,
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
            seen_target: false,
        };
        reach.click(cx)?;
        Ok(reach)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.attempts > DOOR_ATTEMPTS {
            return Poll::Ready(Ok(false));
        }
        match self.phase {
            Phase::Seek | Phase::Click => {
                if saw_cant_reach(cx, self.chat_mark) {
                    if self.clear_door(cx).is_ok() {
                        self.phase = Phase::WaitDoor;
                        self.deadline_ms = cx.active_now().as_millis() as u64 + DOOR_WAIT_MS;
                        self.attempts += 1;
                        return Poll::Pending;
                    }
                    return Poll::Ready(Ok(false));
                }
                if matches!(&self.args.kind, ReachKind::Ground { .. }) {
                    if self.target_gone(cx) {
                        if self.seen_target {
                            return Poll::Ready(Ok(true));
                        }
                        if self.args.wait_if_missing {
                            return Poll::Pending;
                        }
                        return Poll::Ready(Ok(false));
                    }
                    self.seen_target = true;
                    return Poll::Pending;
                }
                if self.target_gone(cx) {
                    if self.args.wait_if_missing {
                        return Poll::Pending;
                    }
                    return Poll::Ready(Ok(false));
                }
                Poll::Ready(Ok(true))
            }
            Phase::WaitDoor => {
                if cx.active_now().as_millis() as u64 >= self.deadline_ms {
                    self.chat_mark = last_chat_seq(cx);
                    if self.click(cx).is_err() {
                        return Poll::Ready(Ok(false));
                    }
                    self.phase = Phase::Click;
                }
                Poll::Pending
            }
        }
    }

    fn cancel(&mut self) {}
}

impl Reach {
    fn click(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        self.phase = Phase::Click;
        self.attempts += 1;
        match &self.args.kind {
            ReachKind::Npc { name, index } => cx.emit(InteractReq::Npc {
                name: name.to_string(),
                action: self.args.op.to_string(),
                index: *index,
            })?,
            ReachKind::Loc { id, name: _ } => {
                let tile = self
                    .args
                    .anchor
                    .ok_or_else(|| ActionError::Failed(Arc::from("reach loc missing anchor")))?;
                cx.emit(InteractReq::Loc {
                    x: tile.x,
                    z: tile.z,
                    level: tile.level,
                    action: self.args.op.to_string(),
                    id: *id,
                })?
            }
            ReachKind::Ground { obj, .. } => {
                let tile = self
                    .args
                    .anchor
                    .ok_or_else(|| ActionError::Failed(Arc::from("reach ground missing anchor")))?;
                cx.emit(InteractReq::Obj {
                    x: tile.x,
                    z: tile.z,
                    level: tile.level,
                    name: Some(obj.to_string()),
                    action: self.args.op.to_string(),
                })?
            }
            ReachKind::Name { name } => {
                if let Some(loc) = nearest_named_loc(cx, name, &self.args.op, self.args.radius) {
                    cx.emit(InteractReq::Loc {
                        x: loc.x,
                        z: loc.z,
                        level: loc.level,
                        action: self.args.op.to_string(),
                        id: Some(loc.id),
                    })?
                } else if self.args.wait_if_missing {
                    self.phase = Phase::Seek;
                    return Ok(());
                } else {
                    return Err(ActionError::Failed(Arc::from("named loc missing")));
                }
            }
        };
        Ok(())
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

    fn target_gone(&self, cx: &ActionContext<'_>) -> bool {
        match &self.args.kind {
            ReachKind::Name { name } => {
                nearest_named_loc(cx, name, &self.args.op, self.args.radius).is_none()
            }
            ReachKind::Npc { name, .. } => cx.snapshot().npcs().is_some_and(|npcs| {
                !npcs.value.iter().any(|npc| {
                    npc.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
            }),
            ReachKind::Ground { id, .. } => {
                let Some(items) = cx.snapshot().ground_items() else {
                    return false;
                };
                !items.value.iter().any(|item| {
                    item.def.id == *id
                        && self.args.anchor.is_none_or(|tile| {
                            chebyshev(item.tile, tile) <= self.args.radius.max(2)
                        })
                })
            }
            _ => false,
        }
    }
}

struct NamedLoc {
    x: i32,
    z: i32,
    level: i32,
    id: i32,
}

fn nearest_named_loc(
    cx: &ActionContext<'_>,
    name: &str,
    op: &str,
    radius: i32,
) -> Option<NamedLoc> {
    let locs = cx.snapshot().locs()?.value;
    let want = name.trim();
    locs.iter()
        .filter(|loc| {
            loc.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(want))
                && loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|a| a.eq_ignore_ascii_case(op))
                && (radius <= 0 || loc.distance <= radius)
        })
        .min_by_key(|loc| loc.distance)
        .map(|loc| NamedLoc {
            x: loc.tile.x,
            z: loc.tile.z,
            level: loc.tile.level,
            id: loc.id,
        })
}

fn last_chat_seq(cx: &ActionContext<'_>) -> i32 {
    cx.snapshot()
        .chat_lines(0)
        .and_then(|lines| lines.value.iter().map(|line| line.sequence).max())
        .unwrap_or(0)
}

fn saw_cant_reach(cx: &ActionContext<'_>, since: i32) -> bool {
    cx.snapshot().chat_lines(since).is_some_and(|lines| {
        lines
            .value
            .iter()
            .any(|line| line.text.to_ascii_lowercase().starts_with(CANT_REACH))
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

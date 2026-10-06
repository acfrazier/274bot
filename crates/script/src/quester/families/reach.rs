//! Native reach/door loop extracted from isolate `reach_entity` / `reach`.
//! Closed barriers open once; already-open passages use the shared native walk.

use crate::native::{walk::Walk, ActionContext, ActionError, NativeMachine, WalkRequest};
use crate::shim::InteractReq;
use api::quest_progress::EvidenceStamp;
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

pub const DOOR_ATTEMPTS: u32 = 8;
pub const PROBE_RADIUS: i32 = 10;
pub const DOOR_WAIT_MS: u64 = 5_000;
pub const CANT_REACH: &str = "i can't reach that";

fn door_wall_reachable(
    here: WorldTile,
    door: &api::snapshot::LocView,
    reach: Option<&api::query::ReachQueryView>,
) -> bool {
    let (Ok(shape), Ok(angle)) = (u8::try_from(door.shape), u8::try_from(door.angle)) else {
        return false;
    };
    api::query::straight_wall_reachable(here, door.tile, shape, angle, |from, to| {
        reach.is_some_and(|view| view.can_step(from, to))
    })
}

#[derive(Clone)]
pub struct ReachArgs {
    pub kind: ReachKind,
    pub op: Arc<str>,
    pub anchor: Option<WorldTile>,
    pub radius: i32,
    pub wait_if_missing: bool,
    /// Select this exact loc tile instead of the nearest same-definition loc.
    pub target_tile: Option<WorldTile>,
    /// Require a known reachable loc candidate for inventory-count repetition.
    pub reachable_only: bool,
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
    Held {
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
    WaitWalk { door: Option<(i32, WorldTile)> },
}

pub struct Reach {
    args: ReachArgs,
    phase: Phase,
    attempts: u32,
    chat_mark: i32,
    deadline_ms: u64,
    before_count: i32,
    clicked_loc: Option<(i32, WorldTile)>,
    walk: Option<Walk>,
    request_id: u64,
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
            walk: None,
            request_id: 0,
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
                if matches!(self.args.kind, ReachKind::Held { .. }) {
                    return match cx.interaction_receipt(self.request_id) {
                        Some(receipt) if receipt.accepted => Poll::Ready(Ok(true)),
                        Some(_) => Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "held operation dispatch rejected",
                        )))),
                        None => Poll::Pending,
                    };
                }
                if saw_cant_reach(cx, self.chat_mark) {
                    if self.clear_door(cx).is_ok() {
                        return Poll::Pending;
                    }
                    return Poll::Ready(Ok(false));
                }
                if !matches!(self.args.kind, ReachKind::Ground { .. }) {
                    match cx.interaction_receipt(self.request_id) {
                        Some(receipt) if receipt.accepted => {}
                        Some(_) => return Poll::Ready(Ok(false)),
                        None => return Poll::Pending,
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
                if cx.active_now().as_millis() as u64 >= self.deadline_ms
                    && self.walk_to_target(cx).is_err()
                {
                    return Poll::Ready(Ok(false));
                }
                Poll::Pending
            }
            Phase::WaitWalk { door } => {
                match self.walk.as_mut().expect("door recovery walk").poll(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Ok(receipt)) => {
                        let arrival = receipt.into_arrival();
                        self.walk = None;
                        if let Err(error) = arrival {
                            return Poll::Ready(Err(error));
                        }
                        if let Some((id, tile)) = door {
                            if self.open_door(id, tile, cx).is_err() {
                                return Poll::Ready(Ok(false));
                            }
                        } else {
                            self.click(cx)?;
                        }
                        Poll::Pending
                    }
                    Poll::Ready(Err(error)) => {
                        self.walk = None;
                        Poll::Ready(Err(error))
                    }
                }
            }
        }
    }

    fn cancel(&mut self) {
        if let Some(walk) = self.walk.as_mut() {
            walk.cancel();
        }
    }
}

impl Reach {
    pub(crate) fn interaction_request_id(&self) -> Option<u64> {
        (self.request_id != 0).then_some(self.request_id)
    }
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
                let Some(loc) = nearest_loc(
                    cx,
                    *id,
                    name.as_deref(),
                    Some(&self.args.op),
                    self.args.anchor,
                    PROBE_RADIUS,
                    self.args.target_tile,
                    self.args.reachable_only,
                ) else {
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
                let Some(item) = nearest_ground(cx, *id, None, 12) else {
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
            ReachKind::Held { id, obj } => {
                let Some(inventory) = cx.snapshot().inventory() else {
                    self.phase = Phase::Seek;
                    return Ok(false);
                };
                let Some(item) = inventory
                    .value
                    .iter()
                    .find(|item| item.def.id == *id && item.count > 0)
                else {
                    self.phase = Phase::Seek;
                    return Ok(false);
                };
                if !item
                    .actions
                    .iter()
                    .flatten()
                    .any(|op| op.eq_ignore_ascii_case(&self.args.op))
                {
                    return Err(ActionError::Unavailable(Arc::from(
                        "held item does not offer the authored operation",
                    )));
                }
                InteractReq::Held {
                    name: obj.to_string(),
                    action: self.args.op.to_string(),
                    slot: Some(item.slot),
                    target_item_id: Some(item.def.id),
                }
            }
            ReachKind::Name { name } => {
                let Some(loc) = nearest_loc(
                    cx,
                    None,
                    Some(name),
                    Some(&self.args.op),
                    self.args.anchor,
                    self.args.radius,
                    self.args.target_tile,
                    self.args.reachable_only,
                ) else {
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
        self.request_id = cx.emit(request)?;
        self.clicked_loc = clicked_loc;
        self.phase = Phase::Click;
        self.attempts += 1;
        self.chat_mark = last_chat_seq(cx);
        Ok(true)
    }

    fn clear_door(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let Some(locs) = cx.snapshot().locs() else {
            return Err(ActionError::Failed(Arc::from("no locs")));
        };
        let door = locs
            .value
            .iter()
            .filter(|loc| {
                (loc.actions
                    .iter()
                    .flatten()
                    .any(|op| op.eq_ignore_ascii_case("open"))
                    || api::query::door_is_open(loc.actions.iter().flatten().map(String::as_str)))
                    && loc.name.as_deref().is_some_and(|name| {
                        name.to_ascii_lowercase().contains("door")
                            || name.to_ascii_lowercase().contains("gate")
                    })
            })
            .min_by_key(|loc| loc.distance);
        let Some(door) = door else {
            return Err(ActionError::Failed(Arc::from("no door")));
        };
        if api::query::door_is_open(door.actions.iter().flatten().map(String::as_str)) {
            return self.walk_to_target(cx);
        }
        let snapshot = cx.snapshot();
        if let Some(here) = snapshot.here() {
            let reach = snapshot.reach().map(|observed| observed.value);
            if door_wall_reachable(here.value, door, reach) {
                return self.open_door(door.id, door.tile, cx);
            }
        }
        self.walk_to(door.tile, None, Some((door.id, door.tile)), cx)
    }

    fn open_door(
        &mut self,
        id: i32,
        tile: WorldTile,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let snapshot = cx.snapshot();
        let locs = snapshot
            .locs()
            .ok_or_else(|| ActionError::Failed(Arc::from("no locs")))?;
        let Some(door) = locs
            .value
            .iter()
            .find(|loc| loc.id == id && loc.tile == tile)
        else {
            return self.walk_to_target(cx);
        };
        if api::query::door_is_open(door.actions.iter().flatten().map(String::as_str)) {
            return self.walk_to_target(cx);
        }
        let here = snapshot
            .here()
            .ok_or_else(|| ActionError::Failed(Arc::from("no player tile")))?;
        let reach = snapshot.reach().map(|observed| observed.value);
        let unavailable = api::query::ReachQueryView::unavailable();
        if !api::query::is_arrived(here.value, tile, 1, || reach.unwrap_or(&unavailable))
            && !door_wall_reachable(here.value, door, reach)
        {
            return Err(ActionError::Failed(Arc::from(
                "door approach not reachable",
            )));
        }
        let op = door
            .actions
            .iter()
            .flatten()
            .find(|op| op.eq_ignore_ascii_case("open"))
            .ok_or_else(|| ActionError::Failed(Arc::from("door has no Open operation")))?
            .clone();
        cx.emit(InteractReq::Loc {
            x: tile.x,
            z: tile.z,
            level: tile.level,
            action: op,
            id: Some(id),
        })?;
        self.phase = Phase::WaitDoor;
        self.deadline_ms = cx.active_now().as_millis() as u64 + DOOR_WAIT_MS;
        Ok(())
    }

    fn walk_to_target(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let clicked = self.clicked_loc;
        let target = clicked
            .map(|(_, tile)| tile)
            .or_else(|| match &self.args.kind {
                ReachKind::Npc { id, .. } => {
                    nearest_npc(cx, *id, &self.args.op, self.args.radius).map(|npc| npc.tile)
                }
                ReachKind::Ground { id, .. } => {
                    nearest_ground(cx, *id, None, 12).map(|item| item.tile)
                }
                _ => self.args.anchor,
            });
        let target = target.ok_or_else(|| ActionError::Failed(Arc::from("no reach target")))?;
        // A clicked loc only gets footprint intent when the shared approach
        // model recognizes it. Doors and other non-footprint locs retain the
        // tile-anchor fallback instead of waiting for a footprint that cannot exist.
        let loc_id = clicked.and_then(|(id, tile)| {
            let locs = cx.snapshot().locs()?;
            let loc = locs
                .value
                .iter()
                .find(|loc| loc.id == id && loc.tile == tile)?;
            api::query::loc_approach::distance_from(loc, tile).map(|_| id)
        });
        self.walk_to(target, loc_id, None, cx)
    }

    fn walk_to(
        &mut self,
        target: WorldTile,
        loc_id: Option<i32>,
        door: Option<(i32, WorldTile)>,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        self.walk = Some(Walk::begin(
            walk_request(target, 1, loc_id, cx.evidence()),
            cx,
        )?);
        self.phase = Phase::WaitWalk { door };
        Ok(())
    }
}

pub fn target_available(
    cx: &ActionContext<'_>,
    kind: &ReachKind,
    op: &str,
    radius: i32,
    anchor: Option<WorldTile>,
    target_tile: Option<WorldTile>,
    reachable_only: bool,
) -> bool {
    match kind {
        ReachKind::Ground { id, .. } => nearest_ground(cx, *id, None, 12).is_some(),
        ReachKind::Held { id, .. } => cx.snapshot().inventory().is_some_and(|inventory| {
            inventory.value.iter().any(|item| {
                item.def.id == *id
                    && item.count > 0
                    && item
                        .actions
                        .iter()
                        .flatten()
                        .any(|action| action.eq_ignore_ascii_case(op))
            })
        }),
        ReachKind::Loc { id, name } => nearest_loc(
            cx,
            *id,
            name.as_deref(),
            Some(op),
            anchor,
            PROBE_RADIUS,
            target_tile,
            reachable_only,
        )
        .is_some(),
        ReachKind::Name { name } => nearest_loc(
            cx,
            None,
            Some(name),
            Some(op),
            anchor,
            radius,
            target_tile,
            reachable_only,
        )
        .is_some(),
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

pub(super) fn nearest_ground<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
    tile: Option<WorldTile>,
    radius: i32,
) -> Option<&'a api::snapshot::GroundItemView> {
    cx.snapshot()
        .ground_items()?
        .value
        .iter()
        .filter(|item| {
            item.def.id == id
                && item.distance <= radius
                && tile.is_none_or(|tile| item.tile == tile)
        })
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

/// An authored anchor owns the search origin; live player distance is only
/// used when no anchor was supplied. Exact target tiles remain pinned.
#[allow(clippy::too_many_arguments)]
pub fn nearest_loc<'a>(
    cx: &'a ActionContext<'_>,
    id: Option<i32>,
    name: Option<&str>,
    op: Option<&str>,
    anchor: Option<WorldTile>,
    radius: i32,
    target_tile: Option<WorldTile>,
    reachable_only: bool,
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
            }) && (radius <= 0
                || anchor.map_or(loc.distance, |anchor| chebyshev(anchor, loc.tile)) <= radius)
                && target_tile.is_none_or(|tile| loc.tile == tile)
                && (!reachable_only
                    || cx.snapshot().reach().is_some_and(|reach| {
                        reach.value.can_reach(
                            loc.tile,
                            &api::query::SceneReachOptions {
                                max_steps: None,
                                adjacent_ok: true,
                            },
                        )
                    }))
        })
        .min_by_key(|loc| anchor.map_or(loc.distance, |anchor| chebyshev(anchor, loc.tile)))
}

/// The same live stand predicate used by native walks, including footprint
/// approach masks and straight-wall operations that do not have a footprint.
pub fn loc_arrived(cx: &ActionContext<'_>, loc: &api::snapshot::LocView) -> bool {
    let snapshot = cx.snapshot();
    snapshot.here().is_some_and(|here| {
        if loc_walk_id(loc).is_some() {
            snapshot.walk_loc_arrived(here.value, loc.tile, 1, loc.id)
        } else {
            (loc.layer == api::snapshot::LocLayer::Wall
                && door_wall_reachable(here.value, loc, snapshot.reach().map(|reach| reach.value)))
                || snapshot.walk_arrived(here.value, loc.tile, 1)
        }
    })
}

pub fn loc_walk_id(loc: &api::snapshot::LocView) -> Option<i32> {
    api::query::loc_approach::distance_from(loc, loc.tile).map(|_| loc.id)
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

/// Build a walk request; `None` preserves tile-anchor arrival, while
/// `Some(loc_id)` explicitly opts into that loc's footprint arrival rule.
pub fn walk_request(
    tile: WorldTile,
    radius: u16,
    loc_id: Option<i32>,
    required_after: EvidenceStamp,
) -> WalkRequest {
    WalkRequest {
        target: tile,
        loc_id,
        radius,
        arrival: nav::arrival::ArrivalKind::Reach,
        options: crate::native::WalkOptions::default(),
        required_after,
        evidence: None,
        cross: Vec::new().into_boxed_slice(),
        protect: false,
        allow: Default::default(),
    }
}

pub fn within(here: WorldTile, dest: WorldTile, radius: i32) -> bool {
    chebyshev(here, dest) <= radius
}

pub fn duration_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

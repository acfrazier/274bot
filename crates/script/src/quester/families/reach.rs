//! Native reach/door loop extracted from isolate `reach_entity` / `reach`.
//! Closed barriers open once; already-open passages use the shared native walk.

use crate::native::{
    defer_budget, walk::Walk, ActionContext, ActionError, NativeMachine, WalkRequest,
};
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

/// A door or gate on `tile` that still offers Open. An unobserved scene
/// counts as shut.
fn door_shut_at(cx: &ActionContext<'_>, tile: WorldTile) -> bool {
    cx.snapshot().locs().is_none_or(|locs| {
        locs.value.iter().any(|loc| {
            loc.tile == tile
                && loc.name.as_deref().is_some_and(|name| {
                    let name = name.to_ascii_lowercase();
                    name.contains("door") || name.contains("gate")
                })
                && !api::query::door_is_open(loc.actions.iter().flatten().map(String::as_str))
                && loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|op| op.eq_ignore_ascii_case("open"))
        })
    })
}

#[derive(Clone)]
pub struct ReachArgs {
    pub kind: ReachKind,
    pub op: Arc<str>,
    /// Owns the chooser's area (see [`Area`]); `None` searches around the player.
    pub anchor: Option<WorldTile>,
    pub radius: i32,
    pub wait_if_missing: bool,
    /// Select this exact loc tile instead of any matching loc in the area.
    pub target_tile: Option<WorldTile>,
    /// Only click a candidate the live reach flood proves reachable.
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
    /// The Open click on this door tile went out; walk once it is no longer
    /// shut, or at the `DOOR_WAIT_MS` timeout.
    WaitDoor {
        door: WorldTile,
    },
    OpenDoor {
        id: i32,
        tile: WorldTile,
    },
    WaitWalk {
        door: Option<(i32, WorldTile)>,
    },
}

pub struct Reach {
    args: ReachArgs,
    phase: Phase,
    attempts: u32,
    chat_mark: i32,
    deadline_ms: u64,
    before_count: i32,
    clicked_loc: Option<(i32, WorldTile)>,
    /// The fungible target last clicked; avoided after "I can't reach that!".
    clicked: Option<AvoidKey>,
    avoid: Avoid,
    walk: Option<Walk>,
    request_id: u64,
    /// The last emitted target click was within interaction range on the
    /// snapshot that chose it, before the server could transform the loc or
    /// teleport the player.
    in_range_at_click: bool,
}

/// What the chooser asks the reach loop to do next.
enum Choice {
    /// Request, avoid key, held count before, target within interaction range.
    Click(InteractReq, Option<AvoidKey>, i32, bool),
    /// No NPC or ground match in the area is reachable from here: nav-walk
    /// to the best in-area match instead of a raw click.
    Approach(WorldTile),
    Missing,
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
            clicked: None,
            avoid: Avoid::default(),
            walk: None,
            request_id: 0,
            in_range_at_click: false,
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
                // The entry frame can still carry the prior scene's locs.
                if cx.entered_scene() {
                    return Poll::Pending;
                }
                if std::task::ready!(defer_budget(self.click(cx)))? {
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
                    // Preserve the response until locs from the entered scene
                    // are observed before choosing a retry or door transition.
                    if cx.entered_scene() {
                        return Poll::Pending;
                    }
                    // A fungible target that answered "I can't reach that!" is
                    // a failed attempt: click another reachable match at once
                    // and avoid the bumped one briefly. With no alternative,
                    // the door fallback still serves the same target.
                    if let Some(key) = self.clicked.filter(|_| self.args.target_tile.is_none()) {
                        let previous = self.avoid;
                        self.avoid.add(key, cx.evidence().tick);
                        let choice = self.choice(cx, self.args.reachable_only, false)?;
                        if matches!(choice, Choice::Click(..)) {
                            let result = self.act(choice, cx);
                            if matches!(result, Err(ActionError::BudgetExhausted)) {
                                self.avoid = previous;
                            }
                            std::task::ready!(defer_budget(result))?;
                            return Poll::Pending;
                        }
                        self.avoid = previous;
                    }
                    return match std::task::ready!(defer_budget(self.clear_door(cx))) {
                        Ok(()) => Poll::Pending,
                        Err(_) => Poll::Ready(Ok(false)),
                    };
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
            Phase::WaitDoor { door } => {
                // Do not infer that a door changed from prior-scene locs.
                if cx.entered_scene() {
                    return Poll::Pending;
                }
                // The engine's `open_door` deletes the shut door and adds the
                // open leaf (moving its collision) in the op's own execution
                // (`content/scripts/doors/scripts/doors.rs2:6-20`): once the
                // clicked tile no longer shows a shut door, walk. 5 s is only
                // the timeout (TICK-FIX #8, C-REACH-WAITDOOR).
                let through = !door_shut_at(cx, door)
                    || cx.active_now().as_millis() as u64 >= self.deadline_ms;
                if through && std::task::ready!(defer_budget(self.walk_to_target(cx))).is_err() {
                    return Poll::Ready(Ok(false));
                }
                Poll::Pending
            }
            Phase::OpenDoor { id, tile } => {
                if cx.entered_scene() {
                    return Poll::Pending;
                }
                if std::task::ready!(defer_budget(self.open_door(id, tile, cx))).is_err() {
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
                        self.phase = match door {
                            Some((id, tile)) => Phase::OpenDoor { id, tile },
                            None => Phase::Seek,
                        };
                        self.poll(cx)
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
    /// Whether the last emitted target click was within interaction range
    /// when it was chosen. Read before the poll that accepts the click.
    pub(crate) fn in_range_at_click(&self) -> bool {
        self.in_range_at_click
    }
    fn click(&mut self, cx: &mut ActionContext<'_>) -> Result<bool, ActionError> {
        let choice = self.choice(cx, self.args.reachable_only, true)?;
        self.act(choice, cx)
    }

    /// Pick the next target with the shared chooser. A reachable match (or,
    /// unless `strict`, an unverified one) is clicked. With `approach`, an
    /// unreachable best NPC or ground item is nav-walked to instead of
    /// clicked; an unreachable loc keeps the click and its door recovery
    /// (anchored quest locs already walk to their stand before this).
    fn choice(
        &self,
        cx: &ActionContext<'_>,
        strict: bool,
        approach: bool,
    ) -> Result<Choice, ActionError> {
        /// `Some(true)` clicks, `Some(false)` approaches.
        fn decide<T>(pick: Pick<T>, strict: bool, approach: bool) -> Option<(T, bool)> {
            match pick {
                Pick::Reachable(value) => Some((value, true)),
                Pick::Unverified(value) if !strict => Some((value, true)),
                Pick::Unreachable(value) if !strict && approach => Some((value, false)),
                _ => None,
            }
        }
        let avoid = self.avoid.at(cx.evidence().tick);
        let op = self.args.op.as_ref();
        let loc_choice = |pick: Pick<&api::snapshot::LocView>| match decide(pick, strict, approach)
        {
            Some((loc, _)) => Choice::Click(
                InteractReq::Loc {
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    action: op.to_string(),
                    id: Some(loc.id),
                },
                Some(AvoidKey::Tile(loc.tile)),
                0,
                loc.distance <= 1,
            ),
            None => Choice::Missing,
        };
        Ok(match &self.args.kind {
            ReachKind::Npc { id, name } => {
                let pick = choose_npc(
                    cx,
                    *id,
                    Some(op),
                    Area::new(self.args.anchor, self.args.radius),
                    &avoid,
                );
                match decide(pick, strict, approach) {
                    Some((npc, true)) => Choice::Click(
                        InteractReq::Npc {
                            name: name.to_string(),
                            action: op.to_string(),
                            index: Some(npc.index as i32),
                        },
                        Some(AvoidKey::Npc(npc.index)),
                        0,
                        npc_adjacent(cx, npc),
                    ),
                    Some((npc, false)) => Choice::Approach(npc.tile),
                    None => Choice::Missing,
                }
            }
            ReachKind::Loc { id, name } => loc_choice(choose_loc(
                cx,
                *id,
                name.as_deref(),
                Some(op),
                Area::new(self.args.anchor, PROBE_RADIUS),
                self.args.target_tile,
                &avoid,
            )),
            ReachKind::Name { name } => loc_choice(choose_loc(
                cx,
                None,
                Some(name),
                Some(op),
                Area::new(self.args.anchor, self.args.radius),
                self.args.target_tile,
                &avoid,
            )),
            ReachKind::Ground { id, obj } => {
                let pick = choose_ground(
                    cx,
                    *id,
                    None,
                    ground_area(self.args.anchor, self.args.radius),
                    &avoid,
                );
                match decide(pick, strict, approach) {
                    Some((item, true)) => {
                        let Some(before) = held_count(cx, *id) else {
                            return Ok(Choice::Missing);
                        };
                        Choice::Click(
                            InteractReq::Obj {
                                x: item.tile.x,
                                z: item.tile.z,
                                level: item.tile.level,
                                name: Some(obj.to_string()),
                                action: op.to_string(),
                            },
                            Some(AvoidKey::Tile(item.tile)),
                            before,
                            false,
                        )
                    }
                    Some((item, false)) => Choice::Approach(item.tile),
                    None => Choice::Missing,
                }
            }
            ReachKind::Held { id, obj } => {
                let Some(inventory) = cx.snapshot().inventory() else {
                    return Ok(Choice::Missing);
                };
                let Some(item) = inventory
                    .value
                    .iter()
                    .find(|item| item.def.id == *id && item.count > 0)
                else {
                    return Ok(Choice::Missing);
                };
                if !item
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case(op))
                {
                    return Err(ActionError::Unavailable(Arc::from(
                        "held item does not offer the authored operation",
                    )));
                }
                Choice::Click(
                    InteractReq::Held {
                        name: obj.to_string(),
                        action: op.to_string(),
                        slot: Some(item.slot),
                        target_item_id: Some(item.def.id),
                    },
                    None,
                    0,
                    false,
                )
            }
        })
    }

    fn act(&mut self, choice: Choice, cx: &mut ActionContext<'_>) -> Result<bool, ActionError> {
        let (request, key, before, in_range) = match choice {
            Choice::Click(request, key, before, in_range) => (request, key, before, in_range),
            Choice::Approach(tile) => {
                self.walk_to(tile, None, None, cx)?;
                // Count admitted approaches, not same-tick budget deferrals.
                self.attempts += 1;
                self.clicked_loc = None;
                self.clicked = None;
                return Ok(true);
            }
            Choice::Missing => {
                self.phase = Phase::Seek;
                return Ok(false);
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
        self.before_count = before;
        self.in_range_at_click = in_range;
        self.clicked_loc = clicked_loc;
        self.clicked = key;
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
        if let Some(here) = snapshot
            .local_player()
            .map(|player| player.value.player.network)
        {
            let reach = snapshot.reach().map(|observed| observed.value);
            if door_wall_reachable(here, door, reach) {
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
            .local_player()
            .map(|player| player.value.player.network)
            .ok_or_else(|| ActionError::Failed(Arc::from("no player tile")))?;
        let reach = snapshot.reach().map(|observed| observed.value);
        let unavailable = api::query::ReachQueryView::unavailable();
        if !api::query::is_arrived(here, tile, 1, || reach.unwrap_or(&unavailable))
            && !door_wall_reachable(here, door, reach)
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
        self.phase = Phase::WaitDoor { door: tile };
        self.deadline_ms = cx.active_now().as_millis() as u64 + DOOR_WAIT_MS;
        Ok(())
    }

    fn walk_to_target(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let clicked = self.clicked_loc;
        let target = clicked.map(|(_, tile)| tile).or_else(|| {
            let avoid = self.avoid.at(cx.evidence().tick);
            match &self.args.kind {
                ReachKind::Npc { id, .. } => choose_npc(
                    cx,
                    *id,
                    Some(&self.args.op),
                    Area::new(self.args.anchor, self.args.radius),
                    &avoid,
                )
                .any()
                .map(|npc| npc.tile),
                ReachKind::Ground { id, .. } => choose_ground(
                    cx,
                    *id,
                    None,
                    ground_area(self.args.anchor, self.args.radius),
                    &avoid,
                )
                .any()
                .map(|item| item.tile),
                _ => self.args.anchor,
            }
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

/// Whether a scene target has a reachable candidate in its area. Held items
/// are available while the pack offers the authored operation.
pub fn target_available(
    cx: &ActionContext<'_>,
    kind: &ReachKind,
    op: &str,
    radius: i32,
    anchor: Option<WorldTile>,
    target_tile: Option<WorldTile>,
) -> bool {
    let none = Avoid::default();
    match kind {
        ReachKind::Ground { id, .. } => {
            choose_ground(cx, *id, None, ground_area(anchor, radius), &none)
                .usable()
                .is_some()
        }
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
        ReachKind::Loc { id, name } => choose_loc(
            cx,
            *id,
            name.as_deref(),
            Some(op),
            Area::new(anchor, PROBE_RADIUS),
            target_tile,
            &none,
        )
        .usable()
        .is_some(),
        ReachKind::Name { name } => choose_loc(
            cx,
            None,
            Some(name),
            Some(op),
            Area::new(anchor, radius),
            target_tile,
            &none,
        )
        .usable()
        .is_some(),
        ReachKind::Npc { id, .. } => {
            choose_npc(cx, *id, Some(op), Area::new(anchor, radius), &none)
                .usable()
                .is_some()
        }
    }
}

/// Ground items keep their historical twelve-tile player probe without an
/// anchor; an anchor owns the area otherwise.
pub fn ground_area(anchor: Option<WorldTile>, radius: i32) -> Area {
    Area::new(anchor, if anchor.is_some() { radius } else { 12 })
}

/// Ticks a target that answered "I can't reach that!" stays out of the chooser.
pub const AVOID_TICKS: u64 = 10;
/// Opportunistic retargets allowed for one approach walk.
pub const RETARGET_LIMIT: u8 = 3;
/// A match found while walking replaces the walk only within this many route steps.
pub const RETARGET_STEPS: u32 = 12;

/// The chooser's search area: the authored anchor's area, or the player's
/// radius when no anchor was authored (`radius <= 0` means the whole scene).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub anchor: Option<WorldTile>,
    pub radius: i32,
}

impl Area {
    pub fn new(anchor: Option<WorldTile>, radius: i32) -> Self {
        Self { anchor, radius }
    }

    /// Whether a target at `tile` (`player_distance` from the player) is in the area.
    pub fn contains(&self, tile: WorldTile, player_distance: i32) -> bool {
        match self.anchor {
            Some(anchor) => chebyshev(anchor, tile) <= self.radius.max(PROBE_RADIUS),
            None => self.radius <= 0 || player_distance <= self.radius,
        }
    }

    fn tie_break(&self, tile: WorldTile, player_distance: i32) -> i32 {
        self.anchor
            .map_or(player_distance, |anchor| chebyshev(anchor, tile))
    }
}

/// One chooser result. `Unreachable` is the best in-area match by anchor
/// distance when no match is route-reachable from the live player tile.
/// `Unverified` is the nearest straight-line match when no live reach flood
/// is posted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick<T> {
    Reachable(T),
    Unverified(T),
    Unreachable(T),
    None,
}

impl<T> Pick<T> {
    /// A match proven reachable by the live flood.
    pub fn reachable(self) -> Option<T> {
        match self {
            Pick::Reachable(value) => Some(value),
            _ => None,
        }
    }

    /// A match not known to be unreachable.
    pub fn usable(self) -> Option<T> {
        match self {
            Pick::Reachable(value) | Pick::Unverified(value) => Some(value),
            _ => None,
        }
    }

    pub fn any(self) -> Option<T> {
        match self {
            Pick::Reachable(value) | Pick::Unverified(value) | Pick::Unreachable(value) => {
                Some(value)
            }
            Pick::None => None,
        }
    }

    /// Strict selection only accepts a proven reachable match.
    pub fn select(self, strict: bool) -> Option<T> {
        if strict {
            self.reachable()
        } else {
            self.any()
        }
    }
}

/// Target identity the chooser can briefly avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvoidKey {
    Npc(usize),
    Tile(WorldTile),
}

/// A small, fixed set of recently unreachable targets, expiring by game tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct Avoid {
    entries: [Option<(AvoidKey, u64)>; 4],
    next: usize,
    now: u64,
}

impl Avoid {
    /// Avoid `key` for [`AVOID_TICKS`] after `tick`.
    pub fn add(&mut self, key: AvoidKey, tick: u64) {
        self.entries[self.next] = Some((key, tick.saturating_add(AVOID_TICKS)));
        self.next = (self.next + 1) % self.entries.len();
    }

    /// Evaluate expiry against the current game tick.
    pub fn at(mut self, tick: u64) -> Self {
        self.now = tick;
        self
    }

    fn contains(&self, key: AvoidKey) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|(entry, until)| *entry == key && self.now < *until)
    }
}

#[derive(Debug, Clone, Copy)]
struct Footprint {
    tile: WorldTile,
    width: i32,
    length: i32,
    distance: i32,
}

fn npc_footprint(npc: &api::snapshot::NpcView) -> Footprint {
    Footprint {
        tile: npc.tile,
        width: npc.size.max(1),
        length: npc.size.max(1),
        distance: npc.distance,
    }
}

fn loc_footprint(loc: &api::snapshot::LocView) -> Footprint {
    Footprint {
        tile: loc.tile,
        width: loc.footprint_width.max(1),
        length: loc.footprint_length.max(1),
        distance: loc.distance,
    }
}

/// The posted reach view when its flood starts on the live player tile.
/// A missing or stale flood leaves reachability unknown.
fn live_reach(
    view: Option<&api::query::ReachQueryView>,
    here: Option<WorldTile>,
) -> Option<&api::query::ReachQueryView> {
    let view = view?;
    (exact_rank_at(view, here?) == Some(0)).then_some(view)
}

/// Shared Quester target chooser. In-area matches that the player can reach
/// (footprint adjacency through wall-valid edges) rank by walking distance,
/// then by anchor distance (player distance without an anchor). Unknown
/// reachability keeps straight-line player distance as the walking estimate.
fn choose<'a, T>(
    cx: &ActionContext<'_>,
    area: Area,
    rows: impl IntoIterator<Item = &'a T>,
    footprint: impl Fn(&T) -> Footprint,
) -> Pick<&'a T> {
    let snapshot = cx.snapshot();
    let view = live_reach(
        snapshot.reach().map(|reach| reach.value),
        snapshot.here().map(|here| here.value),
    );
    let mut reachable: Option<((u32, i32, u16), &'a T)> = None;
    let mut unreachable: Option<((i32, i32), &'a T)> = None;
    for row in rows {
        let fp = footprint(row);
        if !area.contains(fp.tile, fp.distance) {
            continue;
        }
        let tie = area.tie_break(fp.tile, fp.distance);
        let steps = match view {
            Some(view) => route_steps(view, fp),
            None => Some((u32::try_from(fp.distance).unwrap_or(u32::MAX), 0)),
        };
        match steps {
            Some((walk, rank)) => {
                let key = (walk, tie, rank);
                if reachable.is_none_or(|(best, _)| key < best) {
                    reachable = Some((key, row));
                }
            }
            None => {
                let key = (tie, fp.distance);
                if unreachable.is_none_or(|(best, _)| key < best) {
                    unreachable = Some((key, row));
                }
            }
        }
    }
    match (reachable, unreachable) {
        (Some((_, row)), _) if view.is_some() => Pick::Reachable(row),
        (Some((_, row)), _) => Pick::Unverified(row),
        (None, Some((_, row))) => Pick::Unreachable(row),
        (None, None) => Pick::None,
    }
}

pub fn choose_npc_matching<'a>(
    cx: &'a ActionContext<'_>,
    area: Area,
    avoid: &Avoid,
    matches: impl Fn(&api::snapshot::NpcView) -> bool,
) -> Pick<&'a api::snapshot::NpcView> {
    let Some(npcs) = cx.snapshot().npcs() else {
        return Pick::None;
    };
    choose(
        cx,
        area,
        npcs.value
            .iter()
            .filter(|npc| matches(npc) && !avoid.contains(AvoidKey::Npc(npc.index))),
        npc_footprint,
    )
}

pub fn choose_npc<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
    op: Option<&str>,
    area: Area,
    avoid: &Avoid,
) -> Pick<&'a api::snapshot::NpcView> {
    choose_npc_matching(cx, area, avoid, |npc| {
        npc.r#type == Some(id as usize)
            && op.is_none_or(|op| {
                npc.actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case(op))
            })
    })
}

pub fn choose_ground<'a>(
    cx: &'a ActionContext<'_>,
    id: i32,
    tile: Option<WorldTile>,
    area: Area,
    avoid: &Avoid,
) -> Pick<&'a api::snapshot::GroundItemView> {
    let Some(items) = cx.snapshot().ground_items() else {
        return Pick::None;
    };
    choose(
        cx,
        area,
        items.value.iter().filter(|item| {
            item.def.id == id
                && tile.is_none_or(|tile| item.tile == tile)
                && !avoid.contains(AvoidKey::Tile(item.tile))
        }),
        ground_footprint,
    )
}

fn ground_footprint(item: &api::snapshot::GroundItemView) -> Footprint {
    Footprint {
        tile: item.tile,
        width: 1,
        length: 1,
        distance: item.distance,
    }
}

/// An explicit `target_tile` selects that exact loc; without one any matching
/// loc in the area is fungible.
pub fn choose_loc<'a>(
    cx: &'a ActionContext<'_>,
    id: Option<i32>,
    name: Option<&str>,
    op: Option<&str>,
    area: Area,
    target_tile: Option<WorldTile>,
    avoid: &Avoid,
) -> Pick<&'a api::snapshot::LocView> {
    let Some(locs) = cx.snapshot().locs() else {
        return Pick::None;
    };
    choose(
        cx,
        area,
        locs.value.iter().filter(|loc| {
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
            }) && target_tile.is_none_or(|tile| loc.tile == tile)
                && !avoid.contains(AvoidKey::Tile(loc.tile))
        }),
        loc_footprint,
    )
}

/// Route steps from the player to a loc's best footprint stand, when known.
pub fn loc_route_steps(cx: &ActionContext<'_>, loc: &api::snapshot::LocView) -> Option<u32> {
    footprint_route_steps(cx, loc_footprint(loc))
}

/// Route steps from the player to an NPC's best footprint stand, when known.
pub fn npc_route_steps(cx: &ActionContext<'_>, npc: &api::snapshot::NpcView) -> Option<u32> {
    footprint_route_steps(cx, npc_footprint(npc))
}

/// Route steps to a ground item, when known.
pub fn ground_route_steps(
    cx: &ActionContext<'_>,
    item: &api::snapshot::GroundItemView,
) -> Option<u32> {
    footprint_route_steps(cx, ground_footprint(item))
}

/// Whether a live reach flood from the player's tile is posted.
pub fn reach_known(cx: &ActionContext<'_>) -> bool {
    let snapshot = cx.snapshot();
    live_reach(
        snapshot.reach().map(|reach| reach.value),
        snapshot.here().map(|here| here.value),
    )
    .is_some()
}

fn footprint_route_steps(cx: &ActionContext<'_>, fp: Footprint) -> Option<u32> {
    let snapshot = cx.snapshot();
    let view = live_reach(
        snapshot.reach().map(|reach| reach.value),
        snapshot.here().map(|here| here.value),
    )?;
    route_steps(view, fp).map(|(steps, _)| steps)
}

/// Whether the player can operate on an NPC now: orthogonally beside its
/// whole footprint across a wall-valid edge, not inside it. Without a live
/// reach view this keeps the straight-line one-tile check.
pub fn npc_adjacent(cx: &ActionContext<'_>, npc: &api::snapshot::NpcView) -> bool {
    let snapshot = cx.snapshot();
    let here = snapshot.here().map(|here| here.value);
    let (Some(here), Some(view)) = (here, live_reach(snapshot.reach().map(|r| r.value), here))
    else {
        return npc.distance <= 1;
    };
    let fp = npc_footprint(npc);
    if footprint_tiles(fp).any(|tile| tile == here) {
        return false;
    }
    footprint_tiles(fp).any(|tile| {
        reach_bit(view, &view.reachable_adj, tile)
            && local_index(view, tile).and_then(|i| view.adjacent_rank.get(i)) == Some(&0)
    })
}

fn footprint_tiles(fp: Footprint) -> impl Iterator<Item = WorldTile> {
    (0..fp.width).flat_map(move |dx| {
        (0..fp.length).map(move |dz| WorldTile {
            x: fp.tile.x + dx,
            z: fp.tile.z + dz,
            level: fp.tile.level,
        })
    })
}

fn local_index(view: &api::query::ReachQueryView, tile: WorldTile) -> Option<usize> {
    let lx = tile.x - view.base_x;
    let lz = tile.z - view.base_z;
    (tile.level == view.level && lx >= 0 && lz >= 0 && lx < view.width && lz < view.height)
        .then(|| (lx as usize) * (view.height as usize) + (lz as usize))
}

fn reach_bit(view: &api::query::ReachQueryView, words: &[u32], tile: WorldTile) -> bool {
    api::query::ReachQueryView::bit_at(
        words,
        view.width,
        view.height,
        view.base_x,
        view.base_z,
        view.level,
        tile,
    )
}

fn exact_rank_at(view: &api::query::ReachQueryView, tile: WorldTile) -> Option<u16> {
    if !reach_bit(view, &view.reachable, tile) {
        return None;
    }
    let rank = *view.exact_rank.get(local_index(view, tile)?)?;
    (rank != u16::MAX).then_some(rank)
}

/// Best stand for a footprint: the earliest flood rank that is exact on, or
/// wall-valid orthogonally beside, a footprint tile. Returns its walking
/// depth and rank.
fn route_steps(view: &api::query::ReachQueryView, fp: Footprint) -> Option<(u32, u16)> {
    let (rank, stand) = footprint_tiles(fp)
        .filter_map(|tile| {
            if !reach_bit(view, &view.reachable_adj, tile) {
                return None;
            }
            let rank = *view.adjacent_rank.get(local_index(view, tile)?)?;
            (rank != u16::MAX).then(|| (rank, stand_for(view, tile, rank)))
        })
        .min_by_key(|(rank, _)| *rank)?;
    Some((
        flood_depth(view, stand, rank).unwrap_or(u32::from(rank)),
        rank,
    ))
}

/// The tile whose exact rank produced `rank`: the tile itself or one of its
/// orthogonal neighbours.
fn stand_for(view: &api::query::ReachQueryView, tile: WorldTile, rank: u16) -> WorldTile {
    [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
        .into_iter()
        .map(|(dx, dz)| WorldTile {
            x: tile.x + dx,
            z: tile.z + dz,
            level: tile.level,
        })
        .find(|stand| exact_rank_at(view, *stand) == Some(rank))
        .unwrap_or(tile)
}

/// Walking depth of a flooded tile, recovered from dequeue ranks: each BFS
/// tile was discovered by its lowest-ranked neighbour able to step into it.
/// `None` when the posted step masks cannot reproduce the chain.
fn flood_depth(
    view: &api::query::ReachQueryView,
    mut tile: WorldTile,
    mut rank: u16,
) -> Option<u32> {
    let mut depth = 0u32;
    while rank != 0 {
        let (parent_rank, parent) = (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| (dx, dz)))
            .filter(|&(dx, dz)| dx != 0 || dz != 0)
            .filter_map(|(dx, dz)| {
                let from = WorldTile {
                    x: tile.x + dx,
                    z: tile.z + dz,
                    level: tile.level,
                };
                let from_rank = exact_rank_at(view, from)?;
                (from_rank < rank && view.can_step(from, tile)).then_some((from_rank, from))
            })
            .min_by_key(|(from_rank, _)| *from_rank)?;
        tile = parent;
        rank = parent_rank;
        depth += 1;
    }
    Some(depth)
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

/// The same server-position stand predicate used by native walks, including
/// footprint approach masks and straight-wall operations without a footprint.
pub fn loc_arrived(cx: &ActionContext<'_>, loc: &api::snapshot::LocView) -> bool {
    let snapshot = cx.snapshot();
    snapshot.local_player().is_some_and(|player| {
        let here = player.value.player.network;
        if loc_walk_id(loc).is_some() {
            snapshot.walk_loc_arrived(here, loc.tile, 1, loc.id)
        } else {
            (loc.layer == api::snapshot::LocLayer::Wall
                && door_wall_reachable(here, loc, snapshot.reach().map(|reach| reach.value)))
                || snapshot.walk_arrived(here, loc.tile, 1)
        }
    })
}

pub fn loc_walk_id(loc: &api::snapshot::LocView) -> Option<i32> {
    api::query::loc_approach::distance_from(loc, loc.tile).map(|_| loc.id)
}

/// The approach walk for a chosen loc. A footprint loc keeps its own arrival
/// rule. From the facing side of a straight wall (a door or gate), the walk
/// goes to the facing tile at radius 0: the loc's own tile is across the
/// wall, and nav's tile arrival drops the doorstep the wall separates from
/// it. From the loc's own side (or the wall line) its tile at radius 1 is on
/// the player's side. An unobserved player takes the facing tile.
pub fn loc_walk_request(
    loc: &api::snapshot::LocView,
    here: Option<WorldTile>,
    required_after: EvidenceStamp,
) -> WalkRequest {
    if let Some(id) = loc_walk_id(loc) {
        return walk_request(loc.tile, 1, Some(id), required_after);
    }
    let facing = (loc.layer == api::snapshot::LocLayer::Wall)
        .then(|| {
            let (shape, angle) = (u8::try_from(loc.shape).ok()?, u8::try_from(loc.angle).ok()?);
            api::query::straight_wall_facing(loc.tile, shape, angle)
        })
        .flatten();
    let on_facing_side = |stand: WorldTile| {
        here.is_none_or(|here| {
            let normal = (stand.x - loc.tile.x, stand.z - loc.tile.z);
            here.level != loc.tile.level
                || (here.x - loc.tile.x) * normal.0 + (here.z - loc.tile.z) * normal.1 > 0
        })
    };
    match facing {
        Some(stand) if on_facing_side(stand) => walk_request(stand, 0, None, required_after),
        _ => walk_request(loc.tile, 1, None, required_after),
    }
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
        food_guard: false,
        allow: Default::default(),
    }
}

pub fn within(here: WorldTile, dest: WorldTile, radius: i32) -> bool {
    chebyshev(here, dest) <= radius
}

pub fn duration_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

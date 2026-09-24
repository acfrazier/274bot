//! Rust-owned Cell isolate step machine.
//!
//! JS marshals the site row and dispatches the effect table. Discriminator,
//! inv proof, and the kept order stay here. Own map and token counter.
//! Call `hunt_fight::in_area_body` only. Do not call Key, Leave, Enter,
//! Fight, or dialog dispatch. Inside the cell is pump-start leave or Velrak,
//! not an abort copied from the key machine.

use crate::hunt_fight::{in_area_body, SiteBox, Tile};
use crate::observed::{self, EntityRow, ItemRow, Scene};
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;

pub const DOOR_MS: u64 = 8_000;
pub const VELRAK_WALK_MS: u64 = 20_000;
pub const WALK_LEG_MS: u64 = 300_000;

const DUSTY_KEY_ID: i32 = 1590;
const JAIL_KEY_ID: i32 = 1591;
const DOOR_LOC_ID: i32 = 2631;
const DOOR_WITHIN: i32 = 5;
const MAX_ATTEMPTS: u32 = 3;
const DIALOG_STEPS: u32 = 120;
const KBD_LOCS: [i32; 4] = [1765, 1766, 1816, 1817];
const JAIL_DOOR: Tile = Tile {
    x: 2931,
    z: 9690,
    level: 0,
};
const JAIL_DOOR_INSIDE: Tile = Tile {
    x: 2931,
    z: 9689,
    level: 0,
};
const CELL: SiteBox = SiteBox {
    min_x: 2928,
    max_x: 2934,
    min_z: 9683,
    max_z: 9689,
    level: 0,
};
const VELRAK_NAME: &str = "Velrak the explorer";
const VELRAK_PREFER: [&str; 2] = ["So... do you know anywhere good to explore?", "Yes please!"];

thread_local! {
    static CELL_RUNTIMES: RefCell<HashMap<u64, CellRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
}

#[derive(Clone, Debug)]
pub struct CellInv {
    pub id: i32,
    pub count: i32,
    pub name: String,
    pub slot: i32,
    pub has_slot: bool,
}

#[derive(Clone, Debug)]
pub struct CellLoc {
    pub id: i32,
    pub tile: Tile,
}

#[derive(Clone, Debug)]
pub struct CellNpc {
    pub index: i32,
    pub name: String,
    pub actions: Vec<String>,
    pub tile: Tile,
}

#[derive(Clone, Debug)]
pub struct CellObservation {
    pub here: Option<Tile>,
    pub ingame: bool,
    pub hold: bool,
    pub ours: bool,
    pub inv: Vec<CellInv>,
    pub locs: Vec<CellLoc>,
    pub npcs: Vec<CellNpc>,
    pub chat_modal_id: i32,
    pub chat_continue: bool,
    pub chat_options: Vec<String>,
}

impl CellObservation {
    /// Fields read from the isolate scene over the `empty` defaults. A
    /// logout clears `here` (a tile posted with the logout still counts);
    /// every other page keeps its last posted value.
    fn from_scene(scene: &Scene) -> Self {
        let empty = Self::empty();
        let latest = scene.latest();
        Self {
            here: scene.since_logout().here().map(Tile::from),
            ingame: latest.ingame().unwrap_or(empty.ingame),
            hold: latest.hold().unwrap_or(empty.hold),
            ours: latest.ours().unwrap_or(empty.ours),
            inv: latest
                .inv()
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            let slot = row.slot_or_unset();
                            CellInv {
                                id: row.id,
                                count: row.count,
                                name: row.name_or_empty().to_string(),
                                slot,
                                has_slot: row.slot.is_some() && slot >= 0,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            locs: latest
                .locs()
                .map(|rows| {
                    rows.iter()
                        .map(|loc| CellLoc {
                            id: loc.id,
                            tile: loc.tile().into(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            npcs: latest
                .npcs()
                .map(|rows| {
                    rows.iter()
                        .map(|npc| CellNpc {
                            index: npc.index,
                            name: npc.name_or_empty().to_string(),
                            actions: npc.actions.clone(),
                            tile: npc.tile().into(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            chat_modal_id: latest.chat_modal_id().unwrap_or(empty.chat_modal_id),
            chat_continue: latest.chat_continue().unwrap_or(empty.chat_continue),
            chat_options: latest.chat_options().cloned().unwrap_or_default(),
        }
    }

    /// Write this observation into the isolate scene as posts, replacing it.
    fn post(self) {
        observed::replace(0, self.ingame, |post| {
            if let Some(here) = self.here {
                post.here(here.into());
            }
            post.hold(self.hold)
                .ours(self.ours)
                .inv(
                    self.inv
                        .into_iter()
                        .map(|row| ItemRow {
                            id: row.id,
                            count: row.count,
                            name: Some(row.name),
                            slot: row.has_slot.then_some(row.slot),
                            ..ItemRow::default()
                        })
                        .collect(),
                )
                .locs(
                    self.locs
                        .into_iter()
                        .map(|loc| EntityRow {
                            id: loc.id,
                            x: loc.tile.x,
                            z: loc.tile.z,
                            level: loc.tile.level,
                            ..EntityRow::default()
                        })
                        .collect(),
                )
                .npcs(
                    self.npcs
                        .into_iter()
                        .map(|npc| EntityRow {
                            index: npc.index,
                            name: Some(npc.name),
                            actions: npc.actions,
                            x: npc.tile.x,
                            z: npc.tile.z,
                            level: npc.tile.level,
                            ..EntityRow::default()
                        })
                        .collect(),
                )
                .chat_modal_id(self.chat_modal_id)
                .chat_continue(self.chat_continue)
                .chat_options(self.chat_options);
        });
    }

    fn empty() -> Self {
        Self {
            here: None,
            ingame: true,
            hold: false,
            ours: false,
            inv: Vec::new(),
            locs: Vec::new(),
            npcs: Vec::new(),
            chat_modal_id: -1,
            chat_continue: false,
            chat_options: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct CellProj {
    key: String,
    key_item_present: bool,
    boxes: Vec<SiteBox>,
    route_present: bool,
    loc_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Decide,
    AckLeaveLair,
    AckKey,
    AckWalk,
    AfterDelay,
    AckUnlock,
    AckVelrakWalk,
    AckTalk,
    Dialog,
    AckHandoff,
    AckOpen,
    Done,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkPurpose {
    DoorNear,
    DoorExact,
    LeaveExact,
    LeaveNear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterDelay {
    Unlock,
    VelrakFind,
    LeaveOpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaveContext {
    Pump,
    Attempt,
}

struct CellRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    attempts: u32,
    attempt_open: bool,
    entered_retry: bool,
    lair_left: bool,
    failing: bool,
    walk_purpose: WalkPurpose,
    walk_dest: Option<Tile>,
    walk_token: Option<u64>,
    leave_context: LeaveContext,
    after_delay: AfterDelay,
    velrak_reclick: u32,
    dialog_steps: u32,
    click_acked: bool,
    yielded: Option<bool>,
}

impl CellRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            phase: Phase::Decide,
            attempts: 0,
            attempt_open: false,
            entered_retry: false,
            lair_left: false,
            failing: false,
            walk_purpose: WalkPurpose::DoorNear,
            walk_dest: None,
            walk_token: None,
            leave_context: LeaveContext::Pump,
            after_delay: AfterDelay::Unlock,
            velrak_reclick: 0,
            dialog_steps: 0,
            click_acked: false,
            yielded: None,
        }
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_value(&mut self, value: bool) -> Value {
        self.phase = Phase::Done;
        self.yielded = Some(value);
        self.clock.deadline = None;
        self.emit(json!({ "kind": "yield", "value": value }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.phase = Phase::Aborted;
        self.clock.deadline = None;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }
}

fn observation() -> CellObservation {
    observed::with(CellObservation::from_scene)
}

/// Test seam: replace the isolate scene with posts that read back as `obs`.
pub fn set_observation(obs: CellObservation) {
    obs.post();
}

fn live_dist(here: Tile, tile: Tile) -> i32 {
    if here.level != tile.level {
        return 1_000_000;
    }
    (here.x - tile.x).abs().max((here.z - tile.z).abs())
}

fn in_lair(proj: &CellProj, obs: &CellObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    in_area_body(here, 1, &proj.boxes)
}

fn in_cell(obs: &CellObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    in_area_body(here, 1, &[CELL])
}

fn holds(obs: &CellObservation, id: i32) -> bool {
    obs.inv.iter().any(|row| row.id == id && row.count > 0)
}

fn complete(obs: &CellObservation) -> bool {
    holds(obs, DUSTY_KEY_ID) && obs.here.is_some_and(|here| !in_area_body(here, 1, &[CELL]))
}

fn is_kbd(proj: &CellProj, obs: &CellObservation) -> bool {
    proj.key == "kbd-lair"
        || proj.route_present
        || proj.loc_ids.iter().any(|id| KBD_LOCS.contains(id))
        || obs.locs.iter().any(|loc| KBD_LOCS.contains(&loc.id))
}

fn talk_action(actions: &[String]) -> Option<&str> {
    actions
        .iter()
        .find(|action| action.to_ascii_lowercase().starts_with("talk"))
        .map(String::as_str)
}

fn nearest_door(obs: &CellObservation) -> Option<CellLoc> {
    let here = obs.here?;
    obs.locs
        .iter()
        .filter(|loc| loc.id == DOOR_LOC_ID && live_dist(here, loc.tile) <= DOOR_WITHIN)
        .min_by_key(|loc| live_dist(here, loc.tile))
        .cloned()
}

fn nearest_velrak(obs: &CellObservation) -> Option<CellNpc> {
    let here = obs.here?;
    obs.npcs
        .iter()
        .filter(|npc| {
            npc.name.eq_ignore_ascii_case(VELRAK_NAME) && talk_action(&npc.actions).is_some()
        })
        .min_by_key(|npc| live_dist(here, npc.tile))
        .cloned()
}

fn usable_jail_key(obs: &CellObservation) -> Option<CellInv> {
    obs.inv
        .iter()
        .find(|row| {
            row.id == JAIL_KEY_ID
                && row.count > 0
                && row.has_slot
                && row.slot >= 0
                && !row.name.is_empty()
        })
        .cloned()
}

fn contains_ignore(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    hay.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn preferred_slot(options: &[String]) -> i32 {
    for fragment in VELRAK_PREFER {
        if let Some(idx) = options
            .iter()
            .position(|opt| contains_ignore(opt, fragment))
        {
            return idx as i32 + 1;
        }
    }
    options.len() as i32
}

fn dialog_ready(obs: &CellObservation) -> bool {
    obs.chat_modal_id != -1 || obs.chat_continue
}

fn dialog_closed(obs: &CellObservation) -> bool {
    obs.chat_modal_id == -1 && !obs.chat_continue && obs.chat_options.is_empty()
}

fn i32_of(v: Option<&Value>) -> i32 {
    v.and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn present(v: Option<&Value>) -> bool {
    v.is_some_and(|v| !v.is_null())
}

fn push_loc_ids(ids: &mut Vec<i32>, v: &Value) {
    match v {
        Value::Array(rows) => {
            for row in rows {
                push_loc_ids(ids, row);
            }
        }
        Value::Object(map) => {
            for key in ["locId", "id"] {
                if let Some(n) = map.get(key).and_then(Value::as_i64) {
                    ids.push(n as i32);
                }
            }
            for (key, child) in map {
                if key != "locId" && key != "id" {
                    push_loc_ids(ids, child);
                }
            }
        }
        _ => {}
    }
}

fn parse_boxes(input: &Value) -> Vec<SiteBox> {
    input
        .get("boxes")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| SiteBox {
                    min_x: i32_of(row.get("minX")),
                    max_x: i32_of(row.get("maxX")),
                    min_z: i32_of(row.get("minZ")),
                    max_z: i32_of(row.get("maxZ")),
                    level: i32_of(row.get("level")),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_proj(input: &Value) -> CellProj {
    let mut loc_ids = Vec::new();
    for key in ["route", "outLever", "upLadder"] {
        if let Some(v) = input.get(key) {
            push_loc_ids(&mut loc_ids, v);
        }
    }
    CellProj {
        key: input
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        key_item_present: match input.get("keyItem") {
            None | Some(Value::Null) => false,
            Some(_) => true,
        },
        boxes: parse_boxes(input),
        route_present: present(input.get("route"))
            || present(input.get("outLever"))
            || present(input.get("upLadder")),
        loc_ids,
    }
}

fn token_of(input: &Value) -> u64 {
    input
        .get("token")
        .and_then(|t| {
            t.as_u64()
                .or_else(|| t.as_i64().and_then(|i| u64::try_from(i).ok()))
        })
        .unwrap_or(0)
}

fn reply_u64(reply: Option<&Value>, key: &str) -> Option<u64> {
    let v = reply?.get(key)?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
}

fn reply_flag(reply: Option<&Value>, key: &str) -> bool {
    reply.and_then(|r| r.get(key)).and_then(Value::as_bool) == Some(true)
}

fn alloc_token() -> u64 {
    NEXT_TOKEN.with(|n| {
        let mut n = n.borrow_mut();
        let t = *n;
        *n = t.wrapping_add(1).max(1);
        t
    })
}

fn with_cell<T>(token: u64, f: impl FnOnce(&mut CellRuntime) -> T) -> Option<T> {
    CELL_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn walk_wait_settled(token: u64) -> bool {
    crate::walk_wait::dispatch(&json!({ "op": "settled", "token": token }))
        .as_bool()
        .unwrap_or(false)
}

fn gate(rt: &mut CellRuntime, proj: &CellProj, obs: &CellObservation) -> Option<Value> {
    if is_kbd(proj, obs) {
        return Some(rt.aborted("kbd-later"));
    }
    if obs.hold || obs.ours || !obs.ingame {
        return Some(rt.yield_value(false));
    }
    if !proj.key_item_present {
        return Some(rt.yield_value(true));
    }
    if complete(obs) {
        return Some(rt.yield_value(true));
    }
    None
}

fn emit_delay(rt: &mut CellRuntime, n: u32, after: AfterDelay) -> Value {
    rt.after_delay = after;
    rt.phase = Phase::AfterDelay;
    rt.emit(json!({ "kind": "delay-ticks", "n": n }))
}

fn emit_walk(
    rt: &mut CellRuntime,
    tile: Tile,
    radius: i32,
    near: bool,
    purpose: WalkPurpose,
) -> Value {
    rt.walk_dest = Some(tile);
    rt.walk_purpose = purpose;
    rt.walk_token = None;
    rt.phase = Phase::AckWalk;
    rt.clock.arm(WALK_LEG_MS);
    if near {
        rt.emit(json!({
            "kind": "walk-near",
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
            "radius": radius,
        }))
    } else {
        rt.emit(json!({
            "kind": "walk",
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
        }))
    }
}

fn emit_walk_to(rt: &mut CellRuntime, tile: Tile, reclick: bool) -> Value {
    if !reclick {
        rt.velrak_reclick = 0;
    }
    rt.phase = Phase::AckVelrakWalk;
    rt.clock.arm(VELRAK_WALK_MS);
    rt.emit(json!({
        "kind": "walk-to",
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
    }))
}

fn fail_attempt(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    if rt.failing || rt.attempts >= MAX_ATTEMPTS {
        return rt.yield_value(false);
    }
    rt.failing = true;
    rt.attempt_open = false;
    rt.phase = Phase::Decide;
    rt.clock.deadline = None;
    rt.walk_token = None;
    rt.click_acked = false;
    let out = decide(rt, proj);
    rt.failing = false;
    out
}

fn leave_failed(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    match rt.leave_context {
        LeaveContext::Pump => rt.yield_value(false),
        LeaveContext::Attempt => fail_attempt(rt, proj),
    }
}

fn leave_done(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if holds(&obs, DUSTY_KEY_ID) {
        return rt.yield_value(true);
    }
    match rt.leave_context {
        LeaveContext::Pump => {
            rt.phase = Phase::Decide;
            rt.clock.deadline = None;
            decide(rt, proj)
        }
        LeaveContext::Attempt => fail_attempt(rt, proj),
    }
}

fn ensure_attempt(rt: &mut CellRuntime) -> Option<Value> {
    if rt.attempt_open {
        return None;
    }
    if rt.attempts >= MAX_ATTEMPTS {
        return Some(rt.yield_value(false));
    }
    rt.attempts += 1;
    rt.attempt_open = true;
    None
}

fn decide(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if in_lair(proj, &obs) && !in_cell(&obs) && !rt.lair_left {
        rt.phase = Phase::AckLeaveLair;
        return rt.emit(json!({ "kind": "leave" }));
    }
    if !rt.entered_retry {
        rt.entered_retry = true;
        if in_cell(&obs) {
            return begin_cell_leave(rt, proj, LeaveContext::Pump);
        }
    }
    if let Some(stop) = ensure_attempt(rt) {
        return stop;
    }
    if in_cell(&obs) && holds(&obs, DUSTY_KEY_ID) {
        return begin_cell_leave(rt, proj, LeaveContext::Attempt);
    }
    if in_cell(&obs) {
        return begin_velrak(rt);
    }
    if !holds(&obs, JAIL_KEY_ID) {
        rt.phase = Phase::AckKey;
        return rt.emit(json!({ "kind": "key" }));
    }
    begin_door(rt, proj)
}

fn begin_door(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if in_cell(&obs) {
        return if holds(&obs, DUSTY_KEY_ID) {
            begin_cell_leave(rt, proj, LeaveContext::Attempt)
        } else {
            begin_velrak(rt)
        };
    }
    let Some(here) = obs.here else {
        return rt.yield_value(false);
    };
    if live_dist(here, JAIL_DOOR) > 1 {
        return emit_walk(rt, JAIL_DOOR, 1, true, WalkPurpose::DoorNear);
    }
    if here != JAIL_DOOR {
        return emit_walk(rt, JAIL_DOOR, 0, false, WalkPurpose::DoorExact);
    }
    emit_delay(rt, 2, AfterDelay::Unlock)
}

fn begin_cell_leave(rt: &mut CellRuntime, proj: &CellProj, ctx: LeaveContext) -> Value {
    rt.leave_context = ctx;
    let obs = observation();
    if !in_cell(&obs) {
        return leave_done(rt, proj);
    }
    let Some(here) = obs.here else {
        return leave_failed(rt, proj);
    };
    if here != JAIL_DOOR_INSIDE {
        return emit_walk(rt, JAIL_DOOR_INSIDE, 0, false, WalkPurpose::LeaveExact);
    }
    emit_delay(rt, 2, AfterDelay::LeaveOpen)
}

fn begin_velrak(rt: &mut CellRuntime) -> Value {
    emit_delay(rt, 2, AfterDelay::VelrakFind)
}

fn ack_leave_lair(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if !reply_flag(reply, "left") {
        return rt.yield_value(false);
    }
    rt.lair_left = true;
    rt.phase = Phase::Decide;
    decide(rt, proj)
}

fn ack_key(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if in_cell(&obs) && holds(&obs, DUSTY_KEY_ID) {
        return begin_cell_leave(rt, proj, LeaveContext::Attempt);
    }
    if in_cell(&obs) {
        return begin_velrak(rt);
    }
    if !reply_flag(reply, "held") || !holds(&obs, JAIL_KEY_ID) {
        return fail_attempt(rt, proj);
    }
    rt.phase = Phase::Decide;
    decide(rt, proj)
}

fn ack_walk(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if let Some(token) = reply_u64(reply, "walkToken") {
        rt.walk_token = Some(token);
    }
    let Some(walk_token) = rt.walk_token else {
        return rt.aborted("missing walkToken");
    };
    let here = obs.here;
    let dest = rt.walk_dest.unwrap_or(JAIL_DOOR);
    let settled = rt.clock.bound_reached() || walk_wait_settled(walk_token);
    match rt.walk_purpose {
        WalkPurpose::DoorNear => {
            let arrived = here.is_some_and(|h| live_dist(h, dest) <= 1);
            if arrived {
                return begin_door(rt, proj);
            }
            if settled {
                return fail_attempt(rt, proj);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        WalkPurpose::DoorExact => {
            if here == Some(JAIL_DOOR) {
                return emit_delay(rt, 2, AfterDelay::Unlock);
            }
            if settled {
                return fail_attempt(rt, proj);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        WalkPurpose::LeaveExact => {
            if here == Some(JAIL_DOOR_INSIDE) {
                return emit_delay(rt, 2, AfterDelay::LeaveOpen);
            }
            if settled {
                return emit_walk(rt, JAIL_DOOR_INSIDE, 2, true, WalkPurpose::LeaveNear);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        WalkPurpose::LeaveNear => {
            if settled {
                return emit_delay(rt, 2, AfterDelay::LeaveOpen);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
    }
}

fn unlock(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if in_cell(&obs) {
        return if holds(&obs, DUSTY_KEY_ID) {
            begin_cell_leave(rt, proj, LeaveContext::Attempt)
        } else {
            begin_velrak(rt)
        };
    }
    if obs.here != Some(JAIL_DOOR) {
        return fail_attempt(rt, proj);
    }
    let Some(door) = nearest_door(&obs) else {
        return fail_attempt(rt, proj);
    };
    let Some(key) = usable_jail_key(&obs) else {
        return fail_attempt(rt, proj);
    };
    rt.click_acked = false;
    rt.phase = Phase::AckUnlock;
    rt.clock.arm(DOOR_MS);
    rt.emit(json!({
        "kind": "use-on",
        "name": key.name,
        "id": key.id,
        "slot": key.slot,
        "x": door.tile.x,
        "z": door.tile.z,
        "level": door.tile.level,
    }))
}

fn proved_inside(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    rt.phase = Phase::Decide;
    rt.clock.deadline = None;
    rt.click_acked = false;
    let obs = observation();
    if holds(&obs, DUSTY_KEY_ID) {
        begin_cell_leave(rt, proj, LeaveContext::Attempt)
    } else {
        begin_velrak(rt)
    }
}

fn ack_unlock(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if !rt.click_acked {
        if !reply_flag(reply, "queued") {
            return fail_attempt(rt, proj);
        }
        rt.click_acked = true;
    }
    if in_cell(&obs) {
        return proved_inside(rt, proj);
    }
    if rt.clock.bound_reached() {
        return fail_attempt(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn velrak_find(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if holds(&obs, DUSTY_KEY_ID) && in_cell(&obs) {
        return begin_cell_leave(rt, proj, LeaveContext::Attempt);
    }
    if !in_cell(&obs) {
        rt.phase = Phase::Decide;
        return decide(rt, proj);
    }
    let Some(npc) = nearest_velrak(&obs) else {
        return fail_attempt(rt, proj);
    };
    let dist = obs
        .here
        .map(|here| live_dist(here, npc.tile))
        .unwrap_or(1_000_000);
    if dist > 1 {
        return emit_walk_to(rt, npc.tile, false);
    }
    emit_talk(rt, proj, &npc)
}

fn ack_velrak_walk(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    let Some(npc) = nearest_velrak(&obs) else {
        return fail_attempt(rt, proj);
    };
    let dist = obs
        .here
        .map(|here| live_dist(here, npc.tile))
        .unwrap_or(1_000_000);
    if dist <= 1 {
        return emit_talk(rt, proj, &npc);
    }
    if rt.clock.bound_reached() {
        if rt.velrak_reclick == 0 {
            rt.velrak_reclick = 1;
            return emit_walk_to(rt, npc.tile, true);
        }
        return fail_attempt(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn emit_talk(rt: &mut CellRuntime, proj: &CellProj, npc: &CellNpc) -> Value {
    let Some(action) = talk_action(&npc.actions) else {
        return fail_attempt(rt, proj);
    };
    rt.click_acked = false;
    rt.phase = Phase::AckTalk;
    rt.clock.arm(DOOR_MS);
    rt.emit(json!({
        "kind": "npc",
        "name": npc.name,
        "action": action,
        "index": npc.index,
    }))
}

fn ack_talk(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if !rt.click_acked {
        if !reply_flag(reply, "queued") {
            return fail_attempt(rt, proj);
        }
        rt.click_acked = true;
    }
    if dialog_ready(&obs) {
        rt.dialog_steps = 0;
        rt.clock.deadline = None;
        rt.phase = Phase::Dialog;
        return dialog_step(rt, proj);
    }
    if rt.clock.bound_reached() {
        return fail_attempt(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn begin_handoff(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    rt.phase = Phase::AckHandoff;
    rt.clock.arm(DOOR_MS);
    handoff_step(rt, proj)
}

fn dialog_step(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    if rt.dialog_steps >= DIALOG_STEPS {
        return begin_handoff(rt, proj);
    }
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    rt.dialog_steps = rt.dialog_steps.saturating_add(1);
    if obs.chat_continue {
        return rt.emit(json!({ "kind": "continue" }));
    }
    if !obs.chat_options.is_empty() {
        let option = preferred_slot(&obs.chat_options);
        return rt.emit(json!({ "kind": "answer", "option": option }));
    }
    if dialog_closed(&obs) {
        return begin_handoff(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn handoff_step(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if holds(&obs, DUSTY_KEY_ID) {
        return begin_cell_leave(rt, proj, LeaveContext::Attempt);
    }
    if rt.clock.bound_reached() {
        return fail_attempt(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn leave_open(rt: &mut CellRuntime, proj: &CellProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if !in_cell(&obs) {
        return leave_done(rt, proj);
    }
    let Some(door) = nearest_door(&obs) else {
        return leave_failed(rt, proj);
    };
    rt.click_acked = false;
    rt.phase = Phase::AckOpen;
    rt.clock.arm(DOOR_MS);
    rt.emit(json!({
        "kind": "loc",
        "x": door.tile.x,
        "z": door.tile.z,
        "level": door.tile.level,
        "action": "Open",
        "id": door.id,
    }))
}

fn ack_open(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if !rt.click_acked {
        if !reply_flag(reply, "queued") {
            return leave_failed(rt, proj);
        }
        rt.click_acked = true;
    }
    if obs.here.is_some() && !in_cell(&obs) {
        return leave_done(rt, proj);
    }
    if rt.clock.bound_reached() {
        return leave_failed(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn next_effect(rt: &mut CellRuntime, proj: &CellProj, reply: Option<&Value>) -> Value {
    if rt.phase == Phase::Aborted {
        return rt.aborted("aborted");
    }
    if rt.phase == Phase::Done {
        return rt.yield_value(rt.yielded.unwrap_or(false));
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.phase {
        Phase::Decide => decide(rt, proj),
        Phase::AckLeaveLair => ack_leave_lair(rt, proj, reply),
        Phase::AckKey => ack_key(rt, proj, reply),
        Phase::AckWalk => ack_walk(rt, proj, reply),
        Phase::AfterDelay => match rt.after_delay {
            AfterDelay::Unlock => unlock(rt, proj),
            AfterDelay::VelrakFind => velrak_find(rt, proj),
            AfterDelay::LeaveOpen => leave_open(rt, proj),
        },
        Phase::AckUnlock => ack_unlock(rt, proj, reply),
        Phase::AckVelrakWalk => ack_velrak_walk(rt, proj),
        Phase::AckTalk => ack_talk(rt, proj, reply),
        Phase::Dialog => dialog_step(rt, proj),
        Phase::AckHandoff => handoff_step(rt, proj),
        Phase::AckOpen => ack_open(rt, proj, reply),
        Phase::Done => rt.yield_value(rt.yielded.unwrap_or(false)),
        Phase::Aborted => rt.aborted("aborted"),
    }
}

fn each_runtime(f: impl Fn(&mut CellRuntime)) {
    CELL_RUNTIMES.with(|m| {
        for rt in m.borrow_mut().values_mut() {
            f(rt);
        }
    });
}

pub fn on_pause() {
    each_runtime(|rt| {
        let held = rt.clock.held;
        rt.clock.set_freeze(true, held);
    });
}

pub fn on_resume() {
    each_runtime(|rt| {
        let held = rt.clock.held;
        rt.clock.set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    each_runtime(|rt| {
        let paused = rt.clock.paused;
        rt.clock.set_freeze(paused, held);
    });
}

pub fn on_reset() {
    CELL_RUNTIMES.with(|m| m.borrow_mut().clear());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => {
            let token = alloc_token();
            CELL_RUNTIMES.with(|m| {
                m.borrow_mut().insert(token, CellRuntime::new(token));
            });
            json!({ "kind": "started", "token": token })
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_proj(input);
            let reply = input.get("reply");
            match with_cell(token, |rt| next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "token": token }),
            }
        }
        "end" => {
            // The invocation is over: drop the row. Unknown token is a no-op —
            // JS ends on every return, including one after a session reset
            // already cleared the map.
            let token = token_of(input);
            let _ = CELL_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
            json!({ "kind": "ok", "token": token })
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn cell_token_alive(token: u64) -> bool {
    CELL_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

pub fn cell_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_cell(token, |rt| {
        rt.clock.deadline.map(|d| {
            if d > rt.clock.now() {
                d.saturating_duration_since(rt.clock.now()).as_millis() as i64
            } else {
                0
            }
        })
    })
    .flatten()
}

pub fn cell_force_bound_reached(token: u64) -> bool {
    with_cell(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

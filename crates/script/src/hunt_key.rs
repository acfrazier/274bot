//! Rust-owned AcquireKey isolate step machine.
//!
//! JS marshals the site row and dispatches the effect table. Discriminator,
//! inv proof, and the kept order stay here. Own map and token counter.
//! Call `hunt_fight::in_area_body` only. Do not call Leave or Fight dispatch.

use crate::hunt::Kind as HuntKind;
use crate::hunt_fight::{in_area_body, Area, SiteBox, Tile};
use crate::observed::{self, EntityRow, ItemRow, Scene, SceneRow};
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;

pub const KILL_MS: u64 = 90_000;
pub const DOOR_MS: u64 = 8_000;
pub const JAILER_RESPAWN_MS: u64 = 75_000;
pub const WALK_LEG_MS: u64 = 300_000;

const JAIL_KEY_ID: i32 = 1591;
const DROP_RADIUS: i32 = 12;
const JAILER_RADIUS: i32 = 14;
const PICKUP_RADIUS: i32 = 1;
const TAKE_CAP: u32 = 3;
const KBD_LOCS: [i32; 4] = [1765, 1766, 1816, 1817];
const CORRIDOR: Tile = Tile {
    x: 2931,
    z: 9690,
    level: 0,
};
const CELL: SiteBox = SiteBox {
    min_x: 2928,
    max_x: 2934,
    min_z: 9683,
    max_z: 9689,
    level: 0,
};

thread_local! {
    static KEY_RUNTIMES: RefCell<HashMap<u64, KeyRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
}

#[derive(Clone, Debug)]
pub struct KeyInv {
    pub id: i32,
    pub count: i32,
}

#[derive(Clone, Debug)]
pub struct KeyGround {
    pub id: i32,
    pub name: String,
    pub tile: Tile,
}

#[derive(Clone, Debug)]
pub struct KeyNpc {
    pub index: i32,
    pub name: String,
    pub actions: Vec<String>,
    pub tile: Tile,
}

#[derive(Clone, Debug)]
pub struct KeyObservation {
    pub here: Option<Tile>,
    pub ingame: bool,
    pub hold: bool,
    pub ours: bool,
    pub inv: Vec<KeyInv>,
    pub ground: Vec<KeyGround>,
    pub npcs: Vec<KeyNpc>,
    pub locs: Vec<i32>,
}

impl KeyObservation {
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
                        .map(|row| KeyInv {
                            id: row.id,
                            count: row.count,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ground: latest
                .ground()
                .map(|rows| {
                    rows.iter()
                        .map(|row| KeyGround {
                            id: row.id,
                            name: row.name_or_empty().to_string(),
                            tile: row.tile().into(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            npcs: latest
                .npcs()
                .map(|rows| {
                    rows.iter()
                        .map(|row| KeyNpc {
                            index: row.index,
                            name: row.name_or_empty().to_string(),
                            actions: observed::strings(&row.actions),
                            tile: row.tile().into(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            locs: latest
                .locs()
                .map(|rows| rows.iter().map(|loc| loc.id).collect())
                .unwrap_or_default(),
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
                            ..ItemRow::default()
                        })
                        .collect(),
                )
                .ground(
                    self.ground
                        .into_iter()
                        .map(|row| SceneRow {
                            id: row.id,
                            name: Some(row.name.into()),
                            x: row.tile.x,
                            z: row.tile.z,
                            level: row.tile.level,
                            ..SceneRow::default()
                        })
                        .collect(),
                )
                .npcs(
                    self.npcs
                        .into_iter()
                        .map(|row| EntityRow {
                            index: row.index,
                            name: Some(row.name.into()),
                            actions: observed::ops_of(&row.actions),
                            x: row.tile.x,
                            z: row.tile.z,
                            level: row.tile.level,
                            ..EntityRow::default()
                        })
                        .collect(),
                )
                .locs(
                    self.locs
                        .into_iter()
                        .map(|id| SceneRow {
                            id,
                            ..SceneRow::default()
                        })
                        .collect(),
                );
        });
    }

    fn empty() -> Self {
        Self {
            here: None,
            ingame: true,
            hold: false,
            ours: false,
            inv: Vec::new(),
            ground: Vec::new(),
            npcs: Vec::new(),
            locs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct KeyProj {
    key: String,
    key_item_present: bool,
    area: Area,
    route_present: bool,
    loc_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Decide,
    AckLeave,
    AckWalk,
    AckTake,
    AckAttack,
    Respawn,
    Done,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkPurpose {
    Drop,
    Corridor,
}

struct KeyRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    walk_purpose: WalkPurpose,
    walk_dest: Option<Tile>,
    jailer_index: Option<i32>,
    take_attempts: u32,
    corridor_done: bool,
    saw_sustain: bool,
    yielded: Option<bool>,
}

impl KeyRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            phase: Phase::Decide,
            walk_purpose: WalkPurpose::Corridor,
            walk_dest: None,
            jailer_index: None,
            take_attempts: 0,
            corridor_done: false,
            saw_sustain: false,
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

fn observation() -> KeyObservation {
    observed::with(KeyObservation::from_scene)
}

/// Test seam: replace the isolate scene with posts that read back as `obs`.
pub fn set_observation(obs: KeyObservation) {
    obs.post();
}

fn live_dist(here: Tile, tile: Tile) -> i32 {
    if here.level != tile.level {
        return 1_000_000;
    }
    (here.x - tile.x).abs().max((here.z - tile.z).abs())
}

fn in_lair(proj: &KeyProj, obs: &KeyObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    proj.area.contains(here, 1)
}

fn in_cell(obs: &KeyObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    in_area_body(here, 1, &[CELL])
}

fn within_corridor(obs: &KeyObservation) -> bool {
    obs.here
        .is_some_and(|here| live_dist(here, CORRIDOR) <= PICKUP_RADIUS)
}

fn signal(obs: &KeyObservation) -> bool {
    obs.hold || obs.ours
}

fn has_key(obs: &KeyObservation) -> bool {
    obs.inv
        .iter()
        .any(|row| row.id == JAIL_KEY_ID && row.count > 0)
}

fn is_kbd(proj: &KeyProj, obs: &KeyObservation) -> bool {
    proj.key == "kbd-lair"
        || proj.route_present
        || proj.loc_ids.iter().any(|id| KBD_LOCS.contains(id))
        || obs.locs.iter().any(|id| KBD_LOCS.contains(id))
}

fn nearest_drop(obs: &KeyObservation) -> Option<KeyGround> {
    let here = obs.here?;
    obs.ground
        .iter()
        .filter(|row| row.id == JAIL_KEY_ID && live_dist(here, row.tile) <= DROP_RADIUS)
        .min_by_key(|row| live_dist(here, row.tile))
        .cloned()
}

fn nearest_jailer(obs: &KeyObservation) -> Option<KeyNpc> {
    let here = obs.here?;
    obs.npcs
        .iter()
        .filter(|npc| npc.name == "Jailer" && npc.actions.iter().any(|action| action == "Attack"))
        .filter(|npc| live_dist(here, npc.tile) <= JAILER_RADIUS)
        .min_by_key(|npc| live_dist(here, npc.tile))
        .cloned()
}

fn jailer_live(obs: &KeyObservation, index: i32) -> bool {
    obs.npcs
        .iter()
        .any(|npc| npc.index == index && npc.name == "Jailer")
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

fn parse_proj(input: &Value) -> KeyProj {
    let mut loc_ids = Vec::new();
    for key in ["route", "outLever", "upLadder"] {
        if let Some(v) = input.get(key) {
            push_loc_ids(&mut loc_ids, v);
        }
    }
    KeyProj {
        key: input
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        key_item_present: match input.get("keyItem") {
            None | Some(Value::Null) => false,
            Some(_) => true,
        },
        area: Area::of(parse_boxes(input)),
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

fn reply_left(reply: Option<&Value>) -> bool {
    reply.and_then(|r| r.get("left")).and_then(Value::as_bool) == Some(true)
}

fn unexpected(reply: Option<&Value>) -> bool {
    let Some(reply) = reply else {
        return false;
    };
    if reply.get("eatOk").is_some() {
        return true;
    }
    let kind = reply.get("kind").and_then(Value::as_str).unwrap_or("");
    let op = reply.get("op").and_then(Value::as_str).unwrap_or("");
    let action = reply.get("action").and_then(Value::as_str).unwrap_or("");
    matches!(
        kind,
        "walk-to"
            | "bank-open"
            | "attack"
            | "use-on"
            | "loc"
            | "continue"
            | "answer"
            | "teleport"
            | "wait-fed-done"
    ) || matches!(
        op,
        "walk-to" | "bank-open" | "attack" | "use-on" | "loc" | "continue" | "answer" | "teleport"
    ) || action.eq_ignore_ascii_case("attack")
        || action.eq_ignore_ascii_case("eat")
}

fn walk_wait_settled(token: u64) -> bool {
    crate::walk_wait::dispatch(&json!({ "op": "settled", "token": token }))
        .as_bool()
        .unwrap_or(false)
}

fn alloc_token() -> u64 {
    NEXT_TOKEN.with(|n| {
        let mut n = n.borrow_mut();
        let t = *n;
        *n = t.wrapping_add(1).max(1);
        t
    })
}

fn with_key<T>(token: u64, f: impl FnOnce(&mut KeyRuntime) -> T) -> Option<T> {
    KEY_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn gate(rt: &mut KeyRuntime, proj: &KeyProj, obs: &KeyObservation) -> Option<Value> {
    if is_kbd(proj, obs) {
        return Some(rt.aborted("kbd-later"));
    }
    if in_cell(obs) {
        return Some(rt.aborted("cell-later"));
    }
    if signal(obs) || !obs.ingame {
        return Some(rt.yield_value(false));
    }
    if !proj.key_item_present {
        return Some(rt.yield_value(true));
    }
    if has_key(obs) {
        return Some(rt.yield_value(true));
    }
    None
}

fn poll(rt: &mut KeyRuntime) -> Value {
    if !rt.saw_sustain {
        rt.saw_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.saw_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn emit_walk(rt: &mut KeyRuntime, tile: Tile, purpose: WalkPurpose) -> Value {
    rt.walk_dest = Some(tile);
    rt.walk_purpose = purpose;
    rt.phase = Phase::AckWalk;
    rt.clock.arm(WALK_LEG_MS);
    rt.emit(json!({
        "kind": "walk-near",
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
        "radius": PICKUP_RADIUS,
    }))
}

fn emit_obj(rt: &mut KeyRuntime, drop: &KeyGround) -> Value {
    rt.take_attempts = rt.take_attempts.saturating_add(1);
    rt.phase = Phase::AckTake;
    rt.saw_sustain = false;
    rt.clock.arm(DOOR_MS);
    rt.emit(json!({
        "kind": "obj",
        "x": drop.tile.x,
        "z": drop.tile.z,
        "level": drop.tile.level,
        "name": drop.name,
        "action": "Take",
    }))
}

fn begin_take(rt: &mut KeyRuntime, obs: &KeyObservation, drop: &KeyGround) -> Value {
    if drop.name.is_empty() {
        return rt.yield_value(false);
    }
    let Some(here) = obs.here else {
        return rt.yield_value(false);
    };
    if live_dist(here, drop.tile) > PICKUP_RADIUS {
        return emit_walk(rt, drop.tile, WalkPurpose::Drop);
    }
    if rt.take_attempts >= TAKE_CAP {
        return rt.yield_value(false);
    }
    emit_obj(rt, drop)
}

fn begin_attack(rt: &mut KeyRuntime, npc: &KeyNpc) -> Value {
    rt.jailer_index = Some(npc.index);
    rt.phase = Phase::AckAttack;
    rt.saw_sustain = false;
    rt.clock.arm(KILL_MS);
    rt.emit(json!({
        "kind": "npc",
        "name": "Jailer",
        "action": "Attack",
        "index": npc.index,
    }))
}

fn begin_respawn(rt: &mut KeyRuntime) -> Value {
    rt.phase = Phase::Respawn;
    rt.saw_sustain = false;
    rt.clock.arm(JAILER_RESPAWN_MS);
    poll(rt)
}

fn decide(rt: &mut KeyRuntime, proj: &KeyProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if in_lair(proj, &obs) {
        rt.phase = Phase::AckLeave;
        return rt.emit(json!({ "kind": "leave" }));
    }
    if let Some(drop) = nearest_drop(&obs) {
        return begin_take(rt, &obs, &drop);
    }
    if let Some(jailer) = nearest_jailer(&obs) {
        return begin_attack(rt, &jailer);
    }
    if !rt.corridor_done {
        if within_corridor(&obs) {
            rt.corridor_done = true;
        } else {
            return emit_walk(rt, CORRIDOR, WalkPurpose::Corridor);
        }
    }
    begin_respawn(rt)
}

fn ack_leave(rt: &mut KeyRuntime, proj: &KeyProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if is_kbd(proj, &obs) {
        return rt.aborted("kbd-later");
    }
    if in_cell(&obs) {
        return rt.aborted("cell-later");
    }
    if !reply_left(reply) {
        return rt.yield_value(false);
    }
    rt.phase = Phase::Decide;
    decide(rt, proj)
}

fn ack_walk(rt: &mut KeyRuntime, proj: &KeyProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    let Some(walk_token) = reply_u64(reply, "walkToken") else {
        return rt.aborted("missing walkToken");
    };
    let dest = rt.walk_dest.unwrap_or(CORRIDOR);
    let arrived = obs
        .here
        .is_some_and(|here| live_dist(here, dest) <= PICKUP_RADIUS);
    let settled = rt.clock.bound_reached() || walk_wait_settled(walk_token);
    if arrived {
        if rt.walk_purpose == WalkPurpose::Corridor {
            rt.corridor_done = true;
        }
        rt.phase = Phase::Decide;
        return decide(rt, proj);
    }
    if settled {
        return rt.yield_value(false);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn ack_take(rt: &mut KeyRuntime, proj: &KeyProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if rt.clock.bound_reached() {
        if rt.take_attempts >= TAKE_CAP {
            return rt.yield_value(false);
        }
        rt.phase = Phase::Decide;
        return decide(rt, proj);
    }
    poll(rt)
}

fn ack_attack(rt: &mut KeyRuntime, proj: &KeyProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    let index = rt.jailer_index.unwrap_or(-1);
    if !jailer_live(&obs, index) {
        rt.jailer_index = None;
        rt.phase = Phase::Decide;
        return decide(rt, proj);
    }
    if rt.clock.bound_reached() {
        return rt.yield_value(false);
    }
    poll(rt)
}

fn respawn_step(rt: &mut KeyRuntime, proj: &KeyProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, proj, &obs) {
        return stop;
    }
    if in_lair(proj, &obs) {
        rt.phase = Phase::AckLeave;
        return rt.emit(json!({ "kind": "leave" }));
    }
    if nearest_drop(&obs).is_some() || nearest_jailer(&obs).is_some() {
        rt.phase = Phase::Decide;
        return decide(rt, proj);
    }
    if rt.clock.bound_reached() {
        return rt.yield_value(false);
    }
    poll(rt)
}

fn next_effect(rt: &mut KeyRuntime, proj: &KeyProj, reply: Option<&Value>) -> Value {
    if rt.phase == Phase::Aborted {
        return rt.aborted("aborted");
    }
    if rt.phase == Phase::Done {
        return rt.yield_value(rt.yielded.unwrap_or(false));
    }
    if unexpected(reply) {
        return rt.aborted("unexpected reply");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.phase {
        Phase::Decide => decide(rt, proj),
        Phase::AckLeave => ack_leave(rt, proj, reply),
        Phase::AckWalk => ack_walk(rt, proj, reply),
        Phase::AckTake => ack_take(rt, proj),
        Phase::AckAttack => ack_attack(rt, proj),
        Phase::Respawn => respawn_step(rt, proj),
        Phase::Done => rt.yield_value(rt.yielded.unwrap_or(false)),
        Phase::Aborted => rt.aborted("aborted"),
    }
}

fn each_runtime(f: impl Fn(&mut KeyRuntime)) {
    KEY_RUNTIMES.with(|m| {
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
    KEY_RUNTIMES.with(|m| m.borrow_mut().clear());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => {
            let token = alloc_token();
            KEY_RUNTIMES.with(|m| {
                m.borrow_mut().insert(token, KeyRuntime::new(token));
            });
            json!({ "kind": "started", "token": token })
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_proj(input);
            let reply = input.get("reply");
            match with_key(token, |rt| next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "token": token }),
            }
        }
        "end" => {
            // The invocation is over: drop the row. Unknown token is a no-op —
            // JS ends on every return, including one after a session reset
            // already cleared the map.
            let token = token_of(input);
            let _ = KEY_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
            json!({ "kind": "ok", "token": token })
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn key_token_alive(token: u64) -> bool {
    KEY_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

pub fn key_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_key(token, |rt| {
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

pub fn key_force_bound_reached(token: u64) -> bool {
    with_key(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

/// `acquireKey`.
pub(crate) struct Key;

impl HuntKind for Key {
    const NAME: &'static str = "hunt-key";
    const SESSION: bool = false;
    const BOOLEAN: bool = true;
    type Proj = KeyProj;

    fn parse(site: &Value) -> KeyProj {
        parse_proj(site)
    }

    fn area(proj: &mut KeyProj) -> &mut Area {
        &mut proj.area
    }

    fn mint() -> u64 {
        let token = alloc_token();
        KEY_RUNTIMES.with(|m| m.borrow_mut().insert(token, KeyRuntime::new(token)));
        token
    }

    fn ensure(token: u64) {
        KEY_RUNTIMES.with(|m| {
            m.borrow_mut()
                .entry(token)
                .or_insert_with(|| KeyRuntime::new(token));
        });
    }

    fn renew(token: u64) {
        KEY_RUNTIMES.with(|m| m.borrow_mut().insert(token, KeyRuntime::new(token)));
    }

    fn next(token: u64, proj: &KeyProj, reply: Option<&Value>) -> Value {
        with_key(token, |rt| next_effect(rt, proj, reply))
            .unwrap_or_else(|| json!({ "kind": "aborted", "reason": "unknown token" }))
    }

    fn end(token: u64) {
        KEY_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
    }
}

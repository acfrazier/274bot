//! Rust-owned Leave isolate step machine.
//!
//! JS marshals the site row and dispatches the effect table. Discriminator,
//! rune/level precheck, and `!in_area_body` completion stay here. Own map
//! and token counter. Call `hunt_fight::in_area_body` only.

use crate::hunt::{hook, strict_true, Host, Kind as HuntKind};
use crate::hunt_fight::{Area, SiteBox, Tile};
use crate::machine::Ended;
use crate::observed::{self, ItemRow, Scene, SceneRow};
use crate::task_clock::InstantTaskClock;
use api::game_data::SelectedGameData;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;

pub const DOOR_MS: u64 = 8_000;
pub const WALK_LEG_MS: u64 = 300_000;
const CAST_CAP: u32 = 3;
const LOC_WITHIN: i32 = 5;
const KBD_LOCS: [i32; 4] = [1765, 1766, 1816, 1817];

thread_local! {
    static LEAVE_RUNTIMES: RefCell<HashMap<u64, LeaveRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
}

#[derive(Clone, Debug)]
pub struct LeaveLoc {
    pub id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
}

#[derive(Clone, Debug)]
pub struct LeaveInv {
    pub id: i32,
    pub count: i32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct LeaveObservation {
    pub here: Option<Tile>,
    pub ingame: bool,
    pub hold: bool,
    pub ours: bool,
    pub locs: Vec<LeaveLoc>,
    pub inv: Vec<LeaveInv>,
    /// `StatReader::base` for Magic. `None` is a shortfall, including a
    /// missing stats table or a missing magic row.
    pub magic_base: Option<i32>,
    pub tick: u64,
}

impl LeaveObservation {
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
            locs: latest
                .locs()
                .map(|rows| {
                    rows.iter()
                        .map(|loc| LeaveLoc {
                            id: loc.id,
                            x: loc.x,
                            z: loc.z,
                            level: loc.level,
                            distance: loc.distance,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            inv: latest
                .inv()
                .map(|rows| {
                    rows.iter()
                        .map(|row| LeaveInv {
                            id: row.id,
                            count: row.count,
                            name: row.name_or_empty().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            magic_base: latest
                .stats()
                .and_then(|skills| skills.magic)
                .map(|row| row.base),
            tick: scene.tick().unwrap_or(empty.tick),
        }
    }

    /// Write this observation into the isolate scene as posts, replacing it.
    fn post(self) {
        observed::replace(self.tick, self.ingame, |post| {
            if let Some(here) = self.here {
                post.here(here.into());
            }
            post.hold(self.hold)
                .ours(self.ours)
                .locs(
                    self.locs
                        .into_iter()
                        .map(|loc| SceneRow {
                            id: loc.id,
                            x: loc.x,
                            z: loc.z,
                            level: loc.level,
                            distance: loc.distance,
                            ..SceneRow::default()
                        })
                        .collect(),
                )
                .inv(
                    self.inv
                        .into_iter()
                        .map(|row| ItemRow {
                            id: row.id,
                            count: row.count,
                            name: Some(row.name.into()),
                            ..ItemRow::default()
                        })
                        .collect(),
                )
                .stats(observed::Skills {
                    magic: self.magic_base.map(|base| observed::Skill {
                        base,
                        ..observed::Skill::default()
                    }),
                    ..observed::Skills::default()
                });
        });
    }

    fn empty() -> Self {
        Self {
            here: None,
            ingame: true,
            hold: false,
            ours: false,
            locs: Vec::new(),
            inv: Vec::new(),
            magic_base: None,
            tick: 1,
        }
    }
}

#[derive(Clone, Debug)]
struct ExitLoc {
    loc_id: i32,
    op: String,
    stand: Option<Tile>,
}

#[derive(Clone, Debug)]
struct Gate {
    loc_id: i32,
    op: String,
    inside: Option<Tile>,
}

#[derive(Clone, Debug)]
pub(crate) struct LeaveProj {
    leave_by_walk: Option<bool>,
    key: String,
    area: Area,
    escape_id: String,
    walk_out: Option<Tile>,
    exit: Option<ExitLoc>,
    gate: Option<Gate>,
    route_present: bool,
    loc_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    Teleport,
    AckTeleport,
    Proof,
    AfterDelay,
    WalkFamily,
    AckWalk,
    EmitFallback,
    FindLoc,
    AckLoc,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProofKind {
    AfterCast,
    AfterExit,
    AfterGate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkPurpose {
    ExitStand,
    GateExact,
    GateFallback,
    GatelessOut,
    PostGateOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Undecided,
    Exit,
    Gateless,
    Gate,
}

struct LeaveRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    cont: Phase,
    family: Family,
    proof: ProofKind,
    walk_purpose: WalkPurpose,
    casts: u32,
    saw_sustain: bool,
    walk_dest: Option<Tile>,
    walk_radius: i32,
    pending_loc: Option<(i32, String)>,
}

impl LeaveRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            phase: Phase::Start,
            cont: Phase::Start,
            family: Family::Undecided,
            proof: ProofKind::AfterCast,
            walk_purpose: WalkPurpose::GatelessOut,
            casts: 0,
            saw_sustain: false,
            walk_dest: None,
            walk_radius: 0,
            pending_loc: None,
        }
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_value(&mut self, value: bool) -> Value {
        self.phase = Phase::Start;
        self.family = Family::Undecided;
        self.casts = 0;
        self.saw_sustain = false;
        self.walk_dest = None;
        self.pending_loc = None;
        self.clock.deadline = None;
        self.emit(json!({ "kind": "yield", "value": value }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.phase = Phase::Aborted;
        self.clock.deadline = None;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }

    fn log(&mut self, message: String, then: Phase) -> Value {
        self.phase = then;
        self.emit(json!({ "kind": "log", "message": message }))
    }
}

fn observation() -> LeaveObservation {
    observed::with(LeaveObservation::from_scene)
}

/// Test seam: replace the isolate scene with posts that read back as `obs`.
pub fn set_observation(obs: LeaveObservation) {
    obs.post();
}

fn distance_to(a: Tile, b: Tile) -> i32 {
    let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
    if a.level != b.level {
        1_000_000 + xz
    } else {
        xz
    }
}

fn in_area(proj: &LeaveProj, obs: &LeaveObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    proj.area.contains(here, 1)
}

fn signal(obs: &LeaveObservation) -> bool {
    obs.hold || obs.ours
}

fn i32_of(v: Option<&Value>) -> i32 {
    v.and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn parse_tile(v: Option<&Value>) -> Option<Tile> {
    let v = v?;
    if !v.is_object() {
        return None;
    }
    Some(Tile {
        x: i32_of(v.get("x")),
        z: i32_of(v.get("z")),
        level: i32_of(v.get("level")),
    })
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

fn parse_exit(v: Option<&Value>) -> Option<ExitLoc> {
    let v = v.filter(|v| v.is_object())?;
    Some(ExitLoc {
        loc_id: i32_of(v.get("locId")),
        op: v
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        stand: parse_tile(v.get("stand")),
    })
}

fn parse_gate(v: Option<&Value>) -> Option<Gate> {
    let v = v.filter(|v| v.is_object())?;
    Some(Gate {
        loc_id: i32_of(v.get("locId")),
        op: v
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        inside: parse_tile(v.get("inside")),
    })
}

fn parse_proj(input: &Value) -> LeaveProj {
    let mut loc_ids = Vec::new();
    for key in ["exit", "gate", "route", "outLever", "upLadder"] {
        if let Some(v) = input.get(key) {
            push_loc_ids(&mut loc_ids, v);
        }
    }
    LeaveProj {
        leave_by_walk: input.get("leaveByWalk").and_then(Value::as_bool),
        key: input
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        area: Area::of(parse_boxes(input)),
        escape_id: input
            .get("escapeTeleportId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        walk_out: parse_tile(input.get("walkOut")),
        exit: parse_exit(input.get("exit")),
        gate: parse_gate(input.get("gate")),
        route_present: present(input.get("route"))
            || present(input.get("outLever"))
            || present(input.get("upLadder")),
        loc_ids,
    }
}

fn is_kbd(proj: &LeaveProj) -> bool {
    proj.key == "kbd-lair"
        || proj.route_present
        || proj.loc_ids.iter().any(|id| KBD_LOCS.contains(id))
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
        "walk-to" | "bank-open" | "attack" | "use-on" | "wait-fed-done"
    ) || matches!(op, "walk-to" | "bank-open" | "attack" | "use-on")
        || action.eq_ignore_ascii_case("attack")
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

fn with_leave<T>(token: u64, f: impl FnOnce(&mut LeaveRuntime) -> T) -> Option<T> {
    LEAVE_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn loc_by_id(obs: &LeaveObservation, id: i32, within: i32) -> Option<&LeaveLoc> {
    obs.locs
        .iter()
        .filter(|loc| loc.id == id && loc.distance <= within)
        .min_by_key(|loc| loc.distance)
}

fn arrived(here: Option<Tile>, dest: Tile, radius: i32) -> bool {
    here.is_some_and(|h| distance_to(dest, h) <= radius)
}

enum Precheck {
    Ready,
    Skip(&'static str),
}

fn rune_count(obs: &LeaveObservation, id: i32) -> i32 {
    obs.inv
        .iter()
        .filter(|row| row.id == id)
        .map(|row| row.count.max(0))
        .sum()
}

fn precheck(data: Option<&SelectedGameData>, proj: &LeaveProj, obs: &LeaveObservation) -> Precheck {
    let Some(data) = data else {
        return Precheck::Skip("no selected teleport cache");
    };
    if proj.escape_id.is_empty() {
        return Precheck::Skip("no escape teleport");
    }
    let Some(spell) = data.teleport(&proj.escape_id) else {
        return Precheck::Skip("unknown escape teleport");
    };
    let Some(level) = obs.magic_base else {
        return Precheck::Skip("magic short");
    };
    if level < spell.level {
        return Precheck::Skip("magic short");
    }
    for rune in &spell.runes {
        if rune_count(obs, rune.id) < rune.count {
            return Precheck::Skip("rune short");
        }
    }
    Precheck::Ready
}

fn start(rt: &mut LeaveRuntime, proj: &LeaveProj, data: Option<&SelectedGameData>) -> Value {
    let obs = observation();
    if signal(&obs) || !obs.ingame {
        return rt.yield_value(false);
    }
    if !in_area(proj, &obs) {
        return rt.yield_value(true);
    }
    if is_kbd(proj) {
        return rt.aborted("kbd-later");
    }
    if proj.leave_by_walk.is_none() {
        return rt.aborted("missing leaveByWalk");
    }
    if proj.leave_by_walk == Some(true) {
        return walk_family(rt, proj);
    }
    teleport_phase(rt, proj, data)
}

fn teleport_phase(
    rt: &mut LeaveRuntime,
    proj: &LeaveProj,
    data: Option<&SelectedGameData>,
) -> Value {
    let obs = observation();
    match precheck(data, proj, &obs) {
        Precheck::Skip(message) => rt.log(message.into(), Phase::WalkFamily),
        Precheck::Ready => {
            rt.casts = rt.casts.saturating_add(1);
            rt.phase = Phase::AckTeleport;
            rt.emit(json!({ "kind": "teleport", "name": proj.escape_id }))
        }
    }
}

fn begin_proof(
    rt: &mut LeaveRuntime,
    proj: &LeaveProj,
    data: Option<&SelectedGameData>,
    kind: ProofKind,
) -> Value {
    rt.proof = kind;
    rt.saw_sustain = false;
    rt.clock.arm(DOOR_MS);
    rt.phase = Phase::Proof;
    proof_step(rt, proj, data)
}

fn ack_teleport(rt: &mut LeaveRuntime, proj: &LeaveProj, data: Option<&SelectedGameData>) -> Value {
    begin_proof(rt, proj, data, ProofKind::AfterCast)
}

fn proof_step(rt: &mut LeaveRuntime, proj: &LeaveProj, data: Option<&SelectedGameData>) -> Value {
    let obs = observation();
    if signal(&obs) || !obs.ingame {
        return rt.yield_value(false);
    }
    if !in_area(proj, &obs) {
        return proved_out(rt, proj);
    }
    if rt.clock.bound_reached() {
        return proof_failed(rt, proj, data);
    }
    if !rt.saw_sustain {
        rt.saw_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.saw_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn proved_out(rt: &mut LeaveRuntime, proj: &LeaveProj) -> Value {
    match rt.proof {
        ProofKind::AfterCast | ProofKind::AfterExit => rt.yield_value(true),
        ProofKind::AfterGate => {
            let Some(dest) = proj.walk_out else {
                return rt.yield_value(true);
            };
            emit_walk(rt, dest, 3, true, WalkPurpose::PostGateOut)
        }
    }
}

fn proof_failed(rt: &mut LeaveRuntime, proj: &LeaveProj, data: Option<&SelectedGameData>) -> Value {
    match rt.proof {
        ProofKind::AfterCast => cast_failed(rt, proj, data),
        ProofKind::AfterExit | ProofKind::AfterGate => rt.yield_value(false),
    }
}

fn cast_failed(rt: &mut LeaveRuntime, proj: &LeaveProj, data: Option<&SelectedGameData>) -> Value {
    if rt.casts >= CAST_CAP {
        return walk_family(rt, proj);
    }
    let obs = observation();
    match precheck(data, proj, &obs) {
        Precheck::Skip(message) => rt.log(message.into(), Phase::WalkFamily),
        Precheck::Ready => {
            rt.cont = Phase::Teleport;
            rt.phase = Phase::AfterDelay;
            rt.emit(json!({ "kind": "delay-ticks", "n": 3 }))
        }
    }
}

fn walk_family(rt: &mut LeaveRuntime, proj: &LeaveProj) -> Value {
    if let Some(exit) = proj.exit.clone() {
        rt.family = Family::Exit;
        let Some(stand) = exit.stand else {
            return rt.yield_value(false);
        };
        if exit.op.is_empty() {
            return rt.aborted("BLOCKED: missing loc");
        }
        rt.pending_loc = Some((exit.loc_id, exit.op));
        return emit_walk(rt, stand, 2, true, WalkPurpose::ExitStand);
    }
    if proj.gate.is_none() {
        rt.family = Family::Gateless;
        let Some(dest) = proj.walk_out else {
            return rt.yield_value(false);
        };
        return emit_walk(rt, dest, 3, true, WalkPurpose::GatelessOut);
    }
    let gate = proj.gate.clone().expect("gate");
    rt.family = Family::Gate;
    let Some(inside) = gate.inside else {
        return rt.yield_value(false);
    };
    if gate.op.is_empty() {
        return rt.aborted("BLOCKED: missing loc");
    }
    rt.pending_loc = Some((gate.loc_id, gate.op));
    emit_walk(rt, inside, 0, false, WalkPurpose::GateExact)
}

fn emit_walk(
    rt: &mut LeaveRuntime,
    tile: Tile,
    radius: i32,
    near: bool,
    purpose: WalkPurpose,
) -> Value {
    rt.walk_dest = Some(tile);
    rt.walk_radius = radius;
    rt.walk_purpose = purpose;
    rt.clock.arm(WALK_LEG_MS);
    rt.phase = Phase::AckWalk;
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

fn ack_walk(rt: &mut LeaveRuntime, proj: &LeaveProj, reply: Option<&Value>) -> Value {
    let Some(walk_token) = reply_u64(reply, "walkToken") else {
        return rt.aborted("missing walkToken");
    };
    let obs = observation();
    if signal(&obs) || !obs.ingame {
        return rt.yield_value(false);
    }
    let dest = rt.walk_dest.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let radius = rt.walk_radius;
    let here_arrived = arrived(obs.here, dest, radius);
    let settled = rt.clock.bound_reached() || walk_wait_settled(walk_token);
    let out = !in_area(proj, &obs);
    match rt.walk_purpose {
        WalkPurpose::GatelessOut | WalkPurpose::PostGateOut => {
            let trailing = rt.walk_purpose == WalkPurpose::PostGateOut;
            if here_arrived || settled {
                return rt.yield_value(out);
            }
            if !trailing && out {
                return rt.yield_value(true);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        WalkPurpose::GateExact => {
            if here_arrived {
                return after_stand(rt, 2);
            }
            if settled {
                return rt.log("the gate tile is occupied".into(), Phase::EmitFallback);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        WalkPurpose::GateFallback | WalkPurpose::ExitStand => {
            if here_arrived {
                let n = if rt.walk_purpose == WalkPurpose::ExitStand {
                    1
                } else {
                    2
                };
                return after_stand(rt, n);
            }
            if settled {
                return rt.yield_value(false);
            }
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
    }
}

fn after_stand(rt: &mut LeaveRuntime, n: u32) -> Value {
    rt.cont = Phase::FindLoc;
    rt.phase = Phase::AfterDelay;
    rt.emit(json!({ "kind": "delay-ticks", "n": n }))
}

fn find_loc(rt: &mut LeaveRuntime, _proj: &LeaveProj) -> Value {
    let Some((id, op)) = rt.pending_loc.clone() else {
        return rt.yield_value(false);
    };
    if op.is_empty() {
        return rt.aborted("BLOCKED: missing loc");
    }
    let obs = observation();
    let Some(loc) = loc_by_id(&obs, id, LOC_WITHIN) else {
        return rt.yield_value(false);
    };
    rt.phase = Phase::AckLoc;
    rt.emit(json!({
        "kind": "loc",
        "x": loc.x,
        "z": loc.z,
        "level": loc.level,
        "action": op,
        "id": loc.id,
    }))
}

fn ack_loc(
    rt: &mut LeaveRuntime,
    proj: &LeaveProj,
    data: Option<&SelectedGameData>,
    reply: Option<&Value>,
) -> Value {
    let queued = reply
        .and_then(|r| r.get("queued"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !queued {
        return rt.yield_value(false);
    }
    let kind = if rt.family == Family::Exit {
        ProofKind::AfterExit
    } else {
        ProofKind::AfterGate
    };
    begin_proof(rt, proj, data, kind)
}

fn next_effect(
    rt: &mut LeaveRuntime,
    proj: &LeaveProj,
    data: Option<&SelectedGameData>,
    reply: Option<&Value>,
) -> Value {
    if rt.phase == Phase::Aborted {
        return rt.aborted("aborted");
    }
    if unexpected(reply) {
        return rt.aborted("unexpected reply");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.phase {
        Phase::Start => start(rt, proj, data),
        Phase::Teleport => teleport_phase(rt, proj, data),
        Phase::AckTeleport => ack_teleport(rt, proj, data),
        Phase::Proof => proof_step(rt, proj, data),
        Phase::AfterDelay => match rt.cont {
            Phase::Teleport => {
                let obs = observation();
                match precheck(data, proj, &obs) {
                    Precheck::Skip(message) => rt.log(message.into(), Phase::WalkFamily),
                    Precheck::Ready => teleport_phase(rt, proj, data),
                }
            }
            Phase::FindLoc => find_loc(rt, proj),
            _ => rt.aborted("bad continuation"),
        },
        Phase::WalkFamily => walk_family(rt, proj),
        Phase::AckWalk => ack_walk(rt, proj, reply),
        Phase::EmitFallback => {
            let tile = rt.walk_dest.unwrap_or(Tile {
                x: 0,
                z: 0,
                level: 0,
            });
            emit_walk(rt, tile, 2, true, WalkPurpose::GateFallback)
        }
        Phase::FindLoc => find_loc(rt, proj),
        Phase::AckLoc => ack_loc(rt, proj, data, reply),
        Phase::Aborted => rt.aborted("aborted"),
    }
}

fn each_runtime(f: impl Fn(&mut LeaveRuntime)) {
    LEAVE_RUNTIMES.with(|m| {
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
    LEAVE_RUNTIMES.with(|m| m.borrow_mut().clear());
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => {
            let token = alloc_token();
            LEAVE_RUNTIMES.with(|m| {
                m.borrow_mut().insert(token, LeaveRuntime::new(token));
            });
            json!({ "kind": "started", "token": token })
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_proj(input);
            let reply = input.get("reply");
            match with_leave(token, |rt| next_effect(rt, &proj, data, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "token": token }),
            }
        }
        "end" => {
            // The invocation is over: drop the row. Unknown token is a no-op —
            // JS ends on every return, including one after a session reset
            // already cleared the map.
            let token = token_of(input);
            let _ = LEAVE_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
            json!({ "kind": "ok", "token": token })
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn leave_token_alive(token: u64) -> bool {
    LEAVE_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

pub fn leave_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_leave(token, |rt| {
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

pub fn leave_force_bound_reached(token: u64) -> bool {
    with_leave(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

/// `leaveLair`.
pub(crate) struct Leave;

impl HuntKind for Leave {
    const NAME: &'static str = "hunt-leave";
    const SESSION: bool = false;
    const BOOLEAN: bool = true;
    type Proj = LeaveProj;

    fn parse(site: &Value) -> LeaveProj {
        parse_proj(site)
    }

    fn area(proj: &mut LeaveProj) -> &mut Area {
        &mut proj.area
    }

    /// `leaveByWalk() === true` when the host has one.
    fn refresh(proj: &mut LeaveProj, host: &mut dyn Host) -> Result<(), Ended> {
        proj.leave_by_walk = if host.has(hook::LEAVE_BY_WALK) {
            Some(strict_true(host, hook::LEAVE_BY_WALK)?)
        } else {
            None
        };
        Ok(())
    }

    fn mint() -> u64 {
        let token = alloc_token();
        LEAVE_RUNTIMES.with(|m| m.borrow_mut().insert(token, LeaveRuntime::new(token)));
        token
    }

    fn ensure(token: u64) {
        LEAVE_RUNTIMES.with(|m| {
            m.borrow_mut()
                .entry(token)
                .or_insert_with(|| LeaveRuntime::new(token));
        });
    }

    fn renew(token: u64) {
        LEAVE_RUNTIMES.with(|m| m.borrow_mut().insert(token, LeaveRuntime::new(token)));
    }

    fn next(token: u64, proj: &LeaveProj, reply: Option<&Value>) -> Value {
        with_leave(token, |rt| {
            next_effect(
                rt,
                proj,
                crate::supply_v2::selected_data().as_deref(),
                reply,
            )
        })
        .unwrap_or_else(|| json!({ "kind": "aborted", "reason": "unknown token" }))
    }

    fn end(token: u64) {
        LEAVE_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
    }
}

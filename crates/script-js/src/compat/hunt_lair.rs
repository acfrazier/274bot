//! Rust-owned EnterLair isolate step machine.
//!
//! JS marshals the site row and dispatches the effect table. Discriminator,
//! fee proof, choose match, and `in_area_body` completion stay here. Own map
//! and token counter. Call `hunt_fight::in_area_body` only.

use crate::hunt::{hook, number, strict_true, Host, Kind as HuntKind};
use crate::hunt_fight::{Area, SiteBox, Tile};
use crate::machine::Ended;
use crate::observed::{self, EntityRow, ItemRow, Scene, SceneRow};
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;

pub const STAND_MS: u64 = 300_000;
pub const PAY_MS: u64 = 20_000;
pub const DOOR_MS: u64 = 8_000;
pub const APPROACH_LEG_MS: u64 = 120_000;
const KBD_LOCS: [i32; 4] = [1765, 1816, 1817, 1766];
const TALK_ATTEMPTS: u32 = 3;

thread_local! {
    static ENTER_RUNTIMES: RefCell<HashMap<u64, EnterRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
    /// Frozen module-level `feePaidFor`: the site whose fee was paid on an
    /// attempt the entrance did not follow. It lives with the script, not
    /// with a session, so ResetSession keeps it; Stop drops it.
    static FEE_PAID_FOR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_fee_paid(key: Option<String>) {
    FEE_PAID_FOR.with(|paid| *paid.borrow_mut() = key);
}

#[derive(Clone, Debug)]
pub struct EnterChatLine {
    pub seq: i32,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct EnterNpc {
    pub index: i32,
    pub name: String,
    pub actions: Vec<String>,
    pub distance: i32,
}

#[derive(Clone, Debug)]
pub struct EnterLoc {
    pub id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
}

#[derive(Clone, Debug)]
pub struct EnterInv {
    pub id: i32,
    pub count: i32,
    pub slot: i32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct EnterObservation {
    pub here: Option<Tile>,
    pub ingame: bool,
    pub hold: bool,
    pub ours: bool,
    pub chat_continue: bool,
    pub chat_open: bool,
    pub chat_lines: Vec<EnterChatLine>,
    pub chat_options: Vec<String>,
    pub main_modal_id: i32,
    pub npcs: Vec<EnterNpc>,
    pub locs: Vec<EnterLoc>,
    pub inv: Vec<EnterInv>,
    pub tick: u64,
}

impl EnterObservation {
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
            chat_continue: latest.chat_continue().unwrap_or(empty.chat_continue),
            chat_open: latest.chat_open().unwrap_or(empty.chat_open),
            chat_lines: latest
                .chat_lines()
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| EnterChatLine {
                            seq: line.seq,
                            text: line.text.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            chat_options: latest.chat_options().cloned().unwrap_or_default(),
            main_modal_id: latest.main_modal_id().unwrap_or(empty.main_modal_id),
            npcs: latest
                .npcs()
                .map(|rows| {
                    rows.iter()
                        .map(|n| EnterNpc {
                            index: n.index,
                            name: n.name_or_empty().to_string(),
                            actions: observed::strings(&n.actions),
                            distance: n.distance,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            locs: latest
                .locs()
                .map(|rows| {
                    rows.iter()
                        .map(|loc| EnterLoc {
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
                        .map(|row| EnterInv {
                            id: row.id,
                            count: row.count,
                            slot: row.slot_or_unset(),
                            name: row.name_or_empty().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
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
                .chat_continue(self.chat_continue)
                .chat_open(self.chat_open)
                .chat_lines(
                    self.chat_lines
                        .into_iter()
                        .map(|line| observed::ChatLine {
                            seq: line.seq,
                            text: line.text.into(),
                        })
                        .collect(),
                )
                .chat_options(self.chat_options)
                .main_modal_id(self.main_modal_id)
                .npcs(
                    self.npcs
                        .into_iter()
                        .map(|n| EntityRow {
                            index: n.index,
                            name: Some(n.name.into()),
                            actions: observed::ops_of(&n.actions),
                            distance: n.distance,
                            ..EntityRow::default()
                        })
                        .collect(),
                )
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
                            slot: (row.slot != -1).then_some(row.slot),
                            name: Some(row.name.into()),
                            ..ItemRow::default()
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
            chat_continue: false,
            chat_open: false,
            chat_lines: Vec::new(),
            chat_options: Vec::new(),
            main_modal_id: -1,
            npcs: Vec::new(),
            locs: Vec::new(),
            inv: Vec::new(),
            tick: 1,
        }
    }
}

#[derive(Clone, Debug)]
struct TalkGate {
    npc: String,
    op: String,
    choose: String,
    stand: Tile,
}

#[derive(Clone, Debug)]
struct FeeGate {
    npc: String,
    op: String,
    coins: i32,
    stand: Tile,
    entrance_loc: i32,
    entrance_op: String,
    paid_line: String,
    prepaid_line: String,
}

#[derive(Clone, Debug)]
struct Gate {
    loc_id: i32,
    op: String,
    outside: Tile,
}

#[derive(Clone, Debug)]
pub(crate) struct EnterProj {
    parked: bool,
    shield_ready: bool,
    hp_fraction: f64,
    panic_hp: f64,
    key: String,
    area: Area,
    approach: Vec<Tile>,
    talk: Option<TalkGate>,
    fee: Option<FeeGate>,
    gate: Option<Gate>,
    key_item: Option<(i32, String)>,
    loc_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Undecided,
    Talk,
    Fee,
    Gateless,
    Keyed,
    Door,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    EmitStand,
    AckStand,
    AckTalk,
    Dialog,
    EmitPay,
    AckPay,
    PayLoop,
    Settle,
    AfterDelay,
    AckAct,
    Proof,
    Approach,
    AckApproach,
    YieldFalse,
    Aborted,
}

struct EnterRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    kind: Kind,
    /// The walk wait the current leg polls; its ack carries it once.
    walk_token: Option<u64>,
    stand: Option<Tile>,
    stand_radius: i32,
    leg: Option<Tile>,
    approach_cursor: i32,
    approach_started: bool,
    attempt: u32,
    quiet: u32,
    settle_i: u32,
    chat_mark: i32,
    coin_mark: i32,
    proved: bool,
}

impl EnterRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            phase: Phase::Start,
            kind: Kind::Undecided,
            walk_token: None,
            stand: None,
            stand_radius: 0,
            leg: None,
            approach_cursor: 0,
            approach_started: false,
            attempt: 0,
            quiet: 0,
            settle_i: 0,
            chat_mark: 0,
            coin_mark: 0,
            proved: false,
        }
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_value(&mut self, value: bool) -> Value {
        self.phase = Phase::Start;
        self.kind = Kind::Undecided;
        self.stand = None;
        self.leg = None;
        self.approach_started = false;
        self.attempt = 0;
        self.quiet = 0;
        self.settle_i = 0;
        self.proved = false;
        self.clock.deadline = None;
        self.emit(json!({ "kind": "yield", "value": value }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.phase = Phase::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }

    fn log(&mut self, message: String, then: Phase) -> Value {
        self.phase = then;
        self.emit(json!({ "kind": "log", "message": message }))
    }
}

fn observation() -> EnterObservation {
    observed::with(EnterObservation::from_scene)
}

/// Test seam: replace the isolate scene with posts that read back as `obs`.
pub fn set_observation(obs: EnterObservation) {
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

fn nearest_spot(from: Tile, spots: &[Tile]) -> i32 {
    let mut best = 0i32;
    let mut best_dist = i32::MAX;
    for (i, spot) in spots.iter().enumerate() {
        let dist = (spot.x - from.x).abs().max((spot.z - from.z).abs());
        if dist < best_dist {
            best_dist = dist;
            best = i as i32;
        }
    }
    best
}

fn in_area(proj: &EnterProj, obs: &EnterObservation) -> bool {
    obs.here.is_some_and(|here| proj.area.contains(here, 1))
}

fn coin_count(obs: &EnterObservation) -> i32 {
    obs.inv
        .iter()
        .filter(|row| row.name == "Coins")
        .map(|row| row.count)
        .sum()
}

fn has_item(obs: &EnterObservation, id: i32) -> bool {
    obs.inv.iter().any(|row| row.id == id && row.count > 0)
}

fn prepaid(_rt: &EnterRuntime, proj: &EnterProj) -> bool {
    fee_paid_for(&proj.key)
}

fn need_coins(rt: &EnterRuntime, proj: &EnterProj, obs: &EnterObservation) -> bool {
    proj.fee
        .as_ref()
        .is_some_and(|fee| !prepaid(rt, proj) && coin_count(obs) < fee.coins)
}

fn key_missing(proj: &EnterProj, obs: &EnterObservation) -> bool {
    proj.key_item
        .as_ref()
        .is_some_and(|(id, _)| !has_item(obs, *id))
}

fn is_kbd(proj: &EnterProj) -> bool {
    proj.key == "kbd-lair" || proj.loc_ids.iter().any(|id| KBD_LOCS.contains(id))
}

fn signal(obs: &EnterObservation) -> bool {
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

fn push_loc_id(ids: &mut Vec<i32>, v: Option<&Value>) {
    let Some(v) = v else {
        return;
    };
    if let Some(n) = v.as_i64() {
        ids.push(n as i32);
        return;
    }
    if let Some(n) = v.get("locId").and_then(Value::as_i64) {
        ids.push(n as i32);
    }
    if let Some(n) = v.get("id").and_then(Value::as_i64) {
        ids.push(n as i32);
    }
}

fn parse_proj(input: &Value) -> EnterProj {
    let talk = input.get("talkGate").and_then(|v| {
        if !v.is_object() {
            return None;
        }
        Some(TalkGate {
            npc: v
                .get("npc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            op: v
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("Talk-to")
                .to_string(),
            choose: v
                .get("choose")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            stand: parse_tile(v.get("stand")).unwrap_or(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
        })
    });
    let fee = input.get("feeGate").and_then(|v| {
        if !v.is_object() {
            return None;
        }
        let entrance = v.get("entrance");
        Some(FeeGate {
            npc: v
                .get("npc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            op: v
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("Pay")
                .to_string(),
            coins: i32_of(v.get("coins")),
            stand: parse_tile(v.get("stand")).unwrap_or(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
            entrance_loc: i32_of(entrance.and_then(|e| e.get("locId"))),
            entrance_op: entrance
                .and_then(|e| e.get("op"))
                .and_then(Value::as_str)
                .unwrap_or("Enter")
                .to_string(),
            paid_line: v
                .get("paidLine")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            prepaid_line: v
                .get("prepaidLine")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    });
    let gate = input.get("gate").and_then(|v| {
        if !v.is_object() {
            return None;
        }
        Some(Gate {
            loc_id: i32_of(v.get("locId")),
            op: v
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("Open")
                .to_string(),
            outside: parse_tile(v.get("outside")).unwrap_or(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
        })
    });
    let key_item = input.get("keyItem").and_then(|v| {
        if !v.is_object() {
            return None;
        }
        Some((
            i32_of(v.get("id")),
            v.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ))
    });
    let mut loc_ids = Vec::new();
    if let Some(route) = input.get("route").and_then(Value::as_array) {
        for stop in route {
            push_loc_id(&mut loc_ids, Some(stop));
        }
    }
    if let Some(approach) = input.get("approach").and_then(Value::as_array) {
        for stop in approach {
            push_loc_id(&mut loc_ids, Some(stop));
        }
    }
    if let Some(id) = gate.as_ref().map(|g| g.loc_id) {
        loc_ids.push(id);
    }
    if let Some(id) = fee.as_ref().map(|f| f.entrance_loc) {
        loc_ids.push(id);
    }
    let approach = input
        .get("approach")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| parse_tile(Some(row)))
                .collect()
        })
        .unwrap_or_default();
    EnterProj {
        parked: input
            .get("parked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        shield_ready: input
            .get("shieldReady")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hp_fraction: input
            .get("hpFraction")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
        panic_hp: input.get("panicHp").and_then(Value::as_f64).unwrap_or(0.2),
        key: input
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        area: Area::of(parse_boxes(input)),
        approach,
        talk,
        fee,
        gate,
        key_item,
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
        "walk-to" | "bank-open" | "attack" | "teleport" | "wait-fed-done"
    ) || matches!(op, "walk-to" | "bank-open" | "attack" | "teleport")
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

fn with_enter<T>(token: u64, f: impl FnOnce(&mut EnterRuntime) -> T) -> Option<T> {
    ENTER_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn validate_inner(rt: &EnterRuntime, proj: &EnterProj, obs: &EnterObservation) -> bool {
    if proj.parked || !proj.shield_ready || in_area(proj, obs) || proj.hp_fraction < proj.panic_hp {
        return false;
    }
    if need_coins(rt, proj, obs) || key_missing(proj, obs) {
        return false;
    }
    true
}

fn contains_ignore(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    hay.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn paid_now(rt: &EnterRuntime, proj: &EnterProj, obs: &EnterObservation) -> bool {
    let Some(fee) = proj.fee.as_ref() else {
        return false;
    };
    let line = obs.chat_lines.iter().any(|line| {
        line.seq > rt.chat_mark
            && (contains_ignore(&line.text, &fee.paid_line)
                || contains_ignore(&line.text, &fee.prepaid_line))
    });
    line || coin_count(obs) < rt.coin_mark
}

fn fragment_index(options: &[String], fragment: &str) -> Option<usize> {
    options.iter().position(|opt| opt.contains(fragment))
}

fn nearest_npc<'a>(obs: &'a EnterObservation, name: &str, action: &str) -> Option<&'a EnterNpc> {
    obs.npcs
        .iter()
        .filter(|n| {
            n.name.eq_ignore_ascii_case(name)
                && n.actions.iter().any(|a| a.eq_ignore_ascii_case(action))
        })
        .min_by_key(|n| n.distance)
}

fn loc_by_id(obs: &EnterObservation, id: i32, within: i32) -> Option<&EnterLoc> {
    obs.locs
        .iter()
        .filter(|loc| loc.id == id && loc.distance <= within)
        .min_by_key(|loc| loc.distance)
}

fn held_key(obs: &EnterObservation, id: i32) -> Option<&EnterInv> {
    obs.inv.iter().find(|row| row.id == id && row.count > 0)
}

fn start(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if in_area(proj, &obs) {
        return rt.yield_value(true);
    }
    if is_kbd(proj) {
        return rt.aborted("kbd-later");
    }
    if proj.talk.is_some() {
        rt.kind = Kind::Talk;
        rt.phase = Phase::EmitStand;
        let npc = proj
            .talk
            .as_ref()
            .map(|t| t.npc.as_str())
            .unwrap_or("guard");
        return rt.emit(json!({ "kind": "status", "message": format!("walking to the {npc}") }));
    }
    if let Some(fee) = proj.fee.clone() {
        rt.kind = Kind::Fee;
        if need_coins(rt, proj, &obs) {
            let held = coin_count(&obs);
            return rt.log(
                format!(
                    "the way in costs {} coins and the pack holds {held}. Banking for more.",
                    fee.coins
                ),
                Phase::YieldFalse,
            );
        }
        rt.phase = Phase::EmitStand;
        return rt.emit(json!({ "kind": "status", "message": format!("walking to {}", fee.npc) }));
    }
    if proj.gate.is_none() {
        rt.kind = Kind::Gateless;
        rt.phase = Phase::Approach;
        return rt.emit(json!({ "kind": "status", "message": "walking into the dungeon" }));
    }
    rt.kind = if proj.key_item.is_some() {
        Kind::Keyed
    } else {
        Kind::Door
    };
    rt.phase = Phase::EmitStand;
    rt.emit(json!({ "kind": "status", "message": "walking to the dungeon gate" }))
}

fn emit_stand(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let (tile, radius, near) = match rt.kind {
        Kind::Talk => (proj.talk.as_ref().map(|t| t.stand), 2, true),
        Kind::Fee => (proj.fee.as_ref().map(|f| f.stand), 2, true),
        Kind::Keyed | Kind::Door => (proj.gate.as_ref().map(|g| g.outside), 0, false),
        _ => return rt.aborted("no stand"),
    };
    let Some(tile) = tile else {
        return rt.aborted("no stand");
    };
    rt.stand = Some(tile);
    rt.walk_token = None;
    rt.stand_radius = radius;
    rt.clock.arm(STAND_MS);
    rt.phase = Phase::AckStand;
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

fn arrived(here: Option<Tile>, dest: Tile, radius: i32) -> bool {
    here.is_some_and(|h| distance_to(dest, h) <= radius)
}

fn ack_stand(rt: &mut EnterRuntime, proj: &EnterProj, reply: Option<&Value>) -> Value {
    if let Some(token) = reply_u64(reply, "walkToken") {
        rt.walk_token = Some(token);
    }
    let Some(walk_token) = rt.walk_token else {
        return rt.aborted("missing walkToken");
    };
    let obs = observation();
    if signal(&obs) {
        return rt.yield_value(false);
    }
    let dest = rt.stand.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let radius = rt.stand_radius;
    if arrived(obs.here, dest, radius) {
        return after_stand(rt, proj);
    }
    if rt.clock.bound_reached() || walk_wait_settled(walk_token) {
        return rt.yield_value(false);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn after_stand(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    match rt.kind {
        Kind::Talk => talk_attempt(rt, proj),
        Kind::Fee => {
            if prepaid(rt, proj) {
                let npc = proj
                    .fee
                    .as_ref()
                    .map(|f| f.npc.as_str())
                    .unwrap_or("doorman");
                rt.phase = Phase::Settle;
                rt.quiet = 0;
                rt.settle_i = 0;
                return rt.emit(json!({
                    "kind": "log",
                    "message": format!("{npc} is already paid from the last attempt, going straight to the entrance"),
                }));
            }
            rt.phase = Phase::EmitPay;
            let npc = proj
                .fee
                .as_ref()
                .map(|f| f.npc.as_str())
                .unwrap_or("doorman");
            rt.emit(json!({ "kind": "status", "message": format!("paying {npc}") }))
        }
        Kind::Keyed | Kind::Door => {
            rt.phase = Phase::AfterDelay;
            rt.emit(json!({ "kind": "delay-ticks", "n": 2 }))
        }
        _ => rt.aborted("no stand arm"),
    }
}

fn talk_attempt(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if in_area(proj, &obs) {
        return proved(rt, proj);
    }
    if rt.attempt >= TALK_ATTEMPTS {
        return watch_tower(rt, proj);
    }
    let Some(talk) = proj.talk.clone() else {
        return rt.yield_value(false);
    };
    rt.attempt += 1;
    if let Some(npc) = nearest_npc(&obs, &talk.npc, &talk.op) {
        rt.phase = Phase::AckTalk;
        return rt.emit(json!({
            "kind": "npc",
            "name": npc.name,
            "action": talk.op,
            "index": npc.index,
        }));
    }
    rt.log(
        format!("no {} in the scene to talk past. Retrying.", talk.npc),
        Phase::AckStand,
    )
}

fn watch_tower(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let npc = proj
        .talk
        .as_ref()
        .map(|t| t.npc.as_str())
        .unwrap_or("guard");
    rt.log(
        format!("the {npc} did not let us past. It needs Watch Tower complete. Retrying."),
        Phase::YieldFalse,
    )
}

fn dialog_step(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if signal(&obs) {
        return rt.yield_value(false);
    }
    let choose = proj
        .talk
        .as_ref()
        .map(|t| t.choose.clone())
        .unwrap_or_default();
    if !obs.chat_options.is_empty() {
        if let Some(idx) = fragment_index(&obs.chat_options, &choose) {
            rt.phase = Phase::Proof;
            rt.clock.arm(DOOR_MS);
            return rt.emit(json!({ "kind": "answer", "option": idx as i32 + 1 }));
        }
        return watch_tower(rt, proj);
    }
    if obs.chat_continue {
        return rt.emit(json!({ "kind": "continue" }));
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn emit_pay(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    let Some(fee) = proj.fee.clone() else {
        return rt.yield_value(false);
    };
    let Some(npc) = nearest_npc(&obs, &fee.npc, &fee.op) else {
        return rt.log(
            format!("no {} in the scene to pay. Retrying.", fee.npc),
            Phase::YieldFalse,
        );
    };
    rt.chat_mark = obs.chat_lines.iter().map(|l| l.seq).max().unwrap_or(0);
    rt.coin_mark = coin_count(&obs);
    rt.clock.arm(PAY_MS);
    rt.phase = Phase::AckPay;
    rt.emit(json!({
        "kind": "npc",
        "name": npc.name,
        "action": fee.op,
        "index": npc.index,
    }))
}

fn pay_step(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if signal(&obs) {
        return rt.yield_value(false);
    }
    if paid_now(rt, proj, &obs) {
        set_fee_paid(Some(proj.key.clone()));
        rt.phase = Phase::Settle;
        rt.quiet = 0;
        rt.settle_i = 0;
        let fee = proj.fee.as_ref();
        let npc = fee.map(|f| f.npc.as_str()).unwrap_or("doorman");
        let prepaid_line = fee.map(|f| f.prepaid_line.as_str()).unwrap_or("");
        let coins = fee.map(|f| f.coins).unwrap_or(0);
        let message = if obs
            .chat_lines
            .iter()
            .any(|line| line.seq > rt.chat_mark && contains_ignore(&line.text, prepaid_line))
        {
            format!("{npc} says the fee is already paid")
        } else {
            format!("paid {npc} {coins} coins")
        };
        return rt.emit(json!({ "kind": "log", "message": message }));
    }
    if rt.clock.bound_reached() {
        let npc = proj
            .fee
            .as_ref()
            .map(|f| f.npc.as_str())
            .unwrap_or("doorman");
        return rt.log(
            format!("{npc} took no payment. Retrying."),
            Phase::YieldFalse,
        );
    }
    if obs.chat_continue {
        return rt.emit(json!({ "kind": "continue" }));
    }
    if obs.main_modal_id != -1 {
        return rt.emit(json!({ "kind": "close-modal" }));
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn settle_step(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if signal(&obs) {
        return rt.yield_value(false);
    }
    rt.settle_i = rt.settle_i.saturating_add(1);
    if obs.chat_continue {
        rt.quiet = 0;
        return rt.emit(json!({ "kind": "continue" }));
    }
    if obs.main_modal_id != -1 {
        rt.quiet = 0;
        return rt.emit(json!({ "kind": "close-modal" }));
    }
    if obs.chat_open {
        rt.quiet = 0;
        return rt.emit(json!({ "kind": "delay-ticks", "n": 1 }));
    }
    rt.quiet = rt.quiet.saturating_add(1);
    if rt.quiet >= 3 || rt.settle_i >= 40 {
        return emit_entrance(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn emit_entrance(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let Some(fee) = proj.fee.clone() else {
        return rt.yield_value(false);
    };
    let obs = observation();
    let Some(door) = loc_by_id(&obs, fee.entrance_loc, 8) else {
        return rt.log(
            "the dungeon entrance is not in the scene yet. Retrying.".into(),
            Phase::YieldFalse,
        );
    };
    rt.phase = Phase::AckAct;
    rt.clock.arm(DOOR_MS);
    rt.emit(json!({
        "kind": "loc",
        "x": door.x,
        "z": door.z,
        "level": door.level,
        "action": fee.entrance_op,
        "id": door.id,
    }))
}

fn after_delay(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let Some(gate) = proj.gate.clone() else {
        return rt.yield_value(false);
    };
    let obs = observation();
    let door = loc_by_id(&obs, gate.loc_id, 5);
    let key = proj
        .key_item
        .as_ref()
        .and_then(|(id, _)| held_key(&obs, *id));
    if door.is_none() || (proj.key_item.is_some() && key.is_none()) {
        return rt.log(
            "the gate is not in the scene yet, or the key is gone. Retrying.".into(),
            Phase::YieldFalse,
        );
    }
    let door = door.unwrap();
    rt.phase = Phase::AckAct;
    rt.clock.arm(DOOR_MS);
    if let Some(key) = key {
        rt.emit(json!({
            "kind": "use-on",
            "name": key.name,
            "id": key.id,
            "slot": key.slot,
            "x": door.x,
            "z": door.z,
            "level": door.level,
        }))
    } else {
        rt.emit(json!({
            "kind": "loc",
            "x": door.x,
            "z": door.z,
            "level": door.level,
            "action": gate.op,
            "id": door.id,
        }))
    }
}

fn proof_step(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if signal(&obs) {
        return rt.yield_value(false);
    }
    if in_area(proj, &obs) {
        return proved(rt, proj);
    }
    if rt.clock.bound_reached() {
        return proof_failed(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn proof_failed(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    if rt.kind == Kind::Talk && rt.attempt < TALK_ATTEMPTS {
        rt.phase = Phase::AckStand;
        return talk_attempt(rt, proj);
    }
    let message = match rt.kind {
        Kind::Fee => "the entrance did not let us through. Retrying.".to_string(),
        Kind::Talk => {
            let npc = proj
                .talk
                .as_ref()
                .map(|t| t.npc.as_str())
                .unwrap_or("guard");
            format!("the {npc} did not let us past. It needs Watch Tower complete. Retrying.")
        }
        _ => "the gate did not let us through. Retrying.".to_string(),
    };
    rt.log(message, Phase::YieldFalse)
}

fn proved(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    rt.proved = true;
    if rt.kind == Kind::Fee {
        set_fee_paid(None);
    }
    let message = match rt.kind {
        Kind::Talk => {
            let npc = proj
                .talk
                .as_ref()
                .map(|t| t.npc.as_str())
                .unwrap_or("guard");
            format!("the {npc} let us past")
        }
        Kind::Fee => "inside the dungeon".to_string(),
        _ => "inside the dragon lair".to_string(),
    };
    rt.phase = Phase::Approach;
    rt.approach_started = false;
    rt.emit(json!({ "kind": "log", "message": message }))
}

fn approach_pick(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if signal(&obs) && !rt.proved {
        return rt.yield_value(false);
    }
    if proj.approach.is_empty() || obs.here.is_none() {
        return after_approach(rt, proj);
    }
    if !rt.approach_started {
        rt.approach_started = true;
        rt.approach_cursor = nearest_spot(
            obs.here.unwrap_or(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
            &proj.approach,
        );
    }
    while let Some(stop) = usize::try_from(rt.approach_cursor)
        .ok()
        .and_then(|i| proj.approach.get(i).copied())
    {
        if obs.here.is_some_and(|here| distance_to(stop, here) <= 1) {
            rt.approach_cursor += 1;
            continue;
        }
        rt.leg = Some(stop);
        rt.walk_token = None;
        rt.clock.arm(APPROACH_LEG_MS);
        rt.phase = Phase::AckApproach;
        return rt.emit(json!({
            "kind": "walk",
            "x": stop.x,
            "z": stop.z,
            "level": stop.level,
        }));
    }
    after_approach(rt, proj)
}

fn ack_approach(rt: &mut EnterRuntime, proj: &EnterProj, reply: Option<&Value>) -> Value {
    if let Some(token) = reply_u64(reply, "walkToken") {
        rt.walk_token = Some(token);
    }
    let Some(walk_token) = rt.walk_token else {
        return rt.aborted("missing walkToken");
    };
    let obs = observation();
    if signal(&obs) && !rt.proved {
        return rt.yield_value(false);
    }
    let leg = rt.leg.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let done =
        arrived(obs.here, leg, 1) || walk_wait_settled(walk_token) || rt.clock.bound_reached();
    if done {
        rt.approach_cursor += 1;
        rt.phase = Phase::Approach;
        return approach_pick(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn after_approach(rt: &mut EnterRuntime, proj: &EnterProj) -> Value {
    let obs = observation();
    if rt.kind == Kind::Gateless {
        if in_area(proj, &obs) {
            return rt.log("inside the dragon lair".into(), Phase::Start);
        }
        return rt.log(
            "the walk in did not reach the dungeon. Retrying.".into(),
            Phase::YieldFalse,
        );
    }
    if rt.proved || in_area(proj, &obs) {
        return rt.yield_value(true);
    }
    rt.yield_value(false)
}

fn next_effect(rt: &mut EnterRuntime, proj: &EnterProj, reply: Option<&Value>) -> Value {
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
        Phase::Start => start(rt, proj),
        Phase::EmitStand => emit_stand(rt, proj),
        Phase::AckStand => ack_stand(rt, proj, reply),
        Phase::AckTalk => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !queued {
                return talk_attempt(rt, proj);
            }
            rt.phase = Phase::Dialog;
            dialog_step(rt, proj)
        }
        Phase::Dialog => dialog_step(rt, proj),
        Phase::EmitPay => emit_pay(rt, proj),
        Phase::AckPay => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !queued {
                return rt.yield_value(false);
            }
            rt.phase = Phase::PayLoop;
            pay_step(rt, proj)
        }
        Phase::PayLoop => pay_step(rt, proj),
        Phase::Settle => settle_step(rt, proj),
        Phase::AfterDelay => after_delay(rt, proj),
        Phase::AckAct => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !queued {
                return rt.yield_value(false);
            }
            rt.phase = Phase::Proof;
            if rt.clock.deadline.is_none() {
                rt.clock.arm(DOOR_MS);
            }
            proof_step(rt, proj)
        }
        Phase::Proof => proof_step(rt, proj),
        Phase::Approach => approach_pick(rt, proj),
        Phase::AckApproach => ack_approach(rt, proj, reply),
        Phase::YieldFalse => rt.yield_value(false),
        Phase::Aborted => rt.aborted("aborted"),
    }
}

fn each_runtime(f: impl Fn(&mut EnterRuntime)) {
    ENTER_RUNTIMES.with(|m| {
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
    ENTER_RUNTIMES.with(|m| m.borrow_mut().clear());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => {
            let token = alloc_token();
            ENTER_RUNTIMES.with(|m| {
                m.borrow_mut().insert(token, EnterRuntime::new(token));
            });
            json!({ "kind": "started", "token": token })
        }
        "validate" => {
            let token = token_of(input);
            let proj = parse_proj(input);
            let obs = observation();
            match with_enter(
                token,
                |rt| json!({ "value": validate_inner(rt, &proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_proj(input);
            let reply = input.get("reply");
            match with_enter(token, |rt| next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

/// Frozen `feePrepaid(site)`.
pub fn fee_paid_for(key: &str) -> bool {
    FEE_PAID_FOR.with(|paid| paid.borrow().as_deref() == Some(key))
}

/// Stop: the fee proof goes with the script.
pub fn on_stop() {
    set_fee_paid(None);
}

pub fn enter_token_alive(token: u64) -> bool {
    ENTER_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

pub fn enter_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_enter(token, |rt| {
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

pub fn enter_force_bound_reached(token: u64) -> bool {
    with_enter(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

/// `EnterLair` / `enterLair`.
pub(crate) struct Enter;

impl HuntKind for Enter {
    const NAME: &'static str = "hunt-enter";
    const SESSION: bool = true;
    const BOOLEAN: bool = true;
    type Proj = EnterProj;

    fn parse(site: &Value) -> EnterProj {
        parse_proj(site)
    }

    fn area(proj: &mut EnterProj) -> &mut Area {
        &mut proj.area
    }

    /// The old projection's reads; absent hooks keep its defaults.
    fn refresh(proj: &mut EnterProj, host: &mut dyn Host) -> Result<(), Ended> {
        proj.parked = strict_true(host, hook::PARKED)?;
        proj.shield_ready = strict_true(host, hook::SHIELD_READY)?;
        proj.hp_fraction = number(host, hook::HP_FRACTION, 1.0)?;
        proj.panic_hp = number(host, hook::PANIC_HP, 0.2)?;
        Ok(())
    }

    fn mint() -> u64 {
        let token = alloc_token();
        ENTER_RUNTIMES.with(|m| m.borrow_mut().insert(token, EnterRuntime::new(token)));
        token
    }

    fn ensure(token: u64) {
        ENTER_RUNTIMES.with(|m| {
            m.borrow_mut()
                .entry(token)
                .or_insert_with(|| EnterRuntime::new(token));
        });
    }

    fn renew(token: u64) {
        ENTER_RUNTIMES.with(|m| m.borrow_mut().insert(token, EnterRuntime::new(token)));
    }

    fn next(token: u64, proj: &EnterProj, reply: Option<&Value>) -> Value {
        with_enter(token, |rt| next_effect(rt, proj, reply))
            .unwrap_or_else(|| json!({ "kind": "aborted", "reason": "unknown token" }))
    }

    fn end(token: u64) {
        ENTER_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
    }

    fn validate(token: u64, proj: &EnterProj) -> bool {
        let obs = observation();
        with_enter(token, |rt| validate_inner(rt, proj, &obs)).unwrap_or(false)
    }
}

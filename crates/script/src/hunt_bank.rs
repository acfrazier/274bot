//! Rust-owned Bank isolate step machine.
//!
//! JS marshals the site row and dispatches the effect table. The kept order,
//! key arm, and completion stay here. Own map and token counter. Do not call
//! the other hunt machines. A supplied leave is a continuation, not lever ops.

use crate::hunt_fight::{in_area_body, SiteBox, Tile};
use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;

pub const WITHDRAW_MS: u64 = 2_500;
pub const CLOSE_MS: u64 = 3_000;
pub const OPEN_MS: u64 = 5_000;
pub const WALK_LEG_MS: u64 = 300_000;
pub const APPROACH_RADIUS: i32 = 3;

const SHIELD: &str = "Dragonfire shield";
const HEAL_BITES: u32 = 24;
const HEAL_MISSES: u32 = 3;
const FOOD_GUARD: u32 = 12;

thread_local! {
    static BANK_RUNTIMES: RefCell<HashMap<u64, BankRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
    static OBSERVATION: RefCell<BankObservation> = RefCell::new(BankObservation::empty());
}

#[derive(Clone, Debug)]
pub struct BankRow {
    pub id: i32,
    pub count: i32,
    pub name: String,
    pub slot: i32,
    pub has_slot: bool,
    pub ops: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BankObservation {
    pub here: Option<Tile>,
    pub ingame: bool,
    pub hold: bool,
    pub ours: bool,
    pub inv: Vec<BankRow>,
    pub inv_size: i32,
    pub equipment: Vec<BankRow>,
    pub bank: Vec<BankRow>,
    pub bank_side: Vec<BankRow>,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    pub bank_op_result_seq: u64,
    pub bank_op_result: bool,
    pub withdraw_x_result_seq: u64,
    pub withdraw_x_result: bool,
    pub hp_base: i32,
    pub hp_effective: i32,
}

impl BankObservation {
    pub fn empty() -> Self {
        Self {
            here: None,
            ingame: true,
            hold: false,
            ours: false,
            inv: Vec::new(),
            inv_size: 28,
            equipment: Vec::new(),
            bank: Vec::new(),
            bank_side: Vec::new(),
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            bank_op_result_seq: 0,
            bank_op_result: false,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            hp_base: 99,
            hp_effective: 99,
        }
    }
}

#[derive(Clone, Debug)]
struct KeyItem {
    id: i32,
    name: String,
}

#[derive(Clone, Debug)]
struct NamedCount {
    name: String,
    count: i32,
}

#[derive(Clone, Debug)]
struct FlaskPlan {
    name: String,
    want: i32,
    doses: Vec<String>,
}

#[derive(Clone, Debug)]
struct BankProj {
    bank: Option<Tile>,
    key_item: Option<KeyItem>,
    boxes: Vec<SiteBox>,
    coins: Option<i32>,
    fire_at_range: bool,
    withdraw_food: bool,
    heal_to: f64,
    wear: Vec<String>,
    carry: Vec<String>,
    has_pick_weapon: bool,
    food_name: String,
    food_want: i32,
    style: String,
    weapon: String,
    ammo: String,
    ammo_want: i32,
    spell: String,
    keep_extra: Vec<String>,
    runes: Vec<NamedCount>,
    escape_runes: Vec<NamedCount>,
    flasks: Vec<FlaskPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyArm {
    Skip,
    Held,
    Bank,
    Fetch,
    Opaque,
    Nameless,
}

#[derive(Clone, Debug)]
struct Plan {
    key: KeyArm,
    key_id: i32,
    key_name: String,
    shield_seen: bool,
    needs_shield: bool,
    food_seen: bool,
    food_owed: bool,
    junk: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Decide,
    AckLeave,
    AckWalk,
    AckOpen,
    Work,
    WaitOp,
    WaitHeal,
    AckLog,
    AckPick,
    AckPark,
    AckCount,
    Done,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Pick,
    Deposit,
    SlotFree,
    Food,
    Key,
    Gear,
    Coins,
    Style,
    Escape,
    Flasks,
    Wear,
    Heal,
    Topup,
    Close,
    Count,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpKind {
    Deposit,
    SlotFree,
    Withdraw,
    WithdrawX,
    Wear,
    CloseWear,
    CloseHeal,
    CloseFinal,
}

#[derive(Clone, Debug)]
struct Pending {
    kind: OpKind,
    name: String,
    id: i32,
    fail_trip: bool,
    before_inv: i32,
    before_bank: i32,
    before_side: i32,
    before_used: usize,
    before_hp: i32,
    seq: u64,
    x_seq: u64,
    generation: u64,
}

struct BankRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    stage: Stage,
    left: bool,
    plan: Option<Plan>,
    walk_token: Option<u64>,
    walk_dest: Option<Tile>,
    open_acked: bool,
    topup_open: bool,
    in_topup: bool,
    click_acked: bool,
    pending: Option<Pending>,
    deposit_names: Vec<String>,
    deposit_i: usize,
    deposits_ready: bool,
    slot_free_done: bool,
    food_done: bool,
    food_rounds: u32,
    food_warned: bool,
    key_done: bool,
    gear_names: Vec<String>,
    gear_i: usize,
    gear_ready: bool,
    coins_done: bool,
    style_i: usize,
    style_ready: bool,
    escape_i: usize,
    escape_ready: bool,
    flask_i: usize,
    flask_ready: bool,
    wear_names: Vec<String>,
    wear_i: usize,
    wear_ready: bool,
    wear_owed: Vec<String>,
    wear_refused: Vec<String>,
    shield_wear_refused: bool,
    heal_done: bool,
    heal_attempts: u32,
    heal_misses: u32,
    heal_ticks: u32,
    bite_landed: bool,
    topup_done: bool,
    counted: bool,
    yielded: Option<bool>,
    eat_name: String,
    eat_id: i32,
}

impl BankRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            phase: Phase::Decide,
            stage: Stage::Pick,
            left: false,
            plan: None,
            walk_token: None,
            walk_dest: None,
            open_acked: false,
            topup_open: false,
            in_topup: false,
            click_acked: false,
            pending: None,
            deposit_names: Vec::new(),
            deposit_i: 0,
            deposits_ready: false,
            slot_free_done: false,
            food_done: false,
            food_rounds: 0,
            food_warned: false,
            key_done: false,
            gear_names: Vec::new(),
            gear_i: 0,
            gear_ready: false,
            coins_done: false,
            style_i: 0,
            style_ready: false,
            escape_i: 0,
            escape_ready: false,
            flask_i: 0,
            flask_ready: false,
            wear_names: Vec::new(),
            wear_i: 0,
            wear_ready: false,
            wear_owed: Vec::new(),
            wear_refused: Vec::new(),
            shield_wear_refused: false,
            heal_done: false,
            heal_attempts: 0,
            heal_misses: 0,
            heal_ticks: 0,
            bite_landed: false,
            topup_done: false,
            counted: false,
            yielded: None,
            eat_name: String::new(),
            eat_id: 0,
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

fn observation() -> BankObservation {
    OBSERVATION.with(|o| o.borrow().clone())
}

pub fn set_observation(obs: BankObservation) {
    OBSERVATION.with(|o| *o.borrow_mut() = obs);
}

fn live_dist(here: Tile, tile: Tile) -> i32 {
    if here.level != tile.level {
        return 1_000_000;
    }
    (here.x - tile.x).abs().max((here.z - tile.z).abs())
}

fn real_slot(row: &BankRow) -> bool {
    row.has_slot && row.slot >= 0
}

fn eq_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn food_forms(food: &str) -> Vec<String> {
    let key = food.trim().to_lowercase();
    let forms: &[&str] = match key.as_str() {
        "cake" => &["cake", "2/3 cake", "slice of cake"],
        "chocolate cake" => &["chocolate cake", "2/3 chocolate cake", "chocolate slice"],
        "plain pizza" => &["plain pizza", "1/2 plain pizza"],
        "meat pizza" => &["meat pizza", "1/2 meat pizza"],
        "anchovy pizza" => &["anchovy pizza", "1/2 anchovy pizza"],
        "pineapple pizza" => &["pineapple pizza", "1/2 pineapple pizza"],
        "redberry pie" => &["redberry pie", "half a redberry pie"],
        "meat pie" => &["meat pie", "half a meat pie"],
        "apple pie" => &["apple pie", "half an apple pie"],
        _ => return vec![key],
    };
    forms.iter().map(|s| (*s).to_string()).collect()
}

fn is_food_name(name: &str, forms: &[String]) -> bool {
    forms.iter().any(|form| eq_name(form, name))
}

fn needs_shield(proj: &BankProj) -> bool {
    proj.style.eq_ignore_ascii_case("melee") || proj.fire_at_range
}

fn in_lair(proj: &BankProj, obs: &BankObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    in_area_body(here, 1, &proj.boxes)
}

fn near_bank(obs: &BankObservation, bank: Tile) -> bool {
    obs.here
        .is_some_and(|here| live_dist(here, bank) <= APPROACH_RADIUS)
}

fn slotted<'a>(obs: &'a BankObservation) -> impl Iterator<Item = &'a BankRow> {
    obs.inv.iter().filter(|row| real_slot(row) && row.count > 0)
}

fn count_id(rows: &[BankRow], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.id == id && row.count > 0)
        .map(|row| row.count)
        .sum()
}

fn slotted_count_id(obs: &BankObservation, id: i32) -> i32 {
    slotted(obs)
        .filter(|row| row.id == id)
        .map(|row| row.count)
        .sum()
}

fn slotted_has_id(obs: &BankObservation, id: i32) -> bool {
    slotted(obs).any(|row| row.id == id)
}

fn name_count(rows: &[BankRow], name: &str) -> i32 {
    rows.iter()
        .filter(|row| row.count > 0 && eq_name(&row.name, name))
        .map(|row| row.count)
        .sum()
}

fn slotted_has_name(obs: &BankObservation, name: &str) -> bool {
    slotted(obs).any(|row| eq_name(&row.name, name))
}

fn equip_has_name(obs: &BankObservation, name: &str) -> bool {
    obs.equipment
        .iter()
        .any(|row| row.count > 0 && eq_name(&row.name, name))
}

fn held_name(obs: &BankObservation, name: &str) -> bool {
    equip_has_name(obs, name)
        || obs
            .bank_side
            .iter()
            .any(|row| row.count > 0 && eq_name(&row.name, name))
        || slotted_has_name(obs, name)
}

fn food_count(obs: &BankObservation, forms: &[String]) -> i32 {
    slotted(obs)
        .filter(|row| is_food_name(&row.name, forms))
        .map(|row| row.count.max(1))
        .sum()
}

fn pack_used(obs: &BankObservation) -> usize {
    if obs.bank_open {
        obs.bank_side
            .iter()
            .filter(|row| row.count > 0 && !row.name.is_empty())
            .count()
    } else {
        slotted(obs).count()
    }
}

fn pack_full(obs: &BankObservation) -> bool {
    obs.inv_size > 0 && pack_used(obs) >= obs.inv_size as usize
}

fn inv_empty(obs: &BankObservation) -> bool {
    !obs.inv.iter().any(real_slot)
}

fn can_wear(row: &BankRow) -> bool {
    row.ops.iter().any(|op| {
        let n = op.to_ascii_lowercase();
        n.contains("wear") || n.contains("wield")
    })
}

fn find_named<'a>(rows: &'a [BankRow], name: &str) -> Option<&'a BankRow> {
    rows.iter()
        .find(|row| row.count > 0 && eq_name(&row.name, name) && !row.name.is_empty())
}

fn i32_of(v: Option<&Value>) -> i32 {
    v.and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn strings(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn named_counts(input: &Value, key: &str) -> Vec<NamedCount> {
    input
        .get(key)
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let name = row
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| row.get("rune").and_then(Value::as_str))?;
                    let count = row
                        .get("count")
                        .and_then(Value::as_i64)
                        .and_then(|n| i32::try_from(n).ok())
                        .unwrap_or(0);
                    Some(NamedCount {
                        name: name.to_string(),
                        count,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_flasks(input: &Value) -> Vec<FlaskPlan> {
    input
        .get("flasks")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let name = row
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| row.get("flask").and_then(Value::as_str))?;
                    let want = row
                        .get("want")
                        .and_then(Value::as_i64)
                        .and_then(|n| i32::try_from(n).ok())
                        .unwrap_or(0);
                    Some(FlaskPlan {
                        name: name.to_string(),
                        want,
                        doses: strings(row, "doses"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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

fn parse_tile(v: Option<&Value>) -> Option<Tile> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    Some(Tile {
        x: i32_of(v.get("x")),
        z: i32_of(v.get("z")),
        level: i32_of(v.get("level")),
    })
}

fn parse_proj(input: &Value) -> BankProj {
    let key_item = match input.get("keyItem") {
        None | Some(Value::Null) => None,
        Some(item) => Some(KeyItem {
            id: i32_of(item.get("id")),
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
    };
    let heal_to = input.get("healTo").and_then(Value::as_f64).unwrap_or(0.9);
    let coins = input
        .get("coins")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok());
    BankProj {
        bank: parse_tile(input.get("bank")),
        key_item,
        boxes: parse_boxes(input),
        coins,
        fire_at_range: input.get("fireAtRange").and_then(Value::as_bool) == Some(true),
        withdraw_food: input.get("withdrawFood").and_then(Value::as_bool) == Some(true),
        heal_to,
        wear: strings(input, "wear"),
        carry: strings(input, "carry"),
        has_pick_weapon: input.get("hasPickWeapon").and_then(Value::as_bool) == Some(true),
        food_name: input
            .get("foodName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        food_want: i32_of(input.get("foodWant")),
        style: input
            .get("style")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        weapon: input
            .get("weapon")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        ammo: input
            .get("ammo")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        ammo_want: input
            .get("ammoWant")
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(500),
        spell: input
            .get("spell")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        keep_extra: strings(input, "keepExtra"),
        runes: named_counts(input, "runes"),
        escape_runes: named_counts(input, "escapeRunes"),
        flasks: parse_flasks(input),
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

fn bad_reply(reply: Option<&Value>) -> bool {
    matches!(
        reply.and_then(|r| r.get("kind")).and_then(Value::as_str),
        Some("npc" | "obj" | "loc" | "use-on" | "teleport" | "attack" | "eat" | "fight")
    )
}

fn alloc_token() -> u64 {
    NEXT_TOKEN.with(|n| {
        let mut n = n.borrow_mut();
        let t = *n;
        *n = t.wrapping_add(1).max(1);
        t
    })
}

fn with_bank<T>(token: u64, f: impl FnOnce(&mut BankRuntime) -> T) -> Option<T> {
    BANK_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn walk_wait_settled(token: u64) -> bool {
    crate::walk_wait::dispatch(&json!({ "op": "settled", "token": token }))
        .as_bool()
        .unwrap_or(false)
}

fn classify_key(obs: &BankObservation, item: &KeyItem) -> KeyArm {
    let on_side = obs
        .bank_side
        .iter()
        .any(|row| row.id == item.id && row.count > 0);
    let on_inv = slotted(obs).any(|row| row.id == item.id);
    if on_side || on_inv {
        return KeyArm::Held;
    }
    let fallback = obs
        .inv
        .iter()
        .any(|row| row.id == item.id && row.count > 0 && !real_slot(row));
    if let Some(row) = obs
        .bank
        .iter()
        .find(|row| row.id == item.id && row.count > 0)
    {
        if row.name.trim().is_empty() {
            return KeyArm::Nameless;
        }
        return KeyArm::Bank;
    }
    if fallback {
        KeyArm::Opaque
    } else {
        KeyArm::Fetch
    }
}

fn shield_seen(obs: &BankObservation) -> bool {
    equip_has_name(obs, SHIELD)
        || obs
            .bank_side
            .iter()
            .any(|row| row.count > 0 && eq_name(&row.name, SHIELD))
        || obs
            .bank
            .iter()
            .any(|row| row.count > 0 && eq_name(&row.name, SHIELD))
        || slotted_has_name(obs, SHIELD)
}

fn food_seen(obs: &BankObservation, forms: &[String]) -> bool {
    obs.bank
        .iter()
        .chain(obs.bank_side.iter())
        .chain(slotted(obs))
        .any(|row| row.count > 0 && is_food_name(&row.name, forms))
}

fn keep_names(proj: &BankProj, obs: &BankObservation) -> Vec<String> {
    let mut keep = food_forms(&proj.food_name);
    if needs_shield(proj) {
        keep.push(SHIELD.to_string());
    }
    if let Some(item) = &proj.key_item {
        if !item.name.is_empty() {
            keep.push(item.name.clone());
        }
        for row in obs
            .bank
            .iter()
            .chain(obs.bank_side.iter())
            .chain(obs.inv.iter())
        {
            if row.id == item.id && !row.name.is_empty() {
                keep.push(row.name.clone());
            }
        }
    }
    if proj.coins.is_some() {
        keep.push("Coins".to_string());
    }
    if !proj.weapon.is_empty() {
        keep.push(proj.weapon.clone());
    }
    if proj.style.eq_ignore_ascii_case("range") && !proj.ammo.is_empty() {
        keep.push(proj.ammo.clone());
    }
    if proj.style.eq_ignore_ascii_case("mage") {
        for rune in &proj.runes {
            keep.push(rune.name.clone());
        }
    }
    for rune in &proj.escape_runes {
        keep.push(rune.name.clone());
    }
    keep.extend(proj.keep_extra.iter().cloned());
    for name in proj.wear.iter().chain(proj.carry.iter()) {
        if !name.is_empty() {
            keep.push(name.clone());
        }
    }
    for flask in &proj.flasks {
        keep.push(flask.name.clone());
        keep.extend(flask.doses.iter().cloned());
    }
    keep
}

fn kept(keep: &[String], name: &str) -> bool {
    keep.iter().any(|item| eq_name(item, name))
}

fn capture_plan(proj: &BankProj, obs: &BankObservation) -> Plan {
    let forms = food_forms(&proj.food_name);
    let key = match &proj.key_item {
        None => KeyArm::Skip,
        Some(item) => classify_key(obs, item),
    };
    let keep = keep_names(proj, obs);
    let mut junk = Vec::new();
    for row in &obs.bank_side {
        if row.name.is_empty() || row.count <= 0 {
            continue;
        }
        if !kept(&keep, &row.name) && !junk.iter().any(|name: &String| eq_name(name, &row.name)) {
            junk.push(row.name.clone());
        }
    }
    let seen_food = food_seen(obs, &forms);
    Plan {
        key,
        key_id: proj.key_item.as_ref().map(|item| item.id).unwrap_or(0),
        key_name: proj
            .key_item
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        shield_seen: shield_seen(obs),
        needs_shield: needs_shield(proj),
        food_seen: seen_food,
        food_owed: proj.withdraw_food && seen_food,
        junk,
    }
}

fn gate(rt: &mut BankRuntime, obs: &BankObservation) -> Option<Value> {
    if obs.hold || obs.ours || !obs.ingame {
        return Some(rt.yield_value(false));
    }
    None
}

fn emit_walk(rt: &mut BankRuntime, tile: Tile) -> Value {
    rt.walk_dest = Some(tile);
    rt.walk_token = None;
    rt.phase = Phase::AckWalk;
    rt.clock.arm(WALK_LEG_MS);
    rt.emit(json!({
        "kind": "walk-near",
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
        "radius": APPROACH_RADIUS,
        "allow_teleports": false,
        "allow_wilderness": false,
        "allow_bank_fetch": false,
    }))
}

fn emit_open(rt: &mut BankRuntime, topup: bool) -> Value {
    rt.topup_open = topup;
    rt.open_acked = false;
    rt.phase = Phase::AckOpen;
    rt.clock.arm(OPEN_MS);
    rt.emit(json!({ "kind": "bank-open" }))
}

fn emit_log(rt: &mut BankRuntime, message: String) -> Value {
    rt.phase = Phase::AckLog;
    rt.emit(json!({ "kind": "log", "message": message }))
}

fn emit_close(rt: &mut BankRuntime, kind: OpKind) -> Value {
    let obs = observation();
    rt.pending = Some(Pending {
        kind,
        name: String::new(),
        id: 0,
        fail_trip: true,
        before_inv: 0,
        before_bank: 0,
        before_side: 0,
        before_used: pack_used(&obs),
        before_hp: obs.hp_effective,
        seq: obs.bank_op_result_seq,
        x_seq: obs.withdraw_x_result_seq,
        generation: obs.bank_generation,
    });
    rt.click_acked = false;
    rt.phase = Phase::WaitOp;
    rt.clock.arm(CLOSE_MS);
    rt.emit(json!({ "kind": "close" }))
}

fn quantity_label(have: i32, want: i32) -> Option<Quantity> {
    let out = crate::bank_withdraw::step(&json!({
        "count": have,
        "target": want,
        "full": false,
        "ok": true,
    }));
    match out.get("kind").and_then(Value::as_str) {
        Some("x") => Some(Quantity::X {
            count: out
                .get("count")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .max(0.0) as i32,
        }),
        Some("op") => Some(Quantity::Fixed {
            action: out
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("Withdraw-1")
                .to_string(),
        }),
        _ => None,
    }
}

enum Quantity {
    Fixed { action: String },
    X { count: i32 },
}

fn x_action(row: &BankRow) -> Option<String> {
    row.ops
        .iter()
        .find(|op| {
            op.replace('-', " ")
                .trim()
                .eq_ignore_ascii_case("withdraw x")
        })
        .cloned()
}

fn emit_withdraw(
    rt: &mut BankRuntime,
    row: &BankRow,
    action: &str,
    fail_trip: bool,
    have: i32,
) -> Value {
    let obs = observation();
    rt.pending = Some(Pending {
        kind: OpKind::Withdraw,
        name: row.name.clone(),
        id: row.id,
        fail_trip,
        before_inv: have,
        before_bank: row.count,
        before_side: name_count(&obs.bank_side, &row.name),
        before_used: pack_used(&obs),
        before_hp: obs.hp_effective,
        seq: obs.bank_op_result_seq,
        x_seq: obs.withdraw_x_result_seq,
        generation: obs.bank_generation,
    });
    rt.click_acked = false;
    rt.phase = Phase::WaitOp;
    rt.clock.arm(WITHDRAW_MS);
    rt.emit(json!({
        "kind": "withdraw",
        "name": row.name,
        "action": action,
    }))
}

fn emit_withdraw_x(
    rt: &mut BankRuntime,
    row: &BankRow,
    count: i32,
    action: &str,
    fail_trip: bool,
    have: i32,
) -> Value {
    let obs = observation();
    rt.pending = Some(Pending {
        kind: OpKind::WithdrawX,
        name: row.name.clone(),
        id: row.id,
        fail_trip,
        before_inv: have,
        before_bank: row.count,
        before_side: name_count(&obs.bank_side, &row.name),
        before_used: pack_used(&obs),
        before_hp: obs.hp_effective,
        seq: obs.bank_op_result_seq,
        x_seq: obs.withdraw_x_result_seq,
        generation: obs.bank_generation,
    });
    rt.click_acked = false;
    rt.phase = Phase::WaitOp;
    rt.clock.arm(WITHDRAW_MS);
    rt.emit(json!({
        "kind": "withdraw-x",
        "name": row.name,
        "count": count,
        "bank_item_id": row.id,
        "lands_as_id": row.id,
        "action": action,
        "bank_generation": obs.bank_generation,
    }))
}

fn emit_named_withdraw(
    rt: &mut BankRuntime,
    row: &BankRow,
    have: i32,
    want: i32,
    fail_trip: bool,
) -> Value {
    match quantity_label(have, want) {
        Some(Quantity::X { count }) => {
            if let Some(action) = x_action(row) {
                emit_withdraw_x(rt, row, count.max(1), &action, fail_trip, have)
            } else {
                emit_withdraw(rt, row, "Withdraw-10", fail_trip, have)
            }
        }
        Some(Quantity::Fixed { action }) => emit_withdraw(rt, row, &action, fail_trip, have),
        None => emit_withdraw(rt, row, "Withdraw-1", fail_trip, have),
    }
}

fn emit_deposit(rt: &mut BankRuntime, name: &str, kind: OpKind) -> Value {
    let obs = observation();
    rt.pending = Some(Pending {
        kind,
        name: name.to_string(),
        id: 0,
        fail_trip: kind == OpKind::SlotFree,
        before_inv: 0,
        before_bank: 0,
        before_side: name_count(&obs.bank_side, name),
        before_used: pack_used(&obs),
        before_hp: obs.hp_effective,
        seq: obs.bank_op_result_seq,
        x_seq: obs.withdraw_x_result_seq,
        generation: obs.bank_generation,
    });
    rt.click_acked = false;
    rt.phase = Phase::WaitOp;
    rt.clock.arm(WITHDRAW_MS);
    rt.emit(json!({ "kind": "deposit", "name": name }))
}

fn approach(rt: &mut BankRuntime, proj: &BankProj) -> Value {
    let Some(bank) = proj.bank else {
        return rt.yield_value(false);
    };
    let obs = observation();
    if !near_bank(&obs, bank) {
        return emit_walk(rt, bank);
    }
    emit_open(rt, false)
}

fn decide(rt: &mut BankRuntime, proj: &BankProj) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return stop;
    }
    if in_lair(proj, &obs) && !rt.left {
        rt.phase = Phase::AckLeave;
        return rt.emit(json!({ "kind": "leave" }));
    }
    approach(rt, proj)
}

fn ack_leave(rt: &mut BankRuntime, proj: &BankProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return stop;
    }
    if !reply_flag(reply, "left") {
        return rt.yield_value(false);
    }
    rt.left = true;
    rt.phase = Phase::Decide;
    approach(rt, proj)
}

fn ack_walk(rt: &mut BankRuntime, proj: &BankProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return stop;
    }
    if let Some(token) = reply_u64(reply, "walkToken") {
        rt.walk_token = Some(token);
    }
    let Some(walk_token) = rt.walk_token else {
        return rt.aborted("missing walkToken");
    };
    let dest = rt.walk_dest.or(proj.bank);
    let arrived = dest.is_some_and(|tile| near_bank(&obs, tile));
    if arrived {
        return emit_open(rt, false);
    }
    if rt.clock.bound_reached() || walk_wait_settled(walk_token) {
        return rt.yield_value(false);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn ack_open(rt: &mut BankRuntime, proj: &BankProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return stop;
    }
    if !rt.open_acked {
        match reply.and_then(|r| r.get("opened")).and_then(Value::as_bool) {
            Some(true) => rt.open_acked = true,
            _ => return rt.yield_value(false),
        }
    }
    if obs.bank_open && obs.bank_loaded {
        if rt.topup_open {
            rt.topup_open = false;
            rt.in_topup = true;
            rt.food_done = false;
            rt.food_rounds = 0;
            rt.stage = Stage::Food;
            rt.phase = Phase::Work;
            return continue_work(rt, proj);
        }
        if rt.plan.is_none() {
            rt.plan = Some(capture_plan(proj, &obs));
            rt.stage = Stage::Pick;
        }
        rt.phase = Phase::Work;
        return continue_work(rt, proj);
    }
    if rt.clock.bound_reached() {
        return rt.yield_value(false);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn continue_work(rt: &mut BankRuntime, proj: &BankProj) -> Value {
    for _ in 0..64 {
        if let Some(effect) = step_stage(rt, proj) {
            return effect;
        }
    }
    rt.yield_value(false)
}

fn step_stage(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return Some(stop);
    }
    match rt.stage {
        Stage::Pick => pick(rt, proj),
        Stage::Deposit => deposit(rt, proj),
        Stage::SlotFree => slot_free(rt, proj),
        Stage::Food => food(rt, proj),
        Stage::Key => key_arm(rt, proj),
        Stage::Gear => gear(rt, proj),
        Stage::Coins => coins(rt, proj),
        Stage::Style => style_supplies(rt, proj),
        Stage::Escape => escape_runes(rt, proj),
        Stage::Flasks => flasks(rt, proj),
        Stage::Wear => wear(rt, proj),
        Stage::Heal => heal(rt, proj),
        Stage::Topup => topup(rt, proj),
        Stage::Close => close_stage(rt),
        Stage::Count => count_stage(rt, proj),
    }
}

fn pick(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    rt.stage = Stage::Deposit;
    if !(proj.style.eq_ignore_ascii_case("melee") && proj.has_pick_weapon) {
        return None;
    }
    let obs = observation();
    let mut names = Vec::new();
    for row in obs
        .bank
        .iter()
        .chain(obs.bank_side.iter())
        .chain(obs.equipment.iter())
    {
        if !row.name.is_empty() && !names.iter().any(|name: &String| eq_name(name, &row.name)) {
            names.push(row.name.clone());
        }
    }
    rt.phase = Phase::AckPick;
    Some(rt.emit(json!({ "kind": "pick-weapon", "names": names })))
}

fn deposit(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if !rt.deposits_ready {
        let obs = observation();
        let plan = rt.plan.clone();
        let keep = plan
            .as_ref()
            .map(|plan| {
                let keep = keep_names(proj, &obs);
                let _ = plan;
                keep
            })
            .unwrap_or_default();
        let mut names = Vec::new();
        for row in &obs.bank_side {
            if row.count <= 0 {
                continue;
            }
            if row.name.is_empty() {
                return Some(rt.yield_value(false));
            }
            if !kept(&keep, &row.name) && !names.iter().any(|name: &String| eq_name(name, &row.name)) {
                names.push(row.name.clone());
            }
        }
        rt.deposit_names = names;
        rt.deposits_ready = true;
        rt.deposit_i = 0;
    }
    if rt.deposit_i >= rt.deposit_names.len() {
        rt.stage = Stage::SlotFree;
        return None;
    }
    let name = rt.deposit_names[rt.deposit_i].clone();
    Some(emit_deposit(rt, &name, OpKind::Deposit))
}

fn slot_free(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.slot_free_done {
        rt.stage = Stage::Food;
        return None;
    }
    rt.slot_free_done = true;
    let obs = observation();
    let plan = rt.plan.as_ref();
    let needs = plan.map(|plan| plan.needs_shield).unwrap_or(false);
    let shield_only_bank = obs
        .bank
        .iter()
        .any(|row| row.count > 0 && eq_name(&row.name, SHIELD))
        && !equip_has_name(&obs, SHIELD)
        && !obs
            .bank_side
            .iter()
            .any(|row| row.count > 0 && eq_name(&row.name, SHIELD))
        && !slotted_has_name(&obs, SHIELD);
    if !(needs && shield_only_bank && pack_full(&obs)) {
        rt.stage = Stage::Food;
        return None;
    }
    let forms = food_forms(&proj.food_name);
    let food = obs
        .bank_side
        .iter()
        .chain(slotted(&obs))
        .find(|row| is_food_name(&row.name, &forms) && !row.name.is_empty())
        .map(|row| row.name.clone());
    let Some(name) = food else {
        return Some(rt.yield_value(false));
    };
    rt.slot_free_done = false;
    Some(emit_deposit(rt, &name, OpKind::SlotFree))
}

fn food(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.food_done {
        rt.stage = if rt.in_topup {
            Stage::Close
        } else {
            Stage::Key
        };
        return None;
    }
    if !proj.withdraw_food {
        rt.food_done = true;
        rt.stage = if rt.in_topup {
            Stage::Close
        } else {
            Stage::Key
        };
        return None;
    }
    let obs = observation();
    let forms = food_forms(&proj.food_name);
    let have = food_count(&obs, &forms);
    let plan = rt.plan.as_ref();
    if have == 0 && plan.is_some_and(|plan| !plan.food_seen) && !rt.food_warned {
        rt.food_warned = true;
        rt.food_done = true;
        return Some(emit_log(
            rt,
            format!(
                "WARNING: no '{}' in the bank. Deposit food to resume eating.",
                proj.food_name
            ),
        ));
    }
    if have >= proj.food_want || rt.food_rounds >= FOOD_GUARD || pack_full(&obs) {
        rt.food_done = true;
        rt.stage = if rt.in_topup {
            Stage::Close
        } else {
            Stage::Key
        };
        return None;
    }
    let row = obs
        .bank
        .iter()
        .find(|row| row.count > 0 && is_food_name(&row.name, &forms) && !row.name.is_empty());
    let Some(row) = row.cloned() else {
        rt.food_done = true;
        if have == 0 && !rt.food_warned {
            rt.food_warned = true;
            return Some(emit_log(
                rt,
                format!(
                    "WARNING: no '{}' in the bank. Deposit food to resume eating.",
                    proj.food_name
                ),
            ));
        }
        rt.stage = if rt.in_topup {
            Stage::Close
        } else {
            Stage::Key
        };
        return None;
    };
    Some(emit_named_withdraw(rt, &row, have, proj.food_want, false))
}

fn key_arm(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.key_done || rt.in_topup {
        rt.stage = Stage::Gear;
        return None;
    }
    rt.key_done = true;
    let plan = rt.plan.clone();
    let Some(plan) = plan else {
        return Some(rt.yield_value(false));
    };
    match plan.key {
        KeyArm::Skip | KeyArm::Held | KeyArm::Opaque => {
            rt.stage = Stage::Gear;
            None
        }
        KeyArm::Nameless => Some(rt.yield_value(false)),
        KeyArm::Fetch => {
            let name = if plan.key_name.is_empty() {
                "key".to_string()
            } else {
                plan.key_name.clone()
            };
            Some(emit_log(
                rt,
                format!(
                    "no '{name}' in the bank or in the pack. Velrak has to hand out another one."
                ),
            ))
        }
        KeyArm::Bank => {
            let obs = observation();
            let Some(item) = &proj.key_item else {
                rt.stage = Stage::Gear;
                return None;
            };
            let row = obs
                .bank
                .iter()
                .find(|row| row.id == item.id && row.count > 0)
                .cloned();
            let Some(row) = row else {
                return Some(rt.yield_value(false));
            };
            if row.name.trim().is_empty() {
                return Some(rt.yield_value(false));
            }
            Some(emit_withdraw(
                rt,
                &row,
                "Withdraw-1",
                true,
                slotted_count_id(&obs, row.id),
            ))
        }
    }
}

fn gear(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.in_topup {
        rt.stage = Stage::Coins;
        return None;
    }
    if !rt.gear_ready {
        let plan = rt.plan.as_ref();
        if plan.is_some_and(|plan| plan.needs_shield && !plan.shield_seen) {
            rt.phase = Phase::AckPark;
            let reason = if proj.fire_at_range {
                "the metal dragons breathe fire at range, so every style here wears the Dragonfire shield, and there is none in the bank or worn. Duke Horacio in Lumbridge Castle hands one out free."
            } else {
                "melee needs the Dragonfire shield and there is none in the bank or worn. Duke Horacio in Lumbridge Castle hands one out free, or switch to mage or range, which fight from a fire-proof safespot."
            };
            return Some(rt.emit(json!({ "kind": "park", "reason": reason })));
        }
        let mut names = Vec::new();
        if plan.is_some_and(|plan| plan.needs_shield) {
            names.push(SHIELD.to_string());
        }
        if !proj.weapon.is_empty() {
            names.push(proj.weapon.clone());
        }
        names.extend(proj.wear.iter().cloned());
        names.extend(proj.carry.iter().cloned());
        rt.gear_names = names.into_iter().filter(|name| !name.is_empty()).collect();
        rt.gear_i = 0;
        rt.gear_ready = true;
    }
    let obs = observation();
    while rt.gear_i < rt.gear_names.len() {
        let name = rt.gear_names[rt.gear_i].clone();
        rt.gear_i += 1;
        if held_name(&obs, &name) {
            continue;
        }
        let Some(row) = find_named(&obs.bank, &name).cloned() else {
            return Some(emit_log(
                rt,
                format!(
                    "WARNING: no '{name}' in the bank. Carrying on with the gear already worn."
                ),
            ));
        };
        return Some(emit_withdraw(rt, &row, "Withdraw-1", false, 0));
    }
    rt.stage = Stage::Coins;
    None
}

fn coins(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.coins_done || rt.in_topup {
        rt.stage = Stage::Style;
        return None;
    }
    rt.coins_done = true;
    let Some(target) = proj.coins else {
        rt.stage = Stage::Style;
        return None;
    };
    let obs = observation();
    let have = name_count(
        &obs.inv
            .iter()
            .filter(|row| real_slot(row))
            .cloned()
            .collect::<Vec<_>>(),
        "Coins",
    );
    if have >= target {
        rt.stage = Stage::Style;
        return None;
    }
    let Some(row) = find_named(&obs.bank, "Coins").cloned() else {
        return Some(emit_log(
            rt,
            format!("WARNING: the bank cannot cover the {target} coins the way in costs. Deposit coins to resume."),
        ));
    };
    Some(emit_named_withdraw(rt, &row, have, target, false))
}

fn style_supplies(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.in_topup {
        rt.stage = Stage::Escape;
        return None;
    }
    if !rt.style_ready {
        rt.style_ready = true;
        rt.style_i = 0;
    }
    let obs = observation();
    if proj.style.eq_ignore_ascii_case("mage") {
        while rt.style_i < proj.runes.len() {
            let rune = proj.runes[rt.style_i].clone();
            rt.style_i += 1;
            let have = name_count(&slotted(&obs).cloned().collect::<Vec<_>>(), &rune.name);
            if have >= rune.count {
                continue;
            }
            let Some(row) = find_named(&obs.bank, &rune.name).cloned() else {
                return Some(emit_log(
                    rt,
                    format!(
                        "WARNING: the bank cannot supply a single '{}' cast. Deposit runes to resume.",
                        proj.spell
                    ),
                ));
            };
            return Some(emit_named_withdraw(rt, &row, have, rune.count, false));
        }
    } else if proj.style.eq_ignore_ascii_case("range") && !proj.ammo.is_empty() {
        if rt.style_i == 0 {
            rt.style_i = 1;
            let have = name_count(&slotted(&obs).cloned().collect::<Vec<_>>(), &proj.ammo);
            if have < proj.ammo_want {
                if let Some(row) = find_named(&obs.bank, &proj.ammo).cloned() {
                    return Some(emit_named_withdraw(rt, &row, have, proj.ammo_want, false));
                }
                return Some(emit_log(
                    rt,
                    format!(
                        "WARNING: no '{}' in the bank. Deposit ammo to resume.",
                        proj.ammo
                    ),
                ));
            }
        }
    }
    rt.stage = Stage::Escape;
    None
}

fn escape_runes(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.in_topup {
        rt.stage = Stage::Flasks;
        return None;
    }
    if !rt.escape_ready {
        rt.escape_ready = true;
        rt.escape_i = 0;
    }
    let obs = observation();
    while rt.escape_i < proj.escape_runes.len() {
        let rune = proj.escape_runes[rt.escape_i].clone();
        rt.escape_i += 1;
        let have = name_count(&slotted(&obs).cloned().collect::<Vec<_>>(), &rune.name);
        if have >= rune.count {
            continue;
        }
        let Some(row) = find_named(&obs.bank, &rune.name).cloned() else {
            return Some(emit_log(
                rt,
                format!(
                    "WARNING: the escape cannot be cast (no {}). The next trip walks out through the gate.",
                    rune.name
                ),
            ));
        };
        return Some(emit_named_withdraw(rt, &row, have, rune.count, false));
    }
    rt.stage = Stage::Flasks;
    None
}

fn flasks(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.in_topup {
        rt.stage = Stage::Wear;
        return None;
    }
    if !rt.flask_ready {
        rt.flask_ready = true;
        rt.flask_i = 0;
    }
    let obs = observation();
    while rt.flask_i < proj.flasks.len() {
        let plan = proj.flasks[rt.flask_i].clone();
        let doses = if plan.doses.is_empty() {
            vec![plan.name.clone()]
        } else {
            plan.doses.clone()
        };
        let have = doses
            .iter()
            .map(|dose| name_count(&slotted(&obs).cloned().collect::<Vec<_>>(), dose))
            .sum::<i32>();
        if have >= plan.want || pack_full(&obs) {
            rt.flask_i += 1;
            continue;
        }
        let Some(row) = find_named(&obs.bank, &plan.name).cloned() else {
            rt.flask_i += 1;
            return Some(emit_log(
                rt,
                format!(
                    "WARNING: no '{}' in the bank. The trip goes without.",
                    plan.name
                ),
            ));
        };
        return Some(emit_withdraw(rt, &row, "Withdraw-1", false, have));
    }
    rt.stage = Stage::Wear;
    None
}

fn wear(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.in_topup {
        rt.stage = Stage::Heal;
        return None;
    }
    if !rt.wear_ready {
        let mut names = Vec::new();
        if needs_shield(proj) {
            names.push(SHIELD.to_string());
        }
        if !proj.weapon.is_empty() {
            names.push(proj.weapon.clone());
        }
        if proj.style.eq_ignore_ascii_case("range") && !proj.ammo.is_empty() {
            names.push(proj.ammo.clone());
        }
        names.extend(proj.wear.iter().filter(|name| !name.is_empty()).cloned());
        rt.wear_names = names;
        rt.wear_i = 0;
        rt.wear_ready = true;
    }
    let obs = observation();
    while rt.wear_i < rt.wear_names.len() {
        let name = rt.wear_names[rt.wear_i].clone();
        if equip_has_name(&obs, &name) {
            rt.wear_i += 1;
            continue;
        }
        if let Some(row) = slotted(&obs).find(|row| eq_name(&row.name, &name)) {
            if can_wear(row) {
                rt.wear_owed.push(name.clone());
                return Some(emit_wear(rt, &name));
            }
            if eq_name(&name, SHIELD) {
                rt.shield_wear_refused = true;
            } else {
                rt.wear_refused.push(name.clone());
            }
            rt.wear_i += 1;
            return Some(emit_log(
                rt,
                format!("WARNING: '{name}' has no Wear or Wield. Carrying on."),
            ));
        }
        if obs.bank_open && inv_empty(&obs) {
            return Some(emit_close(rt, OpKind::CloseWear));
        }
        rt.wear_i += 1;
    }
    rt.stage = Stage::Heal;
    None
}

fn emit_wear(rt: &mut BankRuntime, name: &str) -> Value {
    let obs = observation();
    rt.pending = Some(Pending {
        kind: OpKind::Wear,
        name: name.to_string(),
        id: 0,
        fail_trip: false,
        before_inv: 0,
        before_bank: 0,
        before_side: 0,
        before_used: pack_used(&obs),
        before_hp: obs.hp_effective,
        seq: obs.bank_op_result_seq,
        x_seq: obs.withdraw_x_result_seq,
        generation: obs.bank_generation,
    });
    rt.click_acked = false;
    rt.phase = Phase::WaitOp;
    rt.clock.arm(WITHDRAW_MS);
    rt.emit(json!({ "kind": "wear", "name": name }))
}

fn heal(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.heal_done {
        rt.stage = Stage::Topup;
        return None;
    }
    let obs = observation();
    let forms = food_forms(&proj.food_name);
    let fraction = if obs.hp_base > 0 {
        obs.hp_effective as f64 / obs.hp_base as f64
    } else {
        1.0
    };
    let food_here = obs
        .bank_side
        .iter()
        .any(|row| row.count > 0 && is_food_name(&row.name, &forms))
        || slotted(&obs).any(|row| is_food_name(&row.name, &forms));
    if fraction >= proj.heal_to || !food_here && rt.heal_attempts == 0 {
        rt.heal_done = true;
        rt.stage = Stage::Topup;
        return None;
    }
    if obs.bank_open {
        return Some(emit_close(rt, OpKind::CloseHeal));
    }
    if rt.heal_attempts >= HEAL_BITES || rt.heal_misses >= HEAL_MISSES {
        rt.heal_done = true;
        rt.stage = Stage::Topup;
        return None;
    }
    let Some(row) = slotted(&obs)
        .find(|row| is_food_name(&row.name, &forms))
        .cloned()
    else {
        rt.heal_done = true;
        rt.stage = Stage::Topup;
        return None;
    };
    rt.eat_name = row.name.clone();
    rt.eat_id = row.id;
    rt.heal_attempts += 1;
    rt.heal_ticks = 0;
    rt.click_acked = false;
    rt.pending = Some(Pending {
        kind: OpKind::CloseHeal,
        name: row.name.clone(),
        id: row.id,
        fail_trip: false,
        before_inv: row.count,
        before_bank: 0,
        before_side: 0,
        before_used: 0,
        before_hp: obs.hp_effective,
        seq: 0,
        x_seq: 0,
        generation: obs.bank_generation,
    });
    rt.phase = Phase::WaitHeal;
    rt.clock.deadline = None;
    Some(rt.emit(json!({ "kind": "held", "name": row.name, "action": "Eat" })))
}

fn topup(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    if rt.topup_done {
        rt.stage = Stage::Close;
        return None;
    }
    Some(start_topup(rt, proj))
}

fn close_stage(rt: &mut BankRuntime) -> Option<Value> {
    let obs = observation();
    if obs.bank_open {
        return Some(emit_close(rt, OpKind::CloseFinal));
    }
    rt.stage = Stage::Count;
    None
}

fn count_stage(rt: &mut BankRuntime, proj: &BankProj) -> Option<Value> {
    let obs = observation();
    if obs.bank_open {
        return Some(emit_close(rt, OpKind::CloseFinal));
    }
    if !ready_with(rt, proj, &obs) {
        return Some(rt.yield_value(false));
    }
    if !rt.counted {
        rt.counted = true;
        rt.phase = Phase::AckCount;
        return Some(rt.emit(json!({ "kind": "count-bank-trip" })));
    }
    Some(rt.yield_value(true))
}

fn ready_with(rt: &BankRuntime, proj: &BankProj, obs: &BankObservation) -> bool {
    if obs.bank_open {
        return false;
    }
    let Some(plan) = &rt.plan else {
        return false;
    };
    match plan.key {
        KeyArm::Held | KeyArm::Bank => {
            if !slotted_has_id(obs, plan.key_id) {
                return false;
            }
        }
        _ => {}
    }
    if plan.needs_shield
        && plan.shield_seen
        && !equip_has_name(obs, SHIELD)
        && !slotted_has_name(obs, SHIELD)
    {
        return false;
    }
    if rt.shield_wear_refused && slotted_has_name(obs, SHIELD) {
        return false;
    }
    let forms = food_forms(&proj.food_name);
    if plan.food_owed && food_count(obs, &forms) == 0 {
        return false;
    }
    for name in &plan.junk {
        if slotted_has_name(obs, name) {
            return false;
        }
    }
    for name in &rt.wear_owed {
        if rt.wear_refused.iter().any(|refused| eq_name(refused, name)) {
            continue;
        }
        if !equip_has_name(obs, name) {
            return false;
        }
    }
    true
}

fn op_proved(rt: &BankRuntime, obs: &BankObservation) -> bool {
    let Some(pending) = &rt.pending else {
        return false;
    };
    match pending.kind {
        OpKind::CloseWear | OpKind::CloseHeal | OpKind::CloseFinal => !obs.bank_open,
        OpKind::Wear => equip_has_name(obs, &pending.name),
        OpKind::Deposit | OpKind::SlotFree => {
            obs.bank_generation == pending.generation
                && obs.bank_op_result
                && obs.bank_op_result_seq > pending.seq
                && (name_count(&obs.bank_side, &pending.name) < pending.before_side
                    || pack_used(obs) < pending.before_used)
        }
        OpKind::Withdraw => {
            let landed = slotted_count_id(obs, pending.id) > pending.before_inv
                || count_id(&obs.bank, pending.id) < pending.before_bank
                || obs
                    .bank_side
                    .iter()
                    .any(|row| row.id == pending.id && row.count > 0)
                || slotted_has_id(obs, pending.id);
            obs.bank_generation == pending.generation
                && obs.bank_op_result
                && obs.bank_op_result_seq > pending.seq
                && landed
        }
        OpKind::WithdrawX => {
            let landed = slotted_count_id(obs, pending.id) > pending.before_inv
                || count_id(&obs.bank, pending.id) < pending.before_bank;
            obs.bank_generation == pending.generation
                && obs.withdraw_x_result
                && obs.withdraw_x_result_seq > pending.x_seq
                && landed
        }
    }
}

fn stale_generation(rt: &BankRuntime, obs: &BankObservation) -> bool {
    let Some(pending) = &rt.pending else {
        return false;
    };
    matches!(
        pending.kind,
        OpKind::Deposit | OpKind::SlotFree | OpKind::Withdraw | OpKind::WithdrawX
    ) && obs.bank_open
        && obs.bank_generation != pending.generation
}

fn finish_op(rt: &mut BankRuntime, proj: &BankProj) -> Value {
    let kind = rt.pending.as_ref().map(|pending| pending.kind);
    let name = rt
        .pending
        .as_ref()
        .map(|pending| pending.name.clone())
        .unwrap_or_default();
    match kind {
        Some(OpKind::Deposit) => rt.deposit_i += 1,
        Some(OpKind::SlotFree) => rt.slot_free_done = true,
        Some(OpKind::Withdraw) | Some(OpKind::WithdrawX) => {
            if rt.stage == Stage::Food {
                let forms = food_forms(&proj.food_name);
                let have = food_count(&observation(), &forms);
                let before = rt
                    .pending
                    .as_ref()
                    .map(|pending| pending.before_inv)
                    .unwrap_or(0);
                if have <= before {
                    rt.food_done = true;
                } else {
                    rt.food_rounds += 1;
                }
            }
        }
        Some(OpKind::Wear) => rt.wear_i += 1,
        Some(OpKind::CloseWear) => {}
        Some(OpKind::CloseHeal) => {}
        Some(OpKind::CloseFinal) => rt.stage = Stage::Count,
        None => {}
    }
    let _ = name;
    rt.pending = None;
    rt.phase = Phase::Work;
    continue_work(rt, proj)
}

fn timeout_op(rt: &mut BankRuntime, proj: &BankProj) -> Value {
    let kind = rt.pending.as_ref().map(|pending| pending.kind);
    let fail = rt.pending.as_ref().is_some_and(|pending| pending.fail_trip);
    let name = rt
        .pending
        .as_ref()
        .map(|pending| pending.name.clone())
        .unwrap_or_default();
    if fail
        || matches!(
            kind,
            Some(OpKind::CloseFinal)
                | Some(OpKind::CloseHeal)
                | Some(OpKind::CloseWear)
                | Some(OpKind::SlotFree)
        )
    {
        return rt.yield_value(false);
    }
    match kind {
        Some(OpKind::Wear) => {
            if eq_name(&name, SHIELD) {
                rt.shield_wear_refused = true;
            } else if !name.is_empty() {
                rt.wear_refused.push(name);
            }
            rt.wear_i += 1;
        }
        Some(OpKind::Deposit) => rt.deposit_i += 1,
        Some(OpKind::Withdraw) | Some(OpKind::WithdrawX) => {
            if rt.stage == Stage::Food {
                rt.food_done = true;
            }
        }
        _ => {}
    }
    rt.pending = None;
    rt.phase = Phase::Work;
    continue_work(rt, proj)
}

fn ack_op(rt: &mut BankRuntime, proj: &BankProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return stop;
    }
    if !rt.click_acked {
        if !reply_flag(reply, "queued") {
            return rt.yield_value(false);
        }
        rt.click_acked = true;
    }
    let kind = rt.pending.as_ref().map(|pending| pending.kind);
    if matches!(
        kind,
        Some(OpKind::CloseWear) | Some(OpKind::CloseHeal) | Some(OpKind::CloseFinal)
    ) {
        if !obs.bank_open {
            return finish_op(rt, proj);
        }
        if rt.clock.bound_reached() {
            return rt.yield_value(false);
        }
        return rt.emit(json!({ "kind": "delay-ticks", "n": 1 }));
    }
    if stale_generation(rt, &obs) {
        return rt.yield_value(false);
    }
    if op_proved(rt, &obs) {
        return finish_op(rt, proj);
    }
    if rt.clock.bound_reached() {
        return timeout_op(rt, proj);
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn heal_proved(rt: &BankRuntime, obs: &BankObservation) -> bool {
    let Some(pending) = &rt.pending else {
        return false;
    };
    slotted_count_id(obs, pending.id) < pending.before_inv || obs.hp_effective > pending.before_hp
}

fn ack_heal(rt: &mut BankRuntime, proj: &BankProj, reply: Option<&Value>) -> Value {
    let obs = observation();
    if let Some(stop) = gate(rt, &obs) {
        return stop;
    }
    if obs.bank_open {
        return emit_close(rt, OpKind::CloseHeal);
    }
    if !rt.click_acked {
        if !reply_flag(reply, "queued") {
            rt.heal_misses += 1;
            rt.click_acked = true;
        } else {
            rt.click_acked = true;
        }
    }
    if heal_proved(rt, &obs) {
        rt.bite_landed = true;
        rt.heal_misses = 0;
        rt.pending = None;
        let forms = food_forms(&proj.food_name);
        let fraction = if obs.hp_base > 0 {
            obs.hp_effective as f64 / obs.hp_base as f64
        } else {
            1.0
        };
        let more = fraction < proj.heal_to
            && slotted(&obs).any(|row| is_food_name(&row.name, &forms))
            && rt.heal_attempts < HEAL_BITES
            && rt.heal_misses < HEAL_MISSES;
        rt.phase = Phase::Work;
        if more {
            return continue_work(rt, proj);
        }
        rt.heal_done = true;
        rt.stage = Stage::Topup;
        return start_topup(rt, proj);
    }
    rt.heal_ticks += 1;
    if rt.heal_ticks >= 2 {
        rt.heal_misses += 1;
        rt.heal_ticks = 0;
        rt.click_acked = false;
        if rt.heal_misses >= HEAL_MISSES || rt.heal_attempts >= HEAL_BITES {
            rt.heal_done = true;
            rt.phase = Phase::Work;
            rt.stage = Stage::Topup;
            return start_topup(rt, proj);
        }
    }
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn start_topup(rt: &mut BankRuntime, proj: &BankProj) -> Value {
    if rt.bite_landed && proj.withdraw_food && !rt.topup_done {
        rt.topup_done = true;
        rt.in_topup = true;
        return emit_open(rt, true);
    }
    rt.topup_done = true;
    rt.stage = Stage::Close;
    rt.phase = Phase::Work;
    continue_work(rt, proj)
}

fn next_effect(rt: &mut BankRuntime, proj: &BankProj, reply: Option<&Value>) -> Value {
    if bad_reply(reply) {
        return rt.aborted("unexpected reply");
    }
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
        Phase::AckLeave => ack_leave(rt, proj, reply),
        Phase::AckWalk => ack_walk(rt, proj, reply),
        Phase::AckOpen => ack_open(rt, proj, reply),
        Phase::Work => continue_work(rt, proj),
        Phase::WaitOp => ack_op(rt, proj, reply),
        Phase::WaitHeal => ack_heal(rt, proj, reply),
        Phase::AckLog | Phase::AckPick => {
            rt.phase = Phase::Work;
            continue_work(rt, proj)
        }
        Phase::AckPark => rt.yield_value(false),
        Phase::AckCount => rt.yield_value(true),
        Phase::Done => rt.yield_value(rt.yielded.unwrap_or(false)),
        Phase::Aborted => rt.aborted("aborted"),
    }
}

fn row_from(row: &crate::isolate_fb::RowReader<'_>) -> BankRow {
    let posted = row.has_slot();
    let slot = if posted { row.slot() } else { -1 };
    BankRow {
        id: row.id(),
        count: row.count(),
        name: row.name().unwrap_or_default().to_string(),
        slot,
        has_slot: posted && slot >= 0,
        ops: row.ops().into_iter().map(str::to_string).collect(),
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    OBSERVATION.with(|slot| {
        let mut o = slot.borrow_mut();
        if snap.has_ingame() {
            o.ingame = snap.ingame();
            if !o.ingame {
                o.here = None;
            }
        }
        if snap.has_here() {
            o.here = snap.here().map(|t| Tile {
                x: t.x(),
                z: t.z(),
                level: t.level(),
            });
        }
        if snap.has_hold() {
            o.hold = snap.hold();
        }
        if snap.has_ours() {
            o.ours = snap.ours();
        }
        if snap.has_inv_size() {
            o.inv_size = snap.inv_size();
        }
        if snap.has_inv() {
            o.inv = snap.inv().iter().map(row_from).collect();
        }
        if snap.has_equipment() {
            o.equipment = snap.equipment().iter().map(row_from).collect();
        }
        if snap.has_bank() {
            o.bank = snap.bank().iter().map(row_from).collect();
        }
        if snap.has_bank_side() {
            o.bank_side = snap.bank_side().iter().map(row_from).collect();
        }
        if snap.has_bank_open() {
            o.bank_open = snap.bank_open();
        }
        if snap.has_bank_loaded() {
            o.bank_loaded = snap.bank_loaded();
        }
        if snap.has_bank_generation() {
            o.bank_generation = snap.bank_generation();
        }
        if snap.has_bank_op_result_seq() {
            o.bank_op_result_seq = snap.bank_op_result_seq();
        }
        if snap.has_bank_op_result() {
            o.bank_op_result = snap.bank_op_result();
        }
        if snap.has_withdraw_x_result_seq() {
            o.withdraw_x_result_seq = snap.withdraw_x_result_seq();
        }
        if snap.has_withdraw_x_result() {
            o.withdraw_x_result = snap.withdraw_x_result();
        }
        if snap.has_stats() {
            if let Some(hp) = snap.stats().iter().find(|stat| stat.name() == "hitpoints") {
                o.hp_base = hp.base();
                o.hp_effective = hp.effective();
            }
        }
    });
}

fn each_runtime(f: impl Fn(&mut BankRuntime)) {
    BANK_RUNTIMES.with(|m| {
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
    BANK_RUNTIMES.with(|m| m.borrow_mut().clear());
    OBSERVATION.with(|o| *o.borrow_mut() = BankObservation::empty());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => {
            let token = alloc_token();
            BANK_RUNTIMES.with(|m| {
                m.borrow_mut().insert(token, BankRuntime::new(token));
            });
            json!({ "kind": "started", "token": token })
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_proj(input);
            let reply = input.get("reply");
            match with_bank(token, |rt| next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "token": token }),
            }
        }
        "end" => {
            // The invocation is over: drop the row. Unknown token is a no-op —
            // JS ends on every return, including one after a session reset
            // already cleared the map.
            let token = token_of(input);
            let _ = BANK_RUNTIMES.with(|m| m.borrow_mut().remove(&token));
            json!({ "kind": "ok", "token": token })
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn bank_token_alive(token: u64) -> bool {
    BANK_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

pub fn bank_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_bank(token, |rt| {
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

pub fn bank_force_bound_reached(token: u64) -> bool {
    with_bank(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

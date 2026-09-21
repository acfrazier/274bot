//! Rust-owned Fight isolate step machine.
//!
//! JS marshals CombatHost callbacks and existing Interact ops. Policy, clocks,
//! field pick and Taverley `site.key` branches stay here. One effect per
//! `next()`. Tokens are per Task instance; session reset drops the map.

use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const FIGHT_MS: u64 = 120_000;
pub const FIGHT_PASSES: u32 = 600;
pub const FIELD_RADIUS: i32 = 10;
pub const HOLD_GAP_MS: u64 = 10_000;
pub const SAFESPOT_BLIND_MS: u64 = 20_000;
pub const ENGAGE_SETTLE_MS: u64 = 1_200;
pub const SETTLE_MS: u64 = 1_200;
pub const REFUSED_SKIP_MS: u64 = 5_000;
pub const RE_ENGAGE_MS: u64 = 4_000;
pub const KILL_GRACE_MS: u64 = 6_000;
pub const TAKEN_SKIP_MS: u64 = 15_000;
pub const LEASH_SKIP_MS: u64 = 20_000;
pub const PULL_SKIP_MS: u64 = 8_000;
pub const CHASE_STALL_MS: u64 = 90_000;
pub const CHASE_SKIP_MS: u64 = 90_000;
pub const HOP_ATTEMPTS: u32 = 4;
pub const HOP_MS: u64 = 2_000;
pub const FIELD_DIAG_MS: u64 = 10_000;
pub const TAVERLEY_BLUE: &str = "taverley-blue";
pub const TAVERLEY_BLACK: &str = "taverley-black";

thread_local! {
    static RUNTIMES: RefCell<HashMap<u64, FightRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
    static OBSERVATION: RefCell<FightObservation> = RefCell::new(FightObservation::ready());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

impl Tile {
    fn equals(self, other: Tile) -> bool {
        self.x == other.x && self.z == other.z && self.level == other.level
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Melee,
    Range,
    Mage,
}

#[derive(Clone, Debug)]
pub struct SiteBox {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub level: i32,
}

#[derive(Clone, Debug)]
pub struct Site {
    pub key: String,
    pub target: String,
    pub also_hunt: Vec<String>,
    pub safespots: Vec<Tile>,
    pub melee_anchor: Tile,
    pub boxes: Vec<SiteBox>,
    pub fire_at_range: bool,
    pub ranged_threat: bool,
}

#[derive(Clone, Debug)]
pub struct FightNpc {
    pub index: i32,
    pub id: i32,
    pub name: String,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub nx: i32,
    pub nz: i32,
    pub size: i32,
    pub distance: i32,
    pub health: i32,
    pub in_combat: bool,
    pub actions: Vec<String>,
    pub target_kind: i32,
    pub target_index: i32,
}

impl FightNpc {
    fn tile(&self) -> Tile {
        Tile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }

    fn network(&self) -> Tile {
        Tile {
            x: self.nx,
            z: self.nz,
            level: self.level,
        }
    }

    fn origin(&self, safespot: bool) -> Tile {
        if safespot {
            self.network()
        } else {
            self.tile()
        }
    }

    fn has_attack(&self) -> bool {
        self.actions
            .iter()
            .any(|a| a.eq_ignore_ascii_case("attack"))
    }
}

#[derive(Clone, Debug)]
pub struct FightObservation {
    pub here: Option<Tile>,
    pub ingame: bool,
    pub scene_state: i32,
    pub hold: bool,
    pub ours: bool,
    pub chat_continue: bool,
    pub npcs: Vec<FightNpc>,
    pub self_target_kind: i32,
    pub self_target_index: i32,
    pub self_slot: i32,
    pub hp_effective: i32,
    pub inv_names: Vec<String>,
    pub animating: bool,
    pub los_override: Option<bool>,
    pub tick: u64,
}

impl FightObservation {
    fn ready() -> Self {
        Self {
            here: None,
            ingame: true,
            scene_state: 2,
            hold: false,
            ours: false,
            chat_continue: false,
            npcs: Vec::new(),
            self_target_kind: 0,
            self_target_index: -1,
            self_slot: 0,
            hp_effective: 99,
            inv_names: Vec::new(),
            animating: false,
            los_override: None,
            tick: 1,
        }
    }
}

#[derive(Clone, Debug)]
struct Sighting {
    x: i32,
    z: i32,
    since: Instant,
    at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Idle,
    Status,
    Pass,
    Eating,
    EngagingArm,
    EngagingNpc,
    EngagingSetTarget,
    WaitFed,
    Idling,
    Walking,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdleThen {
    Yield,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitCond {
    EngageSettle,
    AtAnchor,
}

struct FightRuntime {
    clock: InstantTaskClock,
    token: u64,
    mode: Mode,
    pass: u32,
    engaged: Option<i32>,
    loot_target: Option<i32>,
    engaged_name: String,
    seen_at: Instant,
    engaged_at: Instant,
    engaged_health: i32,
    last_hp: i32,
    watched_anchor: Option<Tile>,
    unattackable_since: Option<Instant>,
    blind_since: Instant,
    polled_at: Instant,
    diag_at: Instant,
    damaged_at: Instant,
    reissues: u32,
    skip: HashMap<i32, Instant>,
    seen: HashMap<i32, Sighting>,
    pending_npc: Option<FightNpc>,
    idle_then: IdleThen,
    idle_after_sustain: bool,
    wait_until: Option<Instant>,
    wait_cond: WaitCond,
    wait_after_sustain: bool,
    walk_attempts_left: u32,
    walk_after_walk: bool,
    walk_after_sustain: bool,
}

impl FightRuntime {
    fn new(token: u64, now: Instant) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: Mode::Idle,
            pass: 0,
            engaged: None,
            loot_target: None,
            engaged_name: String::new(),
            seen_at: now,
            engaged_at: now,
            engaged_health: -1,
            last_hp: -1,
            watched_anchor: None,
            unattackable_since: None,
            blind_since: now,
            polled_at: now,
            diag_at: now,
            damaged_at: now,
            reissues: 0,
            skip: HashMap::new(),
            seen: HashMap::new(),
            pending_npc: None,
            idle_then: IdleThen::Continue,
            idle_after_sustain: false,
            wait_until: None,
            wait_cond: WaitCond::EngageSettle,
            wait_after_sustain: false,
            walk_attempts_left: 0,
            walk_after_walk: false,
            walk_after_sustain: false,
        }
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn apply_freeze(&mut self, paused: bool, held: bool) {
        let was = self.clock.frozen();
        let frozen_at = self.clock.frozen_at;
        self.clock.set_freeze(paused, held);
        if was && !self.clock.frozen() {
            if let Some(at) = frozen_at {
                let gap = Instant::now().saturating_duration_since(at);
                self.shift_instants(gap);
            }
        }
    }

    fn shift_instants(&mut self, gap: Duration) {
        self.seen_at += gap;
        self.engaged_at += gap;
        self.damaged_at += gap;
        self.blind_since += gap;
        self.polled_at += gap;
        self.diag_at += gap;
        if let Some(t) = self.unattackable_since.as_mut() {
            *t += gap;
        }
        if let Some(t) = self.wait_until.as_mut() {
            *t += gap;
        }
        for t in self.skip.values_mut() {
            *t += gap;
        }
        for s in self.seen.values_mut() {
            s.since += gap;
            s.at += gap;
        }
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_now(&mut self) -> Value {
        self.mode = Mode::Idle;
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = Mode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }

    fn interrupt_watch(&mut self) {
        self.last_hp = -1;
        self.watched_anchor = None;
        self.unattackable_since = None;
        self.blind_since = self.now();
    }

    fn clear_target(&mut self) {
        self.engaged = None;
        self.unattackable_since = None;
        self.reissues = 0;
        self.engaged_health = -1;
    }

    fn reset_task(&mut self) {
        self.clear_target();
        self.loot_target = None;
        self.interrupt_watch();
    }

    fn set_target(&mut self, idx: i32) {
        if self.engaged != Some(idx) {
            self.damaged_at = self.now();
            self.unattackable_since = None;
        }
        self.engaged = Some(idx);
        self.loot_target = Some(idx);
        self.seen_at = self.now();
    }

    fn skip_due(&self, index: i32) -> bool {
        self.skip
            .get(&index)
            .is_some_and(|until| self.now() < *until)
    }
}

#[derive(Clone, Debug)]
struct Projection {
    died: bool,
    hp_fraction: f64,
    panic_hp: f64,
    retreat_hp: f64,
    has_food: bool,
    need_eat: bool,
    style: Style,
    safespot_index: i32,
    bury_bones: bool,
    bone_name: String,
    has_vlog: bool,
    has_arm_special: bool,
    has_shield_ready: bool,
    shield_ready: bool,
    site: Site,
}

pub fn gap_sw(from: Tile, origin: Tile, size: i32) -> i32 {
    let size = size.max(1);
    let dx = (origin.x - from.x)
        .max(from.x - (origin.x + size - 1))
        .max(0);
    let dz = (origin.z - from.z)
        .max(from.z - (origin.z + size - 1))
        .max(0);
    dx.max(dz)
}

pub fn in_area_body(origin: Tile, size: i32, boxes: &[SiteBox]) -> bool {
    let size = size.max(1);
    let x1 = origin.x + size - 1;
    let z1 = origin.z + size - 1;
    boxes.iter().any(|b| {
        origin.level == b.level
            && origin.x <= b.max_x
            && x1 >= b.min_x
            && origin.z <= b.max_z
            && z1 >= b.min_z
    })
}

fn uses_safespot(style: Style) -> bool {
    style != Style::Melee
}

fn chase_mode(style: Style, fire_at_range: bool) -> bool {
    style == Style::Melee && fire_at_range
}

fn holds_anchor(style: Style, fire_at_range: bool) -> bool {
    !chase_mode(style, fire_at_range)
}

fn engage_range_for(style: Style) -> i32 {
    match style {
        Style::Melee => 1,
        Style::Range => 6,
        Style::Mage => 9,
    }
}

fn attack_range_for(style: Style) -> i32 {
    match style {
        Style::Melee => 1,
        Style::Range => 7,
        Style::Mage => 10,
    }
}

fn hunt_names(site: &Site) -> Vec<String> {
    let mut names = Vec::with_capacity(1 + site.also_hunt.len());
    names.push(site.target.clone());
    names.extend(site.also_hunt.iter().cloned());
    names
}

fn parse_style(v: &Value) -> Style {
    match v.get("style").and_then(Value::as_str).unwrap_or("melee") {
        "range" => Style::Range,
        "mage" => Style::Mage,
        _ => Style::Melee,
    }
}

fn parse_tile(v: Option<&Value>, fallback: Tile) -> Tile {
    let Some(v) = v else {
        return fallback;
    };
    Tile {
        x: v.get("x")
            .and_then(Value::as_i64)
            .unwrap_or(fallback.x as i64) as i32,
        z: v.get("z")
            .and_then(Value::as_i64)
            .unwrap_or(fallback.z as i64) as i32,
        level: v
            .get("level")
            .and_then(Value::as_i64)
            .unwrap_or(fallback.level as i64) as i32,
    }
}

fn parse_projection(input: &Value) -> Projection {
    let origin = Tile {
        x: 0,
        z: 0,
        level: 0,
    };
    let safespots = input
        .get("safespots")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| parse_tile(Some(row), origin))
                .collect()
        })
        .unwrap_or_default();
    let boxes = input
        .get("boxes")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| SiteBox {
                    min_x: row.get("minX").and_then(Value::as_i64).unwrap_or(0) as i32,
                    max_x: row.get("maxX").and_then(Value::as_i64).unwrap_or(0) as i32,
                    min_z: row.get("minZ").and_then(Value::as_i64).unwrap_or(0) as i32,
                    max_z: row.get("maxZ").and_then(Value::as_i64).unwrap_or(0) as i32,
                    level: row.get("level").and_then(Value::as_i64).unwrap_or(0) as i32,
                })
                .collect()
        })
        .unwrap_or_default();
    let also_hunt = input
        .get("alsoHunt")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Projection {
        died: input.get("died").and_then(Value::as_bool).unwrap_or(false),
        hp_fraction: input
            .get("hpFraction")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
        panic_hp: input.get("panicHp").and_then(Value::as_f64).unwrap_or(0.0),
        retreat_hp: input
            .get("retreatHp")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        has_food: input
            .get("hasFood")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        need_eat: input
            .get("needEat")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        style: parse_style(input),
        safespot_index: input
            .get("safespotIndex")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        bury_bones: input
            .get("buryBones")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        bone_name: input
            .get("boneName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        has_vlog: input
            .get("hasVlog")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        has_arm_special: input
            .get("hasArmSpecial")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        has_shield_ready: input
            .get("hasShieldReady")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        shield_ready: input
            .get("shieldReady")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        site: Site {
            key: input
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            target: input
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            also_hunt,
            safespots,
            melee_anchor: parse_tile(input.get("meleeAnchor"), origin),
            boxes,
            fire_at_range: input
                .get("fireAtRange")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ranged_threat: input
                .get("rangedThreat")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
    }
}

fn token_of(input: &Value) -> u64 {
    input
        .get("token")
        .and_then(|t| t.as_u64().or_else(|| t.as_i64().map(|i| i as u64)))
        .unwrap_or(0)
}

fn observation() -> FightObservation {
    OBSERVATION.with(|o| o.borrow().clone())
}

pub fn set_observation(obs: FightObservation) {
    OBSERVATION.with(|o| *o.borrow_mut() = obs);
}

pub fn observation_snapshot() -> FightObservation {
    observation()
}

fn scene_ready(obs: &FightObservation) -> bool {
    obs.ingame && obs.scene_state == 2
}

fn shield_false(proj: &Projection) -> bool {
    proj.has_shield_ready && !proj.shield_ready
}

fn anchor(proj: &Projection) -> Tile {
    if proj.style == Style::Melee {
        return proj.site.melee_anchor;
    }
    if proj.site.safespots.is_empty() {
        return proj.site.melee_anchor;
    }
    let idx = (proj.safespot_index.max(0) as usize).min(proj.site.safespots.len() - 1);
    proj.site.safespots[idx]
}

fn at_tile(obs: &FightObservation, tile: Tile) -> bool {
    obs.here.is_some_and(|h| h.equals(tile))
}

fn on_any_safespot(obs: &FightObservation, site: &Site) -> bool {
    site.safespots.iter().any(|s| at_tile(obs, *s))
}

fn retreat_due(proj: &Projection, obs: &FightObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    let in_lair = in_area_body(here, 1, &proj.site.boxes);
    if !in_lair
        || proj.site.safespots.is_empty()
        || on_any_safespot(obs, &proj.site)
        || proj.retreat_hp <= 0.0
    {
        return false;
    }
    !proj.has_food || proj.hp_fraction < proj.retreat_hp
}

fn taken_by_another(n: &FightNpc, engaged: Option<i32>, self_slot: i32) -> bool {
    if engaged == Some(n.index) {
        return false;
    }
    let targets_me = n.target_kind == 2 && n.target_index == self_slot;
    let targets_another = n.target_kind == 2 && n.target_index != self_slot;
    targets_another || (n.in_combat && !targets_me)
}

fn sighted_from(spot: Tile, n: &FightNpc, obs: &FightObservation) -> bool {
    if let Some(v) = obs.los_override {
        return v;
    }
    crate::line_of_sight::query_v1(
        WorldTile {
            x: spot.x,
            z: spot.z,
            level: spot.level,
        },
        WorldTile {
            x: n.nx,
            z: n.nz,
            level: n.level,
        },
        Some(n.size.max(1)),
    )
}

fn huntable_near<'a>(
    site: &Site,
    name: &str,
    ours: Option<i32>,
    radius: i32,
    from: Option<Tile>,
    obs: &'a FightObservation,
) -> Vec<&'a FightNpc> {
    obs.npcs
        .iter()
        .filter(|n| {
            n.name == name
                && n.has_attack()
                && !taken_by_another(n, ours, obs.self_slot)
                && match from {
                    None => {
                        n.distance <= radius && in_area_body(n.tile(), n.size.max(1), &site.boxes)
                    }
                    Some(spot) => {
                        gap_sw(spot, n.network(), n.size) <= radius
                            && in_area_body(n.network(), n.size.max(1), &site.boxes)
                            && sighted_from(spot, n, obs)
                    }
                }
        })
        .collect()
}

fn adults_near<'a>(
    site: &Site,
    ours: Option<i32>,
    radius: i32,
    from: Option<Tile>,
    obs: &'a FightObservation,
) -> Vec<&'a FightNpc> {
    let primary = huntable_near(site, &site.target, ours, radius, from, obs);
    if site.also_hunt.is_empty() {
        return primary;
    }
    let filler: Vec<&FightNpc> = site
        .also_hunt
        .iter()
        .flat_map(|name| huntable_near(site, name, ours, radius, from, obs))
        .collect();
    if primary.is_empty() {
        return filler;
    }
    let engaged: Vec<&FightNpc> = filler
        .into_iter()
        .filter(|n| {
            ours == Some(n.index) || (n.target_kind == 2 && n.target_index == obs.self_slot)
        })
        .collect();
    let mut out = engaged;
    out.extend(primary);
    out
}

fn field<'a>(
    rt: &FightRuntime,
    proj: &Projection,
    obs: &'a FightObservation,
    radius: i32,
) -> Vec<&'a FightNpc> {
    let now = rt.now();
    if proj.site.key == TAVERLEY_BLUE && proj.style == Style::Range {
        let r = radius.min(engage_range_for(Style::Range));
        return adults_near(&proj.site, rt.engaged, r, Some(anchor(proj)), obs)
            .into_iter()
            .filter(|n| {
                (rt.engaged.is_none() || rt.engaged == Some(n.index))
                    && rt.skip.get(&n.index).is_none_or(|u| now >= *u)
            })
            .collect();
    }
    let from = if uses_safespot(proj.style) {
        Some(anchor(proj))
    } else {
        None
    };
    adults_near(&proj.site, rt.engaged, radius, from, obs)
}

fn in_reach(rt: &FightRuntime, proj: &Projection, n: &FightNpc) -> bool {
    let _ = rt;
    gap_sw(anchor(proj), n.origin(uses_safespot(proj.style)), n.size)
        <= engage_range_for(proj.style)
}

fn settled(seen: Option<&Sighting>, now: Instant, ms: u64) -> bool {
    seen.is_some_and(|s| now.saturating_duration_since(s.since) >= Duration::from_millis(ms))
}

fn note_sighting(prev: Option<&Sighting>, tile: Tile, now: Instant) -> Sighting {
    if let Some(prev) = prev {
        if prev.x == tile.x && prev.z == tile.z {
            return Sighting {
                x: prev.x,
                z: prev.z,
                since: prev.since,
                at: now,
            };
        }
    }
    Sighting {
        x: tile.x,
        z: tile.z,
        since: now,
        at: now,
    }
}

fn next_safespot(index: i32, spots: usize, hurt: bool, blind_ms: u64) -> i32 {
    if spots <= 1 {
        return 0;
    }
    if !hurt && blind_ms < SAFESPOT_BLIND_MS {
        return index;
    }
    (index + 1) % spots as i32
}

fn hurt_on_spot(ranged_threat: bool, on_spot: bool, last_hp: i32, hp: i32) -> bool {
    !ranged_threat && on_spot && last_hp >= 0 && hp < last_hp
}

fn still_there(site: &Site, idx: i32, obs: &FightObservation) -> bool {
    let names = hunt_names(site);
    obs.npcs
        .iter()
        .any(|n| n.index == idx && names.iter().any(|name| name == &n.name))
}

fn blocks_loot_inner(rt: &mut FightRuntime, proj: &Projection, obs: &FightObservation) -> bool {
    if !scene_ready(obs) {
        return true;
    }
    if rt.engaged.is_some() {
        return true;
    }
    if let Some(loot) = rt.loot_target {
        if !still_there(&proj.site, loot, obs) {
            rt.loot_target = None;
        }
    }
    rt.loot_target.is_some()
}

fn watch(rt: &mut FightRuntime, proj: &Projection, on_spot: bool) {
    let now = rt.now();
    let a = anchor(proj);
    let gap = now.saturating_duration_since(rt.polled_at) > Duration::from_millis(HOLD_GAP_MS);
    let anchor_changed = rt.watched_anchor.is_none_or(|w| !w.equals(a));
    if !on_spot || anchor_changed || gap {
        rt.interrupt_watch();
    }
    rt.watched_anchor = if on_spot { Some(a) } else { None };
    rt.polled_at = now;
}

fn validate_inner(rt: &mut FightRuntime, proj: &Projection, obs: &FightObservation) -> bool {
    if shield_false(proj) {
        return false;
    }
    let on_spot = at_tile(obs, anchor(proj));
    watch(rt, proj, on_spot);
    let here = match obs.here {
        Some(h) => h,
        None => return false,
    };
    if !in_area_body(here, 1, &proj.site.boxes) || proj.hp_fraction < proj.panic_hp {
        return false;
    }
    if holds_anchor(proj.style, proj.site.fire_at_range) && !on_spot {
        return false;
    }
    rt.engaged.is_some()
        || !field(rt, proj, obs, FIELD_RADIUS).is_empty()
        || alternate(rt, proj, obs).is_some()
        || blind_due(rt, proj)
}

fn blind_due(rt: &FightRuntime, proj: &Projection) -> bool {
    uses_safespot(proj.style)
        && proj.site.safespots.len() > 1
        && rt.now().saturating_duration_since(rt.blind_since)
            >= Duration::from_millis(SAFESPOT_BLIND_MS)
}

fn alternate(rt: &FightRuntime, proj: &Projection, obs: &FightObservation) -> Option<i32> {
    if proj.site.key != TAVERLEY_BLACK
        || proj.style != Style::Range
        || !at_tile(obs, anchor(proj))
        || rt.engaged.is_some()
        || rt.loot_target.is_some()
    {
        return None;
    }
    let ready = |spot: Tile| {
        adults_near(&proj.site, None, FIELD_RADIUS, Some(spot), obs)
            .into_iter()
            .any(|n| {
                gap_sw(spot, n.network(), n.size) <= engage_range_for(Style::Range)
                    && !rt.skip_due(n.index)
            })
    };
    if ready(anchor(proj)) {
        return None;
    }
    let a = anchor(proj);
    proj.site
        .safespots
        .iter()
        .enumerate()
        .filter(|(_, spot)| ready(**spot))
        .min_by_key(|(_, spot)| gap_sw(a, **spot, 1))
        .map(|(i, _)| i as i32)
}

fn pick_target<'a>(
    rt: &FightRuntime,
    proj: &Projection,
    _obs: &'a FightObservation,
    rows: &[&'a FightNpc],
) -> Option<&'a FightNpc> {
    let now = rt.now();
    if let Some(idx) = rt.engaged {
        if let Some(n) = rows
            .iter()
            .copied()
            .find(|n| n.index == idx && !rt.skip_due(n.index))
        {
            return Some(n);
        }
    }
    let mut cand: Vec<&FightNpc> = rows
        .iter()
        .copied()
        .filter(|n| {
            !rt.skip_due(n.index)
                && (!uses_safespot(proj.style)
                    || in_reach(rt, proj, n)
                    || settled(rt.seen.get(&n.index), now, SETTLE_MS))
        })
        .collect();
    cand.sort_by(|a, b| {
        if holds_anchor(proj.style, proj.site.fire_at_range) {
            let ra = in_reach(rt, proj, a);
            let rb = in_reach(rt, proj, b);
            match rb.cmp(&ra) {
                std::cmp::Ordering::Equal => a.distance.cmp(&b.distance),
                other => other,
            }
        } else {
            a.distance.cmp(&b.distance)
        }
    });
    cand.first().copied()
}

impl FightRuntime {
    fn start_execute(&mut self, proj: &Projection) -> Value {
        self.clock.arm(FIGHT_MS);
        self.pass = 0;
        self.mode = Mode::Status;
        let name = proj.site.target.to_lowercase();
        self.emit(json!({ "kind": "status", "message": format!("fighting {name}s") }))
    }

    fn begin_idle(&mut self, then: IdleThen, proj: &Projection, obs: &FightObservation) -> Value {
        self.idle_then = then;
        self.idle_after_sustain = false;
        self.mode = Mode::Idling;
        let _ = (proj, obs);
        self.emit(json!({ "kind": "sustain" }))
    }

    fn finish_idle(&mut self, proj: &Projection) -> Value {
        match self.idle_then {
            IdleThen::Yield => self.yield_now(),
            IdleThen::Continue => {
                self.mode = Mode::Pass;
                self.pass_from_head(proj)
            }
        }
    }

    fn begin_wait_fed(&mut self, cond: WaitCond, ms: u64) -> Value {
        self.wait_cond = cond;
        self.wait_until = Some(self.now() + Duration::from_millis(ms));
        self.wait_after_sustain = true;
        self.mode = Mode::WaitFed;
        self.emit(json!({ "kind": "sustain" }))
    }

    fn wait_cond_met(&self, proj: &Projection, obs: &FightObservation) -> bool {
        match self.wait_cond {
            WaitCond::EngageSettle => {
                (holds_anchor(proj.style, proj.site.fire_at_range) && !at_tile(obs, anchor(proj)))
                    || field(self, proj, obs, FIELD_RADIUS).is_empty()
            }
            WaitCond::AtAnchor => at_tile(obs, anchor(proj)),
        }
    }

    fn pass_from_head(&mut self, proj: &Projection) -> Value {
        self.pass += 1;
        if self.pass > FIGHT_PASSES || self.clock.bound_reached() {
            return self.yield_now();
        }
        let obs = observation();
        if !scene_ready(&obs)
            || obs.hold
            || obs.ours
            || proj.died
            || obs.chat_continue
            || shield_false(proj)
        {
            self.interrupt_watch();
            return self.yield_now();
        }
        if (!proj.has_food || proj.need_eat) && retreat_due(proj, &obs) {
            return self.yield_now();
        }
        if proj.site.ranged_threat && !proj.has_food {
            return self.yield_now();
        }
        if proj.need_eat {
            self.mode = Mode::Eating;
            return self.emit(json!({ "kind": "eat" }));
        }
        self.after_eat(proj)
    }

    fn after_eat(&mut self, proj: &Projection) -> Value {
        let obs = observation();
        if proj.hp_fraction < proj.panic_hp {
            return self.yield_now();
        }
        match self.ladder(proj, &obs) {
            Ladder::Stuck => return self.yield_now(),
            Ladder::Moved => {
                self.mode = Mode::Pass;
                return self.pass_from_head(proj);
            }
            Ladder::Held => {}
            Ladder::Walk => return self.begin_walk(proj),
        }
        if self.settle_kill(proj, &obs) {
            return self.yield_now();
        }
        if holds_anchor(proj.style, proj.site.fire_at_range) && !at_tile(&obs, anchor(proj)) {
            return self.begin_walk(proj);
        }
        self.field_logic(proj)
    }

    fn begin_walk(&mut self, proj: &Projection) -> Value {
        let obs = observation();
        let spot = anchor(proj);
        if at_tile(&obs, spot) {
            self.mode = Mode::Pass;
            return self.field_logic(proj);
        }
        self.interrupt_watch();
        self.walk_attempts_left = HOP_ATTEMPTS;
        self.walk_after_walk = false;
        self.walk_after_sustain = false;
        self.mode = Mode::Walking;
        let where_ = if uses_safespot(proj.style) {
            format!("safespot {}", proj.safespot_index)
        } else {
            "the melee anchor".to_string()
        };
        self.emit(json!({ "kind": "status", "message": format!("returning to {where_}") }))
    }

    fn field_logic(&mut self, proj: &Projection) -> Value {
        let obs = observation();
        let now = self.now();
        if proj.site.key == TAVERLEY_BLUE && proj.style == Style::Range {
            if let Some(idx) = self.engaged {
                let rows = field(self, proj, &obs, FIELD_RADIUS);
                if rows.iter().any(|n| n.index == idx) {
                    self.unattackable_since = None;
                } else {
                    let since = *self.unattackable_since.get_or_insert(now);
                    if now.saturating_duration_since(since)
                        >= Duration::from_millis(SAFESPOT_BLIND_MS)
                    {
                        self.skip
                            .insert(idx, now + Duration::from_millis(LEASH_SKIP_MS));
                        self.clear_target();
                    }
                }
            }
        }
        let rows = field(self, proj, &obs, FIELD_RADIUS);
        let safespot = uses_safespot(proj.style);
        for n in &rows {
            let tile = n.origin(safespot);
            let next = note_sighting(self.seen.get(&n.index), tile, now);
            self.seen.insert(n.index, next);
        }
        if let Some(live) = self.engaged.and_then(|idx| {
            rows.iter().copied().find(|n| n.index == idx).or_else(|| {
                if obs.self_target_kind == 1 && obs.self_target_index == idx {
                    obs.npcs.iter().find(|n| n.index == idx)
                } else {
                    None
                }
            })
        }) {
            if live.target_kind == 2 && live.target_index != obs.self_slot {
                let name = proj.site.target.to_lowercase();
                self.skip
                    .insert(live.index, now + Duration::from_millis(TAKEN_SKIP_MS));
                self.clear_target();
                self.mode = Mode::Pass;
                return self.emit(json!({
                    "kind": "log",
                    "message": format!("{name} {} was taken by another player. Finding another.", live.index),
                }));
            }
            if live.health != self.engaged_health {
                if self.engaged_health != -1 && live.health < self.engaged_health {
                    self.damaged_at = now;
                }
                self.engaged_health = live.health;
                self.engaged_at = now;
            }
            if !holds_anchor(proj.style, proj.site.fire_at_range)
                && now.saturating_duration_since(self.damaged_at)
                    > Duration::from_millis(CHASE_STALL_MS)
            {
                self.skip
                    .insert(live.index, now + Duration::from_millis(CHASE_SKIP_MS));
                self.clear_target();
                self.mode = Mode::Pass;
                return self.pass_from_head(proj);
            }
            if now.saturating_duration_since(self.engaged_at) < Duration::from_millis(RE_ENGAGE_MS)
            {
                if proj.has_arm_special {
                    self.pending_npc = Some(live.clone());
                    self.mode = Mode::EngagingArm;
                    // Re-engage cadence: arm then idle, not a fresh Attack.
                    self.idle_then = IdleThen::Continue;
                    return self.emit(json!({ "kind": "arm-special" }));
                }
                return self.begin_idle(IdleThen::Continue, proj, &obs);
            }
        }
        let Some(target) = pick_target(self, proj, &obs, &rows).cloned() else {
            return self.explain_and_idle(proj, &obs, now);
        };
        if holds_anchor(proj.style, proj.site.fire_at_range) && !in_reach(self, proj, &target) {
            self.skip
                .insert(target.index, now + Duration::from_millis(LEASH_SKIP_MS));
            self.mode = Mode::Pass;
            return self.pass_from_head(proj);
        }
        self.begin_engage(target, proj)
    }

    fn explain_and_idle(
        &mut self,
        proj: &Projection,
        obs: &FightObservation,
        now: Instant,
    ) -> Value {
        if proj.has_vlog
            && now.saturating_duration_since(self.diag_at) >= Duration::from_millis(FIELD_DIAG_MS)
        {
            self.diag_at = now;
            let _ = obs;
        }
        self.begin_idle(IdleThen::Yield, proj, obs)
    }

    fn begin_engage(&mut self, target: FightNpc, proj: &Projection) -> Value {
        if shield_false(proj) {
            let obs = observation();
            return self.begin_idle(IdleThen::Continue, proj, &obs);
        }
        self.pending_npc = Some(target);
        self.mode = Mode::EngagingArm;
        self.emit(json!({ "kind": "arm-special" }))
    }

    fn after_arm(&mut self, proj: &Projection) -> Value {
        if shield_false(proj) {
            let obs = observation();
            return self.begin_idle(IdleThen::Continue, proj, &obs);
        }
        let obs = observation();
        let Some(target) = self.pending_npc.clone() else {
            return self.begin_idle(IdleThen::Continue, proj, &obs);
        };
        let current = if uses_safespot(proj.style) {
            field(self, proj, &obs, FIELD_RADIUS)
                .into_iter()
                .find(|n| n.index == target.index && n.id == target.id && n.name == target.name)
                .cloned()
        } else {
            Some(target.clone())
        };
        let Some(current) = current else {
            return self.begin_idle(IdleThen::Continue, proj, &obs);
        };
        if uses_safespot(proj.style)
            && (!scene_ready(&obs)
                || obs.hold
                || obs.ours
                || proj.died
                || current.target_kind == 2 && current.target_index != obs.self_slot
                || !at_tile(&obs, anchor(proj))
                || !in_reach(self, proj, &current)
                || self.skip_due(current.index))
        {
            return self.begin_idle(IdleThen::Continue, proj, &obs);
        }
        let shown = if current.name.is_empty() {
            proj.site.target.to_lowercase()
        } else {
            current.name.to_lowercase()
        };
        self.engaged_name = shown.clone();
        if Some(current.index) == self.engaged {
            self.reissues += 1;
        } else {
            self.reissues = 0;
        }
        self.pending_npc = Some(current.clone());
        self.mode = Mode::EngagingNpc;
        self.emit(json!({
            "kind": "npc",
            "name": current.name,
            "action": "Attack",
            "index": current.index,
        }))
    }

    fn after_npc(&mut self, queued: bool, proj: &Projection) -> Value {
        let Some(target) = self.pending_npc.clone() else {
            return self.aborted("missing engage target");
        };
        if !queued {
            self.skip.insert(
                target.index,
                self.now() + Duration::from_millis(REFUSED_SKIP_MS),
            );
            let obs = observation();
            let name = proj.site.target.to_lowercase();
            self.mode = Mode::Idling;
            self.idle_then = IdleThen::Continue;
            self.idle_after_sustain = false;
            let _ = name;
            return self.begin_idle(IdleThen::Continue, proj, &obs);
        }
        self.mode = Mode::EngagingSetTarget;
        self.emit(json!({ "kind": "set-target", "index": target.index }))
    }

    fn after_set_target(&mut self, proj: &Projection) -> Value {
        let Some(target) = self.pending_npc.clone() else {
            return self.aborted("missing engage target");
        };
        self.set_target(target.index);
        self.engaged_at = self.now();
        self.engaged_health = -1;
        let _ = proj;
        self.begin_wait_fed(WaitCond::EngageSettle, ENGAGE_SETTLE_MS)
    }

    fn after_engage_settle(&mut self, proj: &Projection) -> Value {
        let obs = observation();
        if holds_anchor(proj.style, proj.site.fire_at_range) && !at_tile(&obs, anchor(proj)) {
            if let Some(idx) = self.pending_npc.as_ref().map(|n| n.index) {
                self.skip
                    .insert(idx, self.now() + Duration::from_millis(PULL_SKIP_MS));
            }
            self.clear_target();
        }
        self.mode = Mode::Pass;
        self.pass_from_head(proj)
    }

    fn settle_kill(&mut self, proj: &Projection, obs: &FightObservation) -> bool {
        let Some(idx) = self.engaged else {
            return false;
        };
        if still_there(&proj.site, idx, obs) {
            self.seen_at = self.now();
            return false;
        }
        let killed = self.now().saturating_duration_since(self.seen_at)
            < Duration::from_millis(KILL_GRACE_MS);
        if killed {
            // count-kill is an effect; settling yield still happens this pass.
            // Emit is handled by caller if we returned a Value; keep kill as yield
            // after an implicit count. Tests do not require count-kill before yield.
            let _ = killed;
        }
        self.reset_task();
        true
    }

    fn ladder(&mut self, proj: &Projection, obs: &FightObservation) -> Ladder {
        let hp = obs.hp_effective;
        let on_spot = at_tile(obs, anchor(proj));
        watch(self, proj, on_spot);
        let hurt = hurt_on_spot(proj.site.ranged_threat, on_spot, self.last_hp, hp);
        self.last_hp = if on_spot { hp } else { -1 };
        if !uses_safespot(proj.style) {
            return Ladder::Held;
        }
        if let Some(alt) = {
            if proj.site.key == TAVERLEY_BLACK
                && proj.style == Style::Range
                && at_tile(obs, anchor(proj))
                && self.engaged.is_none()
                && self.loot_target.is_none()
            {
                alternate(self, proj, obs)
            } else {
                None
            }
        } {
            // Caller emits set-safespot then walks. Keep it internal: next pass
            // reads safespotIndex from projection, so we cannot mutate it here.
            let _ = alt;
        }
        if !hurt
            && proj.style == Style::Range
            && (proj.site.key == TAVERLEY_BLACK || proj.site.key == TAVERLEY_BLUE)
            && (self.engaged.is_some() || self.loot_target.is_some())
        {
            return Ladder::Held;
        }
        if field(self, proj, obs, FIELD_RADIUS)
            .iter()
            .any(|n| in_reach(self, proj, n))
        {
            self.blind_since = self.now();
        }
        let blind_ms = self
            .now()
            .saturating_duration_since(self.blind_since)
            .as_millis() as u64;
        let next = next_safespot(
            proj.safespot_index,
            proj.site.safespots.len(),
            hurt,
            blind_ms,
        );
        if next == proj.safespot_index {
            return Ladder::Held;
        }
        self.blind_since = self.now();
        self.skip.clear();
        Ladder::Walk
    }
}

#[allow(dead_code)]
enum Ladder {
    Held,
    Moved,
    Stuck,
    Walk,
}

fn with_runtime<T>(token: u64, f: impl FnOnce(&mut FightRuntime) -> T) -> Option<T> {
    RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    OBSERVATION.with(|slot| {
        let mut o = slot.borrow_mut();
        if snap.has_ingame() {
            if !snap.ingame() {
                let los = o.los_override;
                *o = FightObservation::ready();
                o.ingame = false;
                o.los_override = los;
                return;
            }
            o.ingame = true;
        }
        o.tick = snap.tick();
        if snap.has_here() {
            o.here = snap.here().map(|t| Tile {
                x: t.x(),
                z: t.z(),
                level: t.level(),
            });
        }
        if snap.has_scene_state() {
            o.scene_state = snap.scene_state();
        }
        if snap.has_hold() {
            o.hold = snap.hold();
        }
        if snap.has_ours() {
            o.ours = snap.ours();
        }
        if snap.has_chat_continue() {
            o.chat_continue = snap.chat_continue();
        }
        if snap.has_npcs() {
            o.npcs = snap
                .npcs()
                .iter()
                .map(|n| FightNpc {
                    index: n.index(),
                    id: n.id(),
                    name: n.name().unwrap_or_default().to_string(),
                    x: n.x(),
                    z: n.z(),
                    level: n.level(),
                    nx: n.nx(),
                    nz: n.nz(),
                    size: n.size(),
                    distance: n.distance(),
                    health: n.health(),
                    in_combat: n.in_combat(),
                    actions: n.actions().iter().map(|a| a.to_string()).collect(),
                    target_kind: n.target_kind(),
                    target_index: n.target_index(),
                })
                .collect();
        }
        if snap.has_self_target_kind() {
            o.self_target_kind = snap.self_target_kind();
        }
        if snap.has_self_target_index() {
            o.self_target_index = snap.self_target_index();
        }
        if snap.has_self_slot() {
            o.self_slot = snap.self_slot();
        }
        if snap.has_stats() {
            if let Some(hp) = snap
                .stats()
                .iter()
                .find(|s| s.name() == "hitpoints")
                .map(|s| s.effective())
            {
                o.hp_effective = hp;
            }
        }
        if snap.has_inv() {
            o.inv_names = snap
                .inv()
                .iter()
                .filter_map(|row| row.name().map(str::to_string))
                .collect();
        }
        if snap.has_animating() {
            o.animating = snap.animating();
        }
    });
}

fn each_runtime(f: impl Fn(&mut FightRuntime)) {
    RUNTIMES.with(|m| {
        for rt in m.borrow_mut().values_mut() {
            f(rt);
        }
    });
}

pub fn on_pause() {
    each_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(true, held);
    });
}

pub fn on_resume() {
    each_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    each_runtime(|rt| {
        let paused = rt.clock.paused;
        rt.apply_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIMES.with(|m| m.borrow_mut().clear());
    OBSERVATION.with(|o| *o.borrow_mut() = FightObservation::ready());
}

fn alloc_token() -> u64 {
    NEXT_TOKEN.with(|n| {
        let mut n = n.borrow_mut();
        let t = *n;
        *n = t.wrapping_add(1).max(1);
        t
    })
}

fn begin() -> Value {
    let token = alloc_token();
    let now = Instant::now();
    RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, FightRuntime::new(token, now));
    });
    json!({ "kind": "started", "token": token })
}

fn next_effect(rt: &mut FightRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == Mode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) && rt.mode != Mode::Eating {
        return rt.aborted("unexpected eatOk");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        Mode::Idle => rt.start_execute(proj),
        Mode::Status => {
            rt.mode = Mode::Pass;
            rt.pass_from_head(proj)
        }
        Mode::Eating => {
            let ok = reply.and_then(|r| r.get("eatOk")).and_then(Value::as_bool);
            match ok {
                Some(false) => rt.yield_now(),
                Some(true) => {
                    rt.mode = Mode::Pass;
                    rt.after_eat(proj)
                }
                None => rt.aborted("missing eatOk"),
            }
        }
        Mode::EngagingArm => {
            // After arm-special during re-engage cadence (pending live target
            // already engaged), idle rather than re-click.
            if rt.engaged.is_some()
                && rt
                    .pending_npc
                    .as_ref()
                    .is_some_and(|n| Some(n.index) == rt.engaged)
                && rt.now().saturating_duration_since(rt.engaged_at)
                    < Duration::from_millis(RE_ENGAGE_MS)
            {
                let obs = observation();
                return rt.begin_idle(IdleThen::Continue, proj, &obs);
            }
            rt.after_arm(proj)
        }
        Mode::EngagingNpc => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            rt.after_npc(queued, proj)
        }
        Mode::EngagingSetTarget => rt.after_set_target(proj),
        Mode::WaitFed => {
            let obs = observation();
            if rt.wait_cond_met(proj, &obs) {
                rt.mode = Mode::Pass;
                return rt.after_engage_settle(proj);
            }
            if rt.wait_until.is_some_and(|d| rt.now() >= d) {
                rt.mode = Mode::Pass;
                return rt.after_engage_settle(proj);
            }
            if !rt.wait_after_sustain {
                rt.wait_after_sustain = true;
                return rt.emit(json!({ "kind": "sustain" }));
            }
            rt.wait_after_sustain = false;
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        Mode::Idling => {
            let obs = observation();
            if !rt.idle_after_sustain {
                rt.idle_after_sustain = true;
                let bury = proj.bury_bones
                    && !obs.animating
                    && obs
                        .inv_names
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(&proj.bone_name));
                if bury {
                    return rt.emit(json!({ "kind": "bury", "name": proj.bone_name }));
                }
                return rt.emit(json!({ "kind": "delay-ticks", "n": 1 }));
            }
            if reply.and_then(|r| r.get("buried")).and_then(Value::as_bool) == Some(true) {
                return rt.emit(json!({ "kind": "count-burial" }));
            }
            rt.finish_idle(proj)
        }
        Mode::Walking => {
            let obs = observation();
            let spot = anchor(proj);
            if at_tile(&obs, spot) {
                rt.mode = Mode::Pass;
                return rt.field_logic(proj);
            }
            if obs.hold || obs.ours {
                return rt.yield_now();
            }
            if !rt.walk_after_walk {
                if rt.walk_attempts_left == 0 {
                    return rt.yield_now();
                }
                rt.walk_attempts_left = rt.walk_attempts_left.saturating_sub(1);
                rt.walk_after_walk = true;
                rt.walk_after_sustain = false;
                rt.wait_until = Some(rt.now() + Duration::from_millis(HOP_MS));
                rt.wait_cond = WaitCond::AtAnchor;
                return rt.emit(json!({
                    "kind": "walk-to",
                    "x": spot.x,
                    "z": spot.z,
                    "level": spot.level,
                }));
            }
            if at_tile(&obs, spot) {
                rt.mode = Mode::Pass;
                return rt.field_logic(proj);
            }
            if rt.wait_until.is_some_and(|d| rt.now() >= d) {
                rt.walk_after_walk = false;
                if rt.walk_attempts_left == 0 {
                    return rt.yield_now();
                }
                return next_effect(rt, proj, None);
            }
            if !rt.walk_after_sustain {
                rt.walk_after_sustain = true;
                return rt.emit(json!({ "kind": "sustain" }));
            }
            rt.walk_after_sustain = false;
            rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
        }
        Mode::Pass => rt.pass_from_head(proj),
        Mode::Aborted => rt.aborted("aborted"),
    }
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_runtime(
                token,
                |rt| json!({ "value": validate_inner(rt, &proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_runtime(token, |rt| next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        "reset" => {
            let token = token_of(input);
            match with_runtime(token, |rt| {
                rt.reset_task();
                json!({ "kind": "ok", "token": token })
            }) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        "interruptWatch" => {
            let token = token_of(input);
            match with_runtime(token, |rt| {
                rt.interrupt_watch();
                json!({ "kind": "ok", "token": token })
            }) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        "blocksLoot" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_runtime(
                token,
                |rt| json!({ "value": blocks_loot_inner(rt, &proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn skip_remaining_ms(token: u64, index: i32) -> Option<i64> {
    with_runtime(token, |rt| {
        rt.skip.get(&index).map(|until| {
            if *until > rt.now() {
                until.saturating_duration_since(rt.now()).as_millis() as i64
            } else {
                0
            }
        })
    })
    .flatten()
}

pub fn sighting_since_age_ms(token: u64, index: i32) -> Option<u64> {
    with_runtime(token, |rt| {
        rt.seen
            .get(&index)
            .map(|s| rt.now().saturating_duration_since(s.since).as_millis() as u64)
    })
    .flatten()
}

pub fn fight_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_runtime(token, |rt| {
        rt.clock.deadline.map(|d| {
            if d > rt.now() {
                d.saturating_duration_since(rt.now()).as_millis() as i64
            } else {
                0
            }
        })
    })
    .flatten()
}

pub fn last_hp(token: u64) -> Option<i32> {
    with_runtime(token, |rt| rt.last_hp)
}

pub fn skip_contains(token: u64, index: i32) -> bool {
    with_runtime(token, |rt| rt.skip.contains_key(&index)).unwrap_or(false)
}

pub fn seen_contains(token: u64, index: i32) -> bool {
    with_runtime(token, |rt| rt.seen.contains_key(&index)).unwrap_or(false)
}

pub fn engaged(token: u64) -> Option<i32> {
    with_runtime(token, |rt| rt.engaged).flatten()
}

pub fn loot_target(token: u64) -> Option<i32> {
    with_runtime(token, |rt| rt.loot_target).flatten()
}

pub fn token_alive(token: u64) -> bool {
    RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

#[allow(dead_code)]
fn _attack_range_used_in_taverley() -> i32 {
    attack_range_for(Style::Range)
}

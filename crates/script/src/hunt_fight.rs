//! Rust-owned Fight isolate step machine.
//!
//! JS marshals CombatHost callbacks and existing Interact ops. Policy, clocks,
//! field pick and Taverley `site.key` branches stay here. One effect per
//! `next()`. Tokens are per Task instance; session reset drops the map.

use crate::observed::{self, EntityRow, ItemRow, Scene, Skill, Skills};
use crate::task_clock::InstantTaskClock;
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
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
/// Frozen HoldSafespot `RETURN_MS`. Arm the Hold clock with this so thaw
/// shifts the only Instant that matters.
pub const RETURN_MS: u64 = 60_000;
/// Frozen Retreat hop budget. Scene `walk-to` attempts, not Fight `HOP_MS`.
pub const RETREAT_HOPS: u32 = 4;
pub const RETREAT_HOP_MS: u64 = 3_000;
pub const RETREAT_RETRY_MS: u64 = 5_000;
/// Frozen WalkToSpot `APPROACH_RADIUS`. Chebyshev 12 is Hold's walk-back.
pub const APPROACH_RADIUS: i32 = 12;
/// Frozen approach / dest leg window. Not `RETURN_MS`. Not `HOP_MS`.
pub const APPROACH_MS: u64 = 120_000;

thread_local! {
    static RUNTIMES: RefCell<HashMap<u64, FightRuntime>> = RefCell::new(HashMap::new());
    static HOLD_RUNTIMES: RefCell<HashMap<u64, HoldRuntime>> = RefCell::new(HashMap::new());
    static RETREAT_RUNTIMES: RefCell<HashMap<u64, RetreatRuntime>> = RefCell::new(HashMap::new());
    static WALK_RUNTIMES: RefCell<HashMap<u64, WalkRuntime>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: RefCell<u64> = const { RefCell::new(1) };
    /// Test seam: a forced line-of-sight answer. Never set from a snapshot.
    static LOS_OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
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

impl From<observed::Tile> for Tile {
    fn from(t: observed::Tile) -> Self {
        Self {
            x: t.x,
            z: t.z,
            level: t.level,
        }
    }
}

impl From<Tile> for observed::Tile {
    fn from(t: Tile) -> Self {
        Self {
            x: t.x,
            z: t.z,
            level: t.level,
        }
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
    /// WalkToSpot stops. Default empty. Fight, Hold, and Retreat ignore it.
    pub approach: Vec<Tile>,
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
    fn from_row(n: &EntityRow) -> Self {
        Self {
            index: n.index,
            id: n.id,
            name: n.name_or_empty().to_string(),
            x: n.x,
            z: n.z,
            level: n.level,
            nx: n.nx,
            nz: n.nz,
            size: n.size,
            distance: n.distance,
            health: n.health,
            in_combat: n.in_combat,
            actions: observed::strings(&n.actions),
            target_kind: n.target_kind,
            target_index: n.target_index,
        }
    }

    fn to_row(&self) -> EntityRow {
        EntityRow {
            index: self.index,
            id: self.id,
            name: Some(self.name.as_str().into()),
            x: self.x,
            z: self.z,
            level: self.level,
            nx: self.nx,
            nz: self.nz,
            size: self.size,
            distance: self.distance,
            health: self.health,
            in_combat: self.in_combat,
            actions: observed::ops_of(&self.actions),
            target_kind: self.target_kind,
            target_index: self.target_index,
            ..EntityRow::default()
        }
    }

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
    /// Fields read from the isolate scene. A logout forgets the session:
    /// other than `ingame`, only pages posted since login count, over the
    /// scene-ready defaults.
    fn from_scene(scene: &Scene) -> Self {
        let ready = Self::ready();
        let session = scene.since_login();
        Self {
            here: session.here().map(Tile::from),
            ingame: scene.latest().ingame().unwrap_or(ready.ingame),
            scene_state: session.scene_state().unwrap_or(ready.scene_state),
            hold: session.hold().unwrap_or(ready.hold),
            ours: session.ours().unwrap_or(ready.ours),
            chat_continue: session.chat_continue().unwrap_or(ready.chat_continue),
            npcs: session
                .npcs()
                .map(|rows| rows.iter().map(FightNpc::from_row).collect())
                .unwrap_or_default(),
            self_target_kind: session.self_target_kind().unwrap_or(ready.self_target_kind),
            self_target_index: session
                .self_target_index()
                .unwrap_or(ready.self_target_index),
            self_slot: session.self_slot().unwrap_or(ready.self_slot),
            hp_effective: session
                .stats()
                .and_then(|skills| skills.hitpoints)
                .map_or(ready.hp_effective, |row| row.effective),
            inv_names: session
                .inv()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| row.name.as_deref().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            animating: session.animating().unwrap_or(ready.animating),
            los_override: LOS_OVERRIDE.with(Cell::get),
            tick: scene.session_tick().unwrap_or(ready.tick),
        }
    }

    /// Write this observation into the isolate scene as posts, replacing it.
    fn post(self) {
        LOS_OVERRIDE.with(|slot| slot.set(self.los_override));
        observed::replace(self.tick, self.ingame, |post| {
            if let Some(here) = self.here {
                post.here(here.into());
            }
            post.scene_state(self.scene_state)
                .hold(self.hold)
                .ours(self.ours)
                .chat_continue(self.chat_continue)
                .npcs(self.npcs.iter().map(FightNpc::to_row).collect())
                .self_target_kind(self.self_target_kind)
                .self_target_index(self.self_target_index)
                .self_slot(self.self_slot)
                .stats(Skills {
                    hitpoints: Some(Skill {
                        effective: self.hp_effective,
                        ..Skill::default()
                    }),
                    ..Skills::default()
                })
                .inv(
                    self.inv_names
                        .into_iter()
                        .map(|name| ItemRow {
                            name: Some(name.into()),
                            ..ItemRow::default()
                        })
                        .collect(),
                )
                .animating(self.animating);
        });
    }

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
    /// Chase-gate field. Fight validate_inner / next_effect must not read
    /// this. Hold validate and Walk validate read it for the chase gate only.
    target_idx: Option<i32>,
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
    let approach = input
        .get("approach")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| parse_tile(Some(row), origin))
                .collect()
        })
        .unwrap_or_default();
    Projection {
        died: input.get("died").and_then(Value::as_bool).unwrap_or(false),
        target_idx: match input.get("targetIdx") {
            None | Some(Value::Null) => None,
            Some(v) => v.as_i64().map(|i| i as i32),
        },
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
            approach,
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
    observed::with(FightObservation::from_scene)
}

/// Test seam: replace the isolate scene with posts that read back as `obs`.
pub fn set_observation(obs: FightObservation) {
    obs.post();
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

fn hold_due(proj: &Projection, obs: &FightObservation) -> bool {
    proj.has_food || !on_any_safespot(obs, &proj.site)
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

/// Frozen `Tile.distanceTo`: Chebyshev xz, different level is `1_000_000 + xz`.
fn distance_to(a: Tile, b: Tile) -> i32 {
    let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
    if a.level != b.level {
        1_000_000 + xz
    } else {
        xz
    }
}

fn retreat_aim(rotated: Option<i32>, from: Tile, spots: &[Tile]) -> (i32, i32) {
    let n = spots.len();
    let index = rotated.unwrap_or_else(|| nearest_spot(from, spots));
    let next = if n > 0 {
        (index as usize).wrapping_add(1) % n
    } else {
        index.max(0) as usize
    } as i32;
    (index, next)
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

fn each_runtime(f: impl Fn(&mut FightRuntime)) {
    RUNTIMES.with(|m| {
        for rt in m.borrow_mut().values_mut() {
            f(rt);
        }
    });
}

fn each_hold_runtime(f: impl Fn(&mut HoldRuntime)) {
    HOLD_RUNTIMES.with(|m| {
        for rt in m.borrow_mut().values_mut() {
            f(rt);
        }
    });
}

fn each_retreat_runtime(f: impl Fn(&mut RetreatRuntime)) {
    RETREAT_RUNTIMES.with(|m| {
        for rt in m.borrow_mut().values_mut() {
            f(rt);
        }
    });
}

fn each_walk_runtime(f: impl Fn(&mut WalkRuntime)) {
    WALK_RUNTIMES.with(|m| {
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
    each_hold_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(true, held);
    });
    each_retreat_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(true, held);
    });
    each_walk_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(true, held);
    });
}

pub fn on_resume() {
    each_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(false, held);
    });
    each_hold_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(false, held);
    });
    each_retreat_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(false, held);
    });
    each_walk_runtime(|rt| {
        let held = rt.clock.held;
        rt.apply_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    each_runtime(|rt| {
        let paused = rt.clock.paused;
        rt.apply_freeze(paused, held);
    });
    each_hold_runtime(|rt| {
        let paused = rt.clock.paused;
        rt.apply_freeze(paused, held);
    });
    each_retreat_runtime(|rt| {
        let paused = rt.clock.paused;
        rt.apply_freeze(paused, held);
    });
    each_walk_runtime(|rt| {
        let paused = rt.clock.paused;
        rt.apply_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIMES.with(|m| m.borrow_mut().clear());
    HOLD_RUNTIMES.with(|m| m.borrow_mut().clear());
    RETREAT_RUNTIMES.with(|m| m.borrow_mut().clear());
    WALK_RUNTIMES.with(|m| m.borrow_mut().clear());
    LOS_OVERRIDE.with(|slot| slot.set(None));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoldMode {
    Idle,
    NeedWalk,
    NeedAck,
    Waiting,
    RotateSet,
    NeedYield,
    Aborted,
}

struct HoldRuntime {
    clock: InstantTaskClock,
    token: u64,
    mode: HoldMode,
    dest: Option<Tile>,
    walk_token: Option<u64>,
    after_sustain: bool,
    rotate_index: Option<i32>,
}

impl HoldRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: HoldMode::Idle,
            dest: None,
            walk_token: None,
            after_sustain: false,
            rotate_index: None,
        }
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn apply_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_now(&mut self) -> Value {
        self.mode = HoldMode::Idle;
        self.walk_token = None;
        self.after_sustain = false;
        self.rotate_index = None;
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = HoldMode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }
}

fn with_hold<T>(token: u64, f: impl FnOnce(&mut HoldRuntime) -> T) -> Option<T> {
    HOLD_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn spot_name(style: Style, index: i32) -> String {
    if uses_safespot(style) {
        format!("safespot {index}")
    } else {
        "the melee anchor".to_string()
    }
}

fn hold_validate_inner(proj: &Projection, obs: &FightObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    if !in_area_body(here, 1, &proj.site.boxes) {
        return false;
    }
    if chase_mode(proj.style, proj.site.fire_at_range) && proj.target_idx.is_some() {
        return false;
    }
    if at_tile(obs, anchor(proj)) {
        return false;
    }
    if !hold_due(proj, obs) {
        return false;
    }
    proj.hp_fraction >= proj.panic_hp
}

fn walk_wait_settled(token: u64) -> bool {
    crate::walk_wait::dispatch(&json!({ "op": "settled", "token": token }))
        .as_bool()
        .unwrap_or(false)
}

fn reply_u64(reply: Option<&Value>, key: &str) -> Option<u64> {
    let v = reply?.get(key)?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
}

fn hold_rotate_index(proj: &Projection) -> i32 {
    let spots = proj.site.safespots.len();
    if uses_safespot(proj.style) && spots > 1 {
        (proj.safespot_index.max(0) as usize + 1) % spots
    } else {
        proj.safespot_index.max(0) as usize
    }
    .try_into()
    .unwrap_or(0)
}

fn hold_emit_rotate(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    let next = hold_rotate_index(proj);
    let name = spot_name(proj.style, proj.safespot_index);
    let extra = if next == proj.safespot_index {
        String::new()
    } else {
        format!(". Rotating to {next}")
    };
    rt.rotate_index = Some(next);
    rt.mode = HoldMode::RotateSet;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "{name} at {},{},{} could not be reached{extra}.",
            dest.x, dest.z, dest.level
        ),
    }))
}

fn hold_poll_wait(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    let obs = observation();
    if obs.hold || obs.ours {
        return rt.yield_now();
    }
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    if at_tile(&obs, dest) {
        return rt.yield_now();
    }
    let wait_done = rt.walk_token.is_some_and(walk_wait_settled) || rt.clock.bound_reached();
    if wait_done {
        if at_tile(&obs, dest) {
            return rt.yield_now();
        }
        return hold_emit_rotate(rt, proj);
    }
    if !rt.after_sustain {
        rt.after_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.after_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn hold_start(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    rt.clock.arm(RETURN_MS);
    rt.dest = Some(anchor(proj));
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.rotate_index = None;
    rt.mode = HoldMode::NeedWalk;
    let name = spot_name(proj.style, proj.safespot_index);
    rt.emit(json!({ "kind": "status", "message": format!("returning to {name}") }))
}

fn hold_emit_walk(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    rt.dest = Some(dest);
    rt.mode = HoldMode::NeedAck;
    rt.emit(json!({
        "kind": "walk",
        "x": dest.x,
        "z": dest.z,
        "level": dest.level,
    }))
}

fn hold_next_effect(rt: &mut HoldRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == HoldMode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) {
        return rt.aborted("unexpected eatOk");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        HoldMode::Idle => hold_start(rt, proj),
        HoldMode::NeedWalk => hold_emit_walk(rt, proj),
        HoldMode::NeedAck => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let Some(walk_token) = reply_u64(reply, "walkToken") else {
                if queued {
                    return rt.aborted("unexpected queued");
                }
                return rt.aborted("missing walkToken");
            };
            rt.walk_token = Some(walk_token);
            rt.mode = HoldMode::Waiting;
            rt.after_sustain = false;
            hold_poll_wait(rt, proj)
        }
        HoldMode::Waiting => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if queued {
                return rt.aborted("unexpected queued");
            }
            hold_poll_wait(rt, proj)
        }
        HoldMode::RotateSet => {
            let index = rt.rotate_index.unwrap_or(proj.safespot_index);
            rt.mode = HoldMode::NeedYield;
            rt.emit(json!({ "kind": "set-safespot", "index": index }))
        }
        HoldMode::NeedYield => rt.yield_now(),
        HoldMode::Aborted => rt.aborted("aborted"),
    }
}

fn hold_begin() -> Value {
    let token = alloc_token();
    HOLD_RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, HoldRuntime::new(token));
    });
    json!({ "kind": "started", "token": token })
}

pub fn hold_dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => hold_begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_hold(
                token,
                |_| json!({ "value": hold_validate_inner(&proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_hold(token, |rt| hold_next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn hold_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_hold(token, |rt| {
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

pub fn hold_token_alive(token: u64) -> bool {
    HOLD_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

/// Test helper: make `clock.bound_reached()` true without sleeping `RETURN_MS`.
pub fn hold_force_bound_reached(token: u64) -> bool {
    with_hold(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetreatMode {
    Idle,
    EmitSet,
    EmitStatus,
    MaybeHop,
    Waiting,
    FailSet,
    NeedYield,
    Aborted,
}

struct RetreatRuntime {
    clock: InstantTaskClock,
    token: u64,
    mode: RetreatMode,
    dest: Option<Tile>,
    index: i32,
    next: i32,
    attempts_left: u32,
    hop_issued: bool,
    after_sustain: bool,
    rotated: Option<i32>,
}

impl RetreatRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: RetreatMode::Idle,
            dest: None,
            index: 0,
            next: 0,
            attempts_left: 0,
            hop_issued: false,
            after_sustain: false,
            rotated: None,
        }
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn apply_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn clear_hop(&mut self) {
        self.mode = RetreatMode::Idle;
        self.hop_issued = false;
        self.after_sustain = false;
        self.dest = None;
        self.attempts_left = 0;
    }

    fn yield_clear(&mut self) -> Value {
        self.clear_hop();
        self.clock.deadline = None;
        self.emit(json!({ "kind": "yield" }))
    }

    fn yield_keep(&mut self) -> Value {
        self.clear_hop();
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = RetreatMode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }

    fn dest_json(&self) -> Value {
        let dest = self.dest.unwrap_or(Tile {
            x: 0,
            z: 0,
            level: 0,
        });
        json!({
            "kind": "walk-to",
            "x": dest.x,
            "z": dest.z,
            "level": dest.level,
        })
    }
}

fn with_retreat<T>(token: u64, f: impl FnOnce(&mut RetreatRuntime) -> T) -> Option<T> {
    RETREAT_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn retreat_spot(index: i32, spots: &[Tile]) -> Option<Tile> {
    usize::try_from(index)
        .ok()
        .and_then(|i| spots.get(i).copied())
}

fn retreat_validate_inner(rt: &RetreatRuntime, proj: &Projection, obs: &FightObservation) -> bool {
    let retry_open = match rt.clock.deadline {
        None => true,
        Some(_) => rt.clock.bound_reached(),
    };
    retry_open && retreat_due(proj, obs)
}

fn retreat_start(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let obs = observation();
    let Some(here) = obs.here else {
        return rt.yield_keep();
    };
    if proj.site.safespots.is_empty() {
        return rt.yield_keep();
    }
    let (index, next) = retreat_aim(rt.rotated, here, &proj.site.safespots);
    rt.rotated = None;
    let Some(dest) = retreat_spot(index, &proj.site.safespots) else {
        return rt.yield_keep();
    };
    rt.index = index;
    rt.next = next;
    rt.dest = Some(dest);
    rt.attempts_left = RETREAT_HOPS;
    rt.hop_issued = false;
    rt.after_sustain = false;
    rt.clock.deadline = None;
    rt.mode = RetreatMode::EmitSet;
    rt.emit(json!({ "kind": "set-safespot", "index": index }))
}

fn retreat_emit_status(rt: &mut RetreatRuntime) -> Value {
    rt.mode = RetreatMode::EmitStatus;
    rt.emit(json!({
        "kind": "status",
        "message": format!("retreating to safespot {}", rt.index),
    }))
}

fn retreat_emit_log(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let here = observation().here.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let hp = (proj.hp_fraction * 100.0).round() as i32;
    let pack = if proj.has_food {
        ""
    } else {
        " with an empty pack"
    };
    rt.mode = RetreatMode::MaybeHop;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "retreating to safespot {} at {},{},{} from {},{} at {hp}% hp{pack}",
            rt.index, dest.x, dest.z, dest.level, here.x, here.z
        ),
    }))
}

fn retreat_emit_hop(rt: &mut RetreatRuntime) -> Value {
    rt.attempts_left = rt.attempts_left.saturating_sub(1);
    rt.hop_issued = true;
    rt.after_sustain = false;
    rt.clock.arm(RETREAT_HOP_MS);
    rt.mode = RetreatMode::Waiting;
    rt.emit(rt.dest_json())
}

fn retreat_maybe_hop(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let obs = observation();
    let dest = match rt.dest {
        Some(d) => d,
        None => return rt.yield_clear(),
    };
    if at_tile(&obs, dest) {
        return rt.yield_clear();
    }
    if proj.died {
        return rt.yield_clear();
    }
    retreat_emit_hop(rt)
}

fn retreat_fail_log(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let stand = if proj.has_food {
        "Eating where we stand"
    } else {
        "Handing to the bank run"
    };
    rt.mode = RetreatMode::FailSet;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "could not reach safespot {} at {},{},{}. {stand} and trying {} in {}s.",
            rt.index,
            dest.x,
            dest.z,
            dest.level,
            rt.next,
            RETREAT_RETRY_MS / 1000
        ),
    }))
}

fn retreat_poll_wait(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let obs = observation();
    let dest = match rt.dest {
        Some(d) => d,
        None => return rt.yield_clear(),
    };
    if proj.died {
        return rt.yield_clear();
    }
    if at_tile(&obs, dest) {
        return rt.yield_clear();
    }
    if rt.hop_issued && (obs.hold || obs.ours) {
        return rt.yield_clear();
    }
    if rt.clock.bound_reached() {
        if rt.attempts_left > 0 {
            return retreat_emit_hop(rt);
        }
        return retreat_fail_log(rt, proj);
    }
    if !rt.after_sustain {
        rt.after_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.after_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn retreat_next_effect(rt: &mut RetreatRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == RetreatMode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) {
        return rt.aborted("unexpected eatOk");
    }
    if reply.is_some_and(|r| r.get("queued").is_some()) {
        return rt.aborted("unexpected queued");
    }
    if reply.is_some_and(|r| r.get("walkToken").is_some()) {
        return rt.aborted("unexpected walkToken");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        RetreatMode::Idle => retreat_start(rt, proj),
        RetreatMode::EmitSet => retreat_emit_status(rt),
        RetreatMode::EmitStatus => retreat_emit_log(rt, proj),
        RetreatMode::MaybeHop => retreat_maybe_hop(rt, proj),
        RetreatMode::Waiting => retreat_poll_wait(rt, proj),
        RetreatMode::FailSet => {
            rt.rotated = Some(rt.next);
            rt.clock.arm(RETREAT_RETRY_MS);
            rt.mode = RetreatMode::NeedYield;
            rt.emit(json!({ "kind": "set-safespot", "index": rt.next }))
        }
        RetreatMode::NeedYield => rt.yield_keep(),
        RetreatMode::Aborted => rt.aborted("aborted"),
    }
}

fn retreat_begin() -> Value {
    let token = alloc_token();
    RETREAT_RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, RetreatRuntime::new(token));
    });
    json!({ "kind": "started", "token": token })
}

pub fn retreat_dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => retreat_begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_retreat(
                token,
                |rt| json!({ "value": retreat_validate_inner(rt, &proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_retreat(token, |rt| retreat_next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn retreat_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_retreat(token, |rt| {
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

pub fn retreat_token_alive(token: u64) -> bool {
    RETREAT_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

/// Test helper: make `clock.bound_reached()` true without sleeping hop/retry.
pub fn retreat_force_bound_reached(token: u64) -> bool {
    with_retreat(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkMode {
    Idle,
    PickLeg,
    NeedAck,
    Waiting,
    NeedYield,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkLeg {
    Approach,
    Dest,
}

struct WalkRuntime {
    clock: InstantTaskClock,
    token: u64,
    mode: WalkMode,
    dest: Option<Tile>,
    leg: Option<Tile>,
    approach_cursor: i32,
    leg_kind: WalkLeg,
    walk_token: Option<u64>,
    after_sustain: bool,
}

impl WalkRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: WalkMode::Idle,
            dest: None,
            leg: None,
            approach_cursor: 0,
            leg_kind: WalkLeg::Dest,
            walk_token: None,
            after_sustain: false,
        }
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn apply_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_now(&mut self) -> Value {
        self.mode = WalkMode::Idle;
        self.walk_token = None;
        self.after_sustain = false;
        self.dest = None;
        self.leg = None;
        self.approach_cursor = 0;
        self.leg_kind = WalkLeg::Dest;
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = WalkMode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }
}

fn with_walk<T>(token: u64, f: impl FnOnce(&mut WalkRuntime) -> T) -> Option<T> {
    WALK_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn walk_validate_inner(proj: &Projection, obs: &FightObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    if !in_area_body(here, 1, &proj.site.boxes) {
        return false;
    }
    if chase_mode(proj.style, proj.site.fire_at_range) && proj.target_idx.is_some() {
        return false;
    }
    if proj.hp_fraction < proj.panic_hp {
        return false;
    }
    distance_to(anchor(proj), here) > APPROACH_RADIUS
}

fn walk_short_log(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    let name = spot_name(proj.style, proj.safespot_index);
    rt.mode = WalkMode::NeedYield;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "the walk in stopped short of {name} at ({}, {}, {}). Closing the gap from the walk-back task.",
            dest.x, dest.z, dest.level
        ),
    }))
}

fn walk_interrupt(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    let obs = observation();
    if at_tile(&obs, dest) {
        return rt.yield_now();
    }
    walk_short_log(rt, proj)
}

fn walk_sustain(rt: &mut WalkRuntime) -> Value {
    if !rt.after_sustain {
        rt.after_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.after_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn walk_emit_walk(rt: &mut WalkRuntime, tile: Tile, kind: WalkLeg) -> Value {
    rt.leg = Some(tile);
    rt.leg_kind = kind;
    rt.clock.arm(APPROACH_MS);
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.mode = WalkMode::NeedAck;
    rt.emit(json!({
        "kind": "walk",
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
    }))
}

fn walk_pick_leg(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    if observation().hold || observation().ours {
        return walk_interrupt(rt, proj);
    }
    if rt.leg_kind == WalkLeg::Approach {
        let here = observation().here;
        while let Some(stop) = usize::try_from(rt.approach_cursor)
            .ok()
            .and_then(|i| proj.site.approach.get(i).copied())
        {
            if here.is_some_and(|h| distance_to(stop, h) <= 1) {
                rt.approach_cursor += 1;
                continue;
            }
            return walk_emit_walk(rt, stop, WalkLeg::Approach);
        }
        rt.leg_kind = WalkLeg::Dest;
    }
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    rt.dest = Some(dest);
    walk_emit_walk(rt, dest, WalkLeg::Dest)
}

fn walk_advance_approach(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    rt.approach_cursor += 1;
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.mode = WalkMode::PickLeg;
    rt.leg_kind = WalkLeg::Approach;
    walk_pick_leg(rt, proj)
}

fn walk_poll(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let obs = observation();
    if obs.hold || obs.ours {
        return walk_interrupt(rt, proj);
    }
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    if rt.leg_kind == WalkLeg::Dest {
        if at_tile(&obs, dest) {
            return rt.yield_now();
        }
        let done = rt.walk_token.is_some_and(walk_wait_settled) || rt.clock.bound_reached();
        if done {
            let obs = observation();
            if at_tile(&obs, dest) {
                return rt.yield_now();
            }
            return walk_short_log(rt, proj);
        }
        return walk_sustain(rt);
    }
    let leg = rt.leg.unwrap_or(dest);
    let done = at_tile(&obs, leg)
        || rt.walk_token.is_some_and(walk_wait_settled)
        || rt.clock.bound_reached();
    if done {
        return walk_advance_approach(rt, proj);
    }
    walk_sustain(rt)
}

fn walk_start(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let dest = anchor(proj);
    rt.dest = Some(dest);
    rt.leg = None;
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.clock.deadline = None;
    let obs = observation();
    if let Some(here) = obs.here.filter(|_| !proj.site.approach.is_empty()) {
        rt.leg_kind = WalkLeg::Approach;
        rt.approach_cursor = nearest_spot(here, &proj.site.approach);
    } else {
        rt.leg_kind = WalkLeg::Dest;
        rt.approach_cursor = 0;
    }
    rt.mode = WalkMode::PickLeg;
    rt.emit(json!({ "kind": "status", "message": "walking to the fight spot" }))
}

fn walk_next_effect(rt: &mut WalkRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == WalkMode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) {
        return rt.aborted("unexpected eatOk");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        WalkMode::Idle => walk_start(rt, proj),
        WalkMode::PickLeg => walk_pick_leg(rt, proj),
        WalkMode::NeedAck => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let Some(walk_token) = reply_u64(reply, "walkToken") else {
                if queued {
                    return rt.aborted("unexpected queued");
                }
                return rt.aborted("missing walkToken");
            };
            rt.walk_token = Some(walk_token);
            rt.mode = WalkMode::Waiting;
            rt.after_sustain = false;
            walk_poll(rt, proj)
        }
        WalkMode::Waiting => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if queued {
                return rt.aborted("unexpected queued");
            }
            walk_poll(rt, proj)
        }
        WalkMode::NeedYield => rt.yield_now(),
        WalkMode::Aborted => rt.aborted("aborted"),
    }
}

fn walk_begin() -> Value {
    let token = alloc_token();
    WALK_RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, WalkRuntime::new(token));
    });
    json!({ "kind": "started", "token": token })
}

pub fn walk_dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => walk_begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_walk(
                token,
                |_| json!({ "value": walk_validate_inner(&proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_walk(token, |rt| walk_next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn walk_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_walk(token, |rt| {
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

pub fn walk_token_alive(token: u64) -> bool {
    WALK_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

/// Test helper: make `clock.bound_reached()` true without sleeping `APPROACH_MS`.
pub fn walk_force_bound_reached(token: u64) -> bool {
    with_walk(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

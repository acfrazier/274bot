//! Rust-owned fire lighting and next-tile selection.
//!
//! Frozen `lightFire` uses tinderbox on logs, then Firemaking XP or the
//! cannot-light game-chat line, with `FIRE_START_TICKS` / `FIRE_LIGHT_TICKS`
//! from identical `Firemaking.ts`. Burn lanes are ranked and traversed
//! inside the posted plot using native walkability and step masks, excluding
//! Fire locs and refused tiles. JavaScript marshals inputs and dispatches
//! the returned use-on; it does not rank lanes or poll XP.

use crate::observed::{self, ItemRow, Scene, Text};
use api::query::ReachQueryView;
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Frozen `FIRE_START_TICKS`: wait this many game ticks for the attempt to start.
pub const FIRE_START_TICKS: u64 = 14;
/// Frozen `FIRE_LIGHT_TICKS`: wait this many game ticks for Firemaking XP after start.
pub const FIRE_LIGHT_TICKS: u64 = 150;

const TINDERBOX: &str = "Tinderbox";
const CANT_LIGHT: &str = "can't light a fire here";
const FIRE_LOC: &str = "fire";
const BURN_DIRS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

thread_local! {
    static RUNTIME: RefCell<FireRuntime> = const { RefCell::new(FireRuntime::new()) };
    static LAST_TRACE: RefCell<Option<(u64, u64, Phase)>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemRef {
    name: Text,
    id: i32,
    slot: i32,
    count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

/// The posted reach bit views, as the isolate scene holds them.
type ReachBits = observed::Reach;

/// No posted reach view: nothing is walkable.
static NO_REACH: ReachBits = ReachBits {
    available: false,
    base_x: 0,
    base_z: 0,
    level: 0,
    width: 0,
    height: 0,
    walkable: Vec::new(),
    step: Vec::new(),
    canlight: Vec::new(),
};

impl ReachBits {
    fn walkable_at(&self, tile: Tile) -> bool {
        self.available
            && ReachQueryView::bit_at(
                &self.walkable,
                self.width,
                self.height,
                self.base_x,
                self.base_z,
                self.level,
                WorldTile {
                    x: tile.x,
                    z: tile.z,
                    level: tile.level,
                },
            )
    }

    fn canlight_available(&self) -> bool {
        !self.canlight.is_empty()
    }

    fn canlight_at(&self, tile: Tile) -> bool {
        self.available
            && self.canlight_available()
            && ReachQueryView::bit_at(
                &self.canlight,
                self.width,
                self.height,
                self.base_x,
                self.base_z,
                self.level,
                WorldTile {
                    x: tile.x,
                    z: tile.z,
                    level: tile.level,
                },
            )
    }

    fn can_step(&self, from: Tile, to: Tile) -> bool {
        if !self.available || from.level != self.level || to.level != self.level {
            return false;
        }
        let bit = match (to.x - from.x, to.z - from.z) {
            (-1, 0) => 0,
            (1, 0) => 1,
            (0, -1) => 2,
            (0, 1) => 3,
            (-1, -1) => 4,
            (1, -1) => 5,
            (-1, 1) => 6,
            (1, 1) => 7,
            _ => return false,
        };
        let lx = from.x - self.base_x;
        let lz = from.z - self.base_z;
        if lx < 0 || lz < 0 || lx >= self.width || lz >= self.height {
            return false;
        }
        let i = (lx as usize) * (self.height as usize) + lz as usize;
        self.step.get(i).is_some_and(|mask| mask & (1 << bit) != 0)
    }
}

/// The posted facts the light machine decides from, read from the isolate
/// scene. A logout forgets the session: only pages posted since login count.
/// The tick is always the last posted one.
struct NativeObservation {
    ingame: bool,
    tick: u64,
    here: Option<Tile>,
    animating: bool,
    firemaking_xp: Option<i32>,
    inv: Vec<ItemRef>,
    chat_max_seq: i32,
    cant_light_seq: Option<i32>,
}

impl NativeObservation {
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        let lines = session.chat_lines();
        Self {
            ingame: session.ingame().unwrap_or(false),
            tick: scene.tick().unwrap_or(0),
            here: session.here().map(|tile| Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            }),
            animating: session.animating().unwrap_or(false),
            firemaking_xp: session
                .stats()
                .and_then(|skills| skills.firemaking)
                .map(|row| row.xp),
            inv: session
                .inv()
                .map(|rows| rows.iter().filter_map(item_ref).collect())
                .unwrap_or_default(),
            chat_max_seq: lines.map_or(-1, |lines| {
                lines.iter().map(|line| line.seq).max().unwrap_or(-1)
            }),
            cant_light_seq: lines.and_then(|lines| {
                lines
                    .iter()
                    .filter(|line| contains_ascii_ci(&line.text, CANT_LIGHT))
                    .map(|line| line.seq)
                    .max()
            }),
        }
    }

    fn observe() -> Self {
        observed::with(Self::from_scene)
    }
}

/// The posted Fire locs and reach view since login.
fn fire_locs(scene: &Scene) -> Vec<Tile> {
    scene
        .since_login()
        .locs()
        .map(|rows| {
            rows.iter()
                .filter(|loc| {
                    loc.name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(FIRE_LOC))
                })
                .map(|loc| Tile {
                    x: loc.x,
                    z: loc.z,
                    level: loc.level,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn reach_of(scene: &Scene) -> &ReachBits {
    scene.since_login().reach().unwrap_or(&NO_REACH)
}

/// `hay.to_ascii_lowercase().contains(needle)` for a lowercase ASCII
/// needle, without the copy.
fn contains_ascii_ci(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn item_ref(row: &ItemRow) -> Option<ItemRef> {
    let name = row.name.as_ref()?;
    if name.is_empty() {
        return None;
    }
    Some(ItemRef {
        name: Text::clone(name),
        id: row.id,
        slot: row.slot_or_unset(),
        count: row.count,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// use-on sent; waiting log drop, cannot-light, or animation.
    WaitStart,
    /// Attempt started; waiting Firemaking XP or cannot-light.
    WaitLight,
}

struct FireRuntime {
    paused: bool,
    held: bool,
    frozen_tick: Option<u64>,
    token: u64,
    phase: Phase,
    log_name: String,
    tinder_id: i32,
    tinder_slot: i32,
    log_id: i32,
    log_slot: i32,
    start_xp: Option<i32>,
    start_logs: i32,
    mark_seq: i32,
    deadline_tick: Option<u64>,
}

impl FireRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_tick: None,
            token: 0,
            phase: Phase::Idle,
            log_name: String::new(),
            tinder_id: 0,
            tinder_slot: -1,
            log_id: 0,
            log_slot: -1,
            start_xp: None,
            start_logs: 0,
            mark_seq: 0,
            deadline_tick: None,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn set_freeze(&mut self, paused: bool, held: bool, now_tick: u64) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_tick = Some(now_tick);
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_tick.take() {
                if let Some(deadline) = self.deadline_tick.as_mut() {
                    *deadline = deadline.saturating_add(now_tick.saturating_sub(at));
                }
            }
        }
    }

    fn arm_ticks(&mut self, window: u64, now_tick: u64) {
        self.deadline_tick = Some(now_tick.saturating_add(window));
    }

    fn bound_reached(&self, now_tick: u64) -> bool {
        if self.frozen() {
            return false;
        }
        self.deadline_tick
            .is_some_and(|deadline| now_tick >= deadline)
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.log_name.clear();
        self.tinder_id = 0;
        self.tinder_slot = -1;
        self.log_id = 0;
        self.log_slot = -1;
        self.start_xp = None;
        self.start_logs = 0;
        self.mark_seq = 0;
        self.deadline_tick = None;
        self.frozen_tick = None;
    }

    fn done(&mut self, result: &str, reason: &str) -> Value {
        let token = self.token;
        self.phase = Phase::Idle;
        self.deadline_tick = None;
        json!({
            "kind": "done",
            "token": token,
            "result": result,
            "reason": reason,
        })
    }

    fn wait(&self) -> Value {
        json!({ "kind": "wait", "token": self.token })
    }

    fn use_on(&self, tinder_name: &str, log_name: &str) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{
                "op": "use-on",
                "name": tinder_name,
                "kind": "inv",
                "target_name": log_name,
                "x": 0,
                "z": 0,
                "level": 0,
                "index": null,
                "source_item_id": (self.tinder_slot >= 0).then_some(self.tinder_id),
                "source_item_slot": (self.tinder_slot >= 0).then_some(self.tinder_slot),
                "target_item_id": (self.log_slot >= 0).then_some(self.log_id),
                "target_item_slot": (self.log_slot >= 0).then_some(self.log_slot),
            }],
        })
    }
}

fn now_tick() -> u64 {
    observed::with(|scene| scene.tick().unwrap_or(0))
}

pub fn on_pause() {
    let tick = now_tick();
    RUNTIME.with(|rt| {
        let held = rt.borrow().held;
        rt.borrow_mut().set_freeze(true, held, tick);
    });
}

pub fn on_resume() {
    let tick = now_tick();
    RUNTIME.with(|rt| {
        let held = rt.borrow().held;
        rt.borrow_mut().set_freeze(false, held, tick);
    });
}

pub fn on_hold(held: bool) {
    let tick = now_tick();
    RUNTIME.with(|rt| {
        let paused = rt.borrow().paused;
        rt.borrow_mut().set_freeze(paused, held, tick);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    LAST_TRACE.with(|last| *last.borrow_mut() = None);
}

pub fn dispatch(input: &Value) -> Value {
    let op = input.get("op").and_then(Value::as_str).unwrap_or("");
    let result = match op {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        "next-tile" => next_tile(input),
        "in-fire-plot" => in_fire_plot(input),
        "local-plot" => local_plot_step(input),
        "burn-lane-want" => burn_lane_want(input),
        "is-burn-west" => is_burn_west(input),
        "fire-reaction-ticks" => json!(1),
        "run-in-dir" | "run-in-dir-result" => run_in_dir(input),
        "no-light" => no_light(input),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    };
    if matches!(op, "begin" | "next") {
        trace_light(op, &result);
    }
    result
}

/// Diagnostic observation only: never polls, changes a deadline, or sends an op.
/// Repeated JS condition checks share one wait line per token/tick/phase.
fn trace_light(op: &str, result: &Value) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("BOT_DEBUG").as_deref() == Ok("1")) {
        return;
    }
    let kind = result.get("kind").and_then(Value::as_str).unwrap_or("");
    if op == "next" && kind == "aborted" {
        return;
    }
    let obs = NativeObservation::observe();
    RUNTIME.with(|rt| {
        let rt = rt.borrow();
        let key = (rt.token, obs.tick, rt.phase);
        let duplicate_wait = LAST_TRACE.with(|last| {
            let mut last = last.borrow_mut();
            let duplicate = kind == "wait" && *last == Some(key);
            *last = Some(key);
            duplicate
        });
        if duplicate_wait {
            return;
        }
        eprintln!(
            "[fire-trace] op={op} token={} tick={} phase={:?} kind={kind} result={} reason={} logs={} xp={:?} observed_animating={} tile={:?} deadline={:?} frozen={}",
            rt.token,
            obs.tick,
            rt.phase,
            result.get("result").and_then(Value::as_str).unwrap_or("-"),
            result.get("reason").and_then(Value::as_str).unwrap_or("-"),
            named_count(&obs.inv, &rt.log_name),
            obs.firemaking_xp,
            obs.animating,
            obs.here,
            rt.deadline_tick,
            rt.frozen(),
        );
    });
}

struct Probe<'a> {
    ingame: bool,
    tick: u64,
    animating: bool,
    firemaking_xp: Option<i32>,
    inv: &'a [ItemRef],
    cant_light_seq: Option<i32>,
}

fn first_named<'a>(inv: &'a [ItemRef], name: &str) -> Option<&'a ItemRef> {
    inv.iter()
        .find(|item| item.name.eq_ignore_ascii_case(name) && item.count > 0)
}

fn named_count(inv: &[ItemRef], name: &str) -> i32 {
    inv.iter()
        .filter(|item| item.name.eq_ignore_ascii_case(name))
        .map(|item| item.count)
        .sum()
}

fn begin(input: &Value) -> Value {
    let log_name = input
        .get("logName")
        .or_else(|| input.get("match"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if log_name.is_empty() {
        return json!({ "kind": "done", "result": "stalled", "reason": "no-match" });
    }
    let obs = NativeObservation::observe();
    if !obs.ingame {
        return json!({ "kind": "aborted", "reason": "not ingame" });
    }
    let Some(tinder) = first_named(&obs.inv, TINDERBOX) else {
        return json!({ "kind": "done", "result": "stalled", "reason": "missing-items" });
    };
    let Some(logs) = first_named(&obs.inv, &log_name) else {
        return json!({ "kind": "done", "result": "stalled", "reason": "missing-items" });
    };
    let tinder_name = tinder.name.clone();
    let log_held = logs.name.clone();
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort_runtime();
        rt.log_name = log_name;
        rt.tinder_id = tinder.id;
        rt.tinder_slot = tinder.slot;
        rt.log_id = logs.id;
        rt.log_slot = logs.slot;
        rt.start_xp = obs.firemaking_xp;
        rt.start_logs = named_count(&obs.inv, &log_held);
        rt.mark_seq = obs.chat_max_seq;
        rt.phase = Phase::WaitStart;
        rt.arm_ticks(FIRE_START_TICKS, obs.tick);
        rt.use_on(&tinder_name, &log_held)
    })
}

fn blocked(probe: &Probe<'_>, mark_seq: i32) -> bool {
    probe.cant_light_seq.is_some_and(|seq| seq > mark_seq)
}

fn lit(probe: &Probe<'_>, start_xp: Option<i32>) -> bool {
    match (start_xp, probe.firemaking_xp) {
        (Some(start), Some(now)) => now > start,
        _ => false,
    }
}

fn next(token: u64) -> Value {
    let o = NativeObservation::observe();
    let probe = Probe {
        ingame: o.ingame,
        tick: o.tick,
        animating: o.animating,
        firemaking_xp: o.firemaking_xp,
        inv: &o.inv,
        cant_light_seq: o.cant_light_seq,
    };
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        if token != rt.token || rt.phase == Phase::Idle {
            return json!({ "kind": "aborted", "token": rt.token });
        }
        if rt.frozen() {
            return rt.wait();
        }
        if !probe.ingame {
            return rt.done("stalled", "not-ingame");
        }
        light_step(&mut rt, &probe)
    })
}

fn started(rt: &FireRuntime, probe: &Probe<'_>) -> bool {
    named_count(probe.inv, &rt.log_name) < rt.start_logs
        || blocked(probe, rt.mark_seq)
        || probe.animating
}

fn light_step(rt: &mut FireRuntime, probe: &Probe<'_>) -> Value {
    match rt.phase {
        Phase::WaitStart => {
            if blocked(probe, rt.mark_seq) {
                return rt.done("blocked", "cant-light");
            }
            if started(rt, probe) {
                rt.phase = Phase::WaitLight;
                rt.arm_ticks(FIRE_LIGHT_TICKS, probe.tick);
                if lit(probe, rt.start_xp) {
                    return rt.done("lit", "xp");
                }
                return rt.wait();
            }
            if rt.bound_reached(probe.tick) {
                return rt.done("stalled", "start-timeout");
            }
            rt.wait()
        }
        Phase::WaitLight => {
            if blocked(probe, rt.mark_seq) {
                return rt.done("blocked", "cant-light");
            }
            if lit(probe, rt.start_xp) {
                return rt.done("lit", "xp");
            }
            if rt.bound_reached(probe.tick) {
                return rt.done("stalled", "light-timeout");
            }
            rt.wait()
        }
        Phase::Idle => rt.done("stalled", "idle"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Plot {
    x0: i32,
    x1: i32,
    z0: i32,
    z1: i32,
    level: i32,
}

fn i32_field(value: &Value, key: &str) -> Option<i32> {
    value.get(key).and_then(Value::as_i64).and_then(|n| {
        (i32::MIN as i64..=i32::MAX as i64)
            .contains(&n)
            .then_some(n as i32)
    })
}

fn tile_from(value: Option<&Value>) -> Option<Tile> {
    let value = value?;
    Some(Tile {
        x: i32_field(value, "x")?,
        z: i32_field(value, "z")?,
        level: i32_field(value, "level").unwrap_or(0),
    })
}

fn plot_from(value: &Value) -> Option<Plot> {
    let bank_level = value
        .get("bank")
        .and_then(|bank| i32_field(bank, "level"))
        .unwrap_or(0);
    Some(Plot {
        x0: i32_field(value, "x0")?,
        x1: i32_field(value, "x1")?,
        z0: i32_field(value, "z0")?,
        z1: i32_field(value, "z1")?,
        level: bank_level,
    })
}

fn refused_keys(value: Option<&Value>) -> HashSet<(i32, i32)> {
    let Some(Value::Array(items)) = value else {
        return HashSet::new();
    };
    items
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|key| {
            let (x, z) = key.split_once(',')?;
            Some((x.parse().ok()?, z.parse().ok()?))
        })
        .collect()
}

fn direction_from(value: Option<&Value>) -> Option<(i32, i32)> {
    let value = value?;
    Some((i32_field(value, "dx")?, i32_field(value, "dz")?))
}

fn fire_at(fire_locs: &[Tile], tile: Tile) -> bool {
    fire_locs.contains(&tile)
}

fn run_length(
    start: Tile,
    direction: (i32, i32),
    cap: i32,
    refused: &HashSet<(i32, i32)>,
    fire_locs: &[Tile],
    reach: &ReachBits,
    plot: Plot,
) -> i32 {
    if cap <= 0
        || start.level != plot.level
        || start.x < plot.x0
        || start.x > plot.x1
        || start.z < plot.z0
        || start.z > plot.z1
        || refused.contains(&(start.x, start.z))
        || fire_at(fire_locs, start)
        || !reach.walkable_at(start)
        || !reach.canlight_at(start)
    {
        return 0;
    }
    let mut current = start;
    let mut run = 1;
    while run < cap {
        let next = Tile {
            x: current.x + direction.0,
            z: current.z + direction.1,
            level: current.level,
        };
        if next.x < plot.x0
            || next.x > plot.x1
            || next.z < plot.z0
            || next.z > plot.z1
            || refused.contains(&(next.x, next.z))
            || fire_at(fire_locs, next)
            || !reach.walkable_at(next)
            || !reach.canlight_at(next)
            || !reach.can_step(current, next)
        {
            break;
        }
        run += 1;
        current = next;
    }
    run
}

/// Ranked burn-lane candidate used only while scanning the posted plot.
#[derive(Clone, Copy)]
struct BurnRank {
    tile: Tile,
    direction: (i32, i32),
    run: i32,
    full: bool,
    west: bool,
    distance: i32,
}

fn select_burn_tile(
    plot: Plot,
    here: Option<Tile>,
    refused: &HashSet<(i32, i32)>,
    fire_locs: &[Tile],
    reach: &ReachBits,
    want: i32,
    directions: &[(i32, i32)],
) -> Option<(Tile, (i32, i32), i32)> {
    if plot.x1 < plot.x0 || plot.z1 < plot.z0 {
        return None;
    }
    let here = here.unwrap_or(Tile {
        x: plot.x0,
        z: plot.z0,
        level: plot.level,
    });
    let mut best: Option<BurnRank> = None;
    for z in plot.z0..=plot.z1 {
        for x in plot.x0..=plot.x1 {
            if refused.contains(&(x, z)) {
                continue;
            }
            let tile = Tile {
                x,
                z,
                level: plot.level,
            };
            if fire_locs
                .iter()
                .any(|fire| fire.x == x && fire.z == z && fire.level == plot.level)
            {
                continue;
            }
            if !reach.walkable_at(tile) {
                continue;
            }
            if !reach.canlight_at(tile) {
                continue;
            }
            for &direction in directions {
                let cap = if direction == (-1, 0) { want } else { 1 };
                let run = run_length(tile, direction, cap, refused, fire_locs, reach, plot);
                let full = run >= want;
                let west = direction == (-1, 0);
                let d = (x - here.x).abs().max((z - here.z).abs());
                let better = match best {
                    None => true,
                    Some(cur) => {
                        (full, west, run, -d) > (cur.full, cur.west, cur.run, -cur.distance)
                            || ((full, west, run, d) == (cur.full, cur.west, cur.run, cur.distance)
                                && (x, z) < (cur.tile.x, cur.tile.z))
                    }
                };
                if better {
                    best = Some(BurnRank {
                        tile,
                        direction,
                        run,
                        full,
                        west,
                        distance: d,
                    });
                }
            }
        }
    }
    best.map(|cur| (cur.tile, cur.direction, cur.run))
}

fn next_tile(input: &Value) -> Value {
    let Some(plot) = input.get("plot").and_then(plot_from) else {
        return json!({ "kind": "notImpl", "reason": "missing plot" });
    };
    let here = tile_from(input.get("here"));
    let refused = refused_keys(input.get("refused"));
    let want = input
        .get("want")
        .and_then(Value::as_i64)
        .unwrap_or(1)
        .clamp(1, 27) as i32;
    let directions = input
        .get("directions")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| direction_from(Some(row)))
                .collect::<Vec<_>>()
        })
        .filter(|rows| !rows.is_empty())
        .unwrap_or_else(|| BURN_DIRS.to_vec());
    observed::with(|scene| {
        let reach = reach_of(scene);
        if !reach.available {
            return json!({ "kind": "notImpl", "reason": "missing walkable" });
        }
        if !reach.canlight_available() {
            return json!({ "kind": "notImpl", "reason": "missing canlight" });
        }
        let fire_locs = fire_locs(scene);
        match select_burn_tile(plot, here, &refused, &fire_locs, reach, want, &directions) {
            Some((tile, direction, run)) => json!({
                "kind": "tile",
                "x": tile.x,
                "z": tile.z,
                "level": tile.level,
                "run": run,
                "dir": { "dx": direction.0, "dz": direction.1 },
            }),
            None => json!({ "kind": "none" }),
        }
    })
}

fn in_fire_plot(input: &Value) -> Value {
    let (Some(tile), Some(plot)) = (
        tile_from(input.get("tile")),
        input.get("plot").and_then(plot_from),
    ) else {
        return json!(false);
    };
    json!(
        tile.level == plot.level
            && tile.x >= plot.x0
            && tile.x <= plot.x1
            && tile.z >= plot.z0
            && tile.z <= plot.z1
    )
}

/// One step of the posted-plot scan behind `Firemaking.localFirePlot`
/// (`local-plot`): the first posted plot whose bank level matches the origin
/// and whose inclusive AABB contains it, else the `±half` box around the
/// origin.
///
/// The caller's list is walked by its own iterator in JS (the frozen
/// `for...of`), so this step decides what to do with the row that iterator just
/// produced: `{level_ok}` gates the level comparison before the containment is
/// asked for, `{contained}` names the first hit, and the `{exhausted}` report
/// authorises the fallback box only after the caller's list ends. No
/// coordinate, level or posted row crosses the bridge, so `typeof`, `?? 0`,
/// NaN/Infinity and a mutated posted row keep their existing JS coercion; the
/// fallback box arithmetic and the `Tile` construction also stay in the shim.
fn local_plot_step(input: &Value) -> Value {
    if let Some(contained) = input.get("contained").and_then(Value::as_bool) {
        return json!({ "kind": if contained { "hit" } else { "plot" } });
    }
    if let Some(level_ok) = input.get("level_ok").and_then(Value::as_bool) {
        return json!({ "kind": if level_ok { "contains" } else { "plot" } });
    }
    if input.get("exhausted").and_then(Value::as_bool) == Some(true) {
        return json!({ "kind": "fallback" });
    }
    json!({ "kind": "notImpl", "reason": "missing plot fact" })
}

fn burn_lane_want(input: &Value) -> Value {
    let count = input
        .get("logCount")
        .and_then(Value::as_f64)
        .filter(|count| count.is_finite())
        .map(|count| count.floor() as i64)
        .unwrap_or(0);
    json!(count.clamp(1, 27))
}

fn is_burn_west(input: &Value) -> Value {
    let dir = input.get("dir").unwrap_or(&Value::Null);
    json!(i32_field(dir, "dx") == Some(-1) && i32_field(dir, "dz") == Some(0))
}

fn run_in_dir(input: &Value) -> Value {
    if input.get("op").and_then(Value::as_str) == Some("run-in-dir-result") {
        if !input
            .get("walkable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return json!({ "kind": "run", "run": 0 });
        }
        let (Some(from), Some(plot)) = (
            tile_from(input.get("from")),
            input.get("plot").and_then(plot_from),
        ) else {
            return json!({ "kind": "run", "run": 0 });
        };
        let direction = direction_from(input.get("dir")).unwrap_or((-1, 0));
        let cap = input
            .get("cap")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .floor()
            .clamp(0.0, 27.0) as i32;
        return observed::with(|scene| {
            let reach = reach_of(scene);
            if reach.available && !reach.canlight_available() {
                return json!({ "kind": "notImpl", "reason": "missing canlight" });
            }
            json!({
                "kind": "run",
                "run": if cap > 0 { run_length(from, direction, cap, &refused_keys(input.get("occupied")), &fire_locs(scene), reach, plot) } else { 0 },
            })
        });
    }
    let (Some(from), Some(plot)) = (
        tile_from(input.get("from")),
        input.get("plot").and_then(plot_from),
    ) else {
        return json!({ "kind": "run", "run": 0 });
    };
    let cap = input.get("cap").and_then(Value::as_f64).unwrap_or(0.0);
    if cap <= 0.0
        || from.level != plot.level
        || from.x < plot.x0
        || from.x > plot.x1
        || from.z < plot.z0
        || from.z > plot.z1
        || refused_keys(input.get("occupied")).contains(&(from.x, from.z))
    {
        return json!({ "kind": "run", "run": 0 });
    }
    if input
        .get("hasWalkable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        json!({ "kind": "callback", "callback": "walkable" })
    } else {
        let direction = direction_from(input.get("dir")).unwrap_or((-1, 0));
        let cap = cap.floor().clamp(0.0, 27.0) as i32;
        observed::with(|scene| {
            let reach = reach_of(scene);
            if reach.available && !reach.canlight_available() {
                json!({ "kind": "notImpl", "reason": "missing canlight" })
            } else {
                json!({ "kind": "run", "run": run_length(
                    from, direction, cap, &refused_keys(input.get("occupied")),
                    &fire_locs(scene), reach, plot
                ) })
            }
        })
    }
}

fn no_light(input: &Value) -> Value {
    let keys = input
        .get("keys")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    match input.get("action").and_then(Value::as_str).unwrap_or("") {
        "add" => {
            let key = input.get("key").and_then(Value::as_str).unwrap_or("");
            let mut out = keys;
            if !out.iter().any(|item| item == key) {
                out.push(key.to_string());
            }
            json!(out)
        }
        "has" => json!(input
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| keys.iter().any(|item| item == key))),
        "size" => json!(keys.len()),
        "merge" => {
            let occupied = input
                .get("occupied")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut out = Vec::new();
            for key in occupied {
                if !out.iter().any(|item| item == &key) {
                    out.push(key);
                }
            }
            for key in keys {
                if !out.iter().any(|item| item == &key) {
                    out.push(key);
                }
            }
            json!(out)
        }
        "clear" => json!([]),
        _ => json!({ "kind": "notImpl", "reason": "unknown no-light operation" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reach_covering(
        tiles: &[(i32, i32)],
        base_x: i32,
        base_z: i32,
        width: i32,
        height: i32,
    ) -> ReachBits {
        let n = (width as usize).saturating_mul(height as usize);
        let words = n.div_ceil(32);
        let mut walkable = vec![0u32; words];
        for &(x, z) in tiles {
            let lx = x - base_x;
            let lz = z - base_z;
            if lx < 0 || lz < 0 || lx >= width || lz >= height {
                continue;
            }
            let i = (lx as usize) * (height as usize) + (lz as usize);
            walkable[i / 32] |= 1u32 << (i % 32);
        }
        ReachBits {
            available: true,
            base_x,
            base_z,
            level: 0,
            width,
            height,
            canlight: walkable.clone(),
            walkable,
            step: vec![0xff; n],
        }
    }

    fn plot() -> Plot {
        Plot {
            x0: 3235,
            x1: 3237,
            z0: 3418,
            z1: 3419,
            level: 0,
        }
    }

    #[test]
    fn next_tile_skips_fire_loc_refused_and_unwalkable() {
        let reach = reach_covering(
            &[(3235, 3418), (3236, 3418), (3237, 3418), (3235, 3419)],
            3235,
            3418,
            4,
            3,
        );
        let fire = [Tile {
            x: 3235,
            z: 3418,
            level: 0,
        }];
        let mut refused = HashSet::new();
        refused.insert((3236, 3418));
        let here = Tile {
            x: 3235,
            z: 3418,
            level: 0,
        };
        let picked = select_burn_tile(plot(), Some(here), &refused, &fire, &reach, 1, &[(-1, 0)])
            .expect("tile")
            .0;
        assert_eq!(
            picked,
            Tile {
                x: 3235,
                z: 3419,
                level: 0
            }
        );
    }

    #[test]
    fn next_tile_picks_closest_walkable_not_a_west_lane() {
        let reach = reach_covering(&[(3235, 3418), (3237, 3419)], 3235, 3418, 4, 3);
        let here = Tile {
            x: 3237,
            z: 3419,
            level: 0,
        };
        let picked = select_burn_tile(
            plot(),
            Some(here),
            &HashSet::new(),
            &[],
            &reach,
            1,
            &[(-1, 0)],
        )
        .expect("tile")
        .0;
        assert_eq!(picked, here, "closest tile wins; no west-run ranking");
    }

    #[test]
    fn non_west_lane_is_capped_to_one_during_ranking() {
        let reach = reach_covering(
            &[(3235, 3418), (3236, 3418), (3237, 3418)],
            3235,
            3418,
            4,
            3,
        );
        let selected = select_burn_tile(
            Plot {
                x0: 3235,
                x1: 3237,
                z0: 3418,
                z1: 3418,
                level: 0,
            },
            None,
            &HashSet::new(),
            &[],
            &reach,
            3,
            &[(1, 0)],
        )
        .expect("east tile");
        assert_eq!(
            selected.2, 1,
            "non-west directions cannot win by run length"
        );
    }

    #[test]
    fn unavailable_reach_is_not_a_guessed_walkable_tile() {
        let reach = NO_REACH.clone();
        assert!(
            select_burn_tile(plot(), None, &HashSet::new(), &[], &reach, 1, &[(-1, 0)]).is_none()
        );
    }

    #[test]
    fn full_requested_lane_beats_closer_single_tile() {
        let mut reach = reach_covering(
            &[(3235, 3418), (3236, 3418), (3237, 3418)],
            3235,
            3418,
            4,
            3,
        );
        // The east end has a complete west lane; the current tile is closer
        // but its west step leaves the plot, so its run is only one.
        reach.step[(3237 - 3235) as usize * 3 + (3418 - 3418) as usize] = 1;
        reach.step[(3236 - 3235) as usize * 3 + (3418 - 3418) as usize] = 1;
        let here = Tile {
            x: 3235,
            z: 3418,
            level: 0,
        };
        let selected = select_burn_tile(
            Plot {
                x0: 3235,
                x1: 3237,
                z0: 3418,
                z1: 3418,
                level: 0,
            },
            Some(here),
            &HashSet::new(),
            &[],
            &reach,
            3,
            &[(-1, 0), (1, 0)],
        )
        .expect("lane");
        assert_eq!(selected.0.x, 3237);
        assert_eq!(selected.1, (-1, 0));
        assert_eq!(selected.2, 3);
    }

    #[test]
    fn start_timeout_is_stalled_without_a_second_use_on() {
        let mut rt = FireRuntime::new();
        rt.phase = Phase::WaitStart;
        rt.log_name = "Logs".into();
        rt.start_logs = 5;
        rt.deadline_tick = Some(11);
        let inv = [ItemRef {
            name: "Logs".into(),
            id: 1511,
            slot: 1,
            count: 5,
        }];
        let probe = Probe {
            ingame: true,
            tick: 10,
            animating: false,
            firemaking_xp: Some(0),
            inv: &inv,
            cant_light_seq: None,
        };
        let step = light_step(&mut rt, &probe);
        assert_eq!(step["kind"], "wait");
        let probe = Probe { tick: 11, ..probe };
        let step = light_step(&mut rt, &probe);
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], "stalled");
        assert_eq!(step["reason"], "start-timeout");
        assert_eq!(rt.phase, Phase::Idle);
    }

    #[test]
    fn xp_after_start_is_lit() {
        let mut rt = FireRuntime::new();
        rt.phase = Phase::WaitLight;
        rt.start_xp = Some(100);
        rt.arm_ticks(FIRE_LIGHT_TICKS, 20);
        let probe = Probe {
            ingame: true,
            tick: 21,
            animating: false,
            firemaking_xp: Some(140),
            inv: &[],
            cant_light_seq: None,
        };
        let step = light_step(&mut rt, &probe);
        assert_eq!(step["result"], "lit");
        assert_eq!(step["reason"], "xp");
    }

    #[test]
    fn cant_light_is_blocked() {
        let mut rt = FireRuntime::new();
        rt.phase = Phase::WaitStart;
        rt.mark_seq = 3;
        rt.start_logs = 5;
        rt.log_name = "Logs".into();
        rt.arm_ticks(FIRE_START_TICKS, 1);
        let inv = [ItemRef {
            name: "Logs".into(),
            id: 1511,
            slot: 1,
            count: 5,
        }];
        let probe = Probe {
            ingame: true,
            tick: 2,
            animating: false,
            firemaking_xp: Some(0),
            inv: &inv,
            cant_light_seq: Some(4),
        };
        let step = light_step(&mut rt, &probe);
        assert_eq!(step["result"], "blocked");
    }

    #[test]
    fn pause_and_hold_freeze_the_tick_deadline() {
        let mut rt = FireRuntime::new();
        rt.arm_ticks(FIRE_START_TICKS, 10);
        let before = rt.deadline_tick.expect("armed");
        rt.set_freeze(true, false, 10);
        assert!(rt.frozen());
        assert!(!rt.bound_reached(100));
        rt.set_freeze(false, false, 12);
        assert!(!rt.frozen());
        assert_eq!(rt.deadline_tick.expect("still armed"), before + 2);
        rt.set_freeze(false, true, 12);
        assert!(rt.frozen(), "guardian hold freezes too");
    }

    #[test]
    fn abort_runtime_bumps_the_token() {
        let mut rt = FireRuntime::new();
        rt.log_name = "Logs".into();
        rt.phase = Phase::WaitStart;
        let before = rt.token;
        rt.abort_runtime();
        assert_eq!(rt.token, before.wrapping_add(1));
        assert_eq!(rt.phase, Phase::Idle);
        assert!(rt.log_name.is_empty());
    }

    fn unset_canlight(reach: &mut ReachBits, x: i32, z: i32) {
        let lx = x - reach.base_x;
        let lz = z - reach.base_z;
        let i = (lx as usize) * (reach.height as usize) + (lz as usize);
        reach.canlight[i / 32] &= !(1u32 << (i % 32));
    }

    #[test]
    fn bank_floor_walkable_nearest_loses_to_canlight_tile() {
        let mut reach = reach_covering(&[(3252, 3420), (3261, 3429)], 3250, 3418, 16, 16);
        unset_canlight(&mut reach, 3252, 3420);
        let here = Tile {
            x: 3252,
            z: 3420,
            level: 0,
        };
        let picked = select_burn_tile(
            Plot {
                x0: 3250,
                x1: 3262,
                z0: 3418,
                z1: 3430,
                level: 0,
            },
            Some(here),
            &HashSet::new(),
            &[],
            &reach,
            1,
            &[(-1, 0)],
        )
        .expect("outside-bank tile");
        assert_eq!(
            picked.0,
            Tile {
                x: 3261,
                z: 3429,
                level: 0
            }
        );
    }

    #[test]
    fn denied_start_and_interior_canlight_shorten_west_run() {
        let mut reach = reach_covering(
            &[(3235, 3418), (3236, 3418), (3237, 3418)],
            3235,
            3418,
            4,
            3,
        );
        let plot = Plot {
            x0: 3235,
            x1: 3237,
            z0: 3418,
            z1: 3418,
            level: 0,
        };
        let start = Tile {
            x: 3237,
            z: 3418,
            level: 0,
        };
        assert_eq!(
            run_length(start, (-1, 0), 3, &HashSet::new(), &[], &reach, plot),
            3
        );
        unset_canlight(&mut reach, 3237, 3418);
        assert_eq!(
            run_length(start, (-1, 0), 3, &HashSet::new(), &[], &reach, plot),
            0
        );
        unset_canlight(&mut reach, 3236, 3418);
        reach.canlight[0] |= 1u32 << 6;
        assert_eq!(
            run_length(start, (-1, 0), 3, &HashSet::new(), &[], &reach, plot),
            1,
            "interior canlight denial must stop the west run"
        );
    }

    #[test]
    fn missing_canlight_is_not_a_walkable_rank() {
        let mut reach = reach_covering(&[(3235, 3418)], 3235, 3418, 4, 3);
        reach.canlight.clear();
        observed::post(0, |post| {
            post.reach(reach.clone());
        });
        let result = next_tile(&json!({
            "plot": { "x0": 3235, "x1": 3237, "z0": 3418, "z1": 3419, "bank": { "x": 3235, "z": 3420, "level": 0 } },
            "here": { "x": 3235, "z": 3418, "level": 0 },
            "want": 1,
            "directions": [{ "dx": -1, "dz": 0 }],
        }));
        assert_eq!(result["kind"], "notImpl");
        assert_eq!(result["reason"], "missing canlight");
    }

    #[test]
    fn valid_all_zero_canlight_has_no_candidate() {
        let mut reach = reach_covering(&[(3235, 3418), (3236, 3418)], 3235, 3418, 4, 3);
        for word in &mut reach.canlight {
            *word = 0;
        }
        assert!(reach.canlight_available());
        assert!(
            select_burn_tile(plot(), None, &HashSet::new(), &[], &reach, 1, &[(-1, 0)]).is_none()
        );
        observed::post(0, |post| {
            post.reach(reach);
        });
        let result = next_tile(&json!({
            "plot": { "x0": 3235, "x1": 3237, "z0": 3418, "z1": 3419, "bank": { "x": 3235, "z": 3420, "level": 0 } },
            "want": 1,
        }));
        assert_eq!(result["kind"], "none");
    }

    #[test]
    fn local_plot_steps_are_routed_through_dispatch() {
        let step = json!({ "op": "local-plot", "level_ok": true });
        assert_eq!(
            dispatch(&step),
            json!({ "kind": "contains" }),
            "the op is routed through dispatch"
        );
        assert_eq!(dispatch(&step), local_plot_step(&step));
    }

    #[test]
    fn local_plot_gates_level_before_containment_and_stops_at_the_hit() {
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot", "level_ok": false })),
            json!({ "kind": "plot" }),
            "a level mismatch advances without asking for containment"
        );
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot", "level_ok": true })),
            json!({ "kind": "contains" }),
            "a matching level asks for the inclusive AABB fact"
        );
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot", "contained": true })),
            json!({ "kind": "hit" }),
            "the first containing posted plot is the hit"
        );
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot", "contained": false })),
            json!({ "kind": "plot" }),
            "an inclusive AABB miss advances to the next posted plot"
        );
    }

    #[test]
    fn local_plot_falls_back_only_when_the_posted_list_is_exhausted() {
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot", "exhausted": true })),
            json!({ "kind": "fallback" }),
            "the caller's exhausted list is the only fallback path"
        );
        assert_eq!(
            local_plot_step(&json!({
                "op": "local-plot", "level_ok": true, "contained": false,
            })),
            json!({ "kind": "plot" }),
            "containment outranks the level fact"
        );
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot" })),
            json!({ "kind": "notImpl", "reason": "missing plot fact" }),
            "a step without a plot fact is explicit, never a guessed hit"
        );
        assert_eq!(
            local_plot_step(&json!({ "op": "local-plot", "exhausted": false })),
            json!({ "kind": "notImpl", "reason": "missing plot fact" }),
            "only a true exhaustion report authorises the fallback"
        );
    }
}

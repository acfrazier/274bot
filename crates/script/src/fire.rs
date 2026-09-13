//! Rust-owned fire lighting and next-tile selection.
//!
//! Frozen `lightFire` uses tinderbox on logs, then Firemaking XP or the
//! cannot-light game-chat line, with `FIRE_START_TICKS` / `FIRE_LIGHT_TICKS`
//! from identical `Firemaking.ts`. Next-tile is one walkable posted-plot
//! tile with no Fire loc and outside the refused set — not the foreign
//! west-lane ranker. JavaScript marshals the log name / plot / refused
//! keys and dispatches the returned use-on; it does not poll XP.

use crate::isolate_fb::{RowReader, SnapshotReader};
use api::query::ReachQueryView;
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;

/// Frozen `FIRE_START_TICKS`: wait this many game ticks for the attempt to start.
pub const FIRE_START_TICKS: u64 = 14;
/// Frozen `FIRE_LIGHT_TICKS`: wait this many game ticks for Firemaking XP after start.
pub const FIRE_LIGHT_TICKS: u64 = 150;

const TINDERBOX: &str = "Tinderbox";
const CANT_LIGHT: &str = "can't light a fire here";
const FIRE_LOC: &str = "fire";

thread_local! {
    static RUNTIME: RefCell<FireRuntime> = const { RefCell::new(FireRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemRef {
    name: String,
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

#[derive(Debug, Clone)]
struct ReachBits {
    available: bool,
    base_x: i32,
    base_z: i32,
    level: i32,
    width: i32,
    height: i32,
    walkable: Vec<u32>,
}

impl ReachBits {
    const fn empty() -> Self {
        Self {
            available: false,
            base_x: 0,
            base_z: 0,
            level: 0,
            width: 0,
            height: 0,
            walkable: Vec::new(),
        }
    }

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
}

/// Compact projection of posted snapshot fields this module decides from.
struct NativeObservation {
    ingame: bool,
    tick: u64,
    here: Option<Tile>,
    animating: bool,
    firemaking_xp: Option<i32>,
    inv: Vec<ItemRef>,
    fire_locs: Vec<Tile>,
    chat_max_seq: i32,
    cant_light_seq: Option<i32>,
    reach: ReachBits,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            tick: 0,
            here: None,
            animating: false,
            firemaking_xp: None,
            inv: Vec::new(),
            fire_locs: Vec::new(),
            chat_max_seq: -1,
            cant_light_seq: None,
            reach: ReachBits::empty(),
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        self.tick = snap.tick();
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                self.tick = snap.tick();
                return;
            }
            self.ingame = true;
        }
        if snap.has_here() {
            self.here = snap.here().map(|tile| Tile {
                x: tile.x(),
                z: tile.z(),
                level: tile.level(),
            });
        }
        if snap.has_animating() {
            self.animating = snap.animating();
        }
        if snap.has_stats() {
            self.firemaking_xp = snap.stats().iter().find_map(|row| {
                row.name()
                    .eq_ignore_ascii_case("firemaking")
                    .then_some(row.xp())
            });
        }
        if snap.has_inv() {
            self.inv = snap.inv().iter().filter_map(item_ref).collect();
        }
        if snap.has_locs() {
            self.fire_locs = snap
                .locs()
                .iter()
                .filter(|loc| {
                    loc.name()
                        .is_some_and(|name| name.eq_ignore_ascii_case(FIRE_LOC))
                })
                .map(|loc| Tile {
                    x: loc.x(),
                    z: loc.z(),
                    level: loc.level(),
                })
                .collect();
        }
        if snap.has_chat_lines() {
            let lines = snap.chat_lines();
            self.chat_max_seq = lines.iter().map(|line| line.seq()).max().unwrap_or(-1);
            self.cant_light_seq = lines
                .iter()
                .filter(|line| line.text().to_ascii_lowercase().contains(CANT_LIGHT))
                .map(|line| line.seq())
                .max();
        }
        if snap.has_reach() {
            self.reach = snap
                .reach()
                .map(reach_bits)
                .unwrap_or_else(ReachBits::empty);
        }
    }
}

fn item_ref(row: &RowReader<'_>) -> Option<ItemRef> {
    let name = row.name()?.to_string();
    if name.is_empty() {
        return None;
    }
    Some(ItemRef {
        name,
        id: row.id(),
        slot: row.slot(),
        count: row.count(),
    })
}

fn reach_bits(reach: crate::isolate_fb::ReachReader<'_>) -> ReachBits {
    ReachBits {
        available: reach.available(),
        base_x: reach.base_x(),
        base_z: reach.base_z(),
        level: reach.level(),
        width: reach.width(),
        height: reach.height(),
        walkable: reach.walkable(),
    }
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

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|obs| obs.borrow_mut().update(snap));
}

fn now_tick() -> u64 {
    NATIVE_OBSERVATION.with(|obs| obs.borrow().tick)
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
    NATIVE_OBSERVATION.with(|obs| *obs.borrow_mut() = NativeObservation::new());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        "next-tile" => next_tile(input),
        "in-fire-plot" => in_fire_plot(input),
        "burn-lane-want" => burn_lane_want(input),
        "is-burn-west" => is_burn_west(input),
        "fire-reaction-ticks" => json!(1),
        "run-in-dir" | "run-in-dir-result" => run_in_dir(input),
        "no-light" => no_light(input),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
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
    let obs = NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        (
            o.ingame,
            o.tick,
            o.animating,
            o.firemaking_xp,
            o.inv.clone(),
            o.chat_max_seq,
        )
    });
    if !obs.0 {
        return json!({ "kind": "aborted", "reason": "not ingame" });
    }
    let Some(tinder) = first_named(&obs.4, TINDERBOX) else {
        return json!({ "kind": "done", "result": "stalled", "reason": "missing-items" });
    };
    let Some(logs) = first_named(&obs.4, &log_name) else {
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
        rt.start_xp = obs.3;
        rt.start_logs = named_count(&obs.4, &log_held);
        rt.mark_seq = obs.5;
        rt.phase = Phase::WaitStart;
        rt.arm_ticks(FIRE_START_TICKS, obs.1);
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
    NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
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

/// One walkable posted-plot tile with no Fire loc and outside `refused`.
/// Closest Chebyshev to `here`, then smaller x/z. Not a west-lane ranker.
fn select_burn_tile(
    plot: Plot,
    here: Option<Tile>,
    refused: &HashSet<(i32, i32)>,
    fire_locs: &[Tile],
    reach: &ReachBits,
) -> Option<Tile> {
    if plot.x1 < plot.x0 || plot.z1 < plot.z0 {
        return None;
    }
    let here = here.unwrap_or(Tile {
        x: plot.x0,
        z: plot.z0,
        level: plot.level,
    });
    let mut best: Option<(Tile, i32)> = None;
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
            let d = (x - here.x).abs().max((z - here.z).abs());
            let better = match best {
                None => true,
                Some((cur, cur_d)) => d < cur_d || (d == cur_d && (x, z) < (cur.x, cur.z)),
            };
            if better {
                best = Some((tile, d));
            }
        }
    }
    best.map(|(tile, _)| tile)
}

fn next_tile(input: &Value) -> Value {
    let Some(plot) = input.get("plot").and_then(plot_from) else {
        return json!({ "kind": "notImpl", "reason": "missing plot" });
    };
    let here = tile_from(input.get("here"));
    let refused = refused_keys(input.get("refused"));
    let (reach, fire_locs) = NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        (o.reach.clone(), o.fire_locs.clone())
    });
    if !reach.available {
        return json!({ "kind": "notImpl", "reason": "missing walkable" });
    }
    match select_burn_tile(plot, here, &refused, &fire_locs, &reach) {
        Some(tile) => json!({
            "kind": "tile",
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
            "run": 1,
        }),
        None => json!({ "kind": "none" }),
    }
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
        return json!({
            "kind": "run",
            "run": if input.get("walkable").and_then(Value::as_bool).unwrap_or(false) { 1 } else { 0 },
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
        json!({ "kind": "run", "run": 1 })
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
            walkable,
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
        let picked = select_burn_tile(plot(), Some(here), &refused, &fire, &reach).expect("tile");
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
        let picked =
            select_burn_tile(plot(), Some(here), &HashSet::new(), &[], &reach).expect("tile");
        assert_eq!(picked, here, "closest tile wins; no west-run ranking");
    }

    #[test]
    fn unavailable_reach_is_not_a_guessed_walkable_tile() {
        let reach = ReachBits::empty();
        assert!(select_burn_tile(plot(), None, &HashSet::new(), &[], &reach).is_none());
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
}

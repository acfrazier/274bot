//! Rust-owned fire lighting and next-tile selection.
//!
//! Frozen `lightFire` uses tinderbox on logs, then Firemaking XP or the
//! cannot-light game-chat line, with `FIRE_START_TICKS` / `FIRE_LIGHT_TICKS`
//! from identical `Firemaking.ts`. Burn lanes are ranked inside the posted
//! plot using native walkability and step masks, excluding Fire locs and
//! refused tiles. JavaScript marshals inputs and dispatches the returned
//! use-on; it does not rank lanes or poll XP. The `NoLightTiles` refused set
//! and the `localFirePlot` half-width live here too; `load::fire_v8` walks the
//! caller's own `runInDir` callbacks.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, ItemRow, Scene, Text};
use crate::shim::InteractReq;
use api::query::ReachQueryView;
use api::snapshot::WorldTile;
use serde::Deserialize;
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
    /// use-on sent; waiting log drop, cannot-light, or animation.
    WaitStart,
    /// Attempt started; waiting Firemaking XP or cannot-light.
    WaitLight,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LightArgs {
    log_name: String,
}

/// One frozen `lightFire`: the use-on went out at begin; each step reads
/// the scene for the start, the XP or the cannot-light line. Deadlines
/// count this row's steps (one per eligible tick), so paused and held
/// ticks never count toward them.
pub(crate) struct LightFire {
    phase: Phase,
    log_name: String,
    start_xp: Option<i32>,
    start_logs: i32,
    mark_seq: i32,
    ticks_left: u64,
}

impl Family for LightFire {
    const NAME: &'static str = "light-fire";
    /// A new light replaces the one in flight.
    const EXCLUSIVE: bool = true;
    type Args = LightArgs;
    /// The frozen verdict: `lit`, `blocked` or `stalled`.
    type Output = &'static str;

    fn begin(args: LightArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let log_name = args.log_name.trim().to_string();
        if log_name.is_empty() {
            trace("begin", "no-match", None);
            return Begin::Done("stalled");
        }
        let obs = NativeObservation::observe();
        if !obs.ingame {
            return Begin::Done("stalled");
        }
        let (Some(tinder), Some(logs)) = (
            first_named(&obs.inv, TINDERBOX),
            first_named(&obs.inv, &log_name),
        ) else {
            trace("begin", "missing-items", None);
            return Begin::Done("stalled");
        };
        cx.emit(InteractReq::UseOn {
            name: tinder.name.to_string(),
            kind: "inv".into(),
            target_name: Some(logs.name.to_string()),
            x: 0,
            z: 0,
            level: 0,
            index: None,
            source_item_id: (tinder.slot >= 0).then_some(tinder.id),
            source_item_slot: (tinder.slot >= 0).then_some(tinder.slot),
            target_item_id: (logs.slot >= 0).then_some(logs.id),
            target_item_slot: (logs.slot >= 0).then_some(logs.slot),
        });
        let machine = Self {
            phase: Phase::WaitStart,
            start_logs: named_count(&obs.inv, &logs.name),
            log_name,
            start_xp: obs.firemaking_xp,
            mark_seq: obs.chat_max_seq,
            ticks_left: FIRE_START_TICKS,
        };
        trace("begin", "use-on", Some(&machine));
        Begin::Run(machine)
    }

    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<&'static str> {
        let o = NativeObservation::observe();
        let probe = Probe {
            ingame: o.ingame,
            animating: o.animating,
            firemaking_xp: o.firemaking_xp,
            inv: &o.inv,
            cant_light_seq: o.cant_light_seq,
        };
        let step = if probe.ingame {
            self.light_step(&probe)
        } else {
            Step::Done("stalled")
        };
        if let Step::Done(result) = &step {
            trace("step", result, Some(self));
        }
        step
    }
}

impl LightFire {
    /// Spend one step of the armed window; true once it has run out.
    fn bound_reached(&mut self) -> bool {
        self.ticks_left = self.ticks_left.saturating_sub(1);
        self.ticks_left == 0
    }

    fn started(&self, probe: &Probe<'_>) -> bool {
        named_count(probe.inv, &self.log_name) < self.start_logs
            || blocked(probe, self.mark_seq)
            || probe.animating
    }

    fn light_step(&mut self, probe: &Probe<'_>) -> Step<&'static str> {
        if blocked(probe, self.mark_seq) {
            return Step::Done("blocked");
        }
        match self.phase {
            Phase::WaitStart => {
                if self.started(probe) {
                    self.phase = Phase::WaitLight;
                    self.ticks_left = FIRE_LIGHT_TICKS;
                    return if lit(probe, self.start_xp) {
                        Step::Done("lit")
                    } else {
                        Step::Wait
                    };
                }
                if self.bound_reached() {
                    return Step::Done("stalled");
                }
                Step::Wait
            }
            Phase::WaitLight => {
                if lit(probe, self.start_xp) {
                    return Step::Done("lit");
                }
                if self.bound_reached() {
                    return Step::Done("stalled");
                }
                Step::Wait
            }
        }
    }
}

/// `BOT_DEBUG=1` diagnostic only: never polls, changes a deadline, or sends
/// an op.
fn trace(at: &str, outcome: &str, machine: Option<&LightFire>) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("BOT_DEBUG").as_deref() == Ok("1")) {
        return;
    }
    let obs = NativeObservation::observe();
    eprintln!(
        "[fire-trace] at={at} tick={} outcome={outcome} phase={:?} logs={} xp={:?} observed_animating={} tile={:?} ticks_left={:?}",
        obs.tick,
        machine.map(|m| m.phase),
        machine.map_or(0, |m| named_count(&obs.inv, &m.log_name)),
        obs.firemaking_xp,
        obs.animating,
        obs.here,
        machine.map(|m| m.ticks_left),
    );
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "next-tile" => next_tile(input),
        "in-fire-plot" => in_fire_plot(input),
        "burn-lane-want" => burn_lane_want(input),
        "is-burn-west" => is_burn_west(input),
        "fire-reaction-ticks" => json!(1),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

struct Probe<'a> {
    ingame: bool,
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

fn blocked(probe: &Probe<'_>, mark_seq: i32) -> bool {
    probe.cant_light_seq.is_some_and(|seq| seq > mark_seq)
}

fn lit(probe: &Probe<'_>, start_xp: Option<i32>) -> bool {
    match (start_xp, probe.firemaking_xp) {
        (Some(start), Some(now)) => now > start,
        _ => false,
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

/// Half-width of the frozen `localFirePlot` box when the caller omits `half`.
const LOCAL_FIRE_HALF: f64 = 8.0;
/// The smallest half-width the frozen box allows.
const LOCAL_FIRE_MIN_HALF: f64 = 2.0;

/// Frozen `localFirePlot` half-width: `Math.max(2, Math.floor(half))`, with
/// `half = 8` when the caller omits it. `NaN` stays `NaN` (`Math.max`).
pub(crate) fn local_fire_half(half: Option<f64>) -> f64 {
    let half = half.unwrap_or(LOCAL_FIRE_HALF).floor();
    if half.is_nan() {
        f64::NAN
    } else {
        half.max(LOCAL_FIRE_MIN_HALF)
    }
}

/// The tile keys one `NoLightTiles` instance refused this session, in
/// insertion order (the frozen `Set<string>`). Rust holds the set; the JS
/// instance keeps only its slot, so no call ships the set across.
#[derive(Default)]
struct NoLightKeys {
    order: Vec<String>,
    seen: HashSet<String>,
}

thread_local! {
    /// One slot per `new NoLightTiles()` in this isolate thread. Instances are
    /// session-lived script fields, so slots are never reused; the table dies
    /// with the isolate thread.
    static NO_LIGHT: RefCell<Vec<NoLightKeys>> = const { RefCell::new(Vec::new()) };
}

/// A fresh, empty refused-tile set; returns its slot.
pub(crate) fn no_light_new() -> usize {
    NO_LIGHT.with(|slots| {
        let mut slots = slots.borrow_mut();
        slots.push(NoLightKeys::default());
        slots.len() - 1
    })
}

fn with_no_light<R>(slot: usize, f: impl FnOnce(&mut NoLightKeys) -> R) -> Option<R> {
    NO_LIGHT.with(|slots| slots.borrow_mut().get_mut(slot).map(f))
}

/// `refused.add(key)`; `None` for an unknown slot.
pub(crate) fn no_light_add(slot: usize, key: String) -> Option<()> {
    with_no_light(slot, |keys| {
        if keys.seen.insert(key.clone()) {
            keys.order.push(key);
        }
    })
}

/// `refused.has(key)`.
pub(crate) fn no_light_has(slot: usize, key: &str) -> Option<bool> {
    with_no_light(slot, |keys| keys.seen.contains(key))
}

/// `refused.size`.
pub(crate) fn no_light_size(slot: usize) -> Option<usize> {
    with_no_light(slot, |keys| keys.order.len())
}

/// The refused keys in insertion order (`for (const key of refused)`).
pub(crate) fn no_light_keys(slot: usize) -> Option<Vec<String>> {
    with_no_light(slot, |keys| keys.order.clone())
}

/// `refused.clear()`.
pub(crate) fn no_light_clear(slot: usize) -> Option<()> {
    with_no_light(slot, |keys| {
        keys.order.clear();
        keys.seen.clear();
    })
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
}

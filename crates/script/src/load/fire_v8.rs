//! v1 `Firemaking` helpers (frozen `bot/api/firemaking/Firemaking.ts`) as one
//! native call each: `__rs2b0t_firemaking(op, ...args)`.
//!
//! - `localFirePlot(Tile, origin, half)`: the frozen `±half` box around the
//!   origin (half-width policy in [`crate::fire::local_fire_half`]).
//! - `runInDir(from, plot, dir, occupied, walkable, canStep, cap)`: the frozen
//!   lane walk, calling the caller's `occupied.has`, `walkable(cur)` and
//!   `canStep(cur, next)` once per visited tile in the frozen order.
//! - `noLight*`: the `NoLightTiles` set held in Rust ([`crate::fire`]); the JS
//!   instance passes only its slot.
//! - The pure parsing, log-level, spot-selection and local-plot helpers are
//!   native operations over the existing Firemaking/content facts.

use super::callback_v8::{
    self as cb, get, not_impl, number, Callback, Flow, ForOf, JsResult, Poll,
};
use crate::fire;
use rustyscript::Runtime;
use std::collections::HashSet;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_firemaking", firemaking)
}

fn firemaking<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run(scope, &args);
    cb::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let op = args.get(0).to_rust_string_lossy(scope);
    match op.as_str() {
        "localFirePlot" => local_fire_plot(scope, args.get(1), args.get(2), args.get(3)),
        "parseBurnMode" => parse_burn_mode(scope, args.get(1)),
        "logsForTree" => logs_for_tree(scope, args.get(1)),
        "firemakingLevelForLogs" => firemaking_level_for_logs(scope, args.get(1)),
        "shouldBurnFullLoad" => {
            should_burn_full_load(scope, args.get(1), args.get(2), args.get(3), args.get(4))
        }
        "resolveFireSpot" => resolve_fire_spot(scope, args.get(1), args.get(2)),
        "nearestFireSpot" => nearest_fire_spot(scope, args.get(1), args.get(2)),
        "expandLocalFirePlot" => {
            expand_local_fire_plot(scope, args.get(1), args.get(2), args.get(3), args.get(4))
        }
        "runInDir" => {
            let walkable = Callback::plain(scope, args.get(5), "walkable");
            let can_step = Callback::plain(scope, args.get(6), "canStep");
            let run = run_in_dir(
                scope,
                RunArgs {
                    from: args.get(1),
                    plot: args.get(2),
                    dir: args.get(3),
                    occupied: args.get(4),
                    walkable,
                    can_step,
                    cap: args.get(7),
                },
            )?;
            Ok(cb::num(scope, run))
        }
        "findBurnLane" => find_burn_lane(
            scope,
            FindBurnLaneArgs {
                tile_ctor: args.get(1),
                plot: args.get(2),
                here: args.get(3),
                occupied: args.get(4),
                want: args.get(5),
                walkable: args.get(6),
                can_step: args.get(7),
                directions: args.get(8),
            },
        ),
        "noLightNew" => Ok(cb::num(scope, fire::no_light_new() as f64)),
        "noLightAdd" => {
            let slot = slot(scope, args.get(1))?;
            let key = registry_key(scope, args.get(2))?;
            fire::no_light_add(slot, key).ok_or_else(|| bad_slot(scope))?;
            Ok(v8::undefined(scope).into())
        }
        "noLightHas" => {
            let slot = slot(scope, args.get(1))?;
            let key = registry_key(scope, args.get(2))?;
            let has = fire::no_light_has(slot, &key).ok_or_else(|| bad_slot(scope))?;
            Ok(v8::Boolean::new(scope, has).into())
        }
        "noLightSize" => {
            let slot = slot(scope, args.get(1))?;
            let size = fire::no_light_size(slot).ok_or_else(|| bad_slot(scope))?;
            Ok(cb::num(scope, size as f64))
        }
        "noLightMerge" => no_light_merge(scope, args.get(1), args.get(2)),
        "noLightClear" => {
            let slot = slot(scope, args.get(1))?;
            fire::no_light_clear(slot).ok_or_else(|| bad_slot(scope))?;
            Ok(v8::undefined(scope).into())
        }
        _ => Err(not_impl(scope, "Firemaking")),
    }
}

/// Frozen `localFirePlot(origin, half = 8)`:
///
/// ```text
/// const h = Math.max(2, Math.floor(half));
/// return { bank: new Tile(origin.x, origin.z, origin.level),
///          x0: origin.x - h, x1: origin.x + h, z0: origin.z - h, z1: origin.z + h };
/// ```
fn local_fire_plot<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_class: v8::Local<'s, v8::Value>,
    origin: v8::Local<'s, v8::Value>,
    half: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let half = if half.is_undefined() {
        None
    } else {
        Some(number(scope, half)?)
    };
    let h = cb::num(scope, fire::local_fire_half(half));
    let x = get(scope, origin, "x")?;
    let z = get(scope, origin, "z")?;
    let level = get(scope, origin, "level")?;
    let bank = cb::construct(scope, tile_class, &[x, z, level], "Tile")?;
    let x = get(scope, origin, "x")?;
    let x0 = cb::sub(scope, x, h)?;
    let x = get(scope, origin, "x")?;
    let x1 = cb::add(scope, x, h)?;
    let z = get(scope, origin, "z")?;
    let z0 = cb::sub(scope, z, h)?;
    let z = get(scope, origin, "z")?;
    let z1 = cb::add(scope, z, h)?;
    cb::object(
        scope,
        &[
            ("bank", bank),
            ("x0", x0),
            ("x1", x1),
            ("z0", z0),
            ("z1", z1),
        ],
    )
}

fn normalized_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    trim_label: &'static str,
    lower_label: &'static str,
) -> JsResult<'s, String> {
    let trim = get(scope, value, "trim")?;
    let trimmed = Callback::method(trim, value, trim_label).call(scope, &[])?;
    let lowercase = get(scope, trimmed, "toLowerCase")?;
    let normalized = Callback::method(lowercase, trimmed, lower_label).call(scope, &[])?;
    if normalized.is_string() {
        cb::to_string(scope, normalized)
    } else {
        Ok(String::new())
    }
}

fn parse_burn_mode<'s>(
    scope: &mut v8::HandleScope<'s>,
    label: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let label = normalized_string(
        scope,
        label,
        "Firemaking.parseBurnMode.trim",
        "Firemaking.parseBurnMode.toLowerCase",
    )?;
    Ok(cb::string(scope, fire::parse_burn_mode(&label)))
}

fn logs_for_tree<'s>(
    scope: &mut v8::HandleScope<'s>,
    tree_name: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let tree_name = normalized_string(
        scope,
        tree_name,
        "Firemaking.logsForTree.trim",
        "Firemaking.logsForTree.toLowerCase",
    )?;
    Ok(cb::string(scope, fire::logs_for_tree(&tree_name)))
}

fn firemaking_level_for_logs<'s>(
    scope: &mut v8::HandleScope<'s>,
    log_name: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let log_name = cb::to_string(scope, log_name)?;
    Ok(match fire::firemaking_level_for_logs(&log_name) {
        Some(level) => cb::num(scope, f64::from(level)),
        None => v8::undefined(scope).into(),
    })
}

fn should_burn_full_load<'s>(
    scope: &mut v8::HandleScope<'s>,
    mode: v8::Local<'s, v8::Value>,
    inventory_full: v8::Local<'s, v8::Value>,
    log_count: v8::Local<'s, v8::Value>,
    has_tinderbox: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let chop_then_burn = cb::string(scope, "chop-then-burn");
    let mode_matches = mode.strict_equals(chop_then_burn);
    if !mode_matches || !cb::truthy(scope, inventory_full) {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let zero = cb::num(scope, 0.0);
    let log_count_positive = cb::gt(scope, log_count, zero)?;
    let has_tinderbox = cb::truthy(scope, has_tinderbox);
    Ok(v8::Boolean::new(
        scope,
        fire::should_burn_full_load(mode_matches, true, log_count_positive, has_tinderbox),
    )
    .into())
}

fn resolve_fire_spot<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    fire_spots: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let name = normalized_string(
        scope,
        name,
        "Firemaking.resolveFireSpot.trim",
        "Firemaking.resolveFireSpot.toLowerCase",
    )?;
    let Some(spot) = fire::resolve_fire_spot(&name) else {
        return Ok(v8::null(scope).into());
    };
    let plot = get(scope, fire_spots, spot.name)?;
    let name = cb::string(scope, spot.name);
    cb::object(scope, &[("name", name), ("plot", plot)])
}

fn nearest_fire_spot<'s>(
    scope: &mut v8::HandleScope<'s>,
    from: v8::Local<'s, v8::Value>,
    fire_spots: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let x = get(scope, from, "x")?;
    let x = number(scope, x)?;
    let z = get(scope, from, "z")?;
    let z = number(scope, z)?;
    let Some(spot) = fire::nearest_fire_spot(x, z) else {
        return Ok(v8::null(scope).into());
    };
    let plot = get(scope, fire_spots, spot.name)?;
    let name = cb::string(scope, spot.name);
    cb::object(scope, &[("name", name), ("plot", plot)])
}

fn expand_local_fire_plot<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_class: v8::Local<'s, v8::Value>,
    plot: v8::Local<'s, v8::Value>,
    grow_by: v8::Local<'s, v8::Value>,
    max_half: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let x0 = get(scope, plot, "x0")?;
    let x1 = get(scope, plot, "x1")?;
    let z0 = get(scope, plot, "z0")?;
    let z1 = get(scope, plot, "z1")?;
    let two = cb::num(scope, 2.0);
    let x_sum = cb::add(scope, x0, x1)?;
    let x_middle = cb::div(scope, x_sum, two)?;
    let center_x = number(scope, x_middle)?.floor();
    let z_sum = cb::add(scope, z0, z1)?;
    let z_middle = cb::div(scope, z_sum, two)?;
    let center_z = number(scope, z_middle)?.floor();
    let x_span = cb::sub(scope, x1, x0)?;
    let x_half = cb::div(scope, x_span, two)?;
    let half_x = number(scope, x_half)?.floor();
    let z_span = cb::sub(scope, z1, z0)?;
    let z_half = cb::div(scope, z_span, two)?;
    let half_z = number(scope, z_half)?.floor();
    let grow_by = if grow_by.is_undefined() {
        4.0
    } else {
        number(scope, grow_by)?
    };
    let next = fire::expanded_local_fire_half(half_x, half_z, grow_by);
    let max_half = if max_half.is_undefined() {
        cb::num(scope, 24.0)
    } else {
        max_half
    };
    let next_value = cb::num(scope, next);
    if cb::gt(scope, next_value, max_half)? {
        return Ok(v8::null(scope).into());
    }
    let bank = get(scope, plot, "bank")?;
    let level = get(scope, bank, "level")?;
    let x = cb::num(scope, center_x);
    let z = cb::num(scope, center_z);
    let origin = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
    local_fire_plot(scope, tile_class, origin, next_value)
}

struct RunArgs<'s> {
    from: v8::Local<'s, v8::Value>,
    plot: v8::Local<'s, v8::Value>,
    dir: v8::Local<'s, v8::Value>,
    occupied: v8::Local<'s, v8::Value>,
    walkable: Callback<'s>,
    can_step: Callback<'s>,
    cap: v8::Local<'s, v8::Value>,
}
struct FindBurnLaneArgs<'s> {
    tile_ctor: v8::Local<'s, v8::Value>,
    plot: v8::Local<'s, v8::Value>,
    here: v8::Local<'s, v8::Value>,
    occupied: v8::Local<'s, v8::Value>,
    want: v8::Local<'s, v8::Value>,
    walkable: v8::Local<'s, v8::Value>,
    can_step: v8::Local<'s, v8::Value>,
    directions: v8::Local<'s, v8::Value>,
}

struct BurnLaneContext<'s> {
    tile_ctor: v8::Local<'s, v8::Value>,
    plot: v8::Local<'s, v8::Value>,
    here: v8::Local<'s, v8::Value>,
    occupied: v8::Local<'s, v8::Value>,
    need: f64,
    walkable: Callback<'s>,
    can_step: Callback<'s>,
}

/// Frozen `runInDir`:
///
/// ```text
/// let n = 0; let cur = from;
/// while (n < cap) {
///     if (!inFirePlot(cur, plot) || occupied.has(tileKey(cur)) || !walkable(cur)) break;
///     n++;
///     const next = { x: cur.x + dir.dx, z: cur.z + dir.dz, level: cur.level };
///     if (!canStep(cur, next)) break;
///     cur = next;
/// }
/// return n;
/// ```
fn run_in_dir<'s>(scope: &mut v8::HandleScope<'s>, a: RunArgs<'s>) -> JsResult<'s, f64> {
    let mut n = 0.0;
    let mut cur = a.from;
    let poll = Poll::default();
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = lane_iteration(scope, &a, &poll, cur, &mut n);
        match cb::iteration(scope, flow)? {
            Flow::Continue(Some(next)) => cur = next,
            _ => break,
        }
    }
    Ok(n)
}
fn find_burn_lane<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: FindBurnLaneArgs<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let need = burn_lane_need(scope, args.want)?;
    if args.walkable.is_null_or_undefined() && args.can_step.is_null_or_undefined() {
        return find_burn_lane_from_snapshot(scope, args, need);
    }
    let context = BurnLaneContext {
        tile_ctor: args.tile_ctor,
        plot: args.plot,
        here: args.here,
        occupied: args.occupied,
        need,
        walkable: Callback::plain(scope, args.walkable, "walkable"),
        can_step: Callback::plain(scope, args.can_step, "canStep"),
    };
    let directions = ForOf::open(scope, args.directions, "dirs")?;
    let mut best = None;

    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = find_burn_lane_iteration(scope, &directions, &context, best);
        match cb::iteration(scope, flow)? {
            Flow::Continue(next) => best = next,
            Flow::Break(next) => {
                best = next;
                break;
            }
        }
    }

    let Some(best) = best else {
        return Ok(v8::null(scope).into());
    };
    let start = get(scope, best, "start")?;
    let run = get(scope, best, "run")?;
    let dir = get(scope, best, "dir")?;
    cb::object(scope, &[("start", start), ("run", run), ("dir", dir)])
}
fn find_burn_lane_from_snapshot<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: FindBurnLaneArgs<'s>,
    need: f64,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if args.plot.is_null_or_undefined() {
        return Err(not_impl(scope, "Firemaking.findBurnLane"));
    }
    let plot = snapshot_plot(scope, args.plot)?;
    let here = snapshot_tile(scope, args.here)?;
    let refused = snapshot_occupied(scope, args.occupied)?;
    let directions = snapshot_directions(scope, args.directions)?;
    let selected =
        fire::find_burn_lane_from_snapshot(plot, here, &refused, need as i32, &directions);
    let Some(selected) = selected.map_err(|reason| not_impl(scope, reason))? else {
        return Ok(v8::null(scope).into());
    };

    let x = cb::num(scope, f64::from(selected.start.x));
    let z = cb::num(scope, f64::from(selected.start.z));
    let level = cb::num(scope, f64::from(selected.start.level));
    let start = cb::construct(scope, args.tile_ctor, &[x, z, level], "Tile")?;
    let run = cb::num(scope, f64::from(selected.run));
    let dx = cb::num(scope, f64::from(selected.direction.0));
    let dz = cb::num(scope, f64::from(selected.direction.1));
    let dir = cb::object(scope, &[("dx", dx), ("dz", dz)])?;
    cb::object(scope, &[("start", start), ("run", run), ("dir", dir)])
}

fn snapshot_plot<'s>(
    scope: &mut v8::HandleScope<'s>,
    plot: v8::Local<'s, v8::Value>,
) -> JsResult<'s, fire::Plot> {
    let x0 = get(scope, plot, "x0")?;
    let x0 = integer_coordinate(scope, x0)?;
    let x1 = get(scope, plot, "x1")?;
    let x1 = integer_coordinate(scope, x1)?;
    let z0 = get(scope, plot, "z0")?;
    let z0 = integer_coordinate(scope, z0)?;
    let z1 = get(scope, plot, "z1")?;
    let z1 = integer_coordinate(scope, z1)?;
    let bank = get(scope, plot, "bank")?;
    let level = get(scope, bank, "level")?;
    let level = if level.is_null_or_undefined() {
        0
    } else {
        integer_coordinate(scope, level)?
    };
    Ok(fire::Plot {
        x0,
        x1,
        z0,
        z1,
        level,
    })
}

fn snapshot_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<api::snapshot::WorldTile>> {
    if tile.is_null_or_undefined() {
        return Ok(None);
    }
    let x = get(scope, tile, "x")?;
    let x = integer_coordinate(scope, x)?;
    let z = get(scope, tile, "z")?;
    let z = integer_coordinate(scope, z)?;
    let level = get(scope, tile, "level")?;
    let level = if level.is_null_or_undefined() {
        0
    } else {
        integer_coordinate(scope, level)?
    };
    Ok(Some(api::snapshot::WorldTile { x, z, level }))
}

fn snapshot_occupied<'s>(
    scope: &mut v8::HandleScope<'s>,
    occupied: v8::Local<'s, v8::Value>,
) -> JsResult<'s, HashSet<(i32, i32)>> {
    let keys = ForOf::open(scope, occupied, "occupied")?;
    let mut refused = HashSet::new();
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = snapshot_occupied_iteration(scope, &keys, &mut refused);
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok(refused)
}

fn snapshot_occupied_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    keys: &ForOf<'s>,
    refused: &mut HashSet<(i32, i32)>,
) -> JsResult<'s, Flow<'s>> {
    let Some(key) = keys.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let key = cb::to_string(scope, key);
    let key = keys.body(scope, key)?;
    if let Some((x, z)) = key.split_once(',') {
        if let (Ok(x), Ok(z)) = (x.parse::<i32>(), z.parse::<i32>()) {
            refused.insert((x, z));
        }
    }
    Ok(Flow::Continue(None))
}

fn snapshot_directions<'s>(
    scope: &mut v8::HandleScope<'s>,
    directions: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Vec<(i32, i32)>> {
    let directions = v8::Local::<v8::Array>::try_from(directions)
        .map_err(|_| cb::type_error(scope, "dirs is not an array"))?;
    let directions_value: v8::Local<v8::Value> = directions.into();
    let length = cb::array_length(scope, directions_value)?;
    let mut result = Vec::with_capacity(length as usize);
    for index in 0..length {
        let direction = cb::get_index(scope, directions_value, index)?;
        let dx = get(scope, direction, "dx")?;
        let dx = integer_coordinate(scope, dx)?;
        let dz = get(scope, direction, "dz")?;
        let dz = integer_coordinate(scope, dz)?;
        result.push((dx, dz));
    }
    Ok(result)
}

fn integer_coordinate<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, i32> {
    let value = number(scope, value)?;
    if value.is_finite()
        && value.fract() == 0.0
        && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value)
    {
        Ok(value as i32)
    } else {
        Err(cb::type_error(scope, "Firemaking coordinate"))
    }
}

fn find_burn_lane_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    directions: &ForOf<'s>,
    context: &BurnLaneContext<'s>,
    best: Option<v8::Local<'s, v8::Value>>,
) -> JsResult<'s, Flow<'s>> {
    let Some(dir) = directions.step(scope)? else {
        return Ok(Flow::Break(best));
    };
    let outcome = find_burn_lane_direction(scope, context, dir, best);
    directions.body(scope, outcome)
}

fn burn_lane_need<'s>(
    scope: &mut v8::HandleScope<'s>,
    want: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    let floor = number(scope, want)?.floor();
    let want = if floor.is_nan() || floor == 0.0 {
        1.0
    } else {
        floor
    };
    Ok(want.clamp(1.0, 27.0))
}

fn find_burn_lane_direction<'s>(
    scope: &mut v8::HandleScope<'s>,
    context: &BurnLaneContext<'s>,
    dir: v8::Local<'s, v8::Value>,
    mut best: Option<v8::Local<'s, v8::Value>>,
) -> JsResult<'s, Flow<'s>> {
    let z0 = get(scope, context.plot, "z0")?;
    let mut z = number(scope, z0)?;
    let poll = Poll::default();
    loop {
        poll.check(scope)?;
        let z1_value = get(scope, context.plot, "z1")?;
        let z1 = number(scope, z1_value)?;
        if z.is_nan() || z1.is_nan() || z > z1 {
            break;
        }
        let x0 = get(scope, context.plot, "x0")?;
        let mut x = number(scope, x0)?;
        loop {
            poll.check(scope)?;
            let x1_value = get(scope, context.plot, "x1")?;
            let x1 = number(scope, x1_value)?;
            if x.is_nan() || x1.is_nan() || x > x1 {
                break;
            }

            let scope = &mut v8::EscapableHandleScope::new(scope);
            let outcome = burn_lane_candidate(scope, context, dir, x, z, best);
            match cb::iteration(scope, outcome)? {
                Flow::Continue(next) => best = next,
                Flow::Break(next) => return Ok(Flow::Break(next)),
            }
            x += 1.0;
        }
        z += 1.0;
    }
    Ok(Flow::Continue(best))
}

fn burn_lane_candidate<'s>(
    scope: &mut v8::HandleScope<'s>,
    context: &BurnLaneContext<'s>,
    dir: v8::Local<'s, v8::Value>,
    x: f64,
    z: f64,
    best: Option<v8::Local<'s, v8::Value>>,
) -> JsResult<'s, Flow<'s>> {
    let cap_count = if is_burn_west(scope, dir)? {
        context.need
    } else {
        1.0
    };
    let cap = cb::num(scope, cap_count);
    let bank = get(scope, context.plot, "bank")?;
    let level = get(scope, bank, "level")?;
    let x_value = cb::num(scope, x);
    let z_value = cb::num(scope, z);
    let start = cb::construct(scope, context.tile_ctor, &[x_value, z_value, level], "Tile")?;
    let run = run_in_dir(
        scope,
        RunArgs {
            from: start,
            plot: context.plot,
            dir,
            occupied: context.occupied,
            walkable: context.walkable,
            can_step: context.can_step,
            cap,
        },
    )?;
    if run == 0.0 {
        return Ok(Flow::Continue(best));
    }

    let here_x_value = get(scope, context.here, "x")?;
    let here_x = number(scope, here_x_value)?;
    let here_z_value = get(scope, context.here, "z")?;
    let here_z = number(scope, here_z_value)?;
    let dx = (x - here_x).abs();
    let dz = (z - here_z).abs();
    let distance = if dx.is_nan() || dz.is_nan() {
        f64::NAN
    } else {
        dx.max(dz)
    };
    let replace = match best {
        None => true,
        Some(current) => better_burn_lane(scope, run, distance, dir, current, context.need)?,
    };
    if !replace {
        return Ok(Flow::Continue(best));
    }

    let run = cb::num(scope, run);
    let distance = cb::num(scope, distance);
    let best = cb::object(
        scope,
        &[
            ("start", start),
            ("run", run),
            ("d", distance),
            ("dir", dir),
        ],
    )?;
    Ok(Flow::Continue(Some(best)))
}

fn better_burn_lane<'s>(
    scope: &mut v8::HandleScope<'s>,
    run: f64,
    distance: f64,
    dir: v8::Local<'s, v8::Value>,
    current: v8::Local<'s, v8::Value>,
    need: f64,
) -> JsResult<'s, bool> {
    let current_run_value = get(scope, current, "run")?;
    let current_run = number(scope, current_run_value)?;
    let current_distance_value = get(scope, current, "d")?;
    let current_distance = number(scope, current_distance_value)?;
    let current_dir = get(scope, current, "dir")?;
    let full = run >= need;
    let current_full = current_run >= need;
    let west = is_burn_west(scope, dir)?;
    let current_west = is_burn_west(scope, current_dir)?;

    if full && west && !(current_full && current_west) {
        return Ok(true);
    }
    if current_full && current_west && !(full && west) {
        return Ok(false);
    }
    if full != current_full {
        return Ok(full);
    }
    if west != current_west {
        return Ok(west);
    }
    if run != current_run {
        return Ok(run > current_run);
    }
    Ok(distance < current_distance)
}

fn is_burn_west<'s>(
    scope: &mut v8::HandleScope<'s>,
    dir: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let dx = get(scope, dir, "dx")?;
    if !dx.strict_equals(cb::num(scope, -1.0)) {
        return Ok(false);
    }
    let dz = get(scope, dir, "dz")?;
    Ok(dz.strict_equals(cb::num(scope, 0.0)))
}

/// One lane tile: `Continue(next)` to step on, `Break` to stop. `walkable` /
/// `canStep` may be builtins, so the loop polls for termination.
fn lane_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    a: &RunArgs<'s>,
    poll: &Poll,
    cur: v8::Local<'s, v8::Value>,
    n: &mut f64,
) -> JsResult<'s, Flow<'s>> {
    poll.check(scope)?;
    let count = cb::num(scope, *n);
    if !cb::lt(scope, count, a.cap)? || !in_fire_plot(scope, cur, a.plot)? {
        return Ok(Flow::Break(None));
    }
    let has = get(scope, a.occupied, "has")?;
    let key = tile_key(scope, cur)?;
    let occupied = Callback::method(has, a.occupied, "occupied.has").call(scope, &[key])?;
    if cb::truthy(scope, occupied) || !a.walkable.truthy(scope, &[cur])? {
        return Ok(Flow::Break(None));
    }
    *n += 1.0;
    let x = get(scope, cur, "x")?;
    let dx = get(scope, a.dir, "dx")?;
    let x = cb::add(scope, x, dx)?;
    let z = get(scope, cur, "z")?;
    let dz = get(scope, a.dir, "dz")?;
    let z = cb::add(scope, z, dz)?;
    let level = get(scope, cur, "level")?;
    let next = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
    if !a.can_step.truthy(scope, &[cur, next])? {
        return Ok(Flow::Break(None));
    }
    Ok(Flow::Continue(Some(next)))
}

/// Frozen `inFirePlot(t, plot)`:
/// `t.x >= plot.x0 && t.x <= plot.x1 && t.z >= plot.z0 && t.z <= plot.z1 && t.level === plot.bank.level`.
fn in_fire_plot<'s>(
    scope: &mut v8::HandleScope<'s>,
    t: v8::Local<'s, v8::Value>,
    plot: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    for (axis, bound, inclusive_low) in [
        ("x", "x0", true),
        ("x", "x1", false),
        ("z", "z0", true),
        ("z", "z1", false),
    ] {
        let value = get(scope, t, axis)?;
        let edge = get(scope, plot, bound)?;
        let inside = if inclusive_low {
            cb::ge(scope, value, edge)?
        } else {
            cb::le(scope, value, edge)?
        };
        if !inside {
            return Ok(false);
        }
    }
    let level = get(scope, t, "level")?;
    let bank = get(scope, plot, "bank")?;
    let bank_level = get(scope, bank, "level")?;
    Ok(level.strict_equals(bank_level))
}

/// Frozen `tileKey(t)`: `` `${t.x},${t.z}` `` (V8 strings, no UTF-8 round trip).
fn tile_key<'s>(
    scope: &mut v8::HandleScope<'s>,
    t: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let x = get(scope, t, "x")?;
    let x = cb::catching(scope, |s| x.to_string(s))?.into();
    let z = get(scope, t, "z")?;
    let z = cb::catching(scope, |s| z.to_string(s))?.into();
    let comma = cb::string(scope, ",");
    let head = cb::add(scope, x, comma)?;
    cb::add(scope, head, z)
}

/// The registry key for a tile (the frozen `Set<string>` key).
fn registry_key<'s>(
    scope: &mut v8::HandleScope<'s>,
    t: v8::Local<'s, v8::Value>,
) -> JsResult<'s, String> {
    let key = tile_key(scope, t)?;
    Ok(key.to_rust_string_lossy(scope))
}

fn slot<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, usize> {
    let slot = number(scope, value)?;
    if slot.is_finite() && slot >= 0.0 && slot.fract() == 0.0 {
        Ok(slot as usize)
    } else {
        Err(bad_slot(scope))
    }
}

fn bad_slot<'s>(scope: &mut v8::HandleScope<'s>) -> cb::Throw<'s> {
    not_impl(scope, "Firemaking.NoLightTiles")
}

/// Frozen `merge(occupied)`: `new Set(occupied)`, then every refused key.
fn no_light_merge<'s>(
    scope: &mut v8::HandleScope<'s>,
    slot_value: v8::Local<'s, v8::Value>,
    occupied: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let slot = slot(scope, slot_value)?;
    let all = v8::Set::new(scope);
    if !occupied.is_null_or_undefined() {
        let keys = ForOf::open(scope, occupied, "occupied")?;
        loop {
            let scope = &mut v8::EscapableHandleScope::new(scope);
            let flow = merge_iteration(scope, &keys, all);
            if let Flow::Break(_) = cb::iteration(scope, flow)? {
                break;
            }
        }
    }
    let refused = fire::no_light_keys(slot).ok_or_else(|| bad_slot(scope))?;
    for key in refused {
        let key = cb::string(scope, &key);
        cb::catching(scope, |s| all.add(s, key))?;
    }
    Ok(all.into())
}

/// `all.add(key)` for one key of the caller's `occupied` iterable.
fn merge_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    keys: &ForOf<'s>,
    all: v8::Local<'s, v8::Set>,
) -> JsResult<'s, Flow<'s>> {
    let Some(key) = keys.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let added = cb::catching(scope, |s| all.add(s, key));
    keys.body(scope, added)?;
    Ok(Flow::Continue(None))
}

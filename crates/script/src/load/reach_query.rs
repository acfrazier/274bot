//! Typed V8 marshalling for `Reachability.{walkable,canReach,canStep}`,
//! v2 `api.walkable` / `api.canStep` / `api.canReach`, and walk arrival.
//!
//! The isolate caches the last posted [`ReachQueryView`]. Helpers read it;
//! JS does not re-implement bit math or arrival.

use api::query::{ReachQueryView, SceneReachOptions};
use api::snapshot::WorldTile;
use rustyscript::Runtime;
use std::cell::RefCell;

use crate::isolate_fb::{ReachReader, SnapshotReader};

const PENDING: &str = "__pending__";

struct CachedReach {
    stamp: Option<crate::observed::Stamp>,
    view: ReachQueryView,
}

thread_local! {
    static REACH: RefCell<CachedReach> = RefCell::new(CachedReach {
        stamp: None,
        view: ReachQueryView::unavailable(),
    });
}

/// Refresh the cached view when [`crate::observed::Lens::reach_stamp`] moves.
/// An omitted delta keeps the last stamp and the last view.
pub(crate) fn apply(snap: &SnapshotReader<'_>) {
    let stamp = crate::observed::with(|scene| scene.latest().reach_stamp());
    REACH.with(|slot| {
        let mut cached = slot.borrow_mut();
        if cached.stamp == stamp {
            return;
        }
        cached.stamp = stamp;
        cached.view = match snap.reach() {
            Some(r) => view_from_reader(r),
            None => ReachQueryView::unavailable(),
        };
    });
}

pub(crate) fn on_reset() {
    REACH.with(|slot| {
        *slot.borrow_mut() = CachedReach {
            stamp: None,
            view: ReachQueryView::unavailable(),
        };
    });
}

/// Read the posted reach view (Rust helpers and machine families).
pub(crate) fn with_view<R>(f: impl FnOnce(&ReachQueryView) -> R) -> R {
    REACH.with(|slot| f(&slot.borrow().view))
}

/// The last posted player tile (the tile walk arrival is measured from).
pub(crate) fn posted_here() -> Option<WorldTile> {
    crate::observed::with(|scene| {
        scene.latest().here().map(|here| WorldTile {
            x: here.x,
            z: here.z,
            level: here.level,
        })
    })
}

/// Frozen `isArrived` ([`api::query::is_arrived`]) from the last posted
/// player tile over the cached reach view: the arrival rule every shim and
/// machine walk pre-check uses. No posted tile is not arrived.
pub(crate) fn arrived(dest: WorldTile, radius: i32) -> bool {
    let Some(here) = posted_here() else {
        return false;
    };
    with_view(|view| api::query::is_arrived(here, dest, radius, || view))
}

/// [`arrived`] for a caller's JS numbers, compared as frozen `isArrived`
/// compares them: `level !==`, then Chebyshev `> radius` (a NaN radius
/// never arrives, a fractional one is a plain bound). A dest off the tile
/// grid is unprobeable, so within the radius it arrives, as frozen's
/// `!probe.probeable(dest)` does.
pub(crate) fn arrived_at(x: f64, z: f64, level: f64, radius: f64) -> bool {
    let Some(here) = posted_here() else {
        return false;
    };
    if f64::from(here.level) != level {
        return false;
    }
    let dist = (f64::from(here.x) - x)
        .abs()
        .max((f64::from(here.z) - z).abs());
    if !(dist <= radius) {
        return false;
    }
    if dist == 0.0 {
        return true;
    }
    let on_grid = |n: f64| n.fract() == 0.0 && n >= f64::from(i32::MIN) && n <= f64::from(i32::MAX);
    if !(on_grid(x) && on_grid(z)) {
        return true;
    }
    let dest = WorldTile {
        x: x as i32,
        z: z as i32,
        level: here.level,
    };
    // `dist <= radius` held: the integer distance is the bound.
    arrived(dest, dist as i32)
}

fn view_from_reader(r: ReachReader<'_>) -> ReachQueryView {
    ReachQueryView {
        available: r.available(),
        base_x: r.base_x(),
        base_z: r.base_z(),
        level: r.level(),
        width: r.width(),
        height: r.height(),
        walkable: r.walkable(),
        reachable: r.reachable(),
        reachable_adj: r.reachable_adj(),
        exact_rank: r.exact_rank(),
        adjacent_rank: r.adjacent_rank(),
        step: r.step(),
        canlight: r.canlight(),
    }
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name =
        v8::String::new(&mut scope, "__rs2b0t_reach").ok_or_else(|| "reach name".to_string())?;
    let func =
        v8::Function::new(&mut scope, reach_callback).ok_or_else(|| "reach fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "reach set".to_string())?;
    Ok(())
}

fn reach_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run_reach(scope, &args) {
        Ok(value) => rv.set(value),
        Err(err) if err == PENDING => {}
        Err(err) => match helper_err(scope, &err) {
            Ok(value) => rv.set(value),
            Err(_) => rv.set(v8::null(scope).into()),
        },
    }
}

fn run_reach<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let kind = js_to_string(scope, args.get(0))?;
    match kind.as_str() {
        "walkable" => v1_walkable(scope, args.get(1)),
        "canStep" => v1_can_step(scope, args.get(1), args.get(2)),
        "canReach" => v1_can_reach(scope, args.get(1), args.get(2)),
        "v2-walkable" => v2_walkable(scope, args.get(1)),
        "v2-canStep" => v2_can_step(scope, args.get(1)),
        "v2-canReach" => v2_can_reach(scope, args.get(1)),
        "arrived" => v1_arrived(scope, args.get(1), args.get(2)),
        _ => Err("invalid-args".into()),
    }
}

fn v1_walkable<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(tile) = tile_or_none(scope, tile)? else {
        return bool_val(scope, false);
    };
    bool_val(scope, with_view(|view| view.walkable(tile)))
}

fn v1_can_step<'s>(
    scope: &mut v8::HandleScope<'s>,
    from: v8::Local<v8::Value>,
    to: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(from) = tile_or_none(scope, from)? else {
        return bool_val(scope, false);
    };
    let Some(to) = tile_or_none(scope, to)? else {
        return bool_val(scope, false);
    };
    bool_val(scope, with_view(|view| view.can_step(from, to)))
}

fn v1_can_reach<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<v8::Value>,
    opts: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(tile) = tile_or_none(scope, tile)? else {
        return bool_val(scope, false);
    };
    let Some(options) = v1_reach_options(scope, opts)? else {
        return bool_val(scope, false);
    };
    bool_val(scope, with_view(|view| view.can_reach(tile, &options)))
}

/// `__rs2b0t_reach('arrived', dest, radius)`: [`arrived_at`] over JS
/// `ToNumber` of `dest.x`, `dest.z`, `dest.level` (absent: 0) and `radius`.
/// A missing dest is not arrived.
fn v1_arrived<'s>(
    scope: &mut v8::HandleScope<'s>,
    dest: v8::Local<v8::Value>,
    radius: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if dest.is_null_or_undefined() || !dest.is_object() {
        return bool_val(scope, false);
    }
    let x = optional_field(scope, dest, "x")?;
    let z = optional_field(scope, dest, "z")?;
    let level = optional_field(scope, dest, "level")?;
    let x = js_number(scope, x, f64::NAN);
    let z = js_number(scope, z, f64::NAN);
    let level = js_number(scope, level, 0.0);
    let radius = js_number(scope, Some(radius), f64::NAN);
    bool_val(scope, arrived_at(x, z, level, radius))
}

/// JS `ToNumber` of a present value; `absent` for `undefined`/`null`.
fn js_number(scope: &mut v8::HandleScope, value: Option<v8::Local<v8::Value>>, absent: f64) -> f64 {
    match value {
        Some(value) if !value.is_null_or_undefined() => {
            value.number_value(scope).unwrap_or(f64::NAN)
        }
        _ => absent,
    }
}

fn v2_walkable<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let tile = v2_tile_arg(scope, input, "tile")?;
    match with_view(|view| {
        if !view.available {
            None
        } else {
            Some(ReachQueryView::bit_at(
                &view.walkable,
                view.width,
                view.height,
                view.base_x,
                view.base_z,
                view.level,
                tile,
            ))
        }
    }) {
        None => helper_err(scope, "missing-observation"),
        Some(hit) => helper_ok_bool(scope, hit),
    }
}

fn v2_can_step<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !input.is_object() || input.is_null() || input.is_undefined() {
        return helper_err(scope, "invalid-args");
    }
    let from_v = field(scope, input, "from")?;
    let from = required_tile(scope, from_v)?;
    let to_v = field(scope, input, "to")?;
    let to = required_tile(scope, to_v)?;
    match with_view(|view| {
        if !view.available {
            None
        } else {
            Some(view.can_step(from, to))
        }
    }) {
        None => helper_err(scope, "missing-observation"),
        Some(hit) => helper_ok_bool(scope, hit),
    }
}

fn v2_can_reach<'s>(
    scope: &mut v8::HandleScope<'s>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let tile = v2_tile_arg(scope, input, "tile")?;
    let options = v2_reach_options(scope, input)?;
    match with_view(|view| {
        if !view.available {
            None
        } else {
            Some(view.can_reach(tile, &options))
        }
    }) {
        None => helper_err(scope, "missing-observation"),
        Some(hit) => helper_ok_bool(scope, hit),
    }
}

fn v2_tile_arg(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
    name: &str,
) -> Result<WorldTile, String> {
    if !input.is_object() || input.is_null() || input.is_undefined() {
        return Err("invalid-args".into());
    }
    let tile = field(scope, input, name)?;
    if tile.is_undefined() {
        required_tile(scope, input)
    } else {
        required_tile(scope, tile)
    }
}

fn v1_reach_options(
    scope: &mut v8::HandleScope,
    opts: v8::Local<v8::Value>,
) -> Result<Option<SceneReachOptions>, String> {
    if opts.is_null() || opts.is_undefined() {
        return Ok(Some(SceneReachOptions {
            max_steps: Some(400),
            adjacent_ok: false,
        }));
    }
    if !opts.is_object() {
        return Ok(None);
    }
    let adjacent_ok = match optional_field(scope, opts, "adjacentOk")? {
        None => false,
        Some(v) if v.is_undefined() => false,
        Some(v) => v.boolean_value(scope),
    };
    let max_steps = match optional_field(scope, opts, "maxSteps")? {
        None => Some(400),
        Some(v) if v.is_undefined() => Some(400),
        Some(v) => match optional_u32(scope, v)? {
            Some(n) => Some(n),
            None => return Ok(None),
        },
    };
    Ok(Some(SceneReachOptions {
        max_steps,
        adjacent_ok,
    }))
}

fn v2_reach_options(
    scope: &mut v8::HandleScope,
    input: v8::Local<v8::Value>,
) -> Result<SceneReachOptions, String> {
    let adjacent_ok = match optional_field(scope, input, "adjacentOk")? {
        None => false,
        Some(v) if v.is_undefined() => false,
        Some(v) => v.boolean_value(scope),
    };
    let max_steps = match optional_field(scope, input, "maxSteps")? {
        None => Some(400),
        Some(v) if v.is_undefined() => Some(400),
        Some(v) => Some(required_u32(scope, v)?),
    };
    Ok(SceneReachOptions {
        max_steps,
        adjacent_ok,
    })
}

fn required_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<WorldTile, String> {
    tile_or_none(scope, value)?.ok_or_else(|| "invalid-args".to_string())
}

fn tile_or_none(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<WorldTile>, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Ok(None);
    }
    let x = match required_i32_field(scope, value, "x") {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let z = match required_i32_field(scope, value, "z") {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let level = match optional_field(scope, value, "level")? {
        None => 0,
        Some(v) if v.is_null() || v.is_undefined() => 0,
        Some(v) => match required_i32(scope, v) {
            Ok(n) => n,
            Err(_) => return Ok(None),
        },
    };
    Ok(Some(WorldTile { x, z, level }))
}

fn optional_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid-args".to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "string".to_string())?;
    Ok(obj.get(scope, key.into()))
}

fn field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    optional_field(scope, value, name)?.ok_or_else(|| PENDING.to_string())
}

fn required_i32_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<i32, String> {
    let v = field(scope, value, name)?;
    required_i32(scope, v)
}

fn required_i32(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<i32, String> {
    if !value.is_number() {
        return Err("invalid-args".into());
    }
    let n = value
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Err("invalid-args".into());
    }
    if n < (i32::MIN as f64) || n > (i32::MAX as f64) {
        return Err("invalid-args".into());
    }
    Ok(n as i32)
}

fn optional_u32(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<u32>, String> {
    if !value.is_number() {
        return Ok(None);
    }
    let n = value
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    if !n.is_finite() || n.fract() != 0.0 || n < 0.0 || n > (u32::MAX as f64) {
        return Ok(None);
    }
    Ok(Some(n as u32))
}

fn required_u32(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<u32, String> {
    optional_u32(scope, value)?.ok_or_else(|| "invalid-args".to_string())
}

fn js_to_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<String, String> {
    match value.to_string(scope) {
        Some(s) => Ok(s.to_rust_string_lossy(scope)),
        None => Err(PENDING.into()),
    }
}

fn helper_ok_bool<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, true);
    set_key(scope, obj, "ok", ok.into());
    let value = v8::Boolean::new(scope, value);
    set_key(scope, obj, "value", value.into());
    Ok(obj.into())
}

fn helper_err<'s>(
    scope: &mut v8::HandleScope<'s>,
    error: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let ok = v8::Boolean::new(scope, false);
    set_key(scope, obj, "ok", ok.into());
    let msg = v8::String::new(scope, error).ok_or_else(|| "string".to_string())?;
    set_key(scope, obj, "error", msg.into());
    Ok(obj.into())
}

fn bool_val<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    Ok(v8::Boolean::new(scope, value).into())
}

fn set_key<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(k) = v8::String::new(scope, key) {
        obj.set(scope, k.into(), value);
    }
}

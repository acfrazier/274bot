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

/// [`arrived`] for a caller's JS numbers, with frozen `isArrived`'s
/// comparisons (`arrival.ts`, `Reachability.ts`, `ClientAdapter.toLocal` /
/// `collisionFlags`):
/// - `me.level !== dest.level` is not arrived;
/// - `dist > radius` is not arrived, so a NaN radius or coordinate passes on
///   (`NaN > r` is false); `dist === 0` arrives;
/// - an on-grid dest takes the reach probes ([`api::query::is_arrived`]);
/// - an off-grid dest (fractional or NaN `x`/`z`): with no scene, or outside
///   the scene window, `toLocal` is `null` and the dest is unprobeable, so
///   it arrives; inside (NaN fails every bound check, so it is inside),
///   `collisionFlags` reads `undefined`, which frozen `walkable` takes as
///   walkable and unreached, so it does not.
pub(crate) fn arrived_at(x: f64, z: f64, level: f64, radius: f64) -> bool {
    let Some(here) = posted_here() else {
        return false;
    };
    if f64::from(here.level) != level {
        return false;
    }
    // JS `Math.max(Math.abs(dx), Math.abs(dz))`: NaN in, NaN out.
    let (dx, dz) = ((f64::from(here.x) - x).abs(), (f64::from(here.z) - z).abs());
    let dist = if dx.is_nan() || dz.is_nan() {
        f64::NAN
    } else {
        dx.max(dz)
    };
    if dist > radius {
        return false;
    }
    if dist == 0.0 {
        return true;
    }
    let on_grid = |n: f64| n.fract() == 0.0 && n >= f64::from(i32::MIN) && n <= f64::from(i32::MAX);
    if on_grid(x) && on_grid(z) {
        let dest = WorldTile {
            x: x as i32,
            z: z as i32,
            level: here.level,
        };
        // `dist > radius` failed: the radius does not bound the probes.
        let bound =
            i32::try_from(here.x.abs_diff(dest.x).max(here.z.abs_diff(dest.z))).unwrap_or(i32::MAX);
        return with_view(|view| api::query::is_arrived(here, dest, bound, || view));
    }
    with_view(|view| {
        if !view.available {
            return true;
        }
        let lx = x - f64::from(view.base_x);
        let lz = z - f64::from(view.base_z);
        lx < 0.0 || lz < 0.0 || lx >= f64::from(view.width) || lz >= f64::from(view.height)
    })
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
/// `ToNumber` of `dest.x`, `dest.z` and `radius`, and a numeric
/// `dest.level` (absent: 0; any other type is never equal).
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
    let x = to_number(scope, x);
    let z = to_number(scope, z);
    // `me.level !== dest.level` is strict: only a number can be equal. An
    // absent level is the shim's `?? 0`.
    let level = match level {
        Some(level) if !level.is_null_or_undefined() => {
            if level.is_number() {
                level.number_value(scope).unwrap_or(f64::NAN)
            } else {
                f64::NAN
            }
        }
        _ => 0.0,
    };
    let radius = to_number(scope, Some(radius));
    bool_val(scope, arrived_at(x, z, level, radius))
}

/// JS `ToNumber` (`undefined` → NaN, `null` → 0); a failed read is NaN.
fn to_number(scope: &mut v8::HandleScope, value: Option<v8::Local<v8::Value>>) -> f64 {
    value
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::NAN)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{encode_snapshot, ReachViewInput, TileInput};

    /// Post the player at (10,10) with a 4x4 scene window from (8,8), or no
    /// scene at all.
    fn post(window: bool) {
        crate::observed::on_reset();
        on_reset();
        let mut input = crate::isolate_fb::tests::empty_input(1);
        input.here = Some(TileInput {
            x: 10,
            z: 10,
            level: 0,
        });
        if window {
            input.reach = ReachViewInput {
                available: true,
                base_x: 8,
                base_z: 8,
                level: 0,
                width: 4,
                height: 4,
                walkable: &[0xffff],
                reachable: &[],
                reachable_adj: &[],
                exact_rank: &[],
                adjacent_rank: &[],
                step: &[],
                canlight: &[],
                stamp: 1,
            };
        }
        let bytes = encode_snapshot(&input);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        crate::observed::apply(&snap);
        apply(&snap);
    }

    /// Frozen `isArrived` over JS numbers: `dist > radius` alone rejects,
    /// so a NaN radius passes on to the probes, and an off-grid dest is
    /// arrived only where frozen `toLocal` finds no scene tile.
    #[test]
    fn arrival_numbers_compare_as_frozen() {
        post(false);
        assert!(arrived_at(10.0, 10.0, 0.0, f64::NAN), "dist 0 arrives");
        assert!(!arrived_at(10.0, 10.0, 1.0, 2.0), "another level");
        assert!(!arrived_at(13.0, 10.0, 0.0, 2.5), "dist 3 > 2.5");
        assert!(arrived_at(12.0, 10.0, 0.0, 2.5), "fractional radius bounds");
        assert!(
            arrived_at(13.0, 10.0, 0.0, f64::NAN),
            "NaN radius: no scene to probe, so arrived"
        );
        assert!(arrived_at(11.5, 10.0, 0.0, 2.0), "off grid, no scene");

        post(true);
        assert!(
            !arrived_at(11.5, 10.0, 0.0, 2.0),
            "off grid inside the window: walkable and unreached"
        );
        assert!(
            !arrived_at(f64::NAN, 10.0, 0.0, 2.0),
            "NaN x passes every window bound: inside"
        );
        assert!(
            arrived_at(7.5, 10.0, 0.0, 3.0),
            "off grid outside the window: unprobeable"
        );
        assert!(
            !arrived_at(11.0, 10.0, 0.0, f64::NAN),
            "NaN radius on grid: the walkable, unreached dest is not arrived"
        );
    }
}

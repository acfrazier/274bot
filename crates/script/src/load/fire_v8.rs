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

use super::callback_v8::{self as cb, get, not_impl, number, Callback, ForOf, JsResult};
use crate::fire;
use rustyscript::Runtime;

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
        "noLightNew" => Ok(cb::num(scope, fire::no_light_new() as f64)),
        "noLightAdd" => {
            let slot = slot(scope, args.get(1))?;
            let key = tile_key(scope, args.get(2))?;
            fire::no_light_add(slot, key).ok_or_else(|| bad_slot(scope))?;
            Ok(v8::undefined(scope).into())
        }
        "noLightHas" => {
            let slot = slot(scope, args.get(1))?;
            let key = tile_key(scope, args.get(2))?;
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
    let x0 = cb::num(scope, x0);
    let x = get(scope, origin, "x")?;
    let x1 = cb::add(scope, x, h)?;
    let z = get(scope, origin, "z")?;
    let z0 = cb::sub(scope, z, h)?;
    let z0 = cb::num(scope, z0);
    let z = get(scope, origin, "z")?;
    let z1 = cb::add(scope, z, h)?;
    cb::object(
        scope,
        &[("bank", bank), ("x0", x0), ("x1", x1), ("z0", z0), ("z1", z1)],
    )
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
    loop {
        let count = cb::num(scope, n);
        if !cb::lt(scope, count, a.cap)? {
            break;
        }
        if !in_fire_plot(scope, cur, a.plot)? {
            break;
        }
        let has = get(scope, a.occupied, "has")?;
        let key = tile_key(scope, cur)?;
        let key = cb::string(scope, &key);
        let occupied = Callback::method(has, a.occupied, "occupied.has").call(scope, &[key])?;
        if cb::truthy(scope, occupied) || !a.walkable.truthy(scope, &[cur])? {
            break;
        }
        n += 1.0;
        let x = get(scope, cur, "x")?;
        let dx = get(scope, a.dir, "dx")?;
        let x = cb::add(scope, x, dx)?;
        let z = get(scope, cur, "z")?;
        let dz = get(scope, a.dir, "dz")?;
        let z = cb::add(scope, z, dz)?;
        let level = get(scope, cur, "level")?;
        let next = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
        if !a.can_step.truthy(scope, &[cur, next])? {
            break;
        }
        cur = next;
    }
    Ok(n)
}

/// Frozen `inFirePlot(t, plot)`:
/// `t.x >= plot.x0 && t.x <= plot.x1 && t.z >= plot.z0 && t.z <= plot.z1 && t.level === plot.bank.level`.
fn in_fire_plot<'s>(
    scope: &mut v8::HandleScope<'s>,
    t: v8::Local<'s, v8::Value>,
    plot: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    for (axis, bound, inclusive_low) in [("x", "x0", true), ("x", "x1", false), ("z", "z0", true), ("z", "z1", false)] {
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

/// Frozen `tileKey(t)`: `` `${t.x},${t.z}` ``.
fn tile_key<'s>(scope: &mut v8::HandleScope<'s>, t: v8::Local<'s, v8::Value>) -> JsResult<'s, String> {
    let x = get(scope, t, "x")?;
    let x = cb::to_string(scope, x)?;
    let z = get(scope, t, "z")?;
    let z = cb::to_string(scope, z)?;
    Ok(format!("{x},{z}"))
}

fn slot<'s>(scope: &mut v8::HandleScope<'s>, value: v8::Local<'s, v8::Value>) -> JsResult<'s, usize> {
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
        while let Some(key) = keys.step(scope)? {
            let added = cb::catching(scope, |s| all.add(s, key));
            keys.body(scope, added)?;
        }
    }
    let refused = fire::no_light_keys(slot).ok_or_else(|| bad_slot(scope))?;
    for key in refused {
        let key = cb::string(scope, &key);
        cb::catching(scope, |s| all.add(s, key))?;
    }
    Ok(all.into())
}

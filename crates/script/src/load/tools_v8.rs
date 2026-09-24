//! `Tools` (frozen `bot/api/acquisition/Tools.ts`) as one native call per
//! export: `__rs2b0t_tools(op, ...args)`.
//!
//! Each op walks the frozen loop in Rust and calls the script's own callbacks
//! through [`callback_v8`] in the frozen order, with the frozen arguments and
//! truthiness. Candidate tables and use/wield gates are
//! [`crate::gather_tools`]; the caller's requirement rows and tier lists are
//! read in place (getters run where the frozen body reads them). A `tiered`
//! requirement stays an explicit `notImpl`.

use super::callback_v8::{
    self as cb, get, not_impl, number, Callback, ForOf, JsResult,
};
use crate::gather_tools::{self, ToolKind};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_tools", tools)
}

fn tools<'s>(
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
        "bestAxe" => best(scope, ToolKind::Axe, args.get(1), args.get(2)),
        "bestPickaxe" => best(scope, ToolKind::Pickaxe, args.get(1), args.get(2)),
        "bestFromTiers" => best_from_tiers(scope, args.get(1), args.get(2), args.get(3)),
        "canWieldTool" => can_wield(scope, args.get(1), args.get(2)),
        "hasAllTools" => has_all(scope, args.get(1), args.get(3)),
        "hasToolReq" => {
            let count = Callback::plain(scope, args.get(3), "count");
            let ok = has_tool_req(scope, args.get(1), count, "Tools.hasToolReq")?;
            Ok(v8::Boolean::new(scope, ok).into())
        }
        "toolRestockPlan" => restock_plan(scope, args.get(1), args.get(3), args.get(4)),
        _ => Err(not_impl(scope, "Tools")),
    }
}

fn nullable<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: Option<&'static str>,
) -> v8::Local<'s, v8::Value> {
    match name {
        Some(name) => cb::string(scope, name),
        None => v8::null(scope).into(),
    }
}

/// `bestAxe` / `bestPickaxe`: frozen `bestFromTiers(level, TABLE, available)`
/// over the selected-revision table. `level >= use_level` converts the
/// caller's level at each gated candidate; `available(name)` is truthy-tested.
fn best<'s>(
    scope: &mut v8::HandleScope<'s>,
    kind: ToolKind,
    level: v8::Local<'s, v8::Value>,
    available: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let available = Callback::plain(scope, available, "available");
    let hit = gather_tools::best_tool(
        scope,
        kind,
        |scope, need| Ok(number(scope, level)? >= f64::from(need)),
        |scope, name| {
            let name = cb::string(scope, name);
            available.truthy(scope, &[name])
        },
    )?;
    Ok(nullable(scope, hit))
}

/// Frozen `bestFromTiers(level, tiers, available)`:
/// `for (const t of tiers) if (level >= t.level && available(t.name)) return t.name;`
fn best_from_tiers<'s>(
    scope: &mut v8::HandleScope<'s>,
    level: v8::Local<'s, v8::Value>,
    tiers: v8::Local<'s, v8::Value>,
    available: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let available = Callback::plain(scope, available, "available");
    let tiers = ForOf::open(scope, tiers, "tiers")?;
    while let Some(tier) = tiers.step(scope)? {
        let hit = tier_hit(scope, level, tier, available);
        if let Some(name) = tiers.body(scope, hit)? {
            tiers.close(scope)?;
            return Ok(name);
        }
    }
    Ok(v8::null(scope).into())
}

fn tier_hit<'s>(
    scope: &mut v8::HandleScope<'s>,
    level: v8::Local<'s, v8::Value>,
    tier: v8::Local<'s, v8::Value>,
    available: Callback<'s>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let need = get(scope, tier, "level")?;
    if !cb::ge(scope, level, need)? {
        return Ok(None);
    }
    let name = get(scope, tier, "name")?;
    if !available.truthy(scope, &[name])? {
        return Ok(None);
    }
    Ok(Some(get(scope, tier, "name")?))
}

/// `canWieldTool(name, attack)`: exact (trimmed) name over the selected
/// tables; Attack is converted only for a gated tool.
fn can_wield<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    attack: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let name = if name.is_null_or_undefined() {
        String::new()
    } else {
        cb::to_string(scope, name)?
    };
    let ok = match gather_tools::wield_gate(&name) {
        None => false,
        Some(None) => true,
        Some(Some(need)) => number(scope, attack)? >= f64::from(need),
    };
    Ok(v8::Boolean::new(scope, ok).into())
}

/// Frozen `hasAllTools(reqs, skillLevel, count)`:
/// `reqs.every(r => hasToolReq(r, skillLevel, count))`.
fn has_all<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let count = Callback::plain(scope, count, "count");
    let every = get(scope, reqs, "every")?;
    if !every.is_function() {
        return Err(cb::type_error(scope, "reqs.every is not a function"));
    }
    // Array.prototype.every: length read once, holes skipped, first false exits.
    let length = get(scope, reqs, "length")?;
    let length = number(scope, length)?;
    let length = if length.is_nan() || length <= 0.0 {
        0
    } else {
        length.min(f64::from(u32::MAX)) as u32
    };
    for index in 0..length {
        let present = cb::catching(scope, |s| reqs.to_object(s)?.has_index(s, index))?;
        if !present {
            continue;
        }
        let req = cb::get_index(scope, reqs, index)?;
        if !has_tool_req(scope, req, count, "Tools.hasAllTools")? {
            return Ok(v8::Boolean::new(scope, false).into());
        }
    }
    Ok(v8::Boolean::new(scope, true).into())
}

/// Frozen `hasToolReq(req, skillLevel, count)` for an exact requirement:
/// `count(req.name) >= (req.min ?? 1)`.
fn has_tool_req<'s>(
    scope: &mut v8::HandleScope<'s>,
    req: v8::Local<'s, v8::Value>,
    count: Callback<'s>,
    feature: &str,
) -> JsResult<'s, bool> {
    let kind = get(scope, req, "kind")?;
    if is_tiered(scope, kind) {
        return Err(not_impl(scope, feature));
    }
    let name = get(scope, req, "name")?;
    let have = count.call(scope, &[name])?;
    let min = get(scope, req, "min")?;
    let min = if min.is_null_or_undefined() {
        cb::num(scope, 1.0)
    } else {
        min
    };
    cb::ge(scope, have, min)
}

fn is_tiered(scope: &mut v8::HandleScope, kind: v8::Local<v8::Value>) -> bool {
    kind.is_string() && kind.to_rust_string_lossy(scope) == "tiered"
}

/// Frozen `toolRestockPlan(reqs, skillLevel, invCount, bankCount)` for exact
/// requirements: per row `invCount(r.name)`, then — only for a short pack —
/// `bankCount(r.name)`, then one `{ name, qty, equip }` step.
fn restock_plan<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    inv_count: v8::Local<'s, v8::Value>,
    bank_count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let inv_count = Callback::plain(scope, inv_count, "invCount");
    let bank_count = Callback::plain(scope, bank_count, "bankCount");
    let plan = v8::Array::new(scope, 0);
    let mut len = 0u32;
    let rows = ForOf::open(scope, reqs, "reqs")?;
    while let Some(req) = rows.step(scope)? {
        let step = restock_row(scope, req, inv_count, bank_count);
        if let Some(step) = rows.body(scope, step)? {
            cb::catching(scope, |s| plan.set_index(s, len, step))?;
            len += 1;
        }
    }
    Ok(plan.into())
}

fn restock_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    req: v8::Local<'s, v8::Value>,
    inv_count: Callback<'s>,
    bank_count: Callback<'s>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let kind = get(scope, req, "kind")?;
    if is_tiered(scope, kind) {
        return Err(not_impl(scope, "Tools.toolRestockPlan"));
    }
    let min = get(scope, req, "min")?;
    let min = if min.is_null_or_undefined() {
        cb::num(scope, 1.0)
    } else {
        min
    };
    let restock = get(scope, req, "restock")?;
    let target = if restock.is_null_or_undefined() {
        min
    } else {
        restock
    };
    let name = get(scope, req, "name")?;
    let have = inv_count.call(scope, &[name])?;
    let need = cb::sub(scope, target, have)?;
    if need <= 0.0 {
        return Ok(None);
    }
    let name = get(scope, req, "name")?;
    let available = bank_count.call(scope, &[name])?;
    let zero = cb::num(scope, 0.0);
    if cb::le(scope, available, zero)? {
        return Ok(None);
    }
    let name = get(scope, req, "name")?;
    let available = number(scope, available)?;
    let qty = if need.is_nan() || available.is_nan() {
        f64::NAN
    } else {
        need.min(available)
    };
    let qty = cb::num(scope, qty);
    let equip = get(scope, req, "equip")?;
    let equip = v8::Boolean::new(scope, equip.is_true()).into();
    Ok(Some(cb::object(
        scope,
        &[("name", name), ("qty", qty), ("equip", equip)],
    )?))
}

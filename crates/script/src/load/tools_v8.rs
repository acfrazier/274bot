//! `Tools` (frozen `bot/api/acquisition/Tools.ts`) as one native call per
//! export: `__rs2b0t_tools(op, ...args)`.
//!
//! Callback-bearing operations walk the frozen loops in Rust and invoke the
//! script's own callbacks in order. Native candidate lists and gates come from
//! [`crate::gather_tools`]; JS tier arrays are projections of native facts.

use super::callback_v8::{
    self as cb, get, not_impl, number, Callback, Flow, ForOf, JsResult, Poll,
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
        "toolTiers" => tool_tiers(scope, args.get(1)),
        "bestAxe" => best(scope, ToolKind::Axe, args.get(1), args.get(2)),
        "bestPickaxe" => best(scope, ToolKind::Pickaxe, args.get(1), args.get(2)),
        "bestFromTiers" => best_from_tiers(scope, args.get(1), args.get(2), args.get(3)),
        "canWieldTool" => can_wield(scope, args.get(1), args.get(2)),
        "toolKeepNames" => tool_keep_names(scope, args.get(1)),
        "hasAllTools" => has_all(scope, args.get(1), args.get(2), args.get(3)),
        "hasToolReq" => {
            let skill_level = Callback::plain(scope, args.get(2), "skillLevel");
            let count = Callback::plain(scope, args.get(3), "count");
            let ok = has_tool_req(scope, args.get(1), skill_level, count)?;
            Ok(v8::Boolean::new(scope, ok).into())
        }
        "toolRestockPlan" => {
            restock_plan(scope, args.get(1), args.get(2), args.get(3), args.get(4))
        }
        "missingToolLabels" => missing_labels(scope, args.get(1), args.get(2), args.get(3)),
        "toolKitLabel" => tool_kit_label(scope, args.get(1), args.get(2), args.get(3)),
        "bankHasBetterGatherTool" => {
            bank_has_better_tool(scope, args.get(1), args.get(2), args.get(3), args.get(4))
        }
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
fn tool_tiers<'s>(
    scope: &mut v8::HandleScope<'s>,
    kind: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let kind = cb::to_string(scope, kind)?;
    let kind = match kind.as_str() {
        "axes" => ToolKind::Axe,
        "pickaxes" => ToolKind::Pickaxe,
        _ => return Ok(v8::Array::new(scope, 0).into()),
    };
    let rows = kind.candidates();
    let tiers = v8::Array::new(scope, rows.len() as i32);
    let poll = Poll::default();
    for (index, tool) in rows.iter().enumerate() {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = project_tool_tier_iteration(scope, &poll, tool, tiers, index as u32);
        cb::iteration(scope, flow)?;
    }
    Ok(tiers.into())
}

fn project_tool_tier_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    poll: &Poll,
    tool: &api::gather_tools::GatherTool,
    tiers: v8::Local<'s, v8::Array>,
    index: u32,
) -> JsResult<'s, Flow<'s>> {
    poll.check(scope)?;
    let name = cb::string(scope, tool.name);
    let level = cb::num(scope, f64::from(tool.use_level.unwrap_or(0)));
    let tier = cb::object(scope, &[("name", name), ("level", level)])?;
    if let Some(wield_attack) = tool.wield_attack {
        let key = cb::string(scope, "attackLevel");
        let attack_level = cb::num(scope, f64::from(wield_attack));
        cb::catching(scope, |s| {
            tier.to_object(s)?
                .create_data_property(s, key.cast(), attack_level)
        })?;
    }
    cb::catching(scope, |s| tiers.set_index(s, index, tier))?;
    Ok(Flow::Continue(None))
}

/// `bestAxe` / `bestPickaxe` use the native selected-revision table. The
/// caller's level is converted at each gated candidate and `available(name)`
/// is truthy-tested.
fn best<'s>(
    scope: &mut v8::HandleScope<'s>,
    kind: ToolKind,
    level: v8::Local<'s, v8::Value>,
    available: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let available = Callback::plain(scope, available, "available");
    let hit = native_best_tool(scope, kind, level, TierAvailability::Truthy(available))?;
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
    let hit = best_from_tiers_with(scope, level, tiers, TierAvailability::Truthy(available))?;
    Ok(hit.unwrap_or_else(|| v8::null(scope).into()))
}

#[derive(Clone, Copy)]
enum TierAvailability<'s> {
    Truthy(Callback<'s>),
    Positive(Callback<'s>),
    InventoryOrBank {
        inventory: Callback<'s>,
        bank: Callback<'s>,
    },
}

fn tier_available<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    availability: TierAvailability<'s>,
) -> JsResult<'s, bool> {
    match availability {
        TierAvailability::Truthy(available) => available.truthy(scope, &[name]),
        TierAvailability::Positive(count) => {
            let zero = cb::num(scope, 0.0);
            let have = count.call(scope, &[name])?;
            cb::gt(scope, have, zero)
        }
        TierAvailability::InventoryOrBank { inventory, bank } => {
            let zero = cb::num(scope, 0.0);
            let in_inventory = inventory.call(scope, &[name])?;
            if cb::gt(scope, in_inventory, zero)? {
                return Ok(true);
            }
            let in_bank = bank.call(scope, &[name])?;
            cb::gt(scope, in_bank, zero)
        }
    }
}
fn native_best_tool<'s>(
    scope: &mut v8::HandleScope<'s>,
    kind: ToolKind,
    level: v8::Local<'s, v8::Value>,
    availability: TierAvailability<'s>,
) -> JsResult<'s, Option<&'static str>> {
    gather_tools::best_tool(
        scope,
        kind,
        |scope, need| {
            let need = cb::num(scope, f64::from(need));
            cb::ge(scope, level, need)
        },
        |scope, name| {
            let name = cb::string(scope, name);
            tier_available(scope, name, availability)
        },
    )
}

fn best_from_tiers_with<'s>(
    scope: &mut v8::HandleScope<'s>,
    level: v8::Local<'s, v8::Value>,
    tiers: v8::Local<'s, v8::Value>,
    availability: TierAvailability<'s>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let tiers = ForOf::open(scope, tiers, "tiers")?;
    let hit = loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = tier_iteration(scope, &tiers, level, availability);
        if let Flow::Break(hit) = cb::iteration(scope, flow)? {
            break hit;
        }
    };
    Ok(hit)
}

fn tier_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    tiers: &ForOf<'s>,
    level: v8::Local<'s, v8::Value>,
    availability: TierAvailability<'s>,
) -> JsResult<'s, Flow<'s>> {
    let Some(tier) = tiers.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let hit = tier_hit(scope, level, tier, availability);
    match tiers.body(scope, hit)? {
        Some(name) => {
            tiers.close(scope)?;
            Ok(Flow::Break(Some(name)))
        }
        None => Ok(Flow::Continue(None)),
    }
}

fn tier_hit<'s>(
    scope: &mut v8::HandleScope<'s>,
    level: v8::Local<'s, v8::Value>,
    tier: v8::Local<'s, v8::Value>,
    availability: TierAvailability<'s>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let need = get(scope, tier, "level")?;
    if !cb::ge(scope, level, need)? {
        return Ok(None);
    }
    let name = get(scope, tier, "name")?;
    if !tier_available(scope, name, availability)? {
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
        Some(Some(need)) => {
            let need = cb::num(scope, f64::from(need));
            cb::ge(scope, attack, need)?
        }
    };
    Ok(v8::Boolean::new(scope, ok).into())
}

/// Frozen `hasAllTools(reqs, skillLevel, count)`:
/// `reqs.every(r => hasToolReq(r, skillLevel, count))`.
fn has_all<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    skill_level: v8::Local<'s, v8::Value>,
    count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let skill_level = Callback::plain(scope, skill_level, "skillLevel");
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
    let poll = Poll::default();
    let mut every = true;
    for index in 0..length {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = every_iteration(scope, &poll, reqs, index, skill_level, count);
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            every = false;
            break;
        }
    }
    Ok(v8::Boolean::new(scope, every).into())
}

/// One `every` index: a hole is skipped (no JS runs, hence the [`Poll`]); a
/// failing requirement breaks.
fn every_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    poll: &Poll,
    reqs: v8::Local<'s, v8::Value>,
    index: u32,
    skill_level: Callback<'s>,
    count: Callback<'s>,
) -> JsResult<'s, Flow<'s>> {
    poll.check(scope)?;
    let present = cb::catching(scope, |s| reqs.to_object(s)?.has_index(s, index))?;
    if !present {
        return Ok(Flow::Continue(None));
    }
    let req = cb::get_index(scope, reqs, index)?;
    if has_tool_req(scope, req, skill_level, count)? {
        Ok(Flow::Continue(None))
    } else {
        Ok(Flow::Break(None))
    }
}

/// Frozen `hasToolReq(req, skillLevel, count)` for exact and native tiered rows.
fn has_tool_req<'s>(
    scope: &mut v8::HandleScope<'s>,
    req: v8::Local<'s, v8::Value>,
    skill_level: Callback<'s>,
    count: Callback<'s>,
) -> JsResult<'s, bool> {
    let kind = get(scope, req, "kind")?;
    if is_tiered(scope, kind) {
        let skill = get(scope, req, "skill")?;
        let level = skill_level.call(scope, &[skill])?;
        let Some(kind) = tiered_tool_kind(scope, skill) else {
            return Ok(false);
        };
        let best = native_best_tool(scope, kind, level, TierAvailability::Positive(count))?;
        return Ok(best.is_some());
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

fn tiered_tool_kind(scope: &mut v8::HandleScope, skill: v8::Local<v8::Value>) -> Option<ToolKind> {
    if !skill.is_string() {
        return None;
    }
    let skill = skill.to_rust_string_lossy(scope);
    ToolKind::for_skill(&skill)
}

fn tool_keep_names<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let names = v8::Array::new(scope, 0);
    let reqs = ForOf::open(scope, reqs, "reqs")?;
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = keep_names_iteration(scope, &reqs, names);
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok(names.into())
}

fn keep_names_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: &ForOf<'s>,
    names: v8::Local<'s, v8::Array>,
) -> JsResult<'s, Flow<'s>> {
    let Some(req) = reqs.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let body = keep_names_row(scope, req, names);
    reqs.body(scope, body)?;
    Ok(Flow::Continue(None))
}

fn keep_names_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    req: v8::Local<'s, v8::Value>,
    names: v8::Local<'s, v8::Array>,
) -> JsResult<'s, ()> {
    let kind = get(scope, req, "kind")?;
    if !is_tiered(scope, kind) {
        let name = get(scope, req, "name")?;
        return push_unique(scope, names, name);
    }
    let iterable = get(scope, req, "tiers")?;
    let tiers = ForOf::open(scope, iterable, "tiers")?;
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = keep_names_tier_iteration(scope, &tiers, names);
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok(())
}

fn keep_names_tier_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    tiers: &ForOf<'s>,
    names: v8::Local<'s, v8::Array>,
) -> JsResult<'s, Flow<'s>> {
    let Some(tier) = tiers.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let body = (|| {
        let name = get(scope, tier, "name")?;
        push_unique(scope, names, name)
    })();
    tiers.body(scope, body)?;
    Ok(Flow::Continue(None))
}
fn push_unique<'s>(
    scope: &mut v8::HandleScope<'s>,
    names: v8::Local<'s, v8::Array>,
    name: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    for index in 0..names.length() {
        let present = cb::catching(scope, |s| names.has_index(s, index))?;
        if present && cb::get_index(scope, names.into(), index)?.strict_equals(name) {
            return Ok(());
        }
    }
    let index = names.length();
    cb::catching(scope, |s| names.set_index(s, index, name))?;
    Ok(())
}

fn missing_labels<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    skill_level: v8::Local<'s, v8::Value>,
    count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let skill_level = Callback::plain(scope, skill_level, "skillLevel");
    let count = Callback::plain(scope, count, "count");
    let labels = v8::Array::new(scope, 0);
    let reqs = ForOf::open(scope, reqs, "reqs")?;
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = missing_label_iteration(scope, &reqs, labels, skill_level, count);
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok(labels.into())
}

fn missing_label_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: &ForOf<'s>,
    labels: v8::Local<'s, v8::Array>,
    skill_level: Callback<'s>,
    count: Callback<'s>,
) -> JsResult<'s, Flow<'s>> {
    let Some(req) = reqs.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let body = (|| {
        if has_tool_req(scope, req, skill_level, count)? {
            return Ok(());
        }
        let kind = get(scope, req, "kind")?;
        let label = if is_tiered(scope, kind) {
            get(scope, req, "label")?
        } else {
            get(scope, req, "name")?
        };
        let index = labels.length();
        cb::catching(scope, |s| labels.set_index(s, index, label))?;
        Ok(())
    })();
    reqs.body(scope, body)?;
    Ok(Flow::Continue(None))
}

fn array_length<'s>(
    scope: &mut v8::HandleScope<'s>,
    array: v8::Local<'s, v8::Value>,
) -> JsResult<'s, u32> {
    let length = get(scope, array, "length")?;
    let length = number(scope, length)?;
    Ok(if length.is_nan() || length <= 0.0 {
        0
    } else {
        length.min(f64::from(u32::MAX)) as u32
    })
}

fn has_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    array: v8::Local<'s, v8::Value>,
    index: u32,
) -> JsResult<'s, bool> {
    cb::catching(scope, |s| array.to_object(s)?.has_index(s, index))
}

fn tool_kit_label<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    skill_level: v8::Local<'s, v8::Value>,
    count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let length = get(scope, reqs, "length")?;
    if length.is_number() && length.number_value(scope) == Some(0.0) {
        return Ok(cb::string(scope, "gear"));
    }
    let map = get(scope, reqs, "map")?;
    if !map.is_function() {
        return Err(cb::type_error(scope, "reqs.map is not a function"));
    }
    let skill_level = Callback::plain(scope, skill_level, "skillLevel");
    let count = Callback::plain(scope, count, "count");
    let length = array_length(scope, reqs)?;
    let labels = v8::Array::new(scope, 0);
    let poll = Poll::default();
    for index in 0..length {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = tool_kit_label_iteration(scope, &poll, reqs, index, labels, skill_level, count);
        cb::iteration(scope, flow)?;
    }
    let separator = cb::string(scope, " + ");
    cb::call_method(scope, labels.into(), "join", &[separator], "join")
}

fn tool_kit_label_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    poll: &Poll,
    reqs: v8::Local<'s, v8::Value>,
    index: u32,
    labels: v8::Local<'s, v8::Array>,
    skill_level: Callback<'s>,
    count: Callback<'s>,
) -> JsResult<'s, Flow<'s>> {
    poll.check(scope)?;
    if !has_index(scope, reqs, index)? {
        return Ok(Flow::Continue(None));
    }
    let req = cb::get_index(scope, reqs, index)?;
    let kind = get(scope, req, "kind")?;
    let label = if is_tiered(scope, kind) {
        let skill = get(scope, req, "skill")?;
        let level = skill_level.call(scope, &[skill])?;
        let held = match tiered_tool_kind(scope, skill) {
            Some(kind) => native_best_tool(scope, kind, level, TierAvailability::Positive(count))?,
            None => None,
        };
        match held {
            Some(name) => cb::string(scope, name),
            None => {
                let label = get(scope, req, "label")?;
                let label = cb::to_string(scope, label)?;
                cb::string(scope, &format!("{label} (bronze→rune)"))
            }
        }
    } else {
        get(scope, req, "name")?
    };
    cb::catching(scope, |s| labels.set_index(s, index, label))?;
    Ok(Flow::Continue(None))
}

/// `toolRestockPlan(reqs, skillLevel, invCount, bankCount)` preserves the
/// frozen per-row callback order for exact and tiered requirements.
fn restock_plan<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    skill_level: v8::Local<'s, v8::Value>,
    inv_count: v8::Local<'s, v8::Value>,
    bank_count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let (plan, _) = restock_plan_with_better(scope, reqs, skill_level, inv_count, bank_count)?;
    Ok(plan.into())
}

fn restock_plan_with_better<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    skill_level: v8::Local<'s, v8::Value>,
    inv_count: v8::Local<'s, v8::Value>,
    bank_count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, (v8::Local<'s, v8::Array>, bool)> {
    let skill_level = Callback::plain(scope, skill_level, "skillLevel");
    let inv_count = Callback::plain(scope, inv_count, "invCount");
    let bank_count = Callback::plain(scope, bank_count, "bankCount");
    let plan = v8::Array::new(scope, 0);
    let rows = ForOf::open(scope, reqs, "reqs")?;
    let mut better = false;
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = restock_iteration(
            scope,
            &rows,
            plan,
            skill_level,
            inv_count,
            bank_count,
            &mut better,
        );
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok((plan, better))
}

fn restock_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &ForOf<'s>,
    plan: v8::Local<'s, v8::Array>,
    skill_level: Callback<'s>,
    inv_count: Callback<'s>,
    bank_count: Callback<'s>,
    better: &mut bool,
) -> JsResult<'s, Flow<'s>> {
    let Some(req) = rows.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let row = restock_row(scope, req, skill_level, inv_count, bank_count);
    let (is_tiered, step) = rows.body(scope, row)?;
    if let Some(step) = step {
        *better |= is_tiered;
        let len = plan.length();
        cb::catching(scope, |s| plan.set_index(s, len, step))?;
    }
    Ok(Flow::Continue(None))
}

fn restock_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    req: v8::Local<'s, v8::Value>,
    skill_level: Callback<'s>,
    inv_count: Callback<'s>,
    bank_count: Callback<'s>,
) -> JsResult<'s, (bool, Option<v8::Local<'s, v8::Value>>)> {
    let kind = get(scope, req, "kind")?;
    if is_tiered(scope, kind) {
        return tiered_restock_row(scope, req, skill_level, inv_count, bank_count)
            .map(|step| (true, step));
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
    let zero = cb::num(scope, 0.0);
    if cb::le(scope, need, zero)? {
        return Ok((false, None));
    }
    let name = get(scope, req, "name")?;
    let available = bank_count.call(scope, &[name])?;
    if cb::le(scope, available, zero)? {
        return Ok((false, None));
    }
    let name = get(scope, req, "name")?;
    // `Math.min(need, available)`: ToNumber of each argument, in order.
    let need = number(scope, need)?;
    let available = number(scope, available)?;
    let qty = if need.is_nan() || available.is_nan() {
        f64::NAN
    } else {
        need.min(available)
    };
    let qty = cb::num(scope, qty);
    let equip = get(scope, req, "equip")?;
    let equip = v8::Boolean::new(scope, equip.is_true()).into();
    Ok((
        false,
        Some(cb::object(
            scope,
            &[("name", name), ("qty", qty), ("equip", equip)],
        )?),
    ))
}

fn tiered_restock_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    req: v8::Local<'s, v8::Value>,
    skill_level: Callback<'s>,
    inv_count: Callback<'s>,
    bank_count: Callback<'s>,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let skill = get(scope, req, "skill")?;
    let level = skill_level.call(scope, &[skill])?;
    let Some(kind) = tiered_tool_kind(scope, skill) else {
        return Ok(None);
    };
    let Some(name) = native_best_tool(
        scope,
        kind,
        level,
        TierAvailability::InventoryOrBank {
            inventory: inv_count,
            bank: bank_count,
        },
    )?
    else {
        return Ok(None);
    };
    let name = cb::string(scope, name);
    let zero = cb::num(scope, 0.0);
    let have = inv_count.call(scope, &[name])?;
    if cb::gt(scope, have, zero)? {
        return Ok(None);
    }
    let available = bank_count.call(scope, &[name])?;
    if cb::le(scope, available, zero)? {
        return Ok(None);
    }
    let qty = cb::num(scope, 1.0);
    let equip = get(scope, req, "equip")?;
    let equip = v8::Boolean::new(scope, equip.is_true()).into();
    Ok(Some(cb::object(
        scope,
        &[("name", name), ("qty", qty), ("equip", equip)],
    )?))
}
fn bank_has_better_tool<'s>(
    scope: &mut v8::HandleScope<'s>,
    reqs: v8::Local<'s, v8::Value>,
    skill_level: v8::Local<'s, v8::Value>,
    inv_count: v8::Local<'s, v8::Value>,
    bank_count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let (_, better) = restock_plan_with_better(scope, reqs, skill_level, inv_count, bank_count)?;
    Ok(v8::Boolean::new(scope, better).into())
}

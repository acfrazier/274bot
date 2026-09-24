//! v1 `boostPotions` (frozen `bot/api/combat/boostPotions.ts`) as one native
//! call per export: `__rs2b0t_boost_potions(op, ...args)`.
//!
//! The descriptor table, floor and dose forms are [`crate::boost_potions`].
//! `plannedPotions` walks the caller's carry list and `potionToSip` the
//! caller's plans, calling `s.levels` then `s.held` (receiver `s`) exactly
//! where the frozen loop does and returning the caller's own objects.

use super::callback_v8::{
    self as cb, call, call_method, get, not_impl, number, Callback, Flow, ForOf, JsResult,
};
use crate::boost_potions::{
    boost_faded_by, BoostPotion, FadedOperand, BOOST_FLOOR, BOOST_POTIONS, DEFAULT_WANT,
    EMPTY_VIAL,
};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_boost_potions", boost_potions)
}

fn boost_potions<'s>(
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
        "table" => table(scope),
        "boostFaded" => {
            let faded = faded(scope, args.get(1), args.get(2), args.get(3))?;
            Ok(v8::Boolean::new(scope, faded).into())
        }
        "plannedPotions" => planned_potions(scope, args.get(1), args.get(2)),
        "potionToSip" => potion_to_sip(scope, args.get(1)),
        _ => Err(not_impl(scope, "boostPotions")),
    }
}

fn strings<'s>(scope: &mut v8::HandleScope<'s>, items: &[String]) -> v8::Local<'s, v8::Value> {
    let values: Vec<_> = items.iter().map(|item| cb::string(scope, item)).collect();
    v8::Array::new_with_elements(scope, &values).into()
}

/// The exported descriptor rows, the floor and the drained flask name.
fn table<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let mut rows = Vec::with_capacity(BOOST_POTIONS.len());
    for potion in BOOST_POTIONS {
        let skill = cb::string(scope, potion.skill);
        let short = cb::string(scope, potion.short);
        let flask = cb::string(scope, &potion.flask());
        let doses = strings(scope, &potion.doses());
        rows.push(cb::object(
            scope,
            &[("skill", skill), ("short", short), ("flask", flask), ("doses", doses)],
        )?);
    }
    let potions = v8::Array::new_with_elements(scope, &rows).into();
    let floor = cb::num(scope, BOOST_FLOOR);
    let vial = cb::string(scope, EMPTY_VIAL);
    cb::object(
        scope,
        &[("potions", potions), ("floor", floor), ("empty_vial", vial)],
    )
}

/// Frozen `boostFaded(base, effective, floor = BOOST_FLOOR)`.
///
/// Every operand position in the frozen expression (`-`, `>`, `>=`, `<=`,
/// `*`) is numeric against a number, so each read is `ToNumber` of the caller's
/// value, in the frozen order ([`boost_faded_by`]); objects and numeric strings
/// convert exactly as there. Scope: a BigInt level throws `Cannot convert a
/// BigInt value to a number` where the frozen arithmetic throws `Cannot mix
/// BigInt and other types` (or, for all-BigInt `base <= 0`, returns `false`).
fn faded<'s>(
    scope: &mut v8::HandleScope<'s>,
    base: v8::Local<'s, v8::Value>,
    effective: v8::Local<'s, v8::Value>,
    floor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    boost_faded_by(scope, |scope, operand| match operand {
        FadedOperand::Base => number(scope, base),
        FadedOperand::Effective => number(scope, effective),
        FadedOperand::Floor if floor.is_undefined() => Ok(BOOST_FLOOR),
        FadedOperand::Floor => number(scope, floor),
    })
}

/// Frozen `plannedPotions(carry)`: `BOOST_POTIONS.map(potion => planFor(potion, carry))`.
/// `descriptors` is the shim's exported `BOOST_POTIONS`, so each plan carries
/// the exported descriptor object itself.
fn planned_potions<'s>(
    scope: &mut v8::HandleScope<'s>,
    carry: v8::Local<'s, v8::Value>,
    descriptors: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let mut plans = Vec::with_capacity(BOOST_POTIONS.len());
    for (index, potion) in BOOST_POTIONS.iter().enumerate() {
        let descriptor = cb::get_index(scope, descriptors, index as u32)?;
        plans.push(plan_for(scope, potion, descriptor, carry)?);
    }
    Ok(v8::Array::new_with_elements(scope, &plans).into())
}

/// Frozen `planFor(potion, carry)`: the first carried entry one of whose dose
/// forms equals `entry.item.trim().toLowerCase()` — the item is re-read per
/// dose comparison, as the frozen `find` callback does — else the table flask.
fn plan_for<'s>(
    scope: &mut v8::HandleScope<'s>,
    potion: &BoostPotion,
    descriptor: v8::Local<'s, v8::Value>,
    carry: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let entries = ForOf::open(scope, carry, "carry")?;
    let planned = loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = plan_iteration(scope, &entries, potion, descriptor);
        if let Flow::Break(planned) = cb::iteration(scope, flow)? {
            break planned;
        }
    };
    if let Some(planned) = planned {
        return Ok(planned);
    }
    let flask = cb::string(scope, &potion.flask());
    let want = cb::num(scope, DEFAULT_WANT as f64);
    cb::object(
        scope,
        &[("potion", descriptor), ("flask", flask), ("want", want)],
    )
}

/// One carried entry: the plan on a dose match, else on to the next entry.
fn plan_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    entries: &ForOf<'s>,
    potion: &BoostPotion,
    descriptor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Flow<'s>> {
    let Some(entry) = entries.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let matched = matching_dose(scope, potion, entry);
    let Some(flask) = entries.body(scope, matched)? else {
        return Ok(Flow::Continue(None));
    };
    let want = get(scope, entry, "qty");
    let want = entries.body(scope, want)?;
    entries.close(scope)?;
    let flask = cb::string(scope, &flask);
    let plan = cb::object(
        scope,
        &[("potion", descriptor), ("flask", flask), ("want", want)],
    )?;
    Ok(Flow::Break(Some(plan)))
}

fn matching_dose<'s>(
    scope: &mut v8::HandleScope<'s>,
    potion: &BoostPotion,
    entry: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<String>> {
    for dose in potion.doses() {
        let item = get(scope, entry, "item")?;
        let trimmed = call_method(scope, item, "trim", &[], "entry.item.trim")?;
        let key = call_method(scope, trimmed, "toLowerCase", &[], "entry.item.trim(...).toLowerCase")?;
        let wanted = cb::string(scope, &dose.to_lowercase());
        if wanted.strict_equals(key) {
            return Ok(Some(dose));
        }
    }
    Ok(None)
}

/// Frozen `potionToSip(s)`:
///
/// ```text
/// for (const plan of s.plans) {
///     const { base, effective } = s.levels(plan.potion.skill);
///     if (s.held(plan) > 0 && boostFaded(base, effective)) return plan;
/// }
/// return null;
/// ```
fn potion_to_sip<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let plans = get(scope, state, "plans")?;
    let plans = ForOf::open(scope, plans, "s.plans")?;
    let chosen = loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = sip_iteration(scope, &plans, state);
        if let Flow::Break(chosen) = cb::iteration(scope, flow)? {
            break chosen;
        }
    };
    Ok(chosen.unwrap_or_else(|| v8::null(scope).into()))
}

fn sip_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    plans: &ForOf<'s>,
    state: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Flow<'s>> {
    let Some(plan) = plans.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let due = sip_due(scope, state, plan);
    if plans.body(scope, due)? {
        plans.close(scope)?;
        return Ok(Flow::Break(Some(plan)));
    }
    Ok(Flow::Continue(None))
}

fn sip_due<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: v8::Local<'s, v8::Value>,
    plan: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let levels = get(scope, state, "levels")?;
    let potion = get(scope, plan, "potion")?;
    let skill = get(scope, potion, "skill")?;
    let observed = Callback::method(levels, state, "s.levels").call(scope, &[skill])?;
    let (base, effective) = destructure_levels(scope, observed)?;
    let held = get(scope, state, "held")?;
    let held = call(scope, held, state, &[plan], "s.held")?;
    let zero = cb::num(scope, 0.0);
    if !cb::gt(scope, held, zero)? {
        return Ok(false);
    }
    let floor = v8::undefined(scope).into();
    faded(scope, base, effective, floor)
}

/// `const { base, effective } = s.levels(...)`, with the engine's message
/// for a nullish answer.
fn destructure_levels<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, (v8::Local<'s, v8::Value>, v8::Local<'s, v8::Value>)> {
    if value.is_null_or_undefined() {
        let shown = if value.is_null() { "null" } else { "undefined" };
        let message =
            format!("Cannot destructure property 'base' of 's.levels(...)' as it is {shown}.");
        return Err(cb::type_error(scope, &message));
    }
    let base = get(scope, value, "base")?;
    let effective = get(scope, value, "effective")?;
    Ok((base, effective))
}

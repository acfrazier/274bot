//! v1 `CombatStyleLogic` rune helpers (frozen
//! `bot/api/combat/CombatStyleLogic.ts`) as one native call each:
//! `__rs2b0t_combat_style(op, spellName, wielded, heldOrCasts)`.
//!
//! The remaining per-cast costs are `SelectedGameData::runes_per_cast`; the
//! shim passes the spell name and wielded names already coerced to strings.
//!
//! ```text
//! castsAvailable:   costs ? (costs.length ? Math.min(...costs.map(c => Math.floor(held(c.rune) / c.count))) : Infinity) : 0
//! runeWithdrawList: (costs ?? []).map(c => ({ rune: c.rune, count: c.count * casts }))
//! ```

use super::callback_v8::{self as cb, not_impl, number, Callback, JsResult};
use api::game_data::RemainingRuneCost;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_combat_style", combat_style)
}

fn combat_style<'s>(
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
    let costs = remaining_costs(scope, args.get(1), args.get(2))?;
    match op.as_str() {
        "castsAvailable" => casts_available(scope, costs, args.get(3)),
        "runeWithdrawList" => rune_withdraw_list(scope, costs, args.get(3)),
        _ => Err(not_impl(scope, "CombatStyleLogic")),
    }
}

fn remaining_costs<'s>(
    scope: &mut v8::HandleScope<'s>,
    spell: v8::Local<'s, v8::Value>,
    wielded: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<Vec<RemainingRuneCost>>> {
    let spell = cb::to_string(scope, spell)?;
    let mut names = Vec::new();
    if let Ok(list) = v8::Local::<v8::Array>::try_from(wielded) {
        for index in 0..list.length() {
            let item = cb::get_index(scope, wielded, index)?;
            names.push(cb::to_string(scope, item)?);
        }
    }
    Ok(crate::supply_v2::selected_data().and_then(|data| data.runes_per_cast(&spell, &names)))
}

fn casts_available<'s>(
    scope: &mut v8::HandleScope<'s>,
    costs: Option<Vec<RemainingRuneCost>>,
    held: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Some(costs) = costs else {
        return Ok(cb::num(scope, 0.0));
    };
    let held = Callback::plain(scope, held, "held");
    // `Math.min(...costs.map(...))`: every cost's `held` runs before the minimum.
    let mut least = f64::INFINITY;
    for cost in &costs {
        let rune = cb::string(scope, &cost.rune);
        let have = held.call(scope, &[rune])?;
        let per_cast = cb::num(scope, f64::from(cost.count));
        let casts = cb::div(scope, have, per_cast)?;
        // `Math.floor` is `ToNumber` then floor.
        let casts = number(scope, casts)?.floor();
        least = if casts.is_nan() || least.is_nan() {
            f64::NAN
        } else {
            least.min(casts)
        };
    }
    Ok(cb::num(scope, least))
}

fn rune_withdraw_list<'s>(
    scope: &mut v8::HandleScope<'s>,
    costs: Option<Vec<RemainingRuneCost>>,
    casts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let mut rows = Vec::new();
    for cost in costs.unwrap_or_default() {
        let per_cast = cb::num(scope, f64::from(cost.count));
        let count = cb::mul(scope, per_cast, casts)?;
        let rune = cb::string(scope, &cost.rune);
        rows.push(cb::object(scope, &[("rune", rune), ("count", count)])?);
    }
    Ok(v8::Array::new_with_elements(scope, &rows).into())
}

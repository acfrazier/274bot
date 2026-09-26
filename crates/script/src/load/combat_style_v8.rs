//! v1 `CombatStyleLogic` rune helpers (frozen
//! `bot/api/combat/CombatStyleLogic.ts`) as one native call each:
//! `__rs2b0t_combat_style(op, spellName, wielded, heldOrCasts)`; and the
//! frozen `CombatStyle` tables: `__rs2b0t_describe_combat_style(resolution)`,
//! `__rs2b0t_parse_combat_style` / `__rs2b0t_try_parse_combat_style(name)`
//! and `__rs2b0t_parse_range_style(name)`; and the frozen `AttackClock`
//! ([`crate::attack_clock`]): `__rs2b0t_attack_clock(op, slot, tick)`.
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
    cb::install(runtime, "__rs2b0t_combat_style", combat_style)?;
    cb::install(
        runtime,
        "__rs2b0t_describe_combat_style",
        describe_combat_style,
    )?;
    cb::install(runtime, "__rs2b0t_parse_combat_style", parse_combat_style)?;
    cb::install(
        runtime,
        "__rs2b0t_try_parse_combat_style",
        try_parse_combat_style,
    )?;
    cb::install(runtime, "__rs2b0t_parse_range_style", parse_range_style)?;
    cb::install(runtime, "__rs2b0t_attack_clock", attack_clock)
}

/// `swingStartedThisTick` (`fightUpkeep.ts:16-19`) and the `AttackClock`
/// instance methods (`eatTiming.ts:27-51`) by `op`: `fight`, `new`,
/// `observe`, `attacked`, `reset`. A tick is compared with `===`, so a
/// non-number never matches.
fn attack_clock<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    use crate::attack_clock as clock;
    let result = (|| {
        let op = args.get(0).to_rust_string_lossy(scope);
        let tick = args.get(2);
        let tick = if tick.is_number() {
            tick.number_value(scope).unwrap_or(f64::NAN)
        } else {
            f64::NAN
        };
        let slot = |scope: &mut v8::HandleScope<'s>| {
            let slot = number(scope, args.get(1))?;
            if slot.is_finite() && slot >= 0.0 && slot.fract() == 0.0 {
                Ok(slot as usize)
            } else {
                Err(not_impl(scope, "AttackClock"))
            }
        };
        let known = |scope: &mut v8::HandleScope<'s>, found: Option<()>| {
            found.ok_or_else(|| not_impl(scope, "AttackClock"))
        };
        Ok(match op.as_str() {
            "fight" => v8::Boolean::new(scope, clock::swing_started_this_tick()).into(),
            "new" => cb::num(scope, clock::clock_new() as f64),
            "observe" => {
                let slot = slot(scope)?;
                known(scope, clock::clock_observe(slot, tick))?;
                v8::undefined(scope).into()
            }
            "attacked" => {
                let slot = slot(scope)?;
                let attacked = clock::clock_attacked(slot, tick);
                known(scope, attacked.map(|_| ()))?;
                v8::Boolean::new(scope, attacked == Some(true)).into()
            }
            "reset" => {
                let slot = slot(scope)?;
                known(scope, clock::clock_reset(slot))?;
                v8::undefined(scope).into()
            }
            _ => return Err(not_impl(scope, "AttackClock")),
        })
    })();
    cb::finish(scope, rv, result);
}

/// Frozen `COMBAT_STYLE` (`api/combat/CombatStyle.ts:3-13`): the melee style a
/// trimmed, lowercased token names.
pub(super) fn melee_style(name: &str) -> Option<&'static str> {
    match name.trim().to_lowercase().as_str() {
        "attack" | "accurate" => Some("attack"),
        "strength" | "aggressive" => Some("strength"),
        "controlled" | "shared" => Some("controlled"),
        "defence" | "defense" | "defensive" => Some("defence"),
        _ => None,
    }
}

/// Frozen `RANGE_STYLE_MODE` (`CombatStyle.ts:171-177`).
fn range_mode(name: &str) -> Option<f64> {
    match name.trim().to_lowercase().as_str() {
        "accurate" => Some(0.0),
        "rapid" => Some(1.0),
        "longrange" | "long range" | "long-range" => Some(2.0),
        _ => None,
    }
}

/// Frozen `parseCombatStyle` (`CombatStyle.ts:39-41`): unknown is `strength`.
fn parse_combat_style<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = cb::to_string(scope, args.get(0))
        .map(|name| cb::string(scope, melee_style(&name).unwrap_or("strength")));
    cb::finish(scope, rv, result);
}

/// Frozen `tryParseCombatStyle` (`CombatStyle.ts:47-49`): unknown is `null`.
fn try_parse_combat_style<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = cb::to_string(scope, args.get(0)).map(|name| match melee_style(&name) {
        Some(style) => cb::string(scope, style),
        None => v8::null(scope).into(),
    });
    cb::finish(scope, rv, result);
}

/// Frozen `parseRangeStyle` (`CombatStyle.ts:181-183`): unknown is mode 1.
fn parse_range_style<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = cb::to_string(scope, args.get(0))
        .map(|name| cb::num(scope, range_mode(&name).unwrap_or(1.0)));
    cb::finish(scope, rv, result);
}

/// Frozen `describeCombatStyle(resolution)` (`api/combat/CombatStyle.ts:148-169`):
/// the trained skills of the style the weapon actually offers (`effective`,
/// `:150-163`), with `; <requested> unavailable` when the requested style
/// fell back (`:165-167`).
fn describe_combat_style<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let resolution = args.get(0);
        let effective = cb::get(scope, resolution, "effective")?;
        let description = effective
            .is_string()
            .then(|| effective.to_rust_string_lossy(scope))
            .and_then(|effective| match effective.as_str() {
                "attack" => Some("attack (training Attack)"),
                "strength" => Some("strength (training Strength)"),
                "controlled" => Some("controlled (training Attack, Strength & Defence)"),
                "defence" => Some("defence (training Defence)"),
                _ => None,
            });
        let requested = cb::get(scope, resolution, "requested")?;
        if requested.strict_equals(effective) {
            return Ok(match description {
                Some(description) => cb::string(scope, description),
                None => v8::undefined(scope).into(),
            });
        }
        let Some(description) = description else {
            return Err(cb::type_error(
                scope,
                "Cannot read properties of undefined (reading 'slice')",
            ));
        };
        let requested = cb::to_string(scope, requested)?;
        let open = &description[..description.len() - 1];
        Ok(cb::string(
            scope,
            &format!("{open}; {requested} unavailable)"),
        ))
    })();
    cb::finish(scope, rv, result);
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

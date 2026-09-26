//! Bounded catalog/air queries. Foreign predicates are called only by the
//! Rust loop; no route search or selection policy runs in JavaScript.
use super::callback_v8;
use crate::bank_select::{self, FromTile};
use crate::content::{CookStand, COOK_STANDS};
use api::snapshot::WorldTile;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;
use serde::Deserialize;

/// Frozen `CUSTOM_LOCATION` (`data/cookLocations.ts:14`), lowercased.
const CUSTOM_LOCATION: &str = "custom";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_bank_locations", catalog)?;
    callback_v8::install(runtime, "__rs2b0t_bank_unlocked", unlocked)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_bank", nearest)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_banks", ranked)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_usable_bank", usable)?;
    callback_v8::install(runtime, "__rs2b0t_resolve_cook_location", resolve_cook)
}

fn origin<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, WorldTile> {
    let from = serde_v8::from_v8::<FromTile>(scope, value)
        .map_err(|e| callback_v8::type_error(scope, &format!("bank origin: {e}")))?;
    Ok(WorldTile {
        x: from.x,
        z: from.z,
        level: from.level,
    })
}

fn catalog<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let values: Vec<_> = bank_select::banks()
        .iter()
        .map(bank_select::bank_value)
        .collect();
    let result = serde_v8::to_v8(scope, values)
        .map_err(|e| callback_v8::type_error(scope, &format!("bank catalog: {e}")));
    callback_v8::finish(scope, rv, result);
}

fn unlocked<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    #[derive(Deserialize)]
    struct Identity {
        name: String,
        x: i32,
        z: i32,
        #[serde(default)]
        level: i32,
    }
    let result = (|| {
        let bank = serde_v8::from_v8::<Identity>(scope, args.get(0))
            .map_err(|e| callback_v8::type_error(scope, &format!("bank identity: {e}")))?;
        Ok(v8::Boolean::new(
            scope,
            bank_select::unlocked(
                &bank.name,
                WorldTile {
                    x: bank.x,
                    z: bank.z,
                    level: bank.level,
                },
            ),
        )
        .into())
    })();
    callback_v8::finish(scope, rv, result);
}

fn nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        serde_v8::to_v8(scope, bank_select::nearest(from))
            .map_err(|e| callback_v8::type_error(scope, &format!("bank result: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

fn ranked<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        let values: Vec<_> = bank_select::ranked(from)
            .iter()
            .map(bank_select::bank_value)
            .collect();
        serde_v8::to_v8(scope, values)
            .map_err(|e| callback_v8::type_error(scope, &format!("bank results: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

fn usable<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        let predicate = callback_v8::Callback::plain(scope, args.get(1), "usable");
        let mut best = v8::null(scope).into();
        let mut distance = i64::MAX;
        for bank in bank_select::banks() {
            let value = serde_v8::to_v8(scope, bank_select::bank_value(&bank))
                .map_err(|e| callback_v8::type_error(scope, &format!("bank candidate: {e}")))?;
            let accepted = predicate.call(scope, &[value])?;
            if accepted.boolean_value(scope) {
                let candidate = bank_select::air_distance_squared(from, bank.air_tile());
                if candidate < distance {
                    distance = candidate;
                    best = value;
                }
            }
        }
        Ok(best)
    })();
    callback_v8::finish(scope, rv, result);
}

/// Frozen `resolveCookLocation(setting, from, unlocked?)`
/// (`api/cooking/CookLocations.ts:25-49`) over the host cook stands: blank
/// and `Custom` are `null` (`:30-33`); a named location only when unlocked
/// (`:34-37`); `Auto` the unlocked location whose bank approach is nearest
/// `from` in a straight x/z line, first on ties (`:38-48`, `bankDistance`
/// `geometry/distance.ts:7-11`, `approachOf` `BankLocations.ts:115-117`).
/// `unlocked` defaults to the bank's own requirement (`:28`); a caller's
/// predicate is called with the location's name, which the shim maps to its
/// `CookLocation`. Answers the chosen location's name or `null`.
fn resolve_cook<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let wanted = callback_v8::to_string(scope, args.get(0))?
            .trim()
            .to_lowercase();
        if wanted.is_empty() || wanted == CUSTOM_LOCATION {
            return Ok(None);
        }
        let predicate = args.get(2);
        let predicate = (!predicate.is_undefined())
            .then(|| callback_v8::Callback::plain(scope, predicate, "unlocked"));
        let unlocked = |scope: &mut v8::HandleScope<'s>, stand: &CookStand| match &predicate {
            Some(predicate) => {
                let name = callback_v8::string(scope, stand.name);
                predicate.truthy(scope, &[name])
            }
            None => Ok(bank_select::unlocked(stand.name, stand.bank)),
        };
        if wanted != "auto" {
            let Some(stand) = COOK_STANDS
                .iter()
                .find(|stand| stand.name.to_lowercase() == wanted)
            else {
                return Ok(None);
            };
            return Ok(unlocked(scope, stand)?.then_some(stand.name));
        }
        let from = origin(scope, args.get(1))?;
        let mut best: Option<(&CookStand, i64)> = None;
        for stand in COOK_STANDS {
            if !unlocked(scope, stand)? {
                continue;
            }
            let distance = bank_select::air_distance_squared(
                from,
                bank_select::approach_of(stand.name, stand.bank),
            );
            if best.is_none_or(|(_, best)| distance < best) {
                best = Some((stand, distance));
            }
        }
        Ok(best.map(|(stand, _)| stand.name))
    })()
    .map(|name| match name {
        Some(name) => callback_v8::string(scope, name),
        None => v8::null(scope).into(),
    });
    callback_v8::finish(scope, rv, result);
}

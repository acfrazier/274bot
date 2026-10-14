//! Bounded catalog/air queries. Foreign predicates are called only by the
//! Rust loop; no route search or selection policy runs in JavaScript.
use super::callback_v8;
use crate::bank_select::{self, FromTile};
use crate::cook_locations::{self, CookLocation};
use api::snapshot::WorldTile;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;
use serde::Deserialize;
use serde_json::json;

const NEARBY_BANK_RADIUS: f64 = 14.0;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRouteInput {
    #[serde(default)]
    bank_open: Option<bool>,
    #[serde(default)]
    here: Option<FromTile>,
    #[serde(default)]
    stand: Option<FromTile>,
    #[serde(default)]
    nearby_booth_dist: Option<f64>,
    #[serde(default)]
    nearest: Option<OpenRouteBank>,
    #[serde(default)]
    prefer_nearby: Option<bool>,
    #[serde(default)]
    nearby_radius: Option<f64>,
}

#[derive(Deserialize)]
struct OpenRouteBank {
    tile: FromTile,
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_bank_locations", catalog)?;
    callback_v8::install(runtime, "__rs2b0t_bank_unlocked", unlocked)?;
    callback_v8::install(runtime, "__rs2b0t_locked_bank_at", locked_at)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_bank", nearest)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_banks", ranked)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_usable_bank", usable)?;
    callback_v8::install(runtime, "__rs2b0t_bank_distance", bank_distance)?;
    callback_v8::install(
        runtime,
        "__rs2b0t_resolve_bank_open_route",
        resolve_bank_open_route,
    )?;
    callback_v8::install(runtime, "__rs2b0t_cook_locations", cook_catalog)?;
    callback_v8::install(
        runtime,
        "__rs2b0t_nearest_cook_surface",
        nearest_cook_surface,
    )?;
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

fn locked_at<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let tile = origin(scope, args.get(0))?;
        serde_v8::to_v8(scope, bank_select::locked_bank_at(tile))
            .map_err(|e| callback_v8::type_error(scope, &format!("locked bank result: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

/// Frozen `bankDistance` and pure `resolveBankOpenRoute` share Euclidean x/z
/// distance, independent of plane.
fn bank_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        let bank = origin(scope, args.get(1))?;
        Ok(v8::Number::new(scope, bank_distance_value(&from, &bank)).into())
    })();
    callback_v8::finish(scope, rv, result);
}

fn bank_distance_value(from: &WorldTile, bank: &WorldTile) -> f64 {
    let dx = f64::from(bank.x) - f64::from(from.x);
    let dz = f64::from(bank.z) - f64::from(from.z);
    dx.hypot(dz)
}

fn from_tile(tile: &FromTile) -> WorldTile {
    WorldTile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn resolve_bank_open_route<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let input = args.get(0);
        if input.is_string() && input.to_rust_string_lossy(scope) == "nearbyRadius" {
            return Ok(v8::Number::new(scope, NEARBY_BANK_RADIUS).into());
        }
        let input = serde_v8::from_v8::<OpenRouteInput>(scope, input)
            .map_err(|e| callback_v8::type_error(scope, &format!("bank route input: {e}")))?;
        serde_v8::to_v8(scope, select_open_route(&input))
            .map_err(|e| callback_v8::type_error(scope, &format!("bank route result: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

fn select_open_route(input: &OpenRouteInput) -> &'static str {
    if input.bank_open.unwrap_or(false) {
        return "already-open";
    }
    let prefer_nearby = input.prefer_nearby != Some(false);
    let radius = input.nearby_radius.unwrap_or(NEARBY_BANK_RADIUS);
    if prefer_nearby
        && input
            .nearby_booth_dist
            .is_some_and(|distance| distance <= radius)
    {
        return "scene-booth";
    }
    if prefer_nearby {
        if let (Some(here), Some(nearest)) = (input.here.as_ref(), input.nearest.as_ref()) {
            let here = from_tile(here);
            if bank_distance_value(&here, &from_tile(&nearest.tile)) <= radius {
                let preset_is_far = match input.stand.as_ref() {
                    Some(stand) => bank_distance_value(&here, &from_tile(stand)) > radius,
                    None => true,
                };
                if preset_is_far {
                    return "local-bank";
                }
            }
        }
    }
    if input.stand.is_some() {
        "preset-stand"
    } else {
        "nearest-fallback"
    }
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

/// The bank roster paired with its cook surfaces; `None` without selected
/// game data.
fn cook_roster() -> Option<(Vec<api::named_banks::NamedBank>, Vec<CookLocation>)> {
    let data = crate::supply_v2::selected_data()?;
    let banks = bank_select::banks();
    let locations = cook_locations::build(&banks, data.cook_surfaces());
    Some((banks, locations))
}

/// Frozen `COOK_LOCATIONS` (`api/cooking/CookLocations.ts:12`): one row per
/// roster bank, `bank` its index in `BANK_LOCATIONS`. Every shim module
/// evaluates at load, so without game data the table is empty (like
/// `ITEM_DB`) and `resolveCookLocation` reports the missing data.
fn cook_catalog<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let locations = cook_roster().map_or_else(Vec::new, |(_, locations)| locations);
    let tile = |tile: WorldTile| json!({"x": tile.x, "z": tile.z, "level": tile.level});
    let rows: Vec<_> = locations
        .iter()
        .map(|location| {
            let surface = location.surface.as_ref().map(|plan| {
                let mut surface = json!({
                    "stand": tile(plan.stand),
                    "locName": plan.loc_name,
                    "kind": plan.kind.as_str(),
                    "loc": tile(plan.loc),
                    "arriveRadius": plan.arrive_radius,
                    "label": plan.label,
                });
                if let Some(approach) = plan.approach {
                    surface["approach"] = tile(approach);
                }
                surface
            });
            json!({
                "bank": location.bank,
                "name": location.name,
                "surface": surface,
                "obstacles": api::cook_locations::DEFAULT_OBSTACLES,
                "verified": location.verified,
            })
        })
        .collect();
    let result = serde_v8::to_v8(scope, rows)
        .map_err(|e| callback_v8::type_error(scope, &format!("cook locations: {e}")));
    callback_v8::finish(scope, rv, result);
}

fn nearest_cook_surface<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let Some(data) = crate::supply_v2::selected_data() else {
            return Ok(v8::null(scope).into());
        };
        let surfaces = data.cook_surfaces();
        if surfaces.is_empty() {
            return Ok(v8::null(scope).into());
        }
        let origin = origin(scope, args.get(0))?;
        let max_cheb = args.get(1);
        let max_cheb = if max_cheb.is_undefined() {
            f64::from(api::cook_locations::MAX_SURFACE_CHEB)
        } else {
            callback_v8::number(scope, max_cheb)?
        };
        let Some(surface) = cook_locations::nearest_surface(origin, surfaces, max_cheb) else {
            return Ok(v8::null(scope).into());
        };
        let value = json!({
            "x": surface.x,
            "z": surface.z,
            "level": surface.level,
            "name": surface.name,
            "debugname": surface.debugname,
            "kind": surface.kind.as_str(),
        });
        serde_v8::to_v8(scope, value)
            .map_err(|e| callback_v8::type_error(scope, &format!("cook surface: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

/// Frozen `resolveCookLocation(setting, from, unlocked?)`
/// (`api/cooking/CookLocations.ts:25-49`, [`cook_locations::resolve`]).
/// `unlocked` defaults to the bank's own requirement (`:28`); a caller's
/// predicate is called with the location's index, which the shim maps to
/// its `CookLocation`. Answers that index or `null`.
fn resolve_cook<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let setting = callback_v8::to_string(scope, args.get(0))?;
        let Some(wanted) = cook_locations::wanted(&setting) else {
            return Ok(None);
        };
        let Some((banks, locations)) = cook_roster() else {
            let message = v8::String::new(scope, crate::supply_v2::GAME_DATA_UNAVAILABLE)
                .unwrap_or_else(|| v8::String::empty(scope));
            return Err(callback_v8::Throw::Value(v8::Exception::error(
                scope, message,
            )));
        };
        let predicate = args.get(2);
        let predicate = (!predicate.is_undefined())
            .then(|| callback_v8::Callback::plain(scope, predicate, "unlocked"));
        cook_locations::resolve(
            &locations,
            &banks,
            wanted,
            scope,
            |scope| origin(scope, args.get(1)),
            |scope, index| match &predicate {
                Some(predicate) => {
                    let index = callback_v8::num(scope, index as f64);
                    predicate.truthy(scope, &[index])
                }
                None => Ok(bank_select::bank_unlocked(&banks[locations[index].bank])),
            },
        )
    })()
    .map(|index| match index {
        Some(index) => callback_v8::num(scope, index as f64),
        None => v8::null(scope).into(),
    });
    callback_v8::finish(scope, rv, result);
}

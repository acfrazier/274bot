//! Runtime binding registration and bootstrap wrappers (feature `load` only).
//!
//! Owns capability `register_function` order, shim prelude/content eval, and
//! the native/compat main wrappers. Called from the isolate thread (and the
//! throwaway validate Runtime on Start) — not a separate state machine.

use rustyscript::Runtime;
use std::sync::OnceLock;
use std::time::Instant;

use super::shape::LoadShape;

/// Monotonic clock backing the prelude's `performance.now()` shim
/// (rustyscript's default extensions define no `performance`). First
/// call anchors at isolate-thread start.
static CLOCK_START: OnceLock<Instant> = OnceLock::new();

const WORLD_COORD_MAX: i64 = (1 << 14) - 1;
const TILE_LEVEL_MAX: i64 = 3;
const CROSS_PLANE_DISTANCE: i32 = 1_000_000;

/// Validate JavaScript Tile values at the native-world boundary, then
/// preserve the shim's historical cross-plane representation on top of
/// the host query distance primitive.
fn tile_distance(args: &[serde_json::Value]) -> Result<serde_json::Value, rustyscript::Error> {
    let from = distance_tile(args.first(), "from")?;
    let to = distance_tile(args.get(1), "to")?;
    let host_distance = api::query::chebyshev_to(from, to);
    let distance = if host_distance == i32::MAX {
        let planar = api::query::chebyshev_to(
            from,
            api::WorldTile {
                level: from.level,
                ..to
            },
        );
        CROSS_PLANE_DISTANCE + planar
    } else {
        host_distance
    };
    Ok(serde_json::Value::from(distance))
}

fn distance_tile(
    value: Option<&serde_json::Value>,
    side: &str,
) -> Result<api::WorldTile, rustyscript::Error> {
    let object = value
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            rustyscript::Error::Runtime(format!(
                "invalid tile distance: {side} must be a Tile-like object"
            ))
        })?;
    Ok(api::WorldTile {
        x: tile_integer(object.get("x"), side, "x", 0, WORLD_COORD_MAX)?,
        z: tile_integer(object.get("z"), side, "z", 0, WORLD_COORD_MAX)?,
        level: match object.get("level") {
            None | Some(serde_json::Value::Null) => 0,
            value => tile_integer(value, side, "level", 0, TILE_LEVEL_MAX)?,
        },
    })
}

fn tile_integer(
    value: Option<&serde_json::Value>,
    side: &str,
    field: &str,
    min: i64,
    max: i64,
) -> Result<i32, rustyscript::Error> {
    let integer = value
        .and_then(serde_json::Value::as_i64)
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| {
            rustyscript::Error::Runtime(format!(
                "invalid tile distance: {side}.{field} must be an integer in {min}..={max}"
            ))
        })?;
    Ok(integer as i32)
}

fn json_i32(value: Option<&serde_json::Value>) -> Option<i32> {
    value.and_then(|v| {
        v.as_i64()
            .or_else(|| {
                v.as_f64()
                    .and_then(|n| (n.is_finite() && n.fract() == 0.0).then_some(n as i64))
            })
            .and_then(|n| i32::try_from(n).ok())
    })
}

fn json_f64(value: Option<&serde_json::Value>) -> f64 {
    value
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)))
        .unwrap_or(0.0)
}

fn json_tile(value: Option<&serde_json::Value>) -> Option<api::WorldTile> {
    let object = value.and_then(serde_json::Value::as_object)?;
    Some(api::WorldTile {
        x: json_i32(object.get("x"))?,
        z: json_i32(object.get("z"))?,
        level: match object.get("level") {
            None | Some(serde_json::Value::Null) => 0,
            Some(other) => json_i32(Some(other))?,
        },
    })
}

/// Load `source` into `runtime` as a module and wire the global tick
/// entry. Native sources export `tick(api)`; compat sources
/// default-export a `defineBot` config and tick through `create()`'s
/// `loop()`, and the catalog shape default-exports a
/// `LoopingBot`/`TaskBot`/`TreeBot` subclass that is instantiated and
/// ticked through its `loop()`.
///
/// The shim prelude and the extra rs2b0t-named modules are wired first
/// so relative `../../api/...` imports and the remapped
/// `@rs2b0t/api` bundle resolve to our modules; an import that does
/// not name a shim module (e.g. `../../api/bank/Banking.js` before
/// its task) fails the load honestly.
pub(super) fn wire_runtime(
    runtime: &mut Runtime,
    source: &str,
    shape: LoadShape,
    siblings: &[(String, String)],
    game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
    named_banks: std::sync::Arc<api::named_banks::NamedBankFacts>,
) -> Result<(), String> {
    if shape == LoadShape::Reject {
        return Err("not a bot shape".to_string());
    }
    let source = crate::shim::remap_catalog_imports(source);
    let family = match super::shape::parse_declared_api_version(&source) {
        Ok(Some(2)) => super::shape::ApiFamily::V2,
        _ => super::shape::ApiFamily::Unversioned,
    };
    runtime
        .register_function(
            "__rs2b0t_now",
            |_args: &[rustyscript::serde_json::Value]| {
                let start = CLOCK_START.get_or_init(Instant::now);
                Ok(rustyscript::serde_json::Value::from(
                    start.elapsed().as_millis() as f64,
                ))
            },
        )
        .map_err(|e| format!("register now: {e}"))?;
    runtime
        .register_function("__rs2b0t_tile_distance", tile_distance)
        .map_err(|e| format!("register tile distance: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_range_supply_empty",
            |args: &[serde_json::Value]| {
                Ok(serde_json::Value::Bool(
                    crate::ranged::range_supply_empty_args(args),
                ))
            },
        )
        .map_err(|e| format!("register range supply empty: {e}"))?;
    runtime
        .register_function("__rs2b0t_withdraw_step", |args: &[serde_json::Value]| {
            Ok(crate::bank_withdraw::step(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register withdraw: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_matches_common_bank_loot",
            |args: &[serde_json::Value]| {
                let name = args.first().and_then(|v| v.as_str()).unwrap_or("");
                let id = args.get(1).and_then(|v| v.as_i64()).unwrap_or(-1);
                Ok(serde_json::Value::Bool(i32::try_from(id).ok().is_some_and(
                    |id| api::content::matches_common_bank_loot(name, id),
                )))
            },
        )
        .map_err(|e| format!("register common bank loot: {e}"))?;
    runtime
        .register_function("__rs2b0t_periodic_bank", |args: &[serde_json::Value]| {
            Ok(crate::periodic_bank::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register periodic bank: {e}"))?;
    runtime
        .register_function("__rs2b0t_bank_open", |args: &[serde_json::Value]| {
            Ok(crate::bank_open::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register bank open: {e}"))?;
    let bank_unlock_facts = std::sync::Arc::clone(&named_banks);
    runtime
        .register_function("__rs2b0t_bank_unlocked", move |args: &[serde_json::Value]| {
            let payload = args.first().unwrap_or(&serde_json::Value::Null);
            let Some(name) = payload.get("name").and_then(|v| v.as_str()) else {
                return Ok(serde_json::Value::Bool(false));
            };
            let Some(tile) = json_tile(Some(payload)) else {
                return Ok(serde_json::Value::Bool(false));
            };
            Ok(serde_json::Value::Bool(api::named_banks::bank_unlocked(
                bank_unlock_facts.as_ref(),
                name,
                tile,
            )))
        })
        .map_err(|e| format!("register bank unlocked: {e}"))?;
    runtime
        .register_function("__rs2b0t_walk", |args: &[serde_json::Value]| {
            Ok(crate::walk_wait::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register walk: {e}"))?;
    runtime
        .register_function("__rs2b0t_cake_stall", |args: &[serde_json::Value]| {
            Ok(crate::cake_stall::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register cake stall: {e}"))?;
    runtime
        .register_function("__rs2b0t_death_recovery", |args: &[serde_json::Value]| {
            Ok(crate::death_recovery::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register death recovery: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_selected_loadout",
            |args: &[serde_json::Value]| {
                let rows: Vec<crate::loadouts_store::Loadout> = serde_json::from_value(
                    args.first().cloned().unwrap_or(serde_json::json!([])),
                )
                .unwrap_or_default();
                Ok(crate::loadouts_store::selected_compat_loadout(
                    &rows,
                    args.get(1).and_then(|v| v.as_str()).unwrap_or(""),
                ))
            },
        )
        .map_err(|e| format!("register loadout: {e}"))?;
    let selected_food = game_data.clone();
    runtime
        .register_function("__rs2b0t_food_of", move |args: &[serde_json::Value]| {
            let fallback = args.get(1).cloned().unwrap_or(serde_json::json!(""));
            let food = args
                .first()
                .and_then(|v| v.get("carry"))
                .and_then(|v| v.as_array())
                .and_then(|rows| {
                    rows.iter().find_map(|row| {
                        let name = row.get("item")?.as_str()?;
                        selected_food
                            .as_deref()
                            .and_then(|data| {
                                data.fixed_food_heals()
                                    .find(|(known, _)| known.eq_ignore_ascii_case(name))
                            })
                            .map(|(known, _)| serde_json::json!(known))
                    })
                });
            Ok(food.unwrap_or(fallback))
        })
        .map_err(|e| format!("register food: {e}"))?;
    runtime
        .register_function("__rs2b0t_gear_of", |args: &[serde_json::Value]| {
            Ok(serde_json::to_value(crate::loadouts_store::gear_of(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
            .unwrap_or(serde_json::json!([])))
        })
        .map_err(|e| format!("register gear: {e}"))?;
    runtime
        .register_function("__rs2b0t_supplies_of", |args: &[serde_json::Value]| {
            Ok(serde_json::to_value(crate::loadouts_store::supplies_of(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
            .unwrap_or(serde_json::json!([])))
        })
        .map_err(|e| format!("register supplies: {e}"))?;
    runtime
        .register_function("__rs2b0t_weapon_of", |args: &[serde_json::Value]| {
            let fallback = args
                .get(1)
                .and_then(|v| if v.is_null() { None } else { v.as_str() });
            Ok(crate::loadouts_store::weapon_of(
                args.first().unwrap_or(&serde_json::Value::Null),
                fallback,
            ))
        })
        .map_err(|e| format!("register weapon: {e}"))?;
    let selected_keep_names = game_data.clone();
    runtime
        .register_function(
            "__rs2b0t_combat_keep_names",
            move |args: &[serde_json::Value]| {
                let data = selected_keep_names.as_deref().ok_or_else(|| {
                    rustyscript::Error::Runtime(
                        "combatKeepNames requires selected game data".to_string(),
                    )
                })?;
                let options = crate::keep_list::from_args(args);
                Ok(serde_json::json!(crate::keep_list::combat_keep_names(
                    data, &options
                )))
            },
        )
        .map_err(|e| format!("register combat keep names: {e}"))?;
    let selected_spells = game_data.clone();
    runtime
        .register_function(
            "__rs2b0t_runes_per_cast",
            move |args: &[serde_json::Value]| {
                let spell = args.first().and_then(|v| v.as_str()).unwrap_or("");
                let wielded: Vec<String> = args
                    .get(1)
                    .and_then(|v| v.as_array())
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(match selected_spells.as_deref() {
                    Some(data) => match data.runes_per_cast(spell, &wielded) {
                        Some(costs) => serde_json::json!(costs
                            .into_iter()
                            .map(|cost| serde_json::json!({
                                "rune": cost.rune,
                                "count": cost.count
                            }))
                            .collect::<Vec<_>>()),
                        None => serde_json::Value::Null,
                    },
                    None => serde_json::Value::Null,
                })
            },
        )
        .map_err(|e| format!("register runes per cast: {e}"))?;
    let selected_spell_buttons = game_data.clone();
    runtime
        .register_function(
            "__rs2b0t_spell_button_com",
            move |args: &[serde_json::Value]| {
                let spell = args.first().and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::json!(selected_spell_buttons
                    .as_deref()
                    .map(|data| data.spell_button_com(spell))
                    .unwrap_or(-1)))
            },
        )
        .map_err(|e| format!("register spell button: {e}"))?;
    crate::autocast::configure(game_data.as_deref());
    crate::shop::configure(game_data.clone());
    let selected_autocast = game_data.clone();
    runtime
        .register_function("__rs2b0t_autocast", move |args: &[serde_json::Value]| {
            Ok(crate::autocast::dispatch(
                selected_autocast.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register autocast: {e}"))?;
    let selected_special = game_data.clone();
    runtime
        .register_function("__rs2b0t_special", move |args: &[serde_json::Value]| {
            Ok(crate::special::dispatch(
                selected_special.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register special: {e}"))?;
    let selected_teleport = game_data.clone();
    runtime
        .register_function("__rs2b0t_teleport", move |args: &[serde_json::Value]| {
            Ok(crate::teleport::dispatch(
                selected_teleport.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register teleport: {e}"))?;
    let selected_shop = game_data.clone();
    runtime
        .register_function("__rs2b0t_shop", move |args: &[serde_json::Value]| {
            Ok(crate::shop::dispatch(
                selected_shop.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register shop: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_production",
            move |args: &[serde_json::Value]| {
                Ok(crate::production::dispatch(
                    args.first().unwrap_or(&serde_json::Value::Null),
                ))
            },
        )
        .map_err(|e| format!("register production: {e}"))?;
    runtime
        .register_function("__rs2b0t_dialog", move |args: &[serde_json::Value]| {
            Ok(crate::dialog::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register dialog: {e}"))?;
    runtime
        .register_function("__rs2b0t_fire", move |args: &[serde_json::Value]| {
            Ok(crate::fire::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register fire: {e}"))?;
    runtime
        .register_function("__rs2b0t_trade", move |args: &[serde_json::Value]| {
            Ok(crate::trade::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register trade: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_drive_partner_trade",
            move |args: &[serde_json::Value]| {
                Ok(crate::drive_partner_trade::dispatch(
                    args.first().unwrap_or(&serde_json::Value::Null),
                ))
            },
        )
        .map_err(|e| format!("register drive_partner_trade: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_is_hostile_attacker",
            |args: &[serde_json::Value]| {
                let payload = args.first().unwrap_or(&serde_json::Value::Null);
                let as_i32 = |v: Option<&serde_json::Value>| {
                    v.and_then(|value| {
                        value
                            .as_i64()
                            .or_else(|| {
                                value.as_f64().and_then(|n| {
                                    (n.is_finite() && n.fract() == 0.0).then_some(n as i64)
                                })
                            })
                            .and_then(|n| i32::try_from(n).ok())
                    })
                };
                let Some(distance) = as_i32(payload.get("distance")) else {
                    return Ok(serde_json::Value::Bool(false));
                };
                let Some(max_distance) = as_i32(payload.get("maxDistance")) else {
                    return Ok(serde_json::Value::Bool(false));
                };
                let actions: Vec<&str> = payload
                    .get("actions")
                    .and_then(|v| v.as_array())
                    .map(|rows| rows.iter().filter_map(|row| row.as_str()).collect())
                    .unwrap_or_default();
                Ok(serde_json::Value::Bool(
                    crate::content::is_hostile_attacker(
                        payload.get("name").and_then(|v| v.as_str()),
                        payload
                            .get("inCombat")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        payload
                            .get("targetsAnotherPlayer")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        distance,
                        &actions,
                        max_distance,
                    ),
                ))
            },
        )
        .map_err(|e| format!("register hostile attacker: {e}"))?;
    runtime
        .register_function("__rs2b0t_ent_npc_ids", |_args: &[serde_json::Value]| {
            Ok(serde_json::json!(api::ent::ENT_NPC_IDS))
        })
        .map_err(|e| format!("register ent npc ids: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_ent_life_ticks",
            |_args: &[serde_json::Value]| Ok(serde_json::json!(api::ent::ENT_LIFE_TICKS)),
        )
        .map_err(|e| format!("register ent life ticks: {e}"))?;
    runtime
        .register_function("__rs2b0t_is_ent_npc_id", |args: &[serde_json::Value]| {
            Ok(serde_json::Value::Bool(
                json_i32(args.first()).is_some_and(api::ent::is_ent_npc_id),
            ))
        })
        .map_err(|e| format!("register is ent npc id: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_ent_npc_on_tile",
            |args: &[serde_json::Value]| {
                let npcs = args
                    .first()
                    .and_then(serde_json::Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| {
                                Some((json_i32(row.get("id"))?, json_tile(row.get("tile"))?))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Ok(serde_json::Value::Bool(json_tile(args.get(1)).is_some_and(
                    |tile| crate::ent::ent_npc_on_tile(npcs, tile),
                )))
            },
        )
        .map_err(|e| format!("register ent npc on tile: {e}"))?;
    runtime
        .register_function("__rs2b0t_tool_step", |args: &[serde_json::Value]| {
            Ok(crate::gather_tools::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register tool step: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_boost_potions_step",
            |args: &[serde_json::Value]| {
                Ok(crate::boost_potions::dispatch(
                    args.first().unwrap_or(&serde_json::Value::Null),
                ))
            },
        )
        .map_err(|e| format!("register boost potions step: {e}"))?;
    runtime
        .register_function("__rs2b0t_target_step", |args: &[serde_json::Value]| {
            Ok(crate::targets::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register target step: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_begin", |_args: &[serde_json::Value]| {
            crate::canvas::begin();
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas begin: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_set", |args: &[serde_json::Value]| {
            let prop = args
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let value = args
                .get(1)
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            crate::canvas::set_style(prop, &value);
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas set: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_get", |args: &[serde_json::Value]| {
            let prop = args
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let value = crate::canvas::get_style(prop);
            Ok(serde_json::Value::String(value))
        })
        .map_err(|e| format!("register canvas get: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_set_num", |args: &[serde_json::Value]| {
            let prop = args
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            crate::canvas::set_number(prop, json_f64(args.get(1)));
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas set num: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_get_num", |args: &[serde_json::Value]| {
            let prop = args
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            Ok(serde_json::json!(crate::canvas::get_number(prop)))
        })
        .map_err(|e| format!("register canvas get num: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_fill_gradient_id",
            |_args: &[serde_json::Value]| {
                Ok(serde_json::json!(crate::canvas::fill_gradient_id()))
            },
        )
        .map_err(|e| format!("register canvas fill gradient id: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_set_fill_gradient",
            |args: &[serde_json::Value]| {
                let id = json_f64(args.first()) as u32;
                crate::canvas::set_fill_gradient(id);
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas set fill gradient: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_save", |_args: &[serde_json::Value]| {
            crate::canvas::save();
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas save: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_restore",
            |_args: &[serde_json::Value]| {
                crate::canvas::restore();
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas restore: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_begin_path",
            |_args: &[serde_json::Value]| {
                crate::canvas::begin_path();
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas beginPath: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_close_path",
            |_args: &[serde_json::Value]| {
                crate::canvas::close_path();
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas closePath: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_move_to", |args: &[serde_json::Value]| {
            crate::canvas::move_to(json_f64(args.first()), json_f64(args.get(1)));
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas moveTo: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_line_to", |args: &[serde_json::Value]| {
            crate::canvas::line_to(json_f64(args.first()), json_f64(args.get(1)));
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas lineTo: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_quad_to", |args: &[serde_json::Value]| {
            crate::canvas::quadratic_curve_to(
                json_f64(args.first()),
                json_f64(args.get(1)),
                json_f64(args.get(2)),
                json_f64(args.get(3)),
            );
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas quadraticCurveTo: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_arc", |args: &[serde_json::Value]| {
            match crate::canvas::arc(
                json_f64(args.first()),
                json_f64(args.get(1)),
                json_f64(args.get(2)),
                json_f64(args.get(3)),
                json_f64(args.get(4)),
                args.get(5)
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            ) {
                Ok(()) => Ok(serde_json::Value::Null),
                Err(msg) => Err(rustyscript::Error::Runtime(msg)),
            }
        })
        .map_err(|e| format!("register canvas arc: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_fill", |_args: &[serde_json::Value]| {
            crate::canvas::fill();
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas fill: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_stroke", |_args: &[serde_json::Value]| {
            crate::canvas::stroke();
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas stroke: {e}"))?;
    runtime
        .register_function("__rs2b0t_canvas_clip", |_args: &[serde_json::Value]| {
            crate::canvas::clip();
            Ok(serde_json::Value::Null)
        })
        .map_err(|e| format!("register canvas clip: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_create_linear",
            |args: &[serde_json::Value]| match crate::canvas::create_linear(
                json_f64(args.first()),
                json_f64(args.get(1)),
                json_f64(args.get(2)),
                json_f64(args.get(3)),
            ) {
                Ok(id) => Ok(serde_json::json!(id)),
                Err(msg) => Err(rustyscript::Error::Runtime(msg)),
            },
        )
        .map_err(|e| format!("register canvas createLinearGradient: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_create_radial",
            |args: &[serde_json::Value]| match crate::canvas::create_radial(
                json_f64(args.first()),
                json_f64(args.get(1)),
                json_f64(args.get(2)),
                json_f64(args.get(3)),
                json_f64(args.get(4)),
                json_f64(args.get(5)),
            ) {
                Ok(id) => Ok(serde_json::json!(id)),
                Err(msg) => Err(rustyscript::Error::Runtime(msg)),
            },
        )
        .map_err(|e| format!("register canvas createRadialGradient: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_add_color_stop",
            |args: &[serde_json::Value]| {
                let id = json_f64(args.first()) as u32;
                let offset = json_f64(args.get(1));
                let color = args
                    .get(2)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                match crate::canvas::add_color_stop(id, offset, color) {
                    Ok(()) => Ok(serde_json::Value::Null),
                    Err(msg) => Err(rustyscript::Error::Runtime(msg)),
                }
            },
        )
        .map_err(|e| format!("register canvas addColorStop: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_fill_rect",
            |args: &[serde_json::Value]| {
                let num = |i: usize| {
                    args.get(i)
                        .and_then(serde_json::Value::as_f64)
                        .or_else(|| {
                            args.get(i)
                                .and_then(serde_json::Value::as_i64)
                                .map(|n| n as f64)
                        })
                        .unwrap_or(0.0)
                };
                crate::canvas::fill_rect(num(0), num(1), num(2), num(3));
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas fillRect: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_fill_text",
            |args: &[serde_json::Value]| {
                let text = args
                    .first()
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let num = |i: usize| {
                    args.get(i)
                        .and_then(serde_json::Value::as_f64)
                        .or_else(|| {
                            args.get(i)
                                .and_then(serde_json::Value::as_i64)
                                .map(|n| n as f64)
                        })
                        .unwrap_or(0.0)
                };
                crate::canvas::fill_text(text, num(1), num(2));
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas fillText: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_measure_text",
            |args: &[serde_json::Value]| {
                let text = args
                    .first()
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                match crate::canvas::measure_text(text) {
                    Ok(w) => Ok(serde_json::json!(w)),
                    Err(msg) => Err(rustyscript::Error::Runtime(msg)),
                }
            },
        )
        .map_err(|e| format!("register canvas measureText: {e}"))?;
    runtime
        .register_function(
            "__rs2b0t_canvas_onpaint_done",
            |args: &[serde_json::Value]| {
                let kind = args
                    .first()
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0);
                let msg = args.get(1).and_then(serde_json::Value::as_str);
                crate::canvas::onpaint_done(kind, msg);
                Ok(serde_json::Value::Null)
            },
        )
        .map_err(|e| format!("register canvas onpaint done: {e}"))?;
    runtime
        .eval::<()>(crate::shim::PRELUDE)
        .map_err(|e| format!("shim: {e}"))?;
    super::buyout_plan::install(runtime).map_err(|e| format!("buyout plan: {e}"))?;
    super::paint_chrome::install(runtime).map_err(|e| format!("paint chrome: {e}"))?;
    let content = format!(
        "globalThis.__rs2b0t_host.content = {};",
        crate::shim::content_json(game_data.as_deref(), named_banks.as_ref())
    );
    runtime
        .eval::<()>(content.as_str())
        .map_err(|e| format!("content: {e}"))?;
    let bot = rustyscript::Module::new(crate::shim::BOT_MODULE, source);
    let main = match shape {
        LoadShape::NativeTick if family == super::shape::ApiFamily::V2 => {
            rustyscript::Module::new(crate::shim::MAIN_MODULE, NATIVE_V2_MAIN)
        }
        LoadShape::NativeTick => rustyscript::Module::new(crate::shim::MAIN_MODULE, NATIVE_MAIN),
        LoadShape::CompatDefineBot => rustyscript::Module::new(
            crate::shim::MAIN_MODULE,
            format!("{COMPAT_MAIN}{COMPAT_RUNNER}"),
        ),
        LoadShape::CompatClass => rustyscript::Module::new(
            crate::shim::MAIN_MODULE,
            format!("{COMPAT_CLASS_MAIN}{COMPAT_RUNNER}"),
        ),
        LoadShape::Reject => unreachable!("rejected above"),
    };
    // Side modules load in order, so the shim modules (which the bot
    // imports) must precede the bot's own module.
    let mut side = crate::shim::shim_modules();
    let siblings: Vec<(String, String)> = siblings
        .iter()
        .map(|(url, src)| (url.clone(), crate::shim::remap_catalog_imports(src)))
        .collect();
    for (url, src) in &siblings {
        side.push(rustyscript::Module::new(url, src));
    }
    side.push(bot);
    let side: Vec<&rustyscript::Module> = side.iter().collect();
    // Loading evaluates the modules, so a missing `tick`/default
    // export, an unresolvable import, or a syntax error surfaces
    // here, before any tick runs.
    runtime
        .load_modules(&main, side)
        .map_err(|e| format!("load: {e}"))?;
    Ok(())
}

/// Native wrapper: re-export the module's `tick` behind a global that
/// receives the tick number and a persistent `api` object. `api` is a
/// Proxy: the host owns it (`api.tick` is set each tick); reading or
/// writing any other member throws `not impl` — a script stashes its own
/// state elsewhere, never in host-owned slots.
const NATIVE_MAIN: &str = r#"
import { tick } from './bot.js';
const api = new Proxy({}, {
get(target, prop) {
    if (typeof prop === 'symbol') return undefined;
    if (prop === 'tick') return target.tick;
    throw new Error('not impl: api.' + String(prop));
},
set(target, prop, value) {
    if (prop === 'tick') { target.tick = value; return true; }
    throw new Error('not impl: api.' + String(prop));
},
});
globalThis.__rs_api = api;
globalThis.__rs_tick = (n) => { api.tick = n; return tick(api); };
"#;

/// Explicit v2 wrapper: public NativeApi only. Does not grow shim policy.
/// Snapshot is a Proxy over the host-owned materialized object (no freeze,
/// no per-tick deep copy). request enqueues onto the existing interact drain
/// and returns void. Async single-flight is observed by the Rust isolate.
const NATIVE_V2_MAIN: &str = r#"
import { tick } from './bot.js';
const SNAPSHOT_KEYS = new Set([
  'ingame','here','inv','inv_size','stats','bank','bank_side','bank_open','bank_loaded',
  'bank_generation','banks','nearest_booth','bank_approaches','count_dialog_open',
  'withdraw_x_result_seq','withdraw_x_result','withdraw_load_result_seq','withdraw_load_result',
  'bank_op_result_seq','bank_op_result','walk_outcome_seq','walk_outcome_generation',
  'walk_outcome_failed','walk_outcome_x','walk_outcome_z','walk_outcome_level',
  'walk_outcome_radius','walk_outcome_allow_teleports','walk_outcome_request_id',
]);
const V2_OPS = {
  'held': ['name','action'],
  'open-booth': ['x','z','level','id'],
  'open-stand': ['x','z','level','kind'],
  'close': [],
  'set-note-mode': ['on'],
  'withdraw': ['name','action'],
  'withdraw-load': ['name','bank_generation'],
  'withdraw-x': ['name','count','bank_item_id','lands_as_id','action','bank_generation'],
  'walk': ['x','z','level'],
  'walk-near': ['x','z','level','radius'],
  'walk-nearest-bank': [],
};
const OPTIONAL = {
  'open-booth': ['name','action'],
  'open-stand': ['name','stand_op','choose'],
  'walk': ['allow_teleports','allow_wilderness','allow_bank_fetch','request_id'],
  'walk-near': ['allow_teleports','allow_wilderness','allow_bank_fetch','request_id'],
};
function host() {
  return globalThis.__rs2b0t_host || (globalThis.__rs2b0t_host = { interact: [], log: [] });
}
function readOnlyView(value, root) {
  if (value === null || typeof value !== 'object') return value;
  return new Proxy(value, {
    get(target, prop) {
      if (typeof prop === 'symbol') return target[prop];
      if (root && !SNAPSHOT_KEYS.has(prop)) return undefined;
      return readOnlyView(target[prop], false);
    },
    set() { throw new Error('snapshot is read-only'); },
    deleteProperty() { throw new Error('snapshot is read-only'); },
    defineProperty() { return false; },
    has(target, prop) {
      if (root && typeof prop === 'string') return SNAPSHOT_KEYS.has(prop) && prop in target;
      return prop in target;
    },
    ownKeys(target) {
      const keys = Reflect.ownKeys(target);
      return root ? keys.filter((k) => typeof k !== 'string' || SNAPSHOT_KEYS.has(k)) : keys;
    },
    getOwnPropertyDescriptor(target, prop) {
      if (root && typeof prop === 'string' && !SNAPSHOT_KEYS.has(prop)) return undefined;
      const desc = Object.getOwnPropertyDescriptor(target, prop);
      if (!desc) return desc;
      return { ...desc, writable: false };
    },
  });
}
function settingsReader() {
  return {
    str(name, fallback) {
      const v = (host().settingsBag || {})[name];
      return typeof v === 'string' ? v : (fallback === undefined ? '' : fallback);
    },
    num(name, fallback) {
      const v = (host().settingsBag || {})[name];
      return typeof v === 'number' && Number.isFinite(v) ? v : (fallback === undefined ? 0 : fallback);
    },
    bool(name, fallback) {
      const v = (host().settingsBag || {})[name];
      return typeof v === 'boolean' ? v : (fallback === undefined ? false : fallback);
    },
  };
}
function paintRecorder() {
  return {
    begin(opts) {
      const rec = { title: null, accent: (opts && opts.accent) || null, lines: [], buttons: [] };
      const frame = {
        title(text) { rec.title = String(text); return frame; },
        row(...cols) { rec.lines.push(cols.join(' | ')); return frame; },
        gap() { rec.lines.push(''); return frame; },
        end() { host().paint = { title: rec.title, accent: rec.accent, lines: rec.lines, buttons: rec.buttons }; },
      };
      return frame;
    },
  };
}
function enqueueRequest(op) {
  if (!op || typeof op !== 'object' || typeof op.op !== 'string') {
    throw new Error('not impl: request');
  }
  const required = V2_OPS[op.op];
  if (!required) {
    throw new Error('not impl: request.' + op.op);
  }
  for (const field of required) {
    if (op[field] === undefined || op[field] === null) {
      throw new Error('not impl: request.' + op.op + ' missing ' + field);
    }
  }
  const row = { op: op.op };
  for (const field of required) row[field] = op[field];
  const extra = OPTIONAL[op.op] || [];
  for (const field of extra) {
    if (op[field] !== undefined) row[field] = op[field];
  }
  const h = host();
  h.interact = h.interact || [];
  h.interact.push(row);
}
const api = {
  get tick() { return host().tick || 0; },
  set tick(n) { host().tick = n; },
  get snapshot() { return readOnlyView(host().snapshot || {}, true); },
  get settings() { return settingsReader(); },
  log(message) {
    const h = host();
    h.log = h.log || [];
    h.log.push(String(message));
  },
  stop(reason) {
    const h = host();
    h.stopRequested = true;
    if (typeof reason === 'string') h.stopReason = reason;
  },
  get paint() { return paintRecorder(); },
  request(op) { enqueueRequest(op); },
};
globalThis.__rs_api = api;
globalThis.__rs_api_family = 2;
globalThis.__rs_v2_tick_pending = false;
let lifecycleGeneration = 0;
globalThis.__rs_v2_reset_session = () => { lifecycleGeneration += 1; };
function recordSettlement(generation) {
  if (generation !== lifecycleGeneration) return;
  const h = host();
  h.interact = h.interact || [];
  h.interact.push({ op: 'loop-settled' });
}
globalThis.__rs_tick = (n) => {
  api.tick = n;
  const generation = lifecycleGeneration;
  const r = tick(api);
  const pending = !!(r && typeof r.then === 'function');
  globalThis.__rs_v2_tick_pending = pending;
  if (pending) {
    Promise.resolve(r).then(() => {
      globalThis.__rs_v2_tick_pending = false;
      recordSettlement(generation);
    }, (e) => {
      globalThis.__rs_v2_tick_pending = false;
      host().lastError = String((e && e.message) || e);
    });
  } else {
    recordSettlement(generation);
  }
  return r;
};
"#;

/// Compat wrapper: `create()` the bot instance, then the shared compat
/// runner (onStart once, awaited, then loop/onPaint every tick). The
/// instance is exposed as `__rs_bot` for probe read-back and the
/// EventSignal/ignoredRandoms host read (same global the class shape
/// uses).
const COMPAT_MAIN: &str = r#"
import bot from './bot.js';
const inst = (bot && typeof bot.create === 'function') ? bot.create() : (bot || null);
globalThis.__rs_bot = inst;
"#;

/// Compat class wrapper: instantiate the default-export
/// `LoopingBot`/`TaskBot`/`TreeBot` subclass, then the shared compat
/// runner. The instance is exposed as `__rs_bot` for probe read-back.
const COMPAT_CLASS_MAIN: &str = r#"
import bot from './bot.js';
const inst = new bot();
globalThis.__rs_bot = inst;
"#;

/// The shared compat tick runner (defineBot and class shapes): `onStart`
/// once (awaited), then `loop()` (awaited). `onPaint` runs on every
/// posted tick even while `loop` is parked on an Execution wait —
/// paint is not gated on `loop()` returning. `__rs2b0t_tick_async` is
/// async so an Execution wait parks the whole runner; `__rs_tick` is
/// the synchronous entry the thread calls (it returns immediately —
/// parked or not), and the wait is settled by `__rs2b0t_pump` on later
/// posted ticks instead of a re-entrant `loop()`. Async errors (a cond
/// that throws) land on the host handle's `lastError` for the thread
/// to log.
const COMPAT_RUNNER: &str = r#"
globalThis.__rs2b0t_flush_native_events = () => {
const pending = globalThis.__rs2b0t_pending_native_event_batch;
globalThis.__rs2b0t_pending_native_event_batch = null;
if (pending && pending.length) {
    const dispatch = globalThis.__rs2b0t_dispatch_native_events;
    if (typeof dispatch === 'function') dispatch(pending);
}
};
globalThis.__rs_tick = (n) => {
if (!inst) return;
globalThis.__rs2b0t_tick_async(n).catch((e) => {
    globalThis.__rs2b0t_host.lastError = String((e && e.message) || e);
    globalThis.__rs2b0t_host.loopInFlight = false;
});
};
globalThis.__rs2b0t_tick_async = async (n) => {
const h = globalThis.__rs2b0t_host;
h.tick = n;
const fire = globalThis.__rs2b0t_fire_tick_listeners;
if (typeof fire === 'function') fire();
if (!globalThis.__rs2b0t_started) {
    globalThis.__rs2b0t_started = true;
    // Same single-flight as loop(): a pending onStart must not let a
    // later tick enter loop(). Listeners/chat/onPaint still run.
    h.loopInFlight = true;
    try {
        if (typeof inst.onStart === 'function') { await inst.onStart(); }
    } finally {
        h.loopInFlight = false;
    }
}
// Native Rust selects/deduplicates events; deliver them only after
// onStart has installed subscriptions.
globalThis.__rs2b0t_flush_native_events();

// Single-flight: a never-resolving loop() must not re-enter. Tick
// listeners, chat, and onPaint still run.
if (h.loopInFlight) {
    await Promise.resolve();
    globalThis.__rs2b0t_call_on_paint(inst);
    return;
}
h.loopInFlight = true;
const loopP = (typeof inst.loop === 'function') ? inst.loop() : Promise.resolve();
await Promise.resolve();
globalThis.__rs2b0t_call_on_paint(inst);
try {
    await loopP;
    h.interact = h.interact || [];
    h.interact.push({ op: 'loop-settled' });
} finally {
    h.loopInFlight = false;
}
};
"#;

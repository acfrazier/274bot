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
        .register_function(
            "__rs2b0t_bank_unlocked",
            move |args: &[serde_json::Value]| {
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
            },
        )
        .map_err(|e| format!("register bank unlocked: {e}"))?;
    runtime
        .register_function("__rs2b0t_walk", |args: &[serde_json::Value]| {
            Ok(crate::walk_wait::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register walk: {e}"))?;
    runtime
        .register_function("__rs2b0t_inspect", |args: &[serde_json::Value]| {
            Ok(crate::inspect_wait::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register inspect: {e}"))?;
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
                let rows: Vec<crate::loadouts_store::Loadout> =
                    serde_json::from_value(args.first().cloned().unwrap_or(serde_json::json!([])))
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
            let names = args
                .first()
                .and_then(|v| v.get("carry"))
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|row| row.get("item").and_then(|v| v.as_str()));
            Ok(
                match crate::loadout_plan::food_of_name(selected_food.as_deref(), names) {
                    Some(name) => serde_json::json!(name),
                    None => fallback,
                },
            )
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
    crate::prayer::configure(game_data.as_deref());
    crate::shop::configure(game_data.clone());
    crate::supply_v2::configure(game_data.clone());
    let selected_autocast = game_data.clone();
    runtime
        .register_function("__rs2b0t_autocast", move |args: &[serde_json::Value]| {
            Ok(crate::autocast::dispatch(
                selected_autocast.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register autocast: {e}"))?;
    let selected_prayer = game_data.clone();
    runtime
        .register_function("__rs2b0t_prayer", move |args: &[serde_json::Value]| {
            Ok(crate::prayer::dispatch(
                selected_prayer.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register prayer: {e}"))?;
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
        .register_function("__rs2b0t_fight", |args: &[serde_json::Value]| {
            Ok(crate::hunt_fight::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register fight: {e}"))?;
    runtime
        .register_function("__rs2b0t_hold", |args: &[serde_json::Value]| {
            Ok(crate::hunt_fight::hold_dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register hold: {e}"))?;
    runtime
        .register_function("__rs2b0t_retreat", |args: &[serde_json::Value]| {
            Ok(crate::hunt_fight::retreat_dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register retreat: {e}"))?;
    runtime
        .register_function("__rs2b0t_walkspot", |args: &[serde_json::Value]| {
            Ok(crate::hunt_fight::walk_dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register walkspot: {e}"))?;
    runtime
        .register_function("__rs2b0t_enter", |args: &[serde_json::Value]| {
            Ok(crate::hunt_lair::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register enter: {e}"))?;
    let selected_leave = game_data.clone();
    runtime
        .register_function("__rs2b0t_leave", move |args: &[serde_json::Value]| {
            Ok(crate::hunt_leave::dispatch(
                selected_leave.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register leave: {e}"))?;
    runtime
        .register_function("__rs2b0t_key", |args: &[serde_json::Value]| {
            Ok(crate::hunt_key::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register key: {e}"))?;
    runtime
        .register_function("__rs2b0t_cell", |args: &[serde_json::Value]| {
            Ok(crate::hunt_cell::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register cell: {e}"))?;
    runtime
        .register_function("__rs2b0t_bank", |args: &[serde_json::Value]| {
            Ok(crate::hunt_bank::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register bank: {e}"))?;
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
        .register_function("__rs2b0t_modals", move |args: &[serde_json::Value]| {
            Ok(crate::modals::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register modals: {e}"))?;
    runtime
        .register_function("__rs2b0t_quest_journal", |args: &[serde_json::Value]| {
            Ok(crate::quest_journal::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register quest journal: {e}"))?;
    let selected_clue = game_data.clone();
    runtime
        .register_function("__rs2b0t_clue", move |args: &[serde_json::Value]| {
            Ok(crate::clue::dispatch(
                selected_clue.as_deref(),
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register clue: {e}"))?;
    runtime
        .register_function("__rs2b0t_reach", move |args: &[serde_json::Value]| {
            Ok(crate::reach::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register reach: {e}"))?;
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
            |_args: &[serde_json::Value]| Ok(serde_json::json!(crate::canvas::fill_gradient_id())),
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
    super::supply_v8::install(runtime).map_err(|e| format!("supply v8: {e}"))?;
    super::selected_facts_v8::install(runtime).map_err(|e| format!("selected facts v8: {e}"))?;
    super::gather_methods_v8::install(runtime).map_err(|e| format!("gather methods v8: {e}"))?;
    super::quest_facts_v8::install(runtime).map_err(|e| format!("quest facts v8: {e}"))?;
    super::clue_facts_v8::install(runtime).map_err(|e| format!("clue facts v8: {e}"))?;
    super::clue_logic_v8::install(runtime).map_err(|e| format!("clue logic v8: {e}"))?;
    super::clue_pack_v8::install(runtime).map_err(|e| format!("clue pack v8: {e}"))?;
    super::loadout_v8::install(runtime).map_err(|e| format!("loadout v8: {e}"))?;
    super::line_of_sight::install(runtime).map_err(|e| format!("line of sight: {e}"))?;
    super::distance::install(runtime).map_err(|e| format!("distance: {e}"))?;
    super::reach_query::install(runtime).map_err(|e| format!("reach query: {e}"))?;
    super::melee_weapons_v8::install(runtime).map_err(|e| format!("melee weapons v8: {e}"))?;
    super::partner_trade_v8::install(runtime).map_err(|e| format!("partner trade v8: {e}"))?;
    super::paint_chrome::install(runtime).map_err(|e| format!("paint chrome: {e}"))?;
    super::paint_jive::install(runtime).map_err(|e| format!("paint jive: {e}"))?;
    super::tools_v8::install(runtime).map_err(|e| format!("tools v8: {e}"))?;
    super::boost_potions_v8::install(runtime).map_err(|e| format!("boost potions v8: {e}"))?;
    super::targets_v8::install(runtime).map_err(|e| format!("targets v8: {e}"))?;
    super::fire_v8::install(runtime).map_err(|e| format!("fire v8: {e}"))?;
    super::combat_style_v8::install(runtime).map_err(|e| format!("combat style v8: {e}"))?;
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
/// state elsewhere, never in host-owned slots. A synchronous tick returns
/// `null`; an async one returns its never-rejecting settle promise, which
/// the isolate thread holds so the tick is not re-entered while pending.
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
globalThis.__rs_tick = (n) => {
api.tick = n;
const r = tick(api);
if (!r || typeof r.then !== 'function') return null;
return Promise.resolve(r).then(() => null, (e) => String((e && e.message) || e));
};
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
  'puzzle_board','puzzle_board_generation',
  'withdraw_x_result_seq','withdraw_x_result','withdraw_load_result_seq','withdraw_load_result',
  'bank_op_result_seq','bank_op_result',  'walk_outcome_seq','walk_outcome_generation',
  'walk_outcome_failed','walk_outcome_x','walk_outcome_z','walk_outcome_level',
  'walk_outcome_radius','walk_outcome_allow_teleports','walk_outcome_request_id',
  'route_inspect_seq','route_inspect_generation','route_inspect_request_id',
  'route_inspect_ok','route_inspect_reason','route_inspect_bank_planned',
  'route_inspect_ticks','route_inspect_hops',
  'route_inspect_prev_seq','route_inspect_prev_generation','route_inspect_prev_request_id',
  'route_inspect_prev_ok','route_inspect_prev_reason','route_inspect_prev_bank_planned',
  'route_inspect_prev_ticks','route_inspect_prev_hops',
  'route_inspect_running_id','route_inspect_pending_id','route_inspect_accepted_id',
  'route_inspect_replaced_id','route_inspect_replaced_prev_id',
  'route_inspect_refused_id','route_inspect_refused_id_2','route_inspect_refused_id_3',
  'route_inspect_unobserved','collision','npcs','self_target_kind','self_target_index',
]);
const V2_OPS = {
  'held': ['name','action'],
  'open-booth': ['x','z','level','id'],
  'open-stand': ['x','z','level','kind'],
  'close': [],
  'set-note-mode': ['on'],
  'deposit': ['name'],
  'withdraw': ['name','action'],
  'withdraw-load': ['name','bank_generation'],
  'puzzle-move': ['id','slot','component','generation'],
  'withdraw-x': ['name','count','bank_item_id','lands_as_id','action','bank_generation'],
  'walk': ['x','z','level'],
  'walk-near': ['x','z','level','radius'],
  'walk-nearest-bank': [],
  'inspect-route': ['from','to'],
};
const OPTIONAL = {
  'open-booth': ['name','action'],
  'open-stand': ['name','stand_op','choose'],
  'walk': ['allow_teleports','allow_wilderness','allow_bank_fetch','request_id'],
  'walk-near': ['allow_teleports','allow_wilderness','allow_bank_fetch','request_id'],
  'inspect-route': ['allow_teleports','allow_wilderness','allow_bank_fetch','avoid','request_id'],
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
  if (op.op === 'inspect-route') {
    const from = op.from || {};
    const to = op.to || {};
    row.from_x = from.x;
    row.from_z = from.z;
    row.from_level = from.level == null ? 0 : from.level;
    row.x = to.x;
    row.z = to.z;
    row.level = to.level == null ? 0 : to.level;
    delete row.from;
    delete row.to;
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
  inspectBegin(opts) {
    const o = opts || {};
    const fn = globalThis.rustyscript.functions.__rs2b0t_inspect;
    const token = fn({
      op: 'begin',
      from: o.from,
      to: o.to,
      opts: o.opts || o,
      timeout_ms: o.timeout_ms,
    });
    if (fn({ op: 'settled', token }) === true) return token;
    enqueueRequest({
      op: 'inspect-route',
      from: o.from,
      to: o.to,
      allow_teleports: o.allow_teleports === true,
      allow_wilderness: o.allow_wilderness === true,
      allow_bank_fetch: o.allow_bank_fetch === true,
      avoid: o.avoid || [],
      request_id: token,
    });
    return token;
  },
  inspectSettled(token) {
    return globalThis.rustyscript.functions.__rs2b0t_inspect({ op: 'settled', token }) === true;
  },
  inspectValue(token) {
    return globalThis.rustyscript.functions.__rs2b0t_inspect({ op: 'value', token });
  },
};
globalThis.__rs_api = api;
globalThis.__rs_api_family = 2;
globalThis.__rs_v2_tick_pending = false;
let lifecycleGeneration = 0;
globalThis.__rs_v2_reset_session = () => { lifecycleGeneration += 1; };
function prayerCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}
function helperOk(value) { return { ok: true, value: value }; }
function helperErr(error) { return { ok: false, error: String(error) }; }
function mapPrayerDone(step) {
  if (!step || step.kind === 'aborted' || step.kind !== 'done') {
    return helperErr((step && step.reason) || 'aborted');
  }
  if (step.ok === true) {
    return helperOk(step.value === undefined ? true : step.value);
  }
  return helperErr(step.reason || 'aborted');
}
function enqueueIfButton(component_id) {
  const h = host();
  h.interact = h.interact || [];
  h.interact.push({ op: 'if-button', component_id: component_id });
}
function runPrayerMachine(payload) {
  // Before: a second begin overwrote __rs_prayer_pump; Rust abort of the
  // old token did not resolve the first Promise (hang).
  // After: refuse a second native Set/Clear with busy before Rust begin
  // or click. The admitted operation keeps the pump and must settle.
  if (typeof globalThis.__rs_prayer_pump === 'function') {
    return Promise.resolve(helperErr('busy'));
  }
  const generation = lifecycleGeneration;
  const begin = prayerCall(payload);
  if (begin && begin.kind === 'if-button') {
    enqueueIfButton(begin.component_id);
  }
  if (begin && (begin.kind === 'done' || begin.kind === 'aborted')) {
    return Promise.resolve(mapPrayerDone(begin));
  }
  const token = begin && begin.token;
  return new Promise((resolve) => {
    const pump = () => {
      if (generation !== lifecycleGeneration) {
        if (globalThis.__rs_prayer_pump === pump) delete globalThis.__rs_prayer_pump;
        resolve(helperErr('aborted'));
        return;
      }
      const next = prayerCall({ op: 'next', token: token });
      if (!next || next.kind === 'wait') return;
      if (next.kind === 'if-button') {
        enqueueIfButton(next.component_id);
        return;
      }
      if (globalThis.__rs_prayer_pump === pump) delete globalThis.__rs_prayer_pump;
      resolve(mapPrayerDone(next));
    };
    globalThis.__rs_prayer_pump = pump;
  });
}
api.prayerPoints = function () { return prayerCall({ op: 'points' }); };
api.prayerMax = function () { return prayerCall({ op: 'max' }); };
api.prayerFull = function () { return prayerCall({ op: 'full' }); };
api.prayerKnown = function (input) {
  if (!input || typeof input.name !== 'string') return helperErr('invalid-args');
  return prayerCall({ op: 'known', name: input.name });
};
api.prayerAvailable = function (input) {
  if (!input || typeof input.name !== 'string') return helperErr('invalid-args');
  return prayerCall({ op: 'available', name: input.name });
};
api.prayerActive = function (input) {
  if (!input || typeof input.name !== 'string') return helperErr('invalid-args');
  return prayerCall({ op: 'active', name: input.name });
};
api.prayerSet = function (input) {
  // Sequential await is the preferred example. An already-admitted
  // operation still progresses on later eligible NativeTicks even if
  // this Promise is not returned from tick. A second Set while busy
  // settles immediately with error busy and does not click.
  if (!input || typeof input.name !== 'string' || typeof input.on !== 'boolean') {
    return Promise.resolve(helperErr('invalid-args'));
  }
  return runPrayerMachine({
    op: 'begin-set',
    name: input.name,
    on: { kind: 'boolean', value: input.on },
  });
};
api.prayerClear = function () {
  // Same admission rule as prayerSet: busy if a pump is already installed.
  return runPrayerMachine({ op: 'begin-clear' });
};
function supplyV2(op, input) {
  return globalThis.__rs2b0t_supply_v2(op, input);
}
api.foodCount = function (input) {
  if (!input || !Array.isArray(input.items) || typeof input.foodName !== 'string') {
    return helperErr('invalid-args');
  }
  return supplyV2('foodCount', input);
};
api.foodHealAmount = function (input) {
  if (!input || typeof input.foodName !== 'string') {
    return helperErr('invalid-args');
  }
  return supplyV2('foodHealAmount', input);
};
api.combatKeepNames = function (input) {
  if (!input || typeof input !== 'object' || typeof input.food !== 'string') {
    return helperErr('invalid-args');
  }
  return supplyV2('combatKeepNames', input);
};
api.runesPerCast = function (input) {
  if (!input || typeof input.spellName !== 'string' || !Array.isArray(input.wielded)) {
    return helperErr('invalid-args');
  }
  for (const row of input.wielded) {
    if (typeof row !== 'string') return helperErr('invalid-args');
  }
  return supplyV2('runesPerCast', input);
};
api.escapeRunesFor = function (input) {
  if (!input || typeof input.id !== 'string') {
    return helperErr('invalid-args');
  }
  return supplyV2('escapeRunesFor', input);
};
function gatherV2(op, input) {
  return globalThis.__rs2b0t_gather_methods_v2(op, input);
}
api.gatherMethods = function (input) {
  if (arguments.length === 0) return gatherV2('gatherMethods', {});
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (Object.prototype.hasOwnProperty.call(input, 'skill') && typeof input.skill !== 'string') {
    return helperErr('invalid-args');
  }
  return gatherV2('gatherMethods', input);
};
api.gatherResource = function (input) {
  if (input == null || typeof input !== 'object' || typeof input.name !== 'string') {
    return helperErr('invalid-args');
  }
  return gatherV2('gatherResource', input);
};
// Placement region and limit reuse the scene gates. An omitted region key on
// a present object is missing-region before resource, limit, or region shape.
api.gatherPlacements = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (!Object.prototype.hasOwnProperty.call(input, 'region')) {
    return helperErr('missing-region');
  }
  if (typeof input.resource !== 'string') return helperErr('invalid-args');
  if (!sceneLimitOk(input.limit)) return helperErr('invalid-args');
  if (sceneRegionValue(input.region) === null) return helperErr('invalid-args');
  return gatherV2('gatherPlacements', input);
};
function questV2(op, input) {
  return globalThis.__rs2b0t_quest_facts_v2(op, input);
}
api.questIdentity = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  const hasName = Object.prototype.hasOwnProperty.call(input, 'name');
  const hasId = Object.prototype.hasOwnProperty.call(input, 'id');
  if (hasName === hasId) return helperErr('invalid-args');
  if (hasName && typeof input.name !== 'string') return helperErr('invalid-args');
  if (hasId && typeof input.id !== 'string') return helperErr('invalid-args');
  return questV2('questIdentity', input);
};
api.questPrereqs = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (typeof input.id !== 'string') return helperErr('invalid-args');
  return questV2('questPrereqs', input);
};
function clueV2(op, input) {
  return globalThis.__rs2b0t_clue_facts_v2(op, input);
}
function clueLogicV2(op) {
  return globalThis.__rs2b0t_clue_logic_v2(op);
}
function cluePackV2(input) {
  return globalThis.__rs2b0t_clue_pack_v2('packPlan', input);
}
function clueHardKitV2(input) {
  return globalThis.__rs2b0t_clue_pack_v2('hardKit', input);
}
function clueKeepV2(input) {
  return globalThis.__rs2b0t_clue_pack_v2('keep', input);
}
function clueCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_clue(payload);
}
// The page is the already-posted `snapshot.inv` `(id, count)` sequence and
// nothing else: a missing or empty page is an empty held list, never a
// second snapshot error token, and a row that is not an i32 pair cannot be
// held. The pair is marshalled as `[id, count]`.
function cluePageI32(value) {
  return typeof value === 'number' && Number.isInteger(value)
    && value >= -2147483648 && value <= 2147483647;
}
function clueHeldPage() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
  const page = snapshot.inv;
  if (!Array.isArray(page)) return [];
  const held = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.id) || !cluePageI32(row.count)) continue;
    held.push([row.id, row.count]);
  }
  return held;
}
// The posted cooperative interrupt, re-read every call: EventSignal.pending()
// reads the same `hold || ours` pair. Nothing about it is captured at begin.
function cluePending() {
  const h = host();
  return h.hold === true || h.ours === true;
}
// The posted player tile, read at call time the same way `clueHeldPage` reads
// the pack page. A missing, null, or malformed `here` is not sent, and the
// machine then has no arrival claim to make.
function clueHereTile() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const here = snapshot.here;
  if (!here || typeof here !== 'object' || Array.isArray(here)) return null;
  if (!cluePageI32(here.x) || !cluePageI32(here.z) || !cluePageI32(here.level)) return null;
  return { x: here.x, z: here.z, level: here.level };
}
// One posted scene page (`locs` / `ground`), the same call-time class of page.
// `api.snapshot` hides both (SNAPSHOT_KEYS), so this reads `host().snapshot`
// directly the way `sceneProjection` does, and never through `api.sceneLocs`.
// A row that is not the posted `(id, x, z, level, actions)` shape cannot be
// picked and is dropped here, never a snapshot error. `name` rides along only
// for the page whose verb resolves identity by name, and it is never invented.
function clueScenePage(key, withName) {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
  const page = snapshot[key];
  if (!Array.isArray(page)) return [];
  const rows = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.id) || !cluePageI32(row.x) || !cluePageI32(row.z)
        || !cluePageI32(row.level)) continue;
    if (!Array.isArray(row.actions)) continue;
    const actions = [];
    let ok = true;
    for (const action of row.actions) {
      if (typeof action !== 'string') { ok = false; break; }
      actions.push(action);
    }
    if (!ok) continue;
    const out = { id: row.id, x: row.x, z: row.z, level: row.level, actions: actions };
    if (withName) out.name = typeof row.name === 'string' ? row.name : null;
    rows.push(out);
  }
  return rows;
}
function clueLocPage() {
  return clueScenePage('locs', false);
}
// The posted ground page the collect arm Takes from: the loc shape plus the
// display name the host resolves. Read at call time like every other page.
function clueGroundPage() {
  return clueScenePage('ground', true);
}
// The posted pack page the collect arm reads: the display name the Drop
// resolves and the positive count that occupies a slot. This is not a second
// inventory read and not a change to `clueHeldPage` — the identify page stays
// the `(id, count)` pair, and both come from the one posted `snapshot.inv`.
function clueInvPage() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
  const page = snapshot.inv;
  if (!Array.isArray(page)) return [];
  const rows = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.id) || !cluePageI32(row.count)) continue;
    rows.push({
      id: row.id,
      name: typeof row.name === 'string' ? row.name : null,
      count: row.count,
    });
  }
  return rows;
}
// The posted worn page the Entrana strip reads: the raw rows with their own
// `slot`, marshal-only and omit-if-absent the way the locs page is — a field
// the page did not carry is left off rather than defaulted, and a row that
// posted none of the four is not a row. This reads `host().snapshot` directly
// the way every call-time page here does (`api.snapshot` hides `equipment`,
// and `Equipment.items()` drops the slot); the machine matches on the posted
// `name` and reads the id only for its two hard-trail dagger ids.
function clueEquipmentPage() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
  const page = snapshot.equipment;
  if (!Array.isArray(page)) return [];
  const rows = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    const out = {};
    if (typeof row.name === 'string') out.name = row.name;
    if (cluePageI32(row.id)) out.id = row.id;
    if (cluePageI32(row.count)) out.count = row.count;
    if (cluePageI32(row.slot)) out.slot = row.slot;
    if (Object.keys(out).length > 0) rows.push(out);
  }
  return rows;
}
// The posted nearest Use-quickly booth the strip's and the restore's bank trip
// opens: the loc's own tile, id and — when the page posted them — display name
// and action, exactly as the landed bank helpers queue `open-booth`. A page
// that posted no booth (or no tile or id) hands the machine nothing at all: no
// stand is invented and no tile is copied.
function clueNearestBooth() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const row = snapshot.nearest_booth;
  if (!row || typeof row !== 'object' || Array.isArray(row)) return null;
  if (!cluePageI32(row.x) || !cluePageI32(row.z) || !cluePageI32(row.level)
      || !cluePageI32(row.id)) return null;
  const out = { x: row.x, z: row.z, level: row.level, id: row.id };
  if (typeof row.name === 'string') out.name = row.name;
  if (typeof row.op === 'string') out.op = row.op;
  return out;
}
// The posted bank interface, and only when the page posted the boolean: the
// strip's deposit and the restore's claim go out only behind a posted open
// bank, and an omitted slot is unobserved rather than closed.
function clueBankOpen() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return typeof snapshot.bank_open === 'boolean' ? snapshot.bank_open : null;
}
// The posted main modal id, and only when the page posted it: an omitted slot
// is not the closed `-1` and not a second definition of it, so the machine is
// handed no `main_modal_id` at all. Never `6960`, and never the
// `__rs2b0t_modals` machine.
function clueMainModalId() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return cluePageI32(snapshot.main_modal_id) ? snapshot.main_modal_id : null;
}
// The posted chat slots the talk arm reads: the open chat modal id — `-1` is
// the closed one the page posts itself — and the posted `chat_continue`. Both
// are posted only when the page carried them: an unobserved slot is not an open
// chat and not a close. Neither key is on `SNAPSHOT_KEYS`, so `api.snapshot`
// hides both and this reader goes to `host().snapshot` the way every other
// call-time page here does.
function clueChatModalId() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return cluePageI32(snapshot.chat_modal_id) ? snapshot.chat_modal_id : null;
}
function clueChatContinue() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return typeof snapshot.chat_continue === 'boolean' ? snapshot.chat_continue : null;
}
// The posted chat choices the acquire chain answers, read at call time like the
// other chat slots. Each row keeps its own 1-based posted slot — the machine
// answers that slot and never a position it re-derived — so a row that posted
// no text is dropped without renumbering the rows around it. A page that posted
// no `chat_options` array hands the machine nothing at all: an unobserved list
// is not an empty one, and `api.snapshot` hides the key anyway, so this reads
// `host().snapshot` like every other call-time page here.
function clueChatOptions() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const page = snapshot.chat_options;
  if (!Array.isArray(page)) return null;
  const rows = [];
  for (let i = 0; i < page.length; i += 1) {
    const row = page[i];
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (typeof row.text !== 'string') continue;
    rows.push({ text: row.text, option: i + 1 });
  }
  return rows;
}
// The posted count dialog, and only when the page posted the boolean: the talk
// arm answers a count only behind the posted open fact, and an omitted slot is
// unobserved rather than closed.
function clueCountDialogOpen() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return typeof snapshot.count_dialog_open === 'boolean' ? snapshot.count_dialog_open : null;
}
// The posted inv tab slot count. A page that did not post it hands the machine
// nothing: 28 is the client default, not this machine's to invent.
function clueInvSize() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return cluePageI32(snapshot.inv_size) ? snapshot.inv_size : null;
}
// The posted npc page the guarded encounter observes after its spawn: the
// posted index the Attack verb carries, the posted id and display name the row
// family's cap-documented wizard filter matches, the posted `x`/`z`/`level`
// tile the same-level filter and the absent-distance Chebyshev read are made
// from, the posted distance the frozen radius prefers, the posted health and
// target pairs the kill is read through and the posted `in_combat` flag. Read
// at call time like every other page — `api.snapshot.npcs` is the public
// projection and this machine never scans it. A row that did not post an index
// cannot be Attacked and is dropped here; every other absent field is posted
// as null and matches nothing.
function clueNpcPage() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
  const page = snapshot.npcs;
  if (!Array.isArray(page)) return [];
  const rows = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.index)) continue;
    rows.push({
      index: row.index,
      id: cluePageI32(row.id) ? row.id : null,
      name: typeof row.name === 'string' ? row.name : null,
      x: cluePageI32(row.x) ? row.x : null,
      z: cluePageI32(row.z) ? row.z : null,
      level: cluePageI32(row.level) ? row.level : null,
      distance: cluePageI32(row.distance) ? row.distance : null,
      health: cluePageI32(row.health) ? row.health : null,
      max_health: cluePageI32(row.max_health) ? row.max_health : null,
      in_combat: typeof row.in_combat === 'boolean' ? row.in_combat : null,
      actions: Array.isArray(row.actions)
        ? row.actions.filter((action) => typeof action === 'string')
        : [],
      target_kind: cluePageI32(row.target_kind) ? row.target_kind : null,
      target_index: cluePageI32(row.target_index) ? row.target_index : null,
    });
  }
  return rows;
}
// The posted local-player table slot and its own posted target pair: the
// encounter's `targetsMe` read and the health-0 ownership read. Posted only
// when the page carried it, so an absent slot is never read as zero.
function clueSelfSlot() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return cluePageI32(snapshot.self_slot) ? snapshot.self_slot : null;
}
// The local player's own posted target pair — `target_kind` `1` is an npc —
// which the health-0 kill read compares with the wizard this token owns. Both
// halves are needed, so a page that posted only one posts no target at all.
function clueSelfTarget() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  if (!cluePageI32(snapshot.self_target_kind) || !cluePageI32(snapshot.self_target_index)) {
    return null;
  }
  return { kind: snapshot.self_target_kind, index: snapshot.self_target_index };
}
// The posted Protect from Magic overlay: the selected prayer row's own varp,
// index 95 on both pins. Posted only when the varp page carried it, so the
// machine reads a missing overlay as unobserved rather than as off.
function clueVarp95() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const page = snapshot.varps;
  if (!Array.isArray(page)) return null;
  const row = page.find((v) => v && typeof v === 'object' && v.index === 95);
  return row && cluePageI32(row.value) ? row.value : null;
}
// The posted effective hitpoints of the local player, off the posted
// `snapshot.stats` page: the frozen mid-fight wait. A page that did not post
// the row is not a zero.
function clueHitpoints() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const page = snapshot.stats;
  if (!Array.isArray(page)) return null;
  const row = page.find((s) => s && typeof s === 'object' && s.name === 'hitpoints');
  return row && cluePageI32(row.effective) ? row.effective : null;
}
// The posted puzzle board the plan reads and the generation its click rides:
// the identified component, its observed slot count and its sparse rows, read
// at call time off `host().snapshot` exactly like every other page here. SNAP
// owns the observation — a closed board is the present `{-1, 0, []}`, an empty
// slot contributes no row, and this reader never fills a board to 25. A row
// that did not post an i32 slot and id is dropped here; the machine's own read
// rejects a board that is not 24 pieces around one gap, and it rejects a page
// with no generation rather than clicking on an invented one.
// The posted walk outcome's navigator-named gate shorts: the strict route's own
// `Carry` diagnosis, one row per `item_req` the posted pack could not prove. A
// row that is not the posted `(id, count)` pair cannot be a named short, and
// `name` rides along only as the string the host obj table resolved — the join
// is the id. A page that did not post the vector hands the machine nothing at
// all: an unobserved list is not an empty one, and the fail bit beside it is
// never a substitute for it.
function clueWalkMissingCarry() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const page = snapshot.walk_missing_carry;
  if (!Array.isArray(page)) return null;
  const rows = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.id) || !cluePageI32(row.count)) continue;
    rows.push({
      id: row.id,
      count: row.count,
      name: typeof row.name === 'string' ? row.name : null,
    });
  }
  return rows;
}
// The posted shop interface the gate-toll trip reads. Both slots are posted only
// when the page carried them: an omitted `shop_open` is unobserved rather than a
// closed interface, and a row the page posted without a clickable slot,
// component or display name is still posted as the observation it is — the
// machine skips it rather than this adapter guessing at one.
function clueShopOpen() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  return typeof snapshot.shop_open === 'boolean' ? snapshot.shop_open : null;
}
function clueShopStock() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const page = snapshot.shop_stock;
  if (!Array.isArray(page)) return null;
  const rows = [];
  for (const row of page) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.id)) continue;
    rows.push({
      id: row.id,
      name: typeof row.name === 'string' ? row.name : null,
      count: cluePageI32(row.count) ? row.count : null,
      slot: cluePageI32(row.slot) ? row.slot : null,
      component: cluePageI32(row.component_id) ? row.component_id : null,
    });
  }
  return rows;
}
function cluePuzzleBoard() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const board = snapshot.puzzle_board;
  if (!board || typeof board !== 'object' || Array.isArray(board)) return null;
  if (!cluePageI32(board.component_id) || !cluePageI32(board.size)) return null;
  if (!Array.isArray(board.items)) return null;
  const items = [];
  for (const row of board.items) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    if (!cluePageI32(row.slot) || !cluePageI32(row.id)) continue;
    items.push({ slot: row.slot, id: row.id });
  }
  return { component_id: board.component_id, size: board.size, items: items };
}
// The posted board session generation: `puzzle_board_generation` is the page's
// own session counter, so a non-negative safe integer is the whole of it a
// double can carry. A page that did not post one hands the machine nothing,
// and no click is sent on an invented session.
function cluePuzzleGeneration() {
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
  const generation = snapshot.puzzle_board_generation;
  return typeof generation === 'number' && Number.isSafeInteger(generation) && generation >= 0
    ? generation
    : null;
}
// Every continue step is this envelope, the completion kinds included: the
// machine's `kind` is what tells them apart, and `status: 'continue'` is this
// helper's own slot — never hunt's `status: 'done'`.
function clueStep(step) {
  return { ok: true, status: 'continue', token: step.token, ...step };
}
// A dead token is `stale`; the generation abort the machine reported is
// `aborted`. The identify family's own tokens — the same three `begin`
// preserves — survive here too: a live session that loses its held
// membership reports `none-held`, not `stale`, and the packed 3554
// `access: "constrained"` row reports `constrained`. Internal reasons are
// never handed out as an ok kind.
function clueStepError(reason) {
  if (reason === 'aborted' || reason === 'constrained') return reason;
  if (reason === 'missing-selected-data' || reason === 'family-unavailable:trails'
      || reason === 'none-held') return reason;
  return 'stale';
}
// The begin refusals are the identify family's own tokens, the constrained
// row's own refusal and the abandon latch's: the row this machine left in the
// pack is refused while it is still the one held, so a later pickup of it needs
// a `retry` or a different held row. Anything else internal is `stale`.
function clueBeginError(reason) {
  if (reason === 'constrained' || reason === 'abandoned') return reason;
  if (reason === 'missing-selected-data' || reason === 'family-unavailable:trails'
      || reason === 'none-held') return reason;
  return 'stale';
}
// The machine's own walk, held, npc, if-button, chat and shop steps go onto the
// shared interact drain the way the quest journal enqueues `if-button` /
// `close-modal` and the landed npc consumers enqueue `npc`: a generation check,
// then a push. Each kind has its own explicit arm before the loc fall-through,
// so a held item identity is never enqueued as a loc, an `obj` Take is never
// enqueued as a loc, the closed-handler `continue` / `answer` pair is never
// enqueued as one, a `shop-button` click is never enqueued as one, and the loc
// row always carries the posted id. `held` is
// already an author `V2_OPS` verb, but this machine enqueues its own step
// directly instead of going through `enqueueRequest`; `loc`, `obj`,
// `close-modal`, `npc`, `if-button`, `continue`, `answer` and `shop-button` are
// not `V2_OPS`
// verbs at all, so `api.request({ op: 'npc' })`, `api.request({ op: 'continue' })`,
// `api.request({ op: 'answer' })` and `api.request({ op: 'shop-button' })` stay
// `not impl`.
function enqueueClueVerb(step) {
  const h = host();
  h.interact = h.interact || [];
  if (step.kind === 'walk') {
    h.interact.push({ op: 'walk', x: step.x, z: step.z, level: step.level });
    return;
  }
  if (step.kind === 'if-button') {
    // The landed generic if-button push the prayer isolate and the quest
    // journal already use: the selected Protect from Magic component id.
    h.interact.push({ op: 'if-button', component_id: step.component_id });
    return;
  }
  if (step.kind === 'npc') {
    // The posted identity the Attack rides: the posted name, the action the
    // machine dispatched and the posted scene index. The arm is generic, not
    // Attack-hardcoded — the action is whatever the step carried.
    h.interact.push({
      op: 'npc',
      name: step.name,
      action: step.action,
      index: step.index,
    });
    return;
  }
  if (step.kind === 'answer-count') {
    // The open count dialog the talk arm's challenge step answers: the selected
    // answer the machine parsed, and nothing else. Its own arm, before the loc
    // fall-through, and never a `V2_OPS` request.
    h.interact.push({ op: 'answer-count', value: step.value });
    return;
  }
  if (step.kind === 'continue') {
    // The open giver chat the acquire chain continues: the landed
    // `ContinueDialog` step, with no option and no text of its own. Its own arm,
    // and never a `V2_OPS` request: `continue` is not an author verb.
    h.interact.push({ op: 'continue' });
    return;
  }
  if (step.kind === 'answer') {
    // The posted 1-based option slot the acquire chain answers: the machine's
    // own slot, never a position re-derived here and never the last option of a
    // list it could not match. Never a `V2_OPS` request either.
    h.interact.push({ op: 'answer', option: step.option });
    return;
  }
  if (step.kind === 'close-modal') {
    // The journal's own close push: the main modal this machine's Collecting
    // arm saw posted open, and nothing else.
    h.interact.push({ op: 'close-modal' });
    return;
  }
  if (step.kind === 'obj') {
    // The casket overflow on the posted tile, by the posted name and action.
    // The host matches that identity and refuses a stale row rather than
    // taking another ground row on the same tile.
    h.interact.push({
      op: 'obj',
      x: step.x,
      z: step.z,
      level: step.level,
      name: step.name,
      action: step.action,
    });
    return;
  }
  if (step.kind === 'puzzle-move') {
    // The exact posted board row: its own id, the slot it sits in, the posted
    // component and this call's board generation. The host re-resolves that
    // identity under the board session and refuses a closed board, a stale
    // slot, a wrong-size board and a stale generation; nothing else rides
    // along and no reply is expected.
    h.interact.push({
      op: 'puzzle-move',
      id: step.id,
      slot: step.slot,
      component: step.component,
      generation: step.generation,
    });
    return;
  }
  if (step.kind === 'shop-button') {
    // The open shop interface's own click: the posted stock row's identity and
    // one chunk of one. The landed `shop-button` op and nothing else — its own
    // explicit arm before the loc fall-through, never a `kind: "ops"` shopping
    // list, never a `V2_OPS` author verb (`api.request({ op: 'shop-button' })`
    // stays `not impl`), and never a nested `shop.rs` batch.
    h.interact.push({
      op: 'shop-button',
      kind: step.shop,
      name: step.name,
      id: step.id,
      slot: step.slot,
      component: step.component,
      chunk: step.chunk,
    });
    return;
  }
  if (step.kind === 'held') {
    // The selected item display name; the host resolves the first inventory
    // row with the name. No row id and no tile rides along. The Collecting
    // Drop reuses this arm with `action: 'Drop'`.
    h.interact.push({ op: 'held', name: step.name, action: step.action });
    return;
  }
  if (step.kind === 'unequip') {
    // The landed worn-row verb the shim's `Equipment.unequip` queues: the
    // worn component's own `Remove` for the display name the Entrana strip
    // takes off. `wear` resolves inventory rows alone, so the strip never
    // rides that one. Not a `V2_OPS` author verb — `api.request({ op:
    // 'unequip' })` is `not impl` — and only the display name rides it.
    h.interact.push({ op: 'unequip', name: step.name });
    return;
  }
  if (step.kind === 'wear') {
    // The landed equip-from-pack verb the Entrana restore puts a listed name
    // back on with: the display name and nothing else. Not a `V2_OPS` author
    // verb either — `api.request({ op: 'wear' })` is `not impl`, and this
    // machine enqueues its own step directly.
    h.interact.push({ op: 'wear', name: step.name });
    return;
  }
  if (step.kind === 'deposit') {
    // The landed bank-side deposit by display name: the restricted names the
    // strip put in the pack, and never the ordinary loot deposit.
    h.interact.push({ op: 'deposit', name: step.name });
    return;
  }
  if (step.kind === 'withdraw') {
    // The landed bank withdraw for one listed name with the machine's own
    // posted action label (`Withdraw-1`).
    h.interact.push({ op: 'withdraw', name: step.name, action: step.action });
    return;
  }
  if (step.kind === 'walk-nearest-bank') {
    // The landed Rust-picked stand walk: no tile rides it, because the machine
    // never invents one.
    h.interact.push({ op: 'walk-nearest-bank' });
    return;
  }
  if (step.kind === 'open-booth') {
    // The posted booth's own identity, the way the landed bank helpers queue
    // it: no stand is picked here and no tile is copied.
    const open = { op: 'open-booth', x: step.x, z: step.z, level: step.level, id: step.id };
    if (typeof step.name === 'string') open.name = step.name;
    if (typeof step.action === 'string') open.action = step.action;
    h.interact.push(open);
    return;
  }
  if (step.kind === 'close') {
    // The open bank interface's own close, after the strip's deposit or the
    // restore's claim.
    h.interact.push({ op: 'close' });
    return;
  }
  // The completion envelope is not a verb: `done`, `grind-ready`, `dead`,
  // `abandon`, `supplies-needed`, `no-shop` and `guardian-lost` are `next` kinds
  // and never pushed onto the interact drain. The explicit returns keep them off
  // the loc fall-through, so an unknown kind is never enqueued as a loc either.
  if (step.kind === 'done' || step.kind === 'grind-ready' || step.kind === 'dead'
      || step.kind === 'abandon' || step.kind === 'supplies-needed'
      || step.kind === 'no-shop' || step.kind === 'guardian-lost') {
    return;
  }
  h.interact.push({
    op: 'loc',
    x: step.x,
    z: step.z,
    level: step.level,
    action: step.action,
    id: step.id,
  });
}
api.clue = {
  row: function (input) {
    if (arguments.length === 0) return helperErr('invalid-args');
    if (input == null || typeof input !== 'object' || Array.isArray(input)) {
      return helperErr('invalid-args');
    }
    const hasId = Object.prototype.hasOwnProperty.call(input, 'id');
    const hasAlias = Object.prototype.hasOwnProperty.call(input, 'alias');
    if (hasId === hasAlias) return helperErr('invalid-args');
    if (hasId) {
      // The packed id is a real i32: "3554", 3554.5, a bigint, and
      // new Number(3554) are invalid-args. A present 0 is still a pin.
      const id = input.id;
      if (typeof id !== 'number' || !Number.isInteger(id)
          || id < -2147483648 || id > 2147483647) {
        return helperErr('invalid-args');
      }
    } else if (typeof input.alias !== 'string') {
      return helperErr('invalid-args');
    }
    return clueV2('row', input);
  },
  // Zero parameters: the page is the already-posted snapshot.inv, never an
  // argument. Extra arguments are ignored, not an inventory override.
  heldStep: function () {
    return clueLogicV2('heldStep');
  },
  // Pure slot arithmetic over the caller's own numbers: no page, no
  // inventory, no family. Absent fields default, a present wrong type is
  // invalid-args, and no-room is the whole result when the caller asked for
  // food and the pack cannot spare a slot.
  packPlan: function (input) {
    return cluePackV2(input);
  },
  // Pure kit status over the caller's own facts: no snapshot, no quest tab,
  // no inventory, no equipment, no bank. Every field is required, so an
  // omitted attack or lostCity is invalid-args rather than a default, and a
  // failed check is the whole result.
  hardKit: function (input) {
    return clueHardKitV2(input);
  },
  // Pure keep predicate over the caller's own names: no snapshot, no
  // inventory, no bank page, and no trail family. `name` is required, so
  // keep({}) is invalid-args rather than a missing name; `extra` is optional,
  // additive only, and compared by ASCII equality. A miss is ok false, never
  // an error.
  keep: function (input) {
    return clueKeepV2(input);
  },
  // Owned-session begin over the landed held-step identify. Sync
  // HelperResult: a refused begin — no held membership row, no selected pin,
  // no trail family, the packed `access: "constrained"` row, or the row this
  // machine left in the pack (`abandoned`, cleared by a different held row or
  // by `retry`) — leaves no live token, so a later pickup needs a new begin.
  // Extra input keys are ignored, not captured.
  begin: function (input) {
    const step = clueCall({
      op: 'begin',
      generation: lifecycleGeneration,
      held: clueHeldPage(),
    });
    if (!step || typeof step !== 'object') return helperErr('stale');
    if (step.kind === 'token') return helperOk({ token: step.token });
    if (step.kind === 'aborted') return helperErr(clueBeginError(step.reason));
    return helperErr('stale');
  },
  // One step. `resume` is the callback return (hunt's `reply` slot under this
  // name; both are never read). The wrapper marshals the call-time pages —
  // the parked `snapshot.inv` page, the posted `here` tile, the posted loc
  // page, and for the trail-end collect the posted ground page, the posted
  // pack rows with their slot count and the posted main modal — so the
  // machine never caches a world copy. The guarded encounter adds the posted
  // npc page, the local-player slot and target pair, the posted Protect from
  // Magic overlay and the posted effective hitpoints. The talk arm adds the
  // posted chat modal, its continue flag and the posted count dialog, and the
  // trio acquire chain adds the posted `chat_options` choices, each with its
  // own 1-based posted slot. The Entrana strip and its restore add the posted
  // worn `equipment` rows and the posted bank facts — `nearest_booth` and
  // `bank_open` — and their own `unequip`, `wear`, `deposit`, `withdraw`,
  // `walk-nearest-bank`, `open-booth` and `close` steps. Kinds
  // are `wait`, `yield`, `callback.enabled`, `callback.log`,
  // `callback.setStatus`, `held`, `walk`, `loc`, `close-modal`, `obj`, `npc`,
  // `if-button`, `answer-count`, `continue`, `answer` and the completion
  // envelope — `grind-ready`,
  // `supplies-needed`, `no-shop`, `done`, `dead`, `abandon` and `guardian-lost`.
  // The
  // completion kinds ride this same continue shape and are never enqueued;
  // `done` is the finished collect's own kind, not a hunt `status: 'done'`,
  // and the exact `'clue solved'` string is the machine's own
  // `callback.setStatus` message. The puzzle-box
  // arm adds the posted board and its session generation, and its
  // `puzzle-move` kind. The gate-toll trip adds the posted walk outcome's
  // navigator-named `walk_missing_carry` shorts and the posted shop interface
  // (`shop_open`, `shop_stock`), and its `shop-button` kind — the landed
  // `InteractReq::ShopButton` click on one posted stock row, never a
  // `kind: "ops"` list and never a `V2_OPS` request. `no-shop` is the named
  // wait-class a trip that could not observe the short's own Trade, interface
  // or stock row ends with: the token lives. A `walk`, `held`, `loc`,
  // `close-modal`, `obj`, `npc`,
  // `if-button`, `answer-count`, `puzzle-move`, `continue`, `answer` or
  // `shop-button` step is
  // enqueued onto the
  // interact drain like the journal's `if-button`, and the step is still
  // returned as a continue object. A dead token is the error object, never
  // `undefined` and never an `aborted` continue kind.
  next: function (input) {
    if (arguments.length === 0) return helperErr('invalid-args');
    if (input == null || typeof input !== 'object' || Array.isArray(input)) {
      return helperErr('invalid-args');
    }
    if (!Number.isInteger(input.token)) return helperErr('invalid-args');
    const hasResume = Object.prototype.hasOwnProperty.call(input, 'resume');
    if (hasResume && typeof input.resume !== 'boolean') return helperErr('invalid-args');
    const generation = lifecycleGeneration;
    const payload = {
      op: 'next',
      token: input.token,
      generation: generation,
      held: clueHeldPage(),
      hold: cluePending(),
      locs: clueLocPage(),
      ground: clueGroundPage(),
      inv: clueInvPage(),
      npcs: clueNpcPage(),
      // The worn page the Entrana strip reads, always present the way `inv` is:
      // a page that posted nothing is an empty list, and never a second
      // `Equipment.items()` read.
      equipment: clueEquipmentPage(),
    };
    const here = clueHereTile();
    if (here !== null) payload.here = here;
    // The strip's and the restore's own bank facts, posted-only like the chat
    // slots: an omitted booth or bank slot stays unobserved on the machine
    // rather than becoming an invented stand or a closed bank.
    const nearestBooth = clueNearestBooth();
    if (nearestBooth !== null) payload.nearest_booth = nearestBooth;
    const bankOpen = clueBankOpen();
    if (bankOpen !== null) payload.bank_open = bankOpen;
    const main = clueMainModalId();
    if (main !== null) payload.main_modal_id = main;
    // The talk arm's own call-time facts, posted-only like `main_modal_id`: an
    // omitted chat or count slot is unobserved, never a closed one.
    const chatModal = clueChatModalId();
    if (chatModal !== null) payload.chat_modal_id = chatModal;
    const chatContinue = clueChatContinue();
    if (chatContinue !== null) payload.chat_continue = chatContinue;
    // The acquire chain's own posted choices, posted-only like the slots above:
    // an unobserved list stays unobserved on the machine rather than becoming
    // an empty one it could read a close into.
    const chatOptions = clueChatOptions();
    if (chatOptions !== null) payload.chat_options = chatOptions;
    const countOpen = clueCountDialogOpen();
    if (countOpen !== null) payload.count_dialog_open = countOpen;
    const invSize = clueInvSize();
    if (invSize !== null) payload.inv_size = invSize;
    const selfSlot = clueSelfSlot();
    if (selfSlot !== null) payload.self_slot = selfSlot;
    const selfTarget = clueSelfTarget();
    if (selfTarget !== null) {
      payload.self_target_kind = selfTarget.kind;
      payload.self_target_index = selfTarget.index;
    }
    const hitpoints = clueHitpoints();
    if (hitpoints !== null) payload.hitpoints = hitpoints;
    const varp95 = clueVarp95();
    if (varp95 !== null) payload.varp95 = varp95;
    const puzzleBoard = cluePuzzleBoard();
    if (puzzleBoard !== null) payload.puzzle_board = puzzleBoard;
    const puzzleGeneration = cluePuzzleGeneration();
    if (puzzleGeneration !== null) payload.puzzle_board_generation = puzzleGeneration;
    // The walk outcome's own navigator-named shorts, and the posted shop
    // interface the gate-toll trip reads: all three are posted-only, so a slot
    // the page did not carry stays unobserved on the machine rather than
    // becoming a named short, a closed interface or an empty stock.
    const missingCarry = clueWalkMissingCarry();
    if (missingCarry !== null) payload.walk_missing_carry = missingCarry;
    const shopOpen = clueShopOpen();
    if (shopOpen !== null) payload.shop_open = shopOpen;
    const shopStock = clueShopStock();
    if (shopStock !== null) payload.shop_stock = shopStock;
    if (hasResume) payload.resume = input.resume;
    const step = clueCall(payload);
    if (!step || typeof step !== 'object') return helperErr('stale');
    if (step.kind === 'aborted') return helperErr(clueStepError(step.reason));
    if (step.kind === 'walk' || step.kind === 'held' || step.kind === 'loc'
        || step.kind === 'close-modal' || step.kind === 'obj'
        || step.kind === 'npc' || step.kind === 'if-button'
        || step.kind === 'answer-count' || step.kind === 'puzzle-move'
        || step.kind === 'continue' || step.kind === 'answer'
        || step.kind === 'shop-button' || step.kind === 'wear'
        || step.kind === 'unequip'
        || step.kind === 'deposit' || step.kind === 'withdraw'
        || step.kind === 'walk-nearest-bank' || step.kind === 'open-booth'
        || step.kind === 'close') {
      // Enqueue synchronously, after the generation check: a reset or stop
      // between the call and this push is not a verb for the dead session.
      if (generation !== lifecycleGeneration) return helperErr('stale');
      enqueueClueVerb(step);
      return clueStep(step);
    }
    if (step.kind === 'wait' || step.kind === 'yield'
        || step.kind === 'callback.enabled' || step.kind === 'callback.log'
        || step.kind === 'callback.setStatus') {
      return clueStep(step);
    }
    // The completion envelope rides the same continue shape: `grind-ready` is
    // a live-token continue, `supplies-needed` the arrived dig's wait-class,
    // `no-shop` the gate-toll trip's own wait-class, and `done` / `dead` /
    // `abandon` / `guardian-lost` are terminal kinds the machine emits once.
    // None of them is a verb and none is enqueued.
    if (step.kind === 'grind-ready' || step.kind === 'supplies-needed'
        || step.kind === 'no-shop' || step.kind === 'done' || step.kind === 'dead'
        || step.kind === 'abandon' || step.kind === 'guardian-lost') {
      return clueStep(step);
    }
    return helperErr('stale');
  },
  // The frozen `retry()` on this machine's own seat: the abandon latch's clear
  // and nothing else. Sync, not a Promise, not a `request()` op, not a new host
  // op — and never `on_reset`: the live token is not aborted, `strippedGear` is
  // not cleared and no other latch exists yet. `{ cleared: true }` is the whole
  // value.
  retry: function () {
    const step = clueCall({ op: 'retry' });
    if (!step || typeof step !== 'object' || step.kind !== 'retry') {
      return helperErr('stale');
    }
    return helperOk({ cleared: true });
  },
};
const SCENE_LIMIT_MAX = 64;
function sceneLimitOk(value) {
  return Number.isInteger(value) && value >= 1 && value <= SCENE_LIMIT_MAX;
}
function sceneRegionValue(value) {
  if (value == null || typeof value !== 'object' || Array.isArray(value)) return null;
  for (const key of ['min_x', 'min_z', 'max_x', 'max_z', 'level']) {
    if (!Number.isInteger(value[key])) return null;
  }
  return {
    min_x: value.min_x, min_z: value.min_z,
    max_x: value.max_x, max_z: value.max_z, level: value.level,
  };
}
// A present region must be a box on one level. Omitted is undefined, a bad
// value is null. There is no plane and no { cx, cz, radius } form.
function sceneRegionArg(input) {
  if (!Object.prototype.hasOwnProperty.call(input, 'region')) return undefined;
  return sceneRegionValue(input.region);
}
function sceneBounds(collision) {
  // Bounds only. Collision flags stay on the page and are not this result.
  return {
    available: collision.available,
    base_x: collision.base_x, base_z: collision.base_z,
    level: collision.level,
    width: collision.width, height: collision.height,
  };
}
function sceneRowMatched(entity, ids, actions, region) {
  if (!entity || typeof entity !== 'object') return false;
  if (ids.indexOf(entity.id) === -1) return false;
  if (region && !(entity.level === region.level
      && entity.x >= region.min_x && entity.x <= region.max_x
      && entity.z >= region.min_z && entity.z <= region.max_z)) return false;
  if (!actions) return true;
  const posted = entity.actions;
  if (!Array.isArray(posted)) return false;
  for (const action of posted) {
    if (actions.indexOf(action) !== -1) return true;
  }
  return false;
}
function sceneProjection(key, ids, actions, limit, region) {
  // The copy reads host().snapshot. api.snapshot hides locs and tick, and a
  // missing host page is snapshot-unavailable, not an empty rows list.
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) {
    return helperErr('snapshot-unavailable');
  }
  const collision = snapshot.collision;
  if (!collision || typeof collision !== 'object' || Array.isArray(collision)) {
    return helperErr('snapshot-unavailable');
  }
  // Unavailable collision wins over an empty posted array (post_base).
  if (collision.available !== true) return helperErr('snapshot-unavailable');
  if (typeof snapshot.tick !== 'number') return helperErr('snapshot-unavailable');
  const posted = snapshot[key];
  if (!Array.isArray(posted)) return helperErr('snapshot-unavailable');
  const rows = [];
  let truncated = false;
  for (const entity of posted) {
    if (!sceneRowMatched(entity, ids, actions, region)) continue;
    // Posted order. More matches than limit set truncated and drop the rest.
    if (rows.length >= limit) { truncated = true; break; }
    rows.push({
      id: entity.id,
      x: entity.x, z: entity.z, level: entity.level,
      actions: Array.isArray(entity.actions) ? entity.actions.slice() : [],
    });
  }
  return helperOk({
    as_of_sequence: snapshot.tick,
    scene: sceneBounds(collision),
    rows: rows,
    truncated: truncated,
  });
}
api.sceneLocs = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  const ids = input.ids;
  if (ids == null) return helperErr('missing-ids');
  if (!Array.isArray(ids)) return helperErr('invalid-args');
  if (ids.length === 0) return helperErr('missing-ids');
  for (const id of ids) {
    if (!Number.isInteger(id)) return helperErr('invalid-args');
  }
  if (!sceneLimitOk(input.limit)) return helperErr('invalid-args');
  const region = sceneRegionArg(input);
  if (region === null) return helperErr('invalid-args');
  return sceneProjection('locs', ids, null, input.limit, region);
};
api.sceneNpcs = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  const types = input.types;
  if (!Array.isArray(types) || types.length === 0) return helperErr('invalid-args');
  for (const type of types) {
    if (!Number.isInteger(type)) return helperErr('invalid-args');
  }
  // actions is required. Omitted is not match-any.
  const actions = input.actions;
  if (!Array.isArray(actions) || actions.length === 0) return helperErr('invalid-args');
  for (const action of actions) {
    if (typeof action !== 'string' || action === '') return helperErr('invalid-args');
  }
  if (!sceneLimitOk(input.limit)) return helperErr('invalid-args');
  const region = sceneRegionArg(input);
  if (region === null) return helperErr('invalid-args');
  return sceneProjection('npcs', types, actions, input.limit, region);
};
const QUEST_STATUS_VALUES = ['notStarted', 'inProgress', 'complete', 'unknown'];
// A-Z fold only. Not String.prototype.toLowerCase, which is Unicode and is
// v1's own miss path.
function questStatusFold(text) {
  let out = '';
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    out += String.fromCharCode(code >= 97 && code <= 122 ? code - 32 : code);
  }
  return out;
}
function questStatusRow(row, wanted) {
  // One junk row does not fail the page: skip a non-object, a non-string
  // name, and a status outside the four posted strings. The posted status is
  // copied, never rewritten.
  if (!row || typeof row !== 'object' || Array.isArray(row)) return null;
  if (typeof row.name !== 'string') return null;
  if (QUEST_STATUS_VALUES.indexOf(row.status) === -1) return null;
  return questStatusFold(row.name.trim()) === wanted ? row.status : null;
}
api.questStatus = function (input) {
  // Args first: a bad call is never snapshot-unavailable and never
  // quest-tab-unbound.
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (typeof input.name !== 'string') return helperErr('invalid-args');
  // A present id is not a query key, even beside a legal name.
  if (Object.prototype.hasOwnProperty.call(input, 'id')) {
    return helperErr('invalid-args');
  }
  const wanted = questStatusFold(input.name.trim());
  if (wanted === '') return helperErr('invalid-args');
  // The copy reads host().snapshot. api.snapshot hides quest_statuses and
  // tick, its getter substitutes {} for a missing page, and host() with no
  // global is { interact: [], log: [] }: a missing page is
  // snapshot-unavailable, not an unbound tab.
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) {
    return helperErr('snapshot-unavailable');
  }
  const posted = snapshot.quest_statuses;
  // post_base encodes a null tab, and only a null tab is unbound. Strict
  // equality: a missing key is not null, and neither is a non-array.
  if (posted === null) return helperErr('quest-tab-unbound');
  if (!Array.isArray(posted)) return helperErr('snapshot-unavailable');
  // Null was checked before the tick: an unbound tab needs no sequence. On a
  // bound tab a tick that is not a finite number is snapshot-unavailable.
  const sequence = snapshot.tick;
  if (typeof sequence !== 'number' || !Number.isFinite(sequence)) {
    return helperErr('snapshot-unavailable');
  }
  // Posted order, first legal match. A miss is not-on-tab: do not invent
  // unknown and do not join the identity table to fill it.
  for (const row of posted) {
    const status = questStatusRow(row, wanted);
    if (status !== null) return helperOk({ status: status, as_of_sequence: sequence });
  }
  return helperErr('not-on-tab');
};
// Owned-root quest journal. One token per isolate: Begin clicks the posted
// row id once, Next returns the acquired pair, Close closes only the root
// this token opened. The pair is the only occupancy fact, so these read
// host().snapshot — the page the materializer wrote. api.snapshot hides
// quest_statuses, main_modal_texts and tick, and it substitutes {} for a
// missing page. These three are not Promises and never return a verb kind.
function questJournalCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_quest_journal(payload);
}
function enqueueCloseModal() {
  const h = host();
  h.interact = h.interact || [];
  h.interact.push({ op: 'close-modal' });
}
function questJournalPage() {
  const page = host().snapshot;
  if (!page || typeof page !== 'object' || Array.isArray(page)) {
    return { error: 'snapshot-unavailable' };
  }
  // An omitted pair is not closed and not free: a first post that omits the
  // slot writes no property at all, and a delta that omits it keeps the last
  // pair. main_modal_id is not a second closed definition.
  if (!Object.prototype.hasOwnProperty.call(page, 'main_modal_texts')) {
    return { error: 'snapshot-unavailable' };
  }
  const pair = page.main_modal_texts;
  if (!pair || typeof pair !== 'object' || Array.isArray(pair)) {
    return { error: 'snapshot-unavailable' };
  }
  if (!Array.isArray(pair.texts) || !Number.isInteger(pair.root)) {
    return { error: 'snapshot-unavailable' };
  }
  for (const line of pair.texts) {
    if (typeof line !== 'string') return { error: 'snapshot-unavailable' };
  }
  // as_of_sequence is the posted tick, never api.tick (which stamps 0).
  const sequence = page.tick;
  if (typeof sequence !== 'number' || !Number.isFinite(sequence)) {
    return { error: 'snapshot-unavailable' };
  }
  return { root: pair.root, texts: pair.texts, sequence: sequence };
}
function questJournalRow(posted, wanted) {
  // The same junk predicate questStatus skips. The first legal match wins,
  // and the scan does not continue past it because its id was omitted.
  for (const row of posted) {
    if (questStatusRow(row, wanted) !== null) return row;
  }
  return null;
}
function questJournalClickTarget(row) {
  // The click is the row's posted component_id. An omitted slot omits the
  // property — it is not 0, and a present 0 is a real id that is clicked.
  if (!Object.prototype.hasOwnProperty.call(row, 'component_id')) return null;
  return Number.isInteger(row.component_id) ? row.component_id : null;
}
function questJournalBeginError(reason) {
  // A begin refusal is one of the begin errors; an internal aborted step is
  // never handed out as `aborted`.
  if (reason === 'busy' || reason === 'main-modal-occupied') return reason;
  if (reason === 'snapshot-unavailable') return reason;
  return 'stale';
}
function questJournalStepError(reason) {
  if (reason === 'snapshot-unavailable' || reason === 'modal-timeout') return reason;
  return 'stale';
}
api.questJournalBegin = function (input) {
  // Args first: a bad call is never snapshot-unavailable, never
  // quest-tab-unbound and never unknown-quest.
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (typeof input.name !== 'string') return helperErr('invalid-args');
  // A present id is not a begin key, even beside a legal name.
  if (Object.prototype.hasOwnProperty.call(input, 'id')) return helperErr('invalid-args');
  const wanted = questStatusFold(input.name.trim());
  if (wanted === '') return helperErr('invalid-args');
  const snapshot = host().snapshot;
  if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) {
    return helperErr('snapshot-unavailable');
  }
  const posted = snapshot.quest_statuses;
  // Only a null tab is unbound, and an unbound tab needs no sequence.
  if (posted === null) return helperErr('quest-tab-unbound');
  if (!Array.isArray(posted)) return helperErr('snapshot-unavailable');
  const page = questJournalPage();
  if (page.error) return helperErr(page.error);
  const row = questJournalRow(posted, wanted);
  if (row === null) return helperErr('unknown-quest');
  const componentId = questJournalClickTarget(row);
  if (componentId === null) {
    // No posted target: no click, and never a written 0.
    return helperErr('snapshot-unavailable');
  }
  const generation = lifecycleGeneration;
  const step = questJournalCall({
    op: 'begin',
    name: wanted,
    component_id: componentId,
    sequence: page.sequence,
    generation: generation,
    root: page.root,
    texts: page.texts,
  });
  if (!step || typeof step !== 'object') return helperErr('stale');
  if (step.kind === 'if-button') {
    // Enqueue synchronously, after the generation check.
    if (generation !== lifecycleGeneration) return helperErr('stale');
    enqueueIfButton(step.component_id);
    return helperOk({ token: step.token });
  }
  if (step.kind === 'aborted') return helperErr(questJournalBeginError(step.reason));
  return helperErr('stale');
};
api.questJournalNext = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (!Number.isInteger(input.token)) return helperErr('invalid-args');
  const page = questJournalPage();
  if (page.error) return helperErr(page.error);
  const step = questJournalCall({
    op: 'next',
    token: input.token,
    generation: lifecycleGeneration,
    sequence: page.sequence,
    root: page.root,
    texts: page.texts,
  });
  if (!step || typeof step !== 'object') return helperErr('stale');
  // Not-done is { pending: true } with no ok field. It is not empty lines.
  if (step.kind === 'wait') return { pending: true };
  if (step.kind === 'done') {
    return helperOk({
      lines: step.lines,
      root: step.root,
      as_of_sequence: step.as_of_sequence,
    });
  }
  if (step.kind === 'aborted') return helperErr(questJournalStepError(step.reason));
  return helperErr('stale');
};
api.questJournalClose = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (!Number.isInteger(input.token)) return helperErr('invalid-args');
  const page = questJournalPage();
  if (page.error) return helperErr(page.error);
  const generation = lifecycleGeneration;
  const step = questJournalCall({
    op: 'close',
    token: input.token,
    generation: generation,
    sequence: page.sequence,
    root: page.root,
    texts: page.texts,
  });
  if (!step || typeof step !== 'object') return helperErr('stale');
  if (step.kind === 'close-modal') {
    if (generation !== lifecycleGeneration) return helperErr('stale');
    enqueueCloseModal();
    return { pending: true };
  }
  if (step.kind === 'wait') return { pending: true };
  if (step.kind === 'done') {
    // Only the explicit closed pair. Close never returns journal lines.
    return helperOk({ closed: true, as_of_sequence: step.as_of_sequence });
  }
  if (step.kind === 'aborted') return helperErr(questJournalStepError(step.reason));
  return helperErr('stale');
};
function loadoutV2(op, input) {
  return globalThis.__rs2b0t_loadout_v2(op, input);
}
api.foodOf = function (input) {
  if (!input || typeof input !== 'object' || typeof input.fallback !== 'string') {
    return helperErr('invalid-args');
  }
  return loadoutV2('foodOf', input);
};
api.gearOf = function (input) {
  if (!input || typeof input !== 'object') return helperErr('invalid-args');
  return loadoutV2('gearOf', input);
};
api.suppliesOf = function (input) {
  if (!input || typeof input !== 'object') return helperErr('invalid-args');
  return loadoutV2('suppliesOf', input);
};
api.weaponOf = function (input) {
  if (!input || typeof input !== 'object') return helperErr('invalid-args');
  return loadoutV2('weaponOf', input);
};
api.rangeLoadoutOf = function (input) {
  if (!input || typeof input.weapon !== 'string' || typeof input.ammo !== 'string') {
    return helperErr('invalid-args');
  }
  return loadoutV2('rangeLoadoutOf', input);
};
api.boostFaded = function (input) {
  if (!input || typeof input.base !== 'number' || typeof input.effective !== 'number') {
    return helperErr('invalid-args');
  }
  if (input.floor !== undefined && typeof input.floor !== 'number') {
    return helperErr('invalid-args');
  }
  return loadoutV2('boostFaded', input);
};
api.plannedPotions = function (input) {
  if (!input || !Array.isArray(input.carry)) return helperErr('invalid-args');
  return loadoutV2('plannedPotions', input);
};
api.potionToSip = function (input) {
  if (!input || !Array.isArray(input.plans) || !Array.isArray(input.held) || !Array.isArray(input.levels)) {
    return helperErr('invalid-args');
  }
  return loadoutV2('potionToSip', input);
};
api.lineOfSight = function (input) {
  return globalThis.__rs2b0t_line_of_sight('v2', input);
};
api.walkable = function (input) {
  return globalThis.__rs2b0t_reach('v2-walkable', input);
};
api.canStep = function (input) {
  return globalThis.__rs2b0t_reach('v2-canStep', input);
};
api.canReach = function (input) {
  return globalThis.__rs2b0t_reach('v2-canReach', input);
};
function fightCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_fight(payload);
}
api.fightBegin = function (input) {
  const out = fightCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.fightValidate = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = fightCall({ op: 'validate', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(out === true || out?.value === true);
};
api.fightNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = fightCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield' };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
api.fightReset = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = fightCall({ op: 'reset', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(null);
};
api.fightInterruptWatch = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = fightCall({ op: 'interruptWatch', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(null);
};
api.fightBlocksLoot = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = fightCall({ op: 'blocksLoot', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(out === true || out?.value === true);
};
function holdCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_hold(payload);
}
api.holdBegin = function (input) {
  const out = holdCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.holdValidate = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = holdCall({ op: 'validate', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(out === true || out?.value === true);
};
api.holdNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = holdCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield' };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function retreatCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_retreat(payload);
}
api.retreatBegin = function (input) {
  const out = retreatCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.retreatValidate = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = retreatCall({ op: 'validate', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(out === true || out?.value === true);
};
api.retreatNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = retreatCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield' };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function walkspotCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_walkspot(payload);
}
api.walkspotBegin = function (input) {
  const out = walkspotCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.walkspotValidate = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = walkspotCall({ op: 'validate', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(out === true || out?.value === true);
};
api.walkspotNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = walkspotCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield' };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function enterCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_enter(payload);
}
api.enterBegin = function (input) {
  const out = enterCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.enterValidate = function (input) {
  if (!input || input.token == null) return helperErr('invalid-args');
  const out = enterCall({ op: 'validate', ...input });
  if (out && out.kind === 'aborted') return helperErr(out.reason || 'aborted');
  return helperOk(out === true || out?.value === true);
};
api.enterNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = enterCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield', value: out.value === true };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function leaveCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_leave(payload);
}
api.leaveBegin = function (input) {
  const out = leaveCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.leaveNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = leaveCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield', value: out.value === true };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function keyCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_key(payload);
}
api.keyBegin = function (input) {
  const out = keyCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.keyNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = keyCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield', value: out.value === true };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function cellCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_cell(payload);
}
api.cellBegin = function (input) {
  const out = cellCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.cellNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = cellCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield', value: out.value === true };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
function bankCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_bank(payload);
}
api.bankBegin = function (input) {
  const out = bankCall({ op: 'begin', ...(input || {}) });
  if (!out || out.kind === 'aborted') return helperErr((out && out.reason) || 'aborted');
  return helperOk({ token: out.token });
};
api.bankNext = function (input) {
  if (!input || input.token == null) return { ok: false, error: 'invalid-args' };
  const out = bankCall({ op: 'next', ...input });
  if (!out) return { ok: false, error: 'aborted' };
  if (out.kind === 'aborted') {
    return { ok: false, error: out.reason || 'aborted', kind: 'aborted', token: out.token, status: 'aborted' };
  }
  if (out.kind === 'yield') {
    return { ok: true, status: 'done', token: out.token, kind: 'yield', value: out.value === true };
  }
  return { ok: true, status: 'continue', token: out.token, ...out };
};
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
/// runner (the method invokers the isolate thread drives). The
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

/// The shared compat runner (defineBot and class shapes). JS only invokes
/// the declared methods; the isolate thread owns the tick phases, the
/// onStart-once gate, the `loop()` single-flight and the one paint
/// pass. Each invoker returns a promise that always fulfils — `null`
/// on success, the error text on a throw or rejection — so the thread
/// can hold it and poll its state without an unhandled rejection.
const COMPAT_RUNNER: &str = r#"
globalThis.__rs2b0t_flush_native_events = () => {
const pending = globalThis.__rs2b0t_pending_native_event_batch;
globalThis.__rs2b0t_pending_native_event_batch = null;
if (pending && pending.length) {
    const dispatch = globalThis.__rs2b0t_dispatch_native_events;
    if (typeof dispatch === 'function') dispatch(pending);
}
};
const __rs2b0t_invoke = async (method) => {
    try {
        if (inst && typeof inst[method] === 'function') await inst[method]();
        return null;
    } catch (e) {
        return String((e && e.message) || e);
    }
};
globalThis.__rs2b0t_compat_on_start = () => __rs2b0t_invoke('onStart');
globalThis.__rs2b0t_compat_loop = () => __rs2b0t_invoke('loop');
"#;

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
        .register_function("__rs2b0t_clue_verb", |args: &[serde_json::Value]| {
            Ok(crate::clue::verb_json(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register clue verb: {e}"))?;
    runtime
        .register_function("__rs2b0t_fire", move |args: &[serde_json::Value]| {
            Ok(crate::fire::dispatch(
                args.first().unwrap_or(&serde_json::Value::Null),
            ))
        })
        .map_err(|e| format!("register fire: {e}"))?;
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
        .register_function("__rs2b0t_canvas_get", |args: &[serde_json::Value]| {
            let prop = args
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            Ok(serde_json::Value::String(crate::canvas::get_style(prop)))
        })
        .map_err(|e| format!("register canvas get: {e}"))?;
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
    super::scene_v8::install(runtime).map_err(|e| format!("scene v8: {e}"))?;
    super::distance::install(runtime).map_err(|e| format!("distance: {e}"))?;
    super::reach_query::install(runtime).map_err(|e| format!("reach query: {e}"))?;
    super::melee_weapons_v8::install(runtime).map_err(|e| format!("melee weapons v8: {e}"))?;
    super::partner_trade_v8::install(runtime).map_err(|e| format!("partner trade v8: {e}"))?;
    super::canvas_tape::install(runtime).map_err(|e| format!("canvas tape: {e}"))?;
    super::paint_chrome::install(runtime).map_err(|e| format!("paint chrome: {e}"))?;
    super::paint_jive::install(runtime).map_err(|e| format!("paint jive: {e}"))?;
    super::callback_v8::install_ops(runtime).map_err(|e| format!("callback ops: {e}"))?;
    super::tools_v8::install(runtime).map_err(|e| format!("tools v8: {e}"))?;
    super::boost_potions_v8::install(runtime).map_err(|e| format!("boost potions v8: {e}"))?;
    super::targets_v8::install(runtime).map_err(|e| format!("targets v8: {e}"))?;
    super::fire_v8::install(runtime).map_err(|e| format!("fire v8: {e}"))?;
    super::combat_style_v8::install(runtime).map_err(|e| format!("combat style v8: {e}"))?;
    super::machine_v8::install(runtime).map_err(|e| format!("machine v8: {e}"))?;
    super::dialog_v8::install(runtime).map_err(|e| format!("dialog v8: {e}"))?;
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
import { runMachine } from '../../shim/_kernel.js';
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
const snapshotRootCache = new WeakMap();
const snapshotNestedCache = new WeakMap();
function readOnlyView(value, root) {
  if (value === null || typeof value !== 'object') return value;
  const cache = root ? snapshotRootCache : snapshotNestedCache;
  const cached = cache.get(value);
  if (cached) return cached;
  const proxy = new Proxy(value, {
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
  cache.set(value, proxy);
  return proxy;
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
// Bumped by ResetSession: the clue wrapper's own stale-token guard.
let lifecycleGeneration = 0;
globalThis.__rs_v2_reset_session = () => { lifecycleGeneration += 1; };
function prayerCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}
function helperOk(value) { return { ok: true, value: value }; }
function helperErr(error) { return { ok: false, error: String(error) }; }
function enqueueIfButton(component_id) {
  const h = host();
  h.interact = h.interact || [];
  h.interact.push({ op: 'if-button', component_id: component_id });
}
// One Rust step machine per Set/Clear: Rust owns the click, the wait, the
// clock and the admission. This surface admits one prayer operation at a
// time, so a second call settles `busy` from Rust before it begins or
// clicks anything.
async function prayerMachine(payload) {
  const out = await runMachine('prayer', payload);
  if (out.kind === 'done') {
    const settled = out.value;
    if (settled.ok === true) {
      return helperOk(settled.value === undefined ? true : settled.value);
    }
    return helperErr(settled.reason || 'aborted');
  }
  return out.kind === 'refused' ? helperErr(out.reason) : helperErr('aborted');
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
  return prayerMachine({
    op: 'set',
    name: input.name,
    on: { kind: 'boolean', value: input.on },
    admit: 'refuse-busy',
  });
};
api.prayerClear = function () {
  // Same admission rule as prayerSet: busy while one operation runs.
  return prayerMachine({ op: 'clear', admit: 'refuse-busy' });
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
// Continue envelope and begin/next error mapping. Scene pages are read in
// Rust; verbs are mapped by `__rs2b0t_clue_verb` (never a loc fall-through).
function clueStep(step) {
  return { ok: true, status: 'continue', token: step.token, ...step };
}
function clueStepError(reason) {
  if (reason === 'aborted' || reason === 'constrained') return reason;
  if (reason === 'missing-selected-data' || reason === 'family-unavailable:trails'
      || reason === 'none-held') return reason;
  return 'stale';
}
function clueBeginError(reason) {
  if (reason === 'constrained' || reason === 'abandoned') return reason;
  if (reason === 'missing-selected-data' || reason === 'family-unavailable:trails'
      || reason === 'none-held') return reason;
  return 'stale';
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
    });
    if (!step || typeof step !== 'object') return helperErr('stale');
    if (step.kind === 'token') return helperOk({ token: step.token });
    if (step.kind === 'aborted') return helperErr(clueBeginError(step.reason));
    return helperErr('stale');
  },
  // One step. `resume` is the callback return. Rust reads the isolate scene;
  // this helper never echoes snapshot pages. A verb is enqueued by the Rust
  // mapper (`__rs2b0t_clue_verb`); an unknown kind is not a loc.
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
    };
    if (hasResume) payload.resume = input.resume;
    const step = clueCall(payload);
    if (!step || typeof step !== 'object') return helperErr('stale');
    if (step.kind === 'aborted') return helperErr(clueStepError(step.reason));
    if (generation !== lifecycleGeneration) return helperErr('stale');
    const req = globalThis.rustyscript.functions.__rs2b0t_clue_verb(step);
    if (req && typeof req === 'object' && typeof req.op === 'string') {
      const h = host();
      h.interact = h.interact || [];
      h.interact.push(req);
    }
    if (step.kind === 'wait' || step.kind === 'yield'
        || step.kind === 'callback.enabled' || step.kind === 'callback.log'
        || step.kind === 'callback.setStatus'
        || step.kind === 'grind-ready' || step.kind === 'supplies-needed'
        || step.kind === 'no-shop' || step.kind === 'done' || step.kind === 'dead'
        || step.kind === 'abandon' || step.kind === 'guardian-lost'
        || req && typeof req === 'object' && typeof req.op === 'string') {
      return clueStep(step);
    }
    return helperErr('stale');
  },
    retry: function () {
    const step = clueCall({ op: 'retry' });
    if (!step || typeof step !== 'object' || step.kind !== 'retry') {
      return helperErr('stale');
    }
    return helperOk({ cleared: true });
  },
};
api.sceneLocs = function (input) {
  return globalThis.__rs2b0t_scene_query('locs', input);
};
api.sceneNpcs = function (input) {
  return globalThis.__rs2b0t_scene_query('npcs', input);
};
api.questStatus = function (input) {
  return globalThis.__rs2b0t_quest_status(input);
};
// Owned-root quest journal. One token per isolate. The machine reads the
// isolate scene; these three only coerce args, one native call, and enqueue.
function questJournalCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_quest_journal(payload);
}
function enqueueCloseModal() {
  const h = host();
  h.interact = h.interact || [];
  h.interact.push({ op: 'close-modal' });
}
function questJournalBeginError(reason) {
  if (reason === 'busy' || reason === 'main-modal-occupied') return reason;
  if (reason === 'snapshot-unavailable' || reason === 'quest-tab-unbound' || reason === 'unknown-quest') return reason;
  return 'stale';
}
function questJournalStepError(reason) {
  if (reason === 'snapshot-unavailable' || reason === 'modal-timeout') return reason;
  return 'stale';
}
api.questJournalBegin = function (input) {
  if (arguments.length === 0) return helperErr('invalid-args');
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return helperErr('invalid-args');
  }
  if (typeof input.name !== 'string') return helperErr('invalid-args');
  if (Object.prototype.hasOwnProperty.call(input, 'id')) return helperErr('invalid-args');
  if (input.name.trim() === '') return helperErr('invalid-args');
  const generation = lifecycleGeneration;
  const step = questJournalCall({
    op: 'begin',
    name: input.name,
    generation: generation,
  });
  if (!step || typeof step !== 'object') return helperErr('stale');
  if (step.kind === 'if-button') {
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
  const step = questJournalCall({
    op: 'next',
    token: input.token,
    generation: lifecycleGeneration,
  });
  if (!step || typeof step !== 'object') return helperErr('stale');
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
  const generation = lifecycleGeneration;
  const step = questJournalCall({
    op: 'close',
    token: input.token,
    generation: generation,
  });
  if (!step || typeof step !== 'object') return helperErr('stale');
  if (step.kind === 'close-modal') {
    if (generation !== lifecycleGeneration) return helperErr('stale');
    enqueueCloseModal();
    return { pending: true };
  }
  if (step.kind === 'wait') return { pending: true };
  if (step.kind === 'done') {
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

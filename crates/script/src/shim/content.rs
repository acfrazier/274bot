/// Host-owned policy plus optional selected-revision generated facts, posted
/// once onto `__rs2b0t_host.content` before catalog modules evaluate.
pub(crate) fn content_json(game_data: Option<&api::game_data::SelectedGameData>) -> String {
    use crate::content::{FIRE_PLOTS, LOG_LEVELS, RUNE_ROUTES};
    use api::cake_stall::{BAKER_STALL, CAKE_ITEM_NAMES};
    use api::content::ROCK_TYPE_NAMES;
    let food_heals = game_data
        .map(|data| {
            data.fixed_food_heals()
                .map(|(name, heal)| serde_json::json!([name, heal]))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let spell_db = game_data
        .map(|data| {
            data.spells()
                .iter()
                .map(|spell| {
                    (
                        spell.name.clone(),
                        serde_json::json!({
                            "ssb": spell.ssb,
                            "level": spell.level,
                            "runes": spell
                                .runes
                                .iter()
                                .map(|rune| serde_json::json!({"rune": rune.name, "count": rune.count}))
                                .collect::<Vec<_>>(),
                        }),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
        })
        .unwrap_or_default();
    let staff_runes = game_data
        .map(|data| {
            data.staves()
                .iter()
                .map(|staff| {
                    (
                        staff.name.clone(),
                        serde_json::json!(staff
                            .runes
                            .iter()
                            .map(|rune| rune.name.clone())
                            .collect::<Vec<_>>()),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "selected_facts": game_data.is_some(),
        "food_heals": food_heals,
        "common_bank_loot": api::content::COMMON_BANK_LOOT,
        "random_event_casket_id": api::content::RANDOM_EVENT_CASKET_ID,
        "rune_routes": RUNE_ROUTES.iter().map(|route| {
            serde_json::json!({
                "rune": route.rune,
                "talisman": route.talisman,
                "level": route.level,
                "bank": route.bank,
                "ruins": {"x": route.ruins.x, "z": route.ruins.z, "level": route.ruins.level}
            })
        }).collect::<Vec<_>>(),
        "log_levels": LOG_LEVELS
            .iter()
            .map(|(name, level)| serde_json::json!([*name, *level]))
            .collect::<Vec<_>>(),
        "fire_plots": FIRE_PLOTS.iter().map(|p| {
            serde_json::json!({
                "name": p.name,
                "bank": {"x": p.bank.x, "z": p.bank.z, "level": p.bank.level},
                "x0": p.x0, "x1": p.x1, "z0": p.z0, "z1": p.z1
            })
        }).collect::<Vec<_>>(),
        "rock_type_names": ROCK_TYPE_NAMES,
        "baker_stall": {
            "loc_id": BAKER_STALL.loc_id,
            "name": BAKER_STALL.name,
            "op": BAKER_STALL.op,
            "stall": {"x": BAKER_STALL.stall.x, "z": BAKER_STALL.stall.z, "level": BAKER_STALL.stall.level},
            "stand": {"x": BAKER_STALL.stand.x, "z": BAKER_STALL.stand.z, "level": BAKER_STALL.stand.level},
            "stand_alt": {"x": BAKER_STALL.stand_alt.x, "z": BAKER_STALL.stand_alt.z, "level": BAKER_STALL.stand_alt.level},
            "flee": {"x": BAKER_STALL.flee.x, "z": BAKER_STALL.flee.z, "level": BAKER_STALL.flee.level},
            "cake_items": CAKE_ITEM_NAMES,
        },
        "gather_tools": api::gather_tools::content_json_value(),
        "spell_db": spell_db,
        "staff_runes": staff_runes,
        "herbs": game_data
            .map(|data| {
                data.herbs()
                    .iter()
                    .map(|herb| {
                        serde_json::json!({
                            "key": herb.key,
                            "name": herb.name,
                            "id": herb.id,
                            "unidId": herb.unid_id,
                            "level": herb.level,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "drop_db": game_data
            .map(|data| {
                data.drop_tables()
                    .iter()
                    .map(|row| (row.name.clone(), serde_json::json!(row.display_names.clone())))
                    .collect::<serde_json::Map<_, _>>()
            })
            .unwrap_or_default(),
        "shops": game_data
            .map(api::shop_facts::content_json_value)
            .unwrap_or_else(|| serde_json::json!({})),
        "autocast": game_data.and_then(|data| {
            data.autocast_controls().map(|controls| {
                serde_json::json!({
                    "staff_tab_root": controls.staff_tab_root,
                    "spell_panel_root": controls.spell_panel_root,
                    "choose_com": controls.choose_com,
                    "toggle_com": controls.toggle_com,
                    "spell_grid_base": controls.spell_grid_base,
                    "magic_varp": controls.magic_varp,
                    "selected_value": controls.selected_value,
                    "armed_value": controls.armed_value,
                })
            })
        }),
        "duel": game_data.and_then(|data| {
            data.duel_controls().map(|controls| {
                serde_json::json!({
                    "select_modal": controls.select_modal,
                    "confirm_modal": controls.confirm_modal,
                    "win_modal": controls.win_modal,
                    "select_accept": controls.select_accept,
                    "confirm_accept": controls.confirm_accept,
                    "select_partner": controls.select_partner,
                    "select_status": controls.select_status,
                    "confirm_status": controls.confirm_status,
                })
            })
        }),
        "special": game_data.and_then(|data| {
            data.special_controls().map(|controls| {
                serde_json::json!({
                    "energy_varp": controls.energy_varp,
                    "armed_varp": controls.armed_varp,
                    "armed_value": controls.armed_value,
                    "max_energy": controls.max_energy,
                    "arm_confirm_ticks": controls.arm_confirm_ticks,
                })
            })
        }),
    })
    .to_string()
}

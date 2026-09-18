use crate::{render_betty_views, scenarios, Scenario};

type ScenarioFactory = fn() -> Scenario;

struct Entry {
    name: &'static str,
    factory: ScenarioFactory,
}

impl Entry {
    const fn new(name: &'static str, factory: ScenarioFactory) -> Self {
        Self { name, factory }
    }
}

const REGISTRY: &[Entry] = &[
    Entry::new("walk", scenarios::walk_scenario),
    Entry::new("render_smoke", scenarios::render_smoke_scenario),
    Entry::new(
        "render_betty_views_betty_yaw0",
        render_betty_views::betty_yaw0_scenario,
    ),
    Entry::new(
        "render_betty_views_betty_yaw512",
        render_betty_views::betty_yaw512_scenario,
    ),
    Entry::new(
        "render_betty_views_falador_street_yaw0",
        render_betty_views::falador_street_yaw0_scenario,
    ),
    Entry::new(
        "render_betty_views_falador_street_yaw512",
        render_betty_views::falador_street_yaw512_scenario,
    ),
    Entry::new(
        "render_betty_views_west_bank_yaw0",
        render_betty_views::west_bank_yaw0_scenario,
    ),
    Entry::new(
        "render_betty_views_west_bank_yaw512",
        render_betty_views::west_bank_yaw512_scenario,
    ),
    Entry::new(
        "render_betty_views_dwarven_wall_yaw0",
        render_betty_views::dwarven_wall_yaw0_scenario,
    ),
    Entry::new("nav_full", scenarios::nav_full_scenario),
    Entry::new("nav_door", scenarios::nav_door_scenario),
    Entry::new("nav_cart", scenarios::nav_cart_scenario),
    Entry::new("nav_essence", scenarios::nav_essence_scenario),
    Entry::new("nav_elkoy", scenarios::nav_elkoy_scenario),
    Entry::new("nav_tele", scenarios::nav_tele_scenario),
    Entry::new("nav_shantay", scenarios::nav_shantay_scenario),
    Entry::new("nav_routes", scenarios::nav_routes_scenario),
    Entry::new("nav_paint_path", scenarios::nav_paint_path_scenario),
    Entry::new("bone_burier", scenarios::bone_burier_scenario),
    Entry::new("lamp_redemption", scenarios::lamp_redemption_scenario),
    Entry::new("bone_burier_v2_ts", bone_burier_v2_ts_scenario),
    Entry::new("bone_burier_v2_js", bone_burier_v2_js_scenario),
    Entry::new(
        "strange_plant_owned",
        scenarios::strange_plant_owned_scenario,
    ),
    Entry::new("chicken_killer", scenarios::chicken_killer_scenario),
    Entry::new(
        "chicken_killer_bank",
        scenarios::chicken_killer_bank_scenario,
    ),
    Entry::new("thiever", scenarios::thiever_scenario),
    Entry::new("alcher", scenarios::alcher_scenario),
    Entry::new("alcher_defaults", scenarios::alcher_defaults_scenario),
    Entry::new("alcher_custom", scenarios::alcher_custom_scenario),
    Entry::new(
        "alcher_custom_alias",
        scenarios::alcher_custom_alias_scenario,
    ),
    Entry::new("alcher_custom_name", scenarios::alcher_custom_name_scenario),
    Entry::new("alcher_ordered", scenarios::alcher_ordered_scenario),
    Entry::new("alcher_large_batch", scenarios::alcher_large_batch_scenario),
    Entry::new("alcher_low", scenarios::alcher_low_scenario),
    Entry::new(
        "alcher_fire_battlestaff",
        scenarios::alcher_fire_battlestaff_scenario,
    ),
    Entry::new("alcher_swarm_drain", scenarios::alcher_swarm_drain_scenario),
    Entry::new("bank_fletcher", scenarios::bank_fletcher_scenario),
    Entry::new(
        "bank_fletcher_shafts",
        scenarios::bank_fletcher_shafts_scenario,
    ),
    Entry::new(
        "bank_fletcher_headless",
        scenarios::bank_fletcher_headless_scenario,
    ),
    Entry::new(
        "bank_fletcher_string",
        scenarios::bank_fletcher_string_scenario,
    ),
    Entry::new(
        "bank_fletcher_cut_string",
        scenarios::bank_fletcher_cut_string_scenario,
    ),
    Entry::new("dart_fletcher", scenarios::dart_fletcher_scenario),
    Entry::new("dart_fletcher_iron", scenarios::dart_fletcher_iron_scenario),
    Entry::new("herb_cleaner", scenarios::herb_cleaner_scenario),
    Entry::new("herb_cleaner_named", scenarios::herb_cleaner_named_scenario),
    Entry::new(
        "herb_cleaner_empty_bank",
        scenarios::herb_cleaner_empty_bank_scenario,
    ),
    Entry::new("gem_cutter", scenarios::gem_cutter_scenario),
    Entry::new("gem_cutter_named", scenarios::gem_cutter_named_scenario),
    Entry::new("door_opener", scenarios::door_opener_scenario),
    Entry::new("door_opener_gate", scenarios::door_opener_gate_scenario),
    Entry::new("gnome_course", scenarios::gnome_course_scenario),
    Entry::new(
        "gnome_course_radius",
        scenarios::gnome_course_radius_scenario,
    ),
    Entry::new("wildy_agility", scenarios::wildy_agility_scenario),
    Entry::new("brimhaven_agility", scenarios::brimhaven_agility_scenario),
    Entry::new("flax_picker", scenarios::flax_picker_scenario),
    Entry::new("superheater", scenarios::superheater_scenario),
    Entry::new("superheater_steel", scenarios::superheater_steel_scenario),
    Entry::new(
        "superheater_fire_battlestaff",
        scenarios::superheater_fire_battlestaff_scenario,
    ),
    Entry::new(
        "superheater_silver_low_natures",
        scenarios::superheater_silver_low_natures_scenario,
    ),
    Entry::new(
        "superheater_mithril",
        scenarios::superheater_mithril_scenario,
    ),
    Entry::new("vial_filler", scenarios::vial_filler_scenario),
    Entry::new("vial_filler_east", scenarios::vial_filler_east_scenario),
    Entry::new("potion_maker", scenarios::potion_maker_scenario),
    Entry::new("potion_maker_named", scenarios::potion_maker_named_scenario),
    Entry::new("tanner_bot", scenarios::tanner_bot_scenario),
    Entry::new("tanner_bot_hard", scenarios::tanner_bot_hard_scenario),
    Entry::new("rune_crafter", scenarios::rune_crafter_scenario),
    Entry::new("rune_crafter_earth", scenarios::rune_crafter_earth_scenario),
    Entry::new("mule_crafter", scenarios::mule_crafter_scenario),
    Entry::new("ardy_cakes", scenarios::ardy_cakes_scenario),
    Entry::new("ardy_cakes_fight", scenarios::ardy_cakes_fight_scenario),
    Entry::new("ardy_thiever", scenarios::ardy_thiever_scenario),
    Entry::new("ardy_thiever_fight", scenarios::ardy_thiever_fight_scenario),
    Entry::new(
        "ardy_thiever_knight",
        scenarios::ardy_thiever_knight_scenario,
    ),
    Entry::new("gnome_chop", scenarios::gnome_chop_scenario),
    Entry::new("gnome_fletch_short", scenarios::gnome_fletch_short_scenario),
    Entry::new("gnome_fletch_long", scenarios::gnome_fletch_long_scenario),
    Entry::new("coal_trucks", scenarios::coal_trucks_scenario),
    Entry::new("cook_bot", scenarios::cook_bot_scenario),
    Entry::new("cook_bot_lobster", scenarios::cook_bot_lobster_scenario),
    Entry::new("smelter_bot", scenarios::smelter_bot_scenario),
    Entry::new("smelter_bot_steel", scenarios::smelter_bot_steel_scenario),
    Entry::new("flax_spinner", scenarios::flax_spinner_scenario),
    Entry::new("flax_aio", scenarios::flax_aio_scenario),
    Entry::new("flax_aio_pick", scenarios::flax_aio_pick_scenario),
    Entry::new("flax_aio_spin", scenarios::flax_aio_spin_scenario),
    Entry::new(
        "herblore_secondaries",
        scenarios::herblore_secondaries_scenario,
    ),
    Entry::new(
        "herblore_secondaries_newt",
        scenarios::herblore_secondaries_newt_scenario,
    ),
    Entry::new("chaos_druid", scenarios::chaos_druid_scenario),
    Entry::new("chaos_druid_tower", scenarios::chaos_druid_tower_scenario),
    Entry::new(
        "chaos_druid_yanille",
        scenarios::chaos_druid_yanille_scenario,
    ),
    Entry::new("moss_giant", scenarios::moss_giant_scenario),
    Entry::new("hill_giant", scenarios::hill_giant_scenario),
    Entry::new("auto_fighter", scenarios::auto_fighter_scenario),
    Entry::new("auto_fighter_mage", scenarios::auto_fighter_mage_scenario),
    Entry::new("auto_fighter_range", scenarios::auto_fighter_range_scenario),
    Entry::new("rock_crab", scenarios::rock_crab_scenario),
    Entry::new("rock_crab_range", scenarios::rock_crab_range_scenario),
    Entry::new("green_dragon", scenarios::green_dragon_scenario),
    Entry::new(
        "green_dragon_special",
        scenarios::green_dragon_special_scenario,
    ),
    Entry::new(
        "green_dragon_potions",
        scenarios::green_dragon_potions_scenario,
    ),
    Entry::new("fire_giant", scenarios::fire_giant_scenario),
    Entry::new("ardy_fighter", scenarios::ardy_fighter_scenario),
    Entry::new("auto_fighter_bank", scenarios::auto_fighter_bank_scenario),
    Entry::new("moss_giant_bank", scenarios::moss_giant_bank_scenario),
    Entry::new("hill_giant_bank", scenarios::hill_giant_bank_scenario),
    Entry::new("chaos_druid_bank", scenarios::chaos_druid_bank_scenario),
    Entry::new("ardy_fighter_bank", scenarios::ardy_fighter_bank_scenario),
    Entry::new("rock_crab_bank", scenarios::rock_crab_bank_scenario),
    Entry::new("green_dragon_bank", scenarios::green_dragon_bank_scenario),
    Entry::new("green_dragon_tele", scenarios::green_dragon_tele_scenario),
    Entry::new(
        "fire_giant_approach",
        scenarios::fire_giant_approach_scenario,
    ),
    Entry::new("fire_giant_bank", scenarios::fire_giant_bank_scenario),
    Entry::new("aio_teleport", scenarios::aio_teleport_scenario),
    Entry::new(
        "aio_teleport_falador",
        scenarios::aio_teleport_falador_scenario,
    ),
    Entry::new(
        "aio_teleport_no_staff",
        scenarios::aio_teleport_no_staff_scenario,
    ),
    Entry::new("shop_buyout", scenarios::shop_buyout_scenario),
    Entry::new("shop_buyout_aubury", scenarios::shop_buyout_aubury_scenario),
    Entry::new("shop_buyout_lowe", scenarios::shop_buyout_lowe_scenario),
    Entry::new(
        "shop_buyout_hickton",
        scenarios::shop_buyout_hickton_scenario,
    ),
    Entry::new("shop_buyout_harry", scenarios::shop_buyout_harry_scenario),
    Entry::new("shop_buyout_betty", scenarios::shop_buyout_betty_scenario),
    Entry::new(
        "shop_buyout_gerrant",
        scenarios::shop_buyout_gerrant_scenario,
    ),
    Entry::new("shop_buyout_bob", scenarios::shop_buyout_bob_scenario),
    Entry::new("shop_buyout_nurmof", scenarios::shop_buyout_nurmof_scenario),
    Entry::new("shop_buyout_magic", scenarios::shop_buyout_magic_scenario),
    Entry::new(
        "shop_buyout_lundail",
        scenarios::shop_buyout_lundail_scenario,
    ),
    Entry::new(
        "shop_buyout_fernahei",
        scenarios::shop_buyout_fernahei_scenario,
    ),
    Entry::new("smithing_bot", scenarios::smithing_bot_scenario),
    Entry::new(
        "smithing_bot_platebody",
        scenarios::smithing_bot_platebody_scenario,
    ),
    Entry::new("smithing_bot_nails", scenarios::smithing_bot_nails_scenario),
    Entry::new(
        "smithing_bot_mithril",
        scenarios::smithing_bot_mithril_scenario,
    ),
    Entry::new("leather_crafter", scenarios::leather_crafter_scenario),
    Entry::new(
        "leather_crafter_hard_body",
        scenarios::leather_crafter_hard_body_scenario,
    ),
    Entry::new(
        "leather_crafter_green_body",
        scenarios::leather_crafter_green_body_scenario,
    ),
    Entry::new(
        "leather_crafter_chaps",
        scenarios::leather_crafter_chaps_scenario,
    ),
    Entry::new(
        "leather_crafter_thread_shop",
        scenarios::leather_crafter_thread_shop_scenario,
    ),
    Entry::new("firemaker", scenarios::firemaker_scenario),
    Entry::new("firemaker_oak", scenarios::firemaker_oak_scenario),
    Entry::new("climbing_boots", scenarios::climbing_boots_scenario),
    Entry::new(
        "climbing_boots_teleport",
        scenarios::climbing_boots_teleport_scenario,
    ),
    Entry::new("script_trade", scenarios::script_trade_scenario),
    Entry::new("nature_crafter_air", scenarios::nature_crafter_air_scenario),
    Entry::new("mule_crafter_air", scenarios::mule_crafter_air_scenario),
    Entry::new("flax_runner", scenarios::flax_runner_scenario),
    Entry::new("duel_arena", scenarios::duel_arena_scenario),
];

/// The registered scenario with this name, `None` when unknown.
pub fn get(name: &str) -> Option<Scenario> {
    REGISTRY
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| (entry.factory)())
}

/// Every registered scenario name (for the `--live script_<name>` usage).
pub fn names() -> Vec<&'static str> {
    registered_names().collect()
}

pub(crate) fn registered_names() -> impl Iterator<Item = &'static str> {
    REGISTRY.iter().map(|entry| entry.name)
}

#[cfg(test)]
pub(crate) fn registered_name_refs() -> impl Iterator<Item = &'static &'static str> {
    REGISTRY.iter().map(|entry| &entry.name)
}

fn bone_burier_v2_ts_scenario() -> Scenario {
    scenarios::bone_burier_v2_scenario("bone_burier_v2_ts", "bone_burier_v2.ts")
}

fn bone_burier_v2_js_scenario() -> Scenario {
    scenarios::bone_burier_v2_scenario("bone_burier_v2_js", "bone_burier_v2.js")
}

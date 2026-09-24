//! Shared full-core catalog witness used by headless and headed proof paths.
//!
//! This module intentionally contains the exact compact observations, Start
//! predicates, cycle observers and qualification logic from the original
//! `catalog_boundary_live` harness. It is proof infrastructure, not gameplay.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::line_of_sight::{line_of_sight_v2, CollisionQuery};
use api::obj_names::ObjNames;
use api::snapshot::{ActorKind, GameSnapshot, LocView, NpcView, SceneView, WorldTile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[path = "catalog_core_ranging.rs"]
mod ranging;
pub use ranging::{
    ranging_guild_bank_baseline_ready, ranging_guild_full_baseline_ready,
    ranging_guild_redeem_baseline_ready, ranging_guild_round_baseline_ready, RangingGuildBankCycle,
    RangingGuildFullCycle, RangingGuildRedeemCycle, RangingGuildRoundCycle, ARCHERY_TICKET_ID,
    COINS_PER_TRIP, ENTRY_FEE, RANGED_LIVE, RANGING_GUILD_FULL_MINUTES,
    RANGING_GUILD_FULL_STOP_NEEDLE, RANGING_GUILD_MERCHANT_STAND, RANGING_GUILD_SEERS_BANK,
    RANGING_GUILD_STAND, RUNE_ARROWS_PER_TRADE, SEED_KEEP_TICKETS, SEERS_BANK_RADIUS,
    TARGET_RESULT_MODAL, TICKETS_PER_TRADE, VARP_TARGET_COUNT, VARP_TARGET_HIT, VARP_TARGET_SCORE,
};
#[path = "catalog_core_hunt.rs"]
mod hunt;
pub use hunt::{
    hunt_baseline_ready, leave_lair_box, parse_hunt_receipt_line, HuntBox, HuntCell,
    HuntDeliveryCycle, HuntObservation, HuntOutcome, HuntReceipt, ScriptAct, ScriptActLedger,
    ScriptActRow, ScriptActsPublished, ACQUIRE_KEY_DEST, ACQUIRE_KEY_RADIUS,
    ACQUIRE_KEY_RECEIPT_PREFIX, ACQUIRE_KEY_V2_STOP, BANK_V2_DEST, BANK_V2_RADIUS,
    BANK_V2_RECEIPT_PREFIX, BANK_V2_STOP, CELL_V2_DOOR, CELL_V2_RADIUS, CELL_V2_RECEIPT_PREFIX,
    CELL_V2_STOP, DUSTY_KEY_ID, ENTER_LAIR_MAX_CHEB, ENTER_LAIR_MIN_CHEB,
    ENTER_LAIR_RECEIPT_PREFIX, ENTER_LAIR_V2_STOP, HOLD_SPOT_MAX_CHEB, HOLD_SPOT_MIN_CHEB,
    HOLD_SPOT_RECEIPT_PREFIX, HOLD_SPOT_V2_STOP, HUNT_ACT_ROWS, HUNT_LAIR_FIXTURE, JAILER,
    JAIL_CELL, JAIL_DOOR, JAIL_DOOR_LOC, JAIL_KEY_ID, LEAVE_LAIR_BOX_PAD, LEAVE_LAIR_MAX_CHEB,
    LEAVE_LAIR_MIN_CHEB, LEAVE_LAIR_RADIUS, LEAVE_LAIR_RECEIPT_PREFIX, LEAVE_LAIR_V2_STOP,
    RETREAT_SPOT_MAX_CHEB, RETREAT_SPOT_MIN_CHEB, RETREAT_SPOT_RECEIPT_PREFIX,
    RETREAT_SPOT_V2_STOP, VELRAK, WALK_SPOT_MAX_CHEB, WALK_SPOT_MIN_CHEB, WALK_SPOT_RECEIPT_PREFIX,
    WALK_SPOT_V2_STOP,
};

pub const CORE_SCENARIOS: &str = "bone_burier|chicken_killer|chicken_killer_bank|thiever|alcher|alcher_defaults|alcher_custom|alcher_custom_alias|alcher_custom_name|alcher_ordered|alcher_large_batch|alcher_low|alcher_fire_battlestaff|alcher_swarm_drain|bank_fletcher|bank_fletcher_shafts|bank_fletcher_headless|bank_fletcher_string|bank_fletcher_cut_string|dart_fletcher|dart_fletcher_iron|herb_cleaner|herb_cleaner_named|herb_cleaner_empty_bank|gem_cutter|gem_cutter_named|door_opener|door_opener_gate|gnome_course|gnome_course_radius|wildy_agility|brimhaven_agility|flax_picker|superheater|superheater_steel|superheater_fire_battlestaff|superheater_silver_low_natures|vial_filler|vial_filler_east|potion_maker|potion_maker_named|tanner_bot|tanner_bot_hard|rune_crafter|rune_crafter_earth|mule_crafter|ardy_cakes|ardy_cakes_fight|ardy_thiever|ardy_thiever_fight|ardy_thiever_knight|gnome_chop|gnome_fletch_short|gnome_fletch_long|coal_trucks|cook_bot|cook_bot_lobster|smelter_bot|smelter_bot_steel|flax_spinner|flax_aio|flax_aio_pick|flax_aio_spin|herblore_secondaries|herblore_secondaries_newt|chaos_druid|chaos_druid_tower|chaos_druid_yanille|moss_giant|moss_giant_prepared|moss_giant_dart|hill_giant|auto_fighter|auto_fighter_mage|auto_fighter_range|rock_crab|rock_crab_range|green_dragon|green_dragon_prepared|green_dragon_mage_prepared|green_dragon_special|green_dragon_special_prepared|green_dragon_potions|green_dragon_potions_prepared|fire_giant|fire_giant_prepared|ardy_fighter|auto_fighter_bank|moss_giant_bank|hill_giant_bank|hill_giant_bank_prepared|chaos_druid_bank|ardy_fighter_bank|rock_crab_bank|green_dragon_bank|green_dragon_bank_prepared|green_dragon_bank_default_prepared|green_dragon_tele|green_dragon_tele_prepared|fire_giant_approach|fire_giant_bank|fire_giant_bank_prepared|fire_giant_camelot_prepared|aio_teleport|aio_teleport_falador|aio_teleport_no_staff|shop_buyout|shop_buyout_aubury|shop_buyout_lowe|shop_buyout_hickton|shop_buyout_harry|shop_buyout_betty|shop_buyout_gerrant|smithing_bot|smithing_bot_platebody|leather_crafter|leather_crafter_hard_body|firemaker|firemaker_oak|climbing_boots|climbing_boots_teleport|ranging_guild_round|ranging_guild_redeem|ranging_guild_bank|ranging_guild_full|brimhaven_moss_inspect_v1|route_inspect_brimhaven_v2_ts|prayer_v2_ts|prayer_v1_ts|line_of_sight_v2_ts|actor_observation_v2_ts|fight_field_v2_ts|hold_spot_v2_ts|retreat_spot_v2_ts|walk_spot_v2_ts|enter_lair_v2_ts|leave_lair_v2_ts|acquire_key_v2_ts|cell_v2_ts|bank_v2_ts";
pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";
pub const ADAMANT_SCIMITAR_ID: i32 = 1331;
pub const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
pub const YEW_LONGBOW_ID: i32 = 855;
pub const CERT_YEW_LONGBOW_ID: i32 = 856;
pub const NATURE_RUNE_ID: i32 = 561;
pub const MIND_RUNE_ID: i32 = 558;
pub const COINS_ID: i32 = 995;
pub const RUNE_CHAINBODY_ID: i32 = 1113;
pub const CERT_RUNE_CHAINBODY_ID: i32 = 1114;
pub const RUNE_PLATELEGS_ID: i32 = 1079;
pub const RUNE_FULL_HELM_ID: i32 = 1163;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
pub const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
pub const YEW_LONGBOW_ALCH_COINS: i32 = 768;
pub const HIGH_ALCH_MAGIC_XP: i32 = 65;
/// Canonical 289 `magic_spells.dbrow`: Low needs 21 Magic and 31 client-scale
/// XP per cast; High needs 55 and 65. Low pays 40% of shop cost, High 60%.
pub const LOW_ALCH_LEVEL: i32 = 21;
pub const HIGH_ALCH_LEVEL: i32 = 55;
pub const LOW_ALCH_MAGIC_XP: i32 = 31;
/// High Level Alchemy pays 60% of shop cost: floor(50000 * 0.6) = 30000.
pub const RUNE_CHAINBODY_HIGH_ALCH_COINS: i32 = 30_000;
/// Low Level Alchemy pays 40% of shop cost: floor(50000 * 0.4) = 20000.
pub const RUNE_CHAINBODY_LOW_ALCH_COINS: i32 = 20_000;
/// Both selected catalogs require Attack 30 to wield a Fire battlestaff.
pub const FIRE_BATTLESTAFF_WIELD_ATTACK: i32 = 30;
/// Fire-rune staves, the default first: a spelled row proves the one it selected.
pub const ALCHER_FIRE_STAFFS: [i32; 2] = [STAFF_OF_FIRE_ID, FIRE_BATTLESTAFF_ID];
pub const LOGS_ID: i32 = 1511;
pub const ARROW_SHAFT_ID: i32 = 52;
pub const HEADLESS_ARROW_ID: i32 = 53;
pub const BRONZE_DART_TIP_ID: i32 = 819;
pub const BRONZE_DART_ID: i32 = 806;
/// Stock289 `rune_arrow`. MossGiant dart SETTINGS leave this unused on purpose.
pub const RUNE_ARROW_ID: i32 = 892;
/// Pinned `e2e/mossgiant-dart-test.ts` bank-only dart stack and food.
pub const MOSS_GIANT_DART_SUPPLY: i32 = 80;
pub const MOSS_GIANT_DART_BANK_FOOD: i32 = 15;
pub const MOSS_GIANT_DART_RANGED: i32 = 50;
pub const MOSS_GIANT_DART_FIELD_RADIUS: i32 = 12;
pub const IRON_DART_TIP_ID: i32 = 820;
pub const IRON_DART_ID: i32 = 807;
pub const FEATHER_ID: i32 = 314;
pub const UNIDENTIFIED_GUAM_ID: i32 = 199;
pub const GUAM_LEAF_ID: i32 = 249;
pub const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
pub const MARRENTILL_ID: i32 = 251;
pub const UNCUT_SAPPHIRE_ID: i32 = 1623;
pub const SAPPHIRE_ID: i32 = 1607;
pub const UNCUT_OPAL_ID: i32 = 1625;
pub const CHISEL_ID: i32 = 1755;
pub const CRUSHED_GEMSTONE_ID: i32 = 1633;
pub const WOODEN_DOOR_CLOSED_ID: i32 = 1530;
pub const WOODEN_DOOR_OPEN_ID: i32 = 1531;
pub const WOODEN_GATE_CLOSED_ID: i32 = 1551;
pub const WOODEN_GATE_OPEN_ID: i32 = 1552;
pub const FLAX_ID: i32 = 1779;
pub const COPPER_ORE_ID: i32 = 436;
pub const TIN_ORE_ID: i32 = 438;
pub const IRON_ORE_ID: i32 = 440;
/// Selected 289 `obj.pack`: silver_ore=442, silver_bar=2355.
pub const SILVER_ORE_ID: i32 = 442;
pub const COAL_ID: i32 = 453;
pub const STAFF_OF_FIRE_ID: i32 = 1387;
pub const FIRE_BATTLESTAFF_ID: i32 = 1393;
pub const BRONZE_BAR_ID: i32 = 2349;
pub const IRON_BAR_ID: i32 = 2351;
pub const STEEL_BAR_ID: i32 = 2353;
pub const SILVER_BAR_ID: i32 = 2355;
pub const LUMBRIDGE_DOOR: (i32, i32, i32) = (3208, 3211, 0);
pub const LUMBRIDGE_DOOR_STAND: (i32, i32, i32) = (3208, 3212, 0);
pub const LUMBRIDGE_GATE: (i32, i32, i32) = (3213, 3261, 0);
pub const LUMBRIDGE_GATE_STAND: (i32, i32, i32) = (3213, 3260, 0);
pub const GNOME_START: (i32, i32, i32) = (2474, 3436, 0);
pub const GNOME_AFTER_LOG: (i32, i32, i32) = (2474, 3429, 0);
pub const GNOME_GROUND_RETURN: (i32, i32, i32) = (2487, 3420, 0);
pub const GNOME_PIPE: (i32, i32, i32) = (2484, 3431, 0);
// Selected 274/289 content: 86.5 XP per full lap, then 7.5 for the next log.
pub const GNOME_SECOND_LOG_XP: i32 = 94;
pub const WILDY_START: (i32, i32, i32) = (2998, 3916, 0);
pub const WILDY_PIPE_DEST: (i32, i32, i32) = (3004, 3947, 0);
pub const WILDY_ROPE_DEST: (i32, i32, i32) = (3005, 3958, 0);
pub const WILDY_STONE_DEST: (i32, i32, i32) = (2996, 3960, 0);
// Loc 2297 is stored on raw map plane 1, but LinkBelow exposes player plane 0.
pub const WILDY_LOG_DEST: (i32, i32, i32) = (2994, 3945, 0);
pub const WILDY_ROCKS_DEST: (i32, i32, i32) = (2994, 3933, 0);
pub const WILDY_LAP_XP: i32 = 586;
pub const WILDY_FURTHER_XP: i32 = 598;
pub const BRIMHAVEN_START: (i32, i32, i32) = (2809, 3194, 0);
pub const BRIMHAVEN_ARENA_VARP: i32 = 309;
pub const BRIMHAVEN_TICKET_ID: i32 = 2996;
pub const FLAX_FIELD: (i32, i32, i32) = (2741, 3444, 0);
pub const FLAX_FIELD_SCOPE: i32 = 12;
pub const FALADOR_CHICKENS: (i32, i32, i32) = (3029, 3294, 0);
pub const EMPTY_VIAL_ID: i32 = 229;
pub const VIAL_OF_WATER_ID: i32 = 227;
pub const EYE_OF_NEWT_ID: i32 = 221;
pub const GUAM_UNF_ID: i32 = 91;
pub const ATTACK_POTION_3_ID: i32 = 121;
pub const RANARR_WEED_ID: i32 = 257;
pub const RANARR_UNF_ID: i32 = 99;
pub const SNAPE_GRASS_ID: i32 = 231;
pub const PRAYER_POTION_3_ID: i32 = 139;
pub const FALADOR_WEST_BANK: (i32, i32, i32) = (2946, 3369, 0);
pub const FALADOR_EAST_BANK: (i32, i32, i32) = (3013, 3355, 0);
pub const FALADOR_FOUNTAIN: (i32, i32, i32) = (2949, 3381, 0);
pub const COW_HIDE_ID: i32 = 1739;
pub const SOFT_LEATHER_ID: i32 = 1741;
pub const HARD_LEATHER_ID: i32 = 1743;
pub const TANNER_IF: i32 = 679;
pub const SOFT_TAN_ALL_COM: i32 = 8686;
pub const HARD_TAN_ALL_COM: i32 = 8690;
pub const SHOPMAIN: i32 = 3824;
pub const AL_KHARID_BANK: (i32, i32, i32) = (3269, 3167, 0);
pub const TANNER_STAND: (i32, i32, i32) = (3277, 3191, 0);
pub const DOMMIK_STAND: (i32, i32, i32) = (3316, 3192, 0);
pub const RUNE_ESSENCE_ID: i32 = 1436;
pub const NOTED_ESSENCE_ID: i32 = 1437;
pub const AIR_TALISMAN_ID: i32 = 1438;
pub const EARTH_TALISMAN_ID: i32 = 1440;
pub const AIR_RUNE_ID: i32 = 556;
pub const EARTH_RUNE_ID: i32 = 557;
pub const TEMPLE_Z: i32 = 4000;
pub const VARROCK_EAST_BANK: (i32, i32, i32) = (3253, 3420, 0);
pub const RUNECRAFTER_AIR_RUINS: (i32, i32, i32) = (2988, 3294, 0);
pub const RUNECRAFTER_EARTH_RUINS: (i32, i32, i32) = (3303, 3477, 0);
pub const MULECRAFTER_AIR_RUINS: (i32, i32, i32) = (2983, 3288, 0);
pub const AIR_ALTAR: (i32, i32, i32) = (2841, 4829, 0);
pub const EARTH_ALTAR: (i32, i32, i32) = (2655, 4830, 0);
pub const CAKE_ID: i32 = 1891;
pub const BREAD_ID: i32 = 2309;
pub const CHOCOLATE_SLICE_ID: i32 = 1901;
pub const CHOCOLATE_CAKE_ID: i32 = 1897;
pub const NOTED_CAKE_ID: i32 = 1892;
pub const NOTED_BREAD_ID: i32 = 2310;
pub const NOTED_CHOCOLATE_SLICE_ID: i32 = 1902;
pub const ARDY_CAKES_BALLAST_KNIVES: i32 = 22;
pub const ARDY_CAKES_STAND: (i32, i32, i32) = (2668, 3312, 0);
pub const ARDY_THIEVER_STAND: (i32, i32, i32) = (2661, 3306, 0);
pub const AUTO_FIGHTER_MAGE_LEVEL: i32 = 13;
pub const AUTO_FIGHTER_MAGE_CASTS: i32 = 150;
pub const AUTO_FIGHTER_MAGE_AIR_RUNES: i32 = AUTO_FIGHTER_MAGE_CASTS * 2;
pub const AUTOCAST_MAGIC_VARP: i32 = 108;
pub const AUTOCAST_ARMED_VALUE: i32 = 3;
pub const ARDY_BANK: (i32, i32, i32) = (2655, 3286, 0);
pub const MAGIC_LOGS_ID: i32 = 1513;
pub const NOTED_MAGIC_LOGS_ID: i32 = 1514;
pub const UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 72;
pub const UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 70;
pub const MAGIC_SHORTBOW_ID: i32 = 861;
pub const MAGIC_LONGBOW_ID: i32 = 859;
pub const NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 73;
pub const NOTED_UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 71;
pub const KNIFE_ID: i32 = 946;
pub const STEEL_AXE_ID: i32 = 1353;
pub const RUNE_AXE_ID: i32 = 1359;
pub const GNOME_BALLAST_KNIVES: i32 = 26;
pub const MAGIC_TREE_ID: i32 = 1306;
pub const RUNE_PICKAXE_ID: i32 = 1275;
pub const NOTED_COAL_ID: i32 = 454;
pub const COAL_BALLAST_KNIVES: i32 = 26;
pub const GNOME_SOUTH_BANK_MAGIC_STAND: (i32, i32, i32) = (2433, 3409, 0);
pub const GNOME_SOUTH_BANK_MAGIC_TREE: (i32, i32, i32) = (2432, 3410, 0);

pub const GNOME_BANK_STAND: (i32, i32, i32) = (2445, 3425, 1);
pub const GNOME_BANK_STAIR_SOUTH: (i32, i32, i32) = (2444, 3416, 0);
pub const COAL_MINE: (i32, i32, i32) = (2582, 3481, 0);
pub const COAL_MINE_TRUCK_STAND: (i32, i32, i32) = (2575, 3486, 0);
pub const SEERS_BANK: (i32, i32, i32) = (2725, 3491, 0);
pub const RAW_SALMON_ID: i32 = 331;
pub const SALMON_ID: i32 = 329;
pub const NOTED_RAW_SALMON_ID: i32 = 332;
pub const NOTED_SALMON_ID: i32 = 330;
pub const RAW_LOBSTER_ID: i32 = 377;
pub const LOBSTER_ID: i32 = 379;
pub const NOTED_RAW_LOBSTER_ID: i32 = 378;
pub const NOTED_LOBSTER_ID: i32 = 380;
pub const BURNT_FISH_1_ID: i32 = 323;
pub const BURNT_FISH_2_ID: i32 = 343;
pub const BURNT_LOBSTER_ID: i32 = 381;
pub const NOTED_COPPER_ORE_ID: i32 = 437;
pub const NOTED_TIN_ORE_ID: i32 = 439;
pub const NOTED_IRON_ORE_ID: i32 = 441;
pub const NOTED_BRONZE_BAR_ID: i32 = 2350;
pub const NOTED_STEEL_BAR_ID: i32 = 2354;
pub const BOW_STRING_ID: i32 = 1777;
pub const NOTED_FLAX_ID: i32 = 1780;
pub const NOTED_BOW_STRING_ID: i32 = 1778;
pub const BALL_OF_WOOL_ID: i32 = 1759;
pub const COOKING_FIXTURE_LEVEL: i32 = 80;
pub const CATHERBY_BANK: (i32, i32, i32) = (2809, 3441, 0);
pub const CATHERBY_RANGE_STAND: (i32, i32, i32) = (2817, 3443, 0);
pub const AL_KHARID_FURNACE: (i32, i32, i32) = (3275, 3185, 0);
pub const FLAX_SPINNER_BANK: (i32, i32, i32) = (2722, 3493, 0);
pub const FLAX_SPINNER_WHEEL: (i32, i32, i32) = (2711, 3471, 1);
pub const FLAX_AIO_BANK: (i32, i32, i32) = (2725, 3493, 0);
pub const RED_SPIDERS_EGGS_ID: i32 = 223;
pub const NOTED_RED_SPIDERS_EGGS_ID: i32 = 224;
pub const NOTED_EYE_OF_NEWT_ID: i32 = 222;
pub const EGG_FIELD: (i32, i32, i32) = (3120, 9952, 0);
pub const BETTY_SHOP: (i32, i32, i32) = (3012, 3259, 0);
pub const TROUT_ID: i32 = 333;
pub const BIG_BONES_ID: i32 = 532;
pub const NOTED_BIG_BONES_ID: i32 = 533;
pub const LIMPWURT_ROOT_ID: i32 = 225;
pub const NOTED_LIMPWURT_ROOT_ID: i32 = 226;
pub const LAW_RUNE_ID: i32 = 563;
pub const BONES_ID: i32 = 526;
pub const NOTED_BONES_ID: i32 = 527;
pub const NOTED_HERB_ID: i32 = 200;
pub const LANTADYME_HERB_ID: i32 = 2485;
pub const NOTED_LANTADYME_HERB_ID: i32 = 2486;
pub const CHAOS_DRUID_FIELD: (i32, i32, i32) = (3110, 9936, 0);
pub const CHAOS_DRUID_TOWER_FIELD: (i32, i32, i32) = (2562, 3356, 0);
pub const CHAOS_DRUID_YANILLE_FIELD: (i32, i32, i32) = (2580, 9501, 0);
/// ArdyCakes / ArdyThiever Flee kite tile. Fight mode must kill instead of landing here.
pub const ARDY_FLEE_TILE: (i32, i32, i32) = (2655, 3298, 0);
pub const MOSS_GIANT_SAFESPOT: (i32, i32, i32) = (2553, 3406, 0);
pub const HILL_GIANT_PIT: (i32, i32, i32) = (3110, 9832, 0);
pub const CHAOS_DRUID_FOOD: i32 = 12;
pub const MOSS_GIANT_FOOD: i32 = 10;
pub const HILL_GIANT_FOOD: i32 = 8;
pub const AUTO_FIGHTER_FOOD: i32 = 8;
/// Bank cells prepare the pack below the card's own restock line so the bank
/// trip itself has to withdraw: MossGiant only banks once its food is gone.
pub const MOSS_GIANT_BANK_FOOD: i32 = 2;
/// ChaosDruidKiller's `tripPrepared` needs `foodWithdraw` (12) carried in the
/// field; below that its own first trip end is `prepare-trip`, so the bank cell
/// prepares the shortfall instead of a full pack.
pub const CHAOS_DRUID_BANK_FOOD: i32 = 8;
/// AutoFighter's BankRun withdraws food up to its declared `foodWithdraw`
/// default of 10.
pub const AUTO_FIGHTER_BANK_RESTOCK: i32 = 10;
/// Restock lines the other bank cards withdraw to: MossGiant's declared
/// `foodWithdraw` default 20 and HillGiant's 12 (8 carried + 4 withdrawn).
pub const MOSS_GIANT_BANK_RESTOCK: i32 = 20;
pub const HILL_GIANT_BANK_RESTOCK: i32 = 4;
pub const HILL_GIANT_BANK_PREPARED_RESTOCK: i32 = 12;
pub const ROCK_CRAB_FOOD: i32 = 8;
/// Base GreenDragon's ordinary trip withdraws twenty Lobsters.
/// Special, potion, bank, and teleport cells retain their twelve-food inputs.
pub const GREEN_DRAGON_BASE_FOOD: i32 = 20;
pub const GREEN_DRAGON_FOOD: i32 = 12;
pub const GREEN_DRAGON_BANK_PREPARED_FOOD: i32 = 26;
pub const GREEN_DRAGON_BANK_PREPARED_RESTOCK: i32 = 27;
pub const FIRE_GIANT_FOOD: i32 = 12;
pub const FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD: i32 = 1;
pub const FIRE_GIANT_BANK_PREPARED_RESTOCK: i32 = 25;
/// Camelot escape restock line; barrel prepared keeps `FIRE_GIANT_BANK_PREPARED_RESTOCK`.
pub const FIRE_GIANT_CAMELOT_PREPARED_RESTOCK: i32 = 24;
pub const COMBAT_ATTACK_LEVEL: i32 = 40;
pub const REMAINING_COMBAT_PREPARED_LEVEL: i32 = 70;
pub const BANK_PRESSURE_PREPARED_LEVEL: i32 = 99;
pub const GREEN_DRAGON_TELE_PREPARED_LEVEL: i32 = 99;
pub const GREEN_DRAGON_TELE_PREPARED_HP: i32 = 70;
pub const RUNE_SCIMITAR_ID: i32 = 1333;
pub const DRAGONFIRE_SHIELD_ID: i32 = 1540;
pub const NOTED_DRAGONFIRE_SHIELD_ID: i32 = 1541;
pub const BRASS_KEY_ID: i32 = 983;
pub const DRAGON_BONES_ID: i32 = 536;
pub const NOTED_DRAGON_BONES_ID: i32 = 537;
pub const GREEN_DRAGONHIDE_ID: i32 = 1753;
pub const NOTED_GREEN_DRAGONHIDE_ID: i32 = 1754;
pub const BLACK_DRAGONHIDE_ID: i32 = 1747;
pub const RED_DRAGONHIDE_ID: i32 = 1749;
pub const BLUE_DRAGONHIDE_ID: i32 = 1751;
pub const GLARIALS_AMULET_ID: i32 = 295;
pub const ROPE_ID: i32 = 954;
pub const ROCK_CRAB_SPOT: (i32, i32, i32) = (2704, 3726, 0);
/// The dormant field resource the RockCrab branches wake: the frozen 289
/// content's `Rocks` NPC, which the source's own AI turns into the
/// `Rock Crab` target.
pub const ROCK_CRAB_DORMANT_NAME: &str = "Rocks";
/// Chebyshev radius around [`ROCK_CRAB_SPOT`] the dormant Rocks and the scoped
/// stand both live inside. The source's own `DEFAULT_SPOTS` field is smaller;
/// this stays the audited outer bound.
pub const ROCK_CRAB_FIELD_RADIUS: i32 = 50;
/// Dormant Rocks carried in the bounded per-frame projection: the nearest few
/// to the player, the ones this player can actually wake. A live frame can
/// hold more; only their index/tile identity is retained.
pub const DORMANT_ROCK_FACTS_MAX: usize = 8;
/// Inside the native visibility window while remaining outside dormant
/// RockCrab wake range; the live witness observes Rocks before Start.
pub const ROCK_CRAB_SAFE_STAND: (i32, i32, i32) = (2712, 3707, 0);
pub const GREEN_DRAGON_FIELD: (i32, i32, i32) = (3096, 3814, 0);
pub const FIRE_GIANT_ROOM: (i32, i32, i32) = (2575, 9893, 0);
pub const WILDERNESS_MIN_Z: i32 = 3520;
pub const DUNGEON_MIN_Z: i32 = 9000;
/// `com_mode`: the combat-tab style varp the ranged branches set directly.
pub const COMBAT_MODE_VARP: i32 = 43;
/// `rapid`, the ranged style the range branches select through `setCombatMode`.
pub const RAPID_COMBAT_MODE: i32 = 1;
/// `%sa_energy`, the special-attack bar's 0-1000 pool.
pub const SA_ENERGY_VARP: i32 = 300;
/// `%sa_attack`, set while a special is armed and cleared when the hit lands.
pub const SA_ARMED_VARP: i32 = 301;
pub const SA_ARMED_VALUE: i32 = 1;
/// Dragon dagger special cost, per the frozen card's own weapon table.
pub const DRAGON_DAGGER_SPECIAL_COST: i32 = 250;
pub const MAPLE_SHORTBOW_ID: i32 = 853;
pub const BRONZE_ARROW_ID: i32 = 882;
pub const DRAGON_DAGGER_ID: i32 = 1215;
pub const SUPER_ATTACK_3_ID: i32 = 145;
pub const SUPER_ATTACK_2_ID: i32 = 147;
pub const SUPER_STRENGTH_3_ID: i32 = 157;
pub const SUPER_STRENGTH_2_ID: i32 = 159;
/// Ranged level both range branches need before Start.
pub const RANGED_LEVEL: i32 = 40;
/// The bank stands the five bank cells have to reach. These are the frozen
/// cards' own declared walk targets (MossGiant `bankTile`, HillGiant
/// WEST_BANK, ChaosDruidKiller Edgeville `bankStand`) or the nearest bank the
/// card's own `nearestBank(here)` ranking resolves from its anchor (the two
/// Ardougne cards: East Ardougne, 3 tiles from `ARDY_BANK`).
pub const MOSS_GIANT_BANK: (i32, i32, i32) = (2615, 3332, 0);
pub const HILL_GIANT_BANK: (i32, i32, i32) = (3185, 3440, 0);
pub const CHAOS_DRUID_BANK: (i32, i32, i32) = (3094, 3491, 0);
pub const ARDOUGNE_EAST_BANK: (i32, i32, i32) = (2655, 3283, 0);
/// ArdyFighter's own DEFAULT_LOOT names, minus `clue scroll` (the card keeps
/// clue items in its deposit matcher and one id per scroll tier would be a
/// guess). These are the Guard drop-table rows the cards' lists were written
/// for: ArdyFighter's own loot list and the AutoFighter bank cell's injected
/// one.
pub const STEEL_ARROW_ID: i32 = 886;
pub const BODY_TALISMAN_ID: i32 = 1446;
pub const BLOOD_RUNE_ID: i32 = 565;
pub const CHAOS_RUNE_ID: i32 = 562;
/// The six verifiable Guard drops ArdyFighter lists in `DEFAULT_LOOT`
/// (`iron ore, steel arrow, body talisman, blood/chaos/nature rune`, whose
/// item names resolve to exactly these ids) and the class its bank trip
/// deposits. A clue-only or junk drop is not one of these.
pub const GUARD_DROP_IDS: [i32; 6] = [
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];
/// RockCrab PeriodicBank listed loot the bank cell can verify (the card's
/// default list also has clues and other gems; these two are the audit's
/// reachable, id-stable pair).
pub const CASKET_ID: i32 = 405;
pub const NOTED_CASKET_ID: i32 = 406;
pub const NOTED_UNCUT_SAPPHIRE_ID: i32 = 1624;
pub const FIRE_RUNE_ID: i32 = 554;
/// Varrock teleport Magic level and the frozen GreenDragon rune recipe
/// (`Law×1 + Air×3 + Fire×1`). The cell carries three casts so one failed
/// click cannot starve the escape.
pub const VARROCK_TELE_MAGIC: i32 = 25;
pub const VARROCK_TELE_LAW: i32 = 3;
pub const VARROCK_TELE_AIR: i32 = 9;
pub const VARROCK_TELE_FIRE: i32 = 3;
pub const GREEN_DRAGON_BANK_RESTOCK: i32 = 20;
pub const FIRE_GIANT_BANK_RESTOCK: i32 = 20;
/// GreenDragon `bankTile` default (Edgeville), distinct from ChaosDruidKiller's
/// Edgeville stand at z=3491.
pub const GREEN_DRAGON_BANK: (i32, i32, i32) = (3094, 3493, 0);
/// FireGiantLogic `RAFT_STAND` / `WASHED_OUT` / `BARREL_BANK`.
pub const FIRE_GIANT_RAFT: (i32, i32, i32) = (2510, 3493, 0);
pub const FIRE_GIANT_WASH: (i32, i32, i32) = (2527, 3413, 0);
pub const FIRE_GIANT_BANK: (i32, i32, i32) = (2616, 3332, 0);
/// FireGiantLogic `ESCAPE_TELES.Camelot` land and Magic 45 / Air×5+Law×1.
pub const CAMELOT_TELE_LAND: (i32, i32, i32) = (2757, 3478, 0);
pub const CAMELOT_TELE_MAGIC: i32 = 45;
pub const CAMELOT_TELE_AIR: i32 = 5;
pub const CAMELOT_TELE_LAW: i32 = 1;
/// Source `teleStock` default 2 spare casts plus the one needed to leave.
pub const CAMELOT_TELE_STOCK: i32 = 2;
pub const CAMELOT_AIR_CARRY: i32 = CAMELOT_TELE_AIR * (CAMELOT_TELE_STOCK + 1);
pub const CAMELOT_LAW_CARRY: i32 = CAMELOT_TELE_LAW * (CAMELOT_TELE_STOCK + 1);
/// Frozen Varrock teleport land (the same tile FireGiant's Varrock escape uses).
pub const VARROCK_TELE_LAND: (i32, i32, i32) = (3213, 3424, 0);
/// Nearest RockCrab `DEFAULT_SPOTS` loc to the safe stand (2712,3707,0) — spot
/// 4 at (2710,3717,0). `onStart` picks that as `currentSpot()`, and PeriodicBank
/// returns there.
pub const ROCK_CRAB_BANK_RET: (i32, i32, i32) = (2710, 3717, 0);
/// AIO Teleport start: Lumbridge bank, r8, so Varrock/Falador arrival is a
/// real tile change. Generated tele_coord `0_46_52_21_50` is Falador land.
pub const LUMBRIDGE_BANK: (i32, i32, i32) = (3092, 3245, 0);
pub const FALADOR_TELE_LAND: (i32, i32, i32) = (2965, 3378, 0);
pub const FALADOR_TELE_MAGIC: i32 = 37;
pub const STAFF_OF_AIR_ID: i32 = 1381;
pub const STAFF_OF_WATER_ID: i32 = 1383;
pub const WATER_RUNE_ID: i32 = 555;
pub const AIO_LAW_PACK: i32 = 2;
pub const AIO_ELEMENT_PACK: i32 = 20;
pub const AIO_LAW_BANK: i32 = 200;
/// Frozen ShopBuyout presets: Aemad, Aubury, Lowe, Hickton, Harry, Betty, Gerrant.
pub const AEMAD_STAND: (i32, i32, i32) = (2613, 3294, 0);
pub const AUBURY_STAND: (i32, i32, i32) = (3253, 3401, 0);
pub const LOWE_STAND: (i32, i32, i32) = (3231, 3421, 0);
pub const HICKTON_STAND: (i32, i32, i32) = (2821, 3442, 0);
pub const HARRY_STAND: (i32, i32, i32) = (2833, 3443, 0);
/// Frozen shopPresets Betty stand (3012,3258) — not herblore BETTY_SHOP 3259.
pub const BETTY_STAND: (i32, i32, i32) = (3012, 3258, 0);
pub const GERRANT_STAND: (i32, i32, i32) = (3013, 3224, 0);
/// Frozen Gerrant bankStand / Draynor operable approach (not canonical 3093,3243).
pub const DRAYNOR_BANK: (i32, i32, i32) = (3092, 3243, 0);
pub const FISHING_BAIT_ID: i32 = 313;
pub const SHOP_COIN_BANK: i32 = 20_000;
/// Varrock West anvil used by SmithingBot. Bronze platebody is Smithing 18,
/// not the audit's copied Smithing 1 (dagger) seed.
pub const VARROCK_WEST_BANK: (i32, i32, i32) = (3185, 3440, 0);
pub const VARROCK_ANVIL: (i32, i32, i32) = (3188, 3425, 0);
pub const HAMMER_ID: i32 = 2347;
pub const BRONZE_DAGGER_ID: i32 = 1205;
pub const BRONZE_PLATEBODY_ID: i32 = 1117;
pub const BRONZE_PLATEBODY_SMITHING: i32 = 18;
pub const SMITH_BAR_BANK: i32 = 28;
pub const NEEDLE_ID: i32 = 1733;
pub const THREAD_ID: i32 = 1734;
pub const LEATHER_GLOVES_ID: i32 = 1059;
pub const HARDLEATHER_BODY_ID: i32 = 1131;
/// Selected 274 leather-interface identity. 289 mismatch is LIVE unavailable,
/// not a new native.
pub const LEATHER_IF: i32 = 2311;
pub const LEATHER_GLOVES_MAKE10: i32 = 8636;
pub const HARD_LEATHER_CRAFTING: i32 = 28;
pub const LEATHER_BANK: i32 = 28;
pub const THREAD_BANK: i32 = 100;
pub const OAK_LOGS_ID: i32 = 1521;
pub const TINDERBOX_ID: i32 = 590;
pub const OAK_FIREMAKING: i32 = 15;
pub const FIRE_LOG_BANK: i32 = 28;
/// Posted native `FIRE_PLOTS` Varrock East AABB (not frozen FIRE_SPOTS).
pub const FIRE_PLOT_VARROCK_EAST_X0: i32 = 3235;
pub const FIRE_PLOT_VARROCK_EAST_X1: i32 = 3275;
pub const FIRE_PLOT_VARROCK_EAST_Z0: i32 = 3418;
pub const FIRE_PLOT_VARROCK_EAST_Z1: i32 = 3432;
/// ClimbingBoots native witness (frozen `ClimbingBoots.ts` + clean 289 pack).
/// Tenzing's standing targets: Falador West bank stand, the hut door, and the
/// inside tile he is bought from. These are route targets, never PASS
/// predicates. Boots id 3105 and the 12-coin pair come from
/// `death_sherpa.rs2` (inv_del(coins,12) + inv_add(death_climbingboots,1)).
pub const CLIMBING_BOOTS_ID: i32 = 3105;
pub const CLIMBING_BOOTS_PAIR_COINS: i32 = 12;
pub const TENZING_HUT_DOOR: (i32, i32, i32) = (2823, 3555, 0);
pub const TENZING_INSIDE: (i32, i32, i32) = (2820, 3556, 0);
pub const TENZING_NAME: &str = "Tenzing";
/// `useTeleport`/`runeStock` bounds and the Falador cast cost. Magic below 37
/// walks even with useTeleport=true, so the teleport cell must prove a real
/// cast: magic XP plus a Law/Air/Water spend and a landing, not the option.
pub const CLIMBING_BOOTS_RUNE_STOCK_MIN: i32 = 1;
pub const CLIMBING_BOOTS_WALK_PACK_COINS: i32 = 28 * CLIMBING_BOOTS_PAIR_COINS;
pub const CLIMBING_BOOTS_TELE_PACK_COINS: i32 = 25 * CLIMBING_BOOTS_PAIR_COINS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreCase {
    BoneBurier,
    ChickenKiller,
    ChickenKillerBank,
    Thiever,
    Alcher,
    AlcherDefaults,
    AlcherCustom,
    AlcherCustomAlias,
    AlcherCustomName,
    AlcherOrdered,
    AlcherLargeBatch,
    AlcherLow,
    AlcherFireBattlestaff,
    AlcherSwarmDrain,
    BankFletcher,
    BankFletcherShafts,
    BankFletcherHeadless,
    BankFletcherString,
    BankFletcherCutString,
    DartFletcher,
    DartFletcherIron,
    HerbCleaner,
    HerbCleanerNamed,
    HerbCleanerEmptyBank,
    GemCutter,
    GemCutterNamed,
    DoorOpener,
    DoorOpenerGate,
    GnomeCourse,
    GnomeCourseRadius,
    WildyAgility,
    BrimhavenAgility,
    FlaxPicker,
    Superheater,
    SuperheaterSteel,
    SuperheaterFireBattlestaff,
    SuperheaterSilverLowNatures,
    VialFiller,
    VialFillerEast,
    PotionMaker,
    PotionMakerNamed,
    TannerBot,
    TannerBotHard,
    RuneCrafter,
    RuneCrafterEarth,
    MuleCrafter,
    ArdyCakes,
    ArdyCakesFight,
    ArdyThiever,
    ArdyThieverFight,
    ArdyThieverKnight,
    GnomeChop,
    GnomeFletchShort,
    GnomeFletchLong,
    CoalTrucks,
    CookBot,
    CookBotLobster,
    SmelterBot,
    SmelterBotSteel,
    FlaxSpinner,
    FlaxAio,
    FlaxAioPick,
    FlaxAioSpin,
    HerbloreSecondaries,
    HerbloreSecondariesNewt,
    ChaosDruid,
    ChaosDruidTower,
    ChaosDruidYanille,
    MossGiant,
    MossGiantPrepared,
    MossGiantDart,
    HillGiant,
    AutoFighter,
    AutoFighterMage,
    AutoFighterRange,
    RockCrab,
    RockCrabRange,
    GreenDragon,
    GreenDragonPrepared,
    GreenDragonMagePrepared,
    GreenDragonSpecial,
    GreenDragonSpecialPrepared,
    GreenDragonPotions,
    GreenDragonPotionsPrepared,
    FireGiant,
    FireGiantPrepared,
    ArdyFighter,
    AutoFighterBank,
    MossGiantBank,
    HillGiantBank,
    HillGiantBankPrepared,
    ChaosDruidBank,
    ArdyFighterBank,
    RockCrabBank,
    GreenDragonBank,
    GreenDragonBankPrepared,
    GreenDragonBankDefaultPrepared,
    GreenDragonTele,
    GreenDragonTelePrepared,
    FireGiantApproach,
    FireGiantBank,
    FireGiantBankPrepared,
    FireGiantCamelotPrepared,
    AioTeleport,
    AioTeleportFalador,
    AioTeleportNoStaff,
    ShopBuyout,
    ShopBuyoutAubury,
    ShopBuyoutLowe,
    ShopBuyoutHickton,
    ShopBuyoutHarry,
    ShopBuyoutBetty,
    ShopBuyoutGerrant,
    SmithingBot,
    SmithingBotPlatebody,
    LeatherCrafter,
    LeatherCrafterHardBody,
    Firemaker,
    FiremakerOak,
    ClimbingBoots,
    ClimbingBootsTeleport,
    RangingGuildRound,
    RangingGuildRedeem,
    RangingGuildBank,
    RangingGuildFull,
    BrimhavenMossInspectV1,
    RouteInspectBrimhavenV2,
    PrayerV2,
    PrayerV1,
    LineOfSightV2,
    ActorObservationV2,
    FightFieldV2,
    HoldSpotV2,
    RetreatSpotV2,
    WalkSpotV2,
    EnterLairV2,
    LeaveLairV2,
    AcquireKeyV2,
    CellV2,
    BankV2,
}

impl CoreCase {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "bone_burier" => Ok(Self::BoneBurier),
            "chicken_killer" => Ok(Self::ChickenKiller),
            "chicken_killer_bank" => Ok(Self::ChickenKillerBank),
            "thiever" => Ok(Self::Thiever),
            "alcher" => Ok(Self::Alcher),
            "alcher_defaults" => Ok(Self::AlcherDefaults),
            "alcher_custom" => Ok(Self::AlcherCustom),
            "alcher_custom_alias" => Ok(Self::AlcherCustomAlias),
            "alcher_custom_name" => Ok(Self::AlcherCustomName),
            "alcher_ordered" => Ok(Self::AlcherOrdered),
            "alcher_large_batch" => Ok(Self::AlcherLargeBatch),
            "alcher_low" => Ok(Self::AlcherLow),
            "alcher_fire_battlestaff" => Ok(Self::AlcherFireBattlestaff),
            "alcher_swarm_drain" => Ok(Self::AlcherSwarmDrain),
            "bank_fletcher" => Ok(Self::BankFletcher),
            "bank_fletcher_shafts" => Ok(Self::BankFletcherShafts),
            "bank_fletcher_headless" => Ok(Self::BankFletcherHeadless),
            "bank_fletcher_string" => Ok(Self::BankFletcherString),
            "bank_fletcher_cut_string" => Ok(Self::BankFletcherCutString),
            "dart_fletcher" => Ok(Self::DartFletcher),
            "dart_fletcher_iron" => Ok(Self::DartFletcherIron),
            "herb_cleaner" => Ok(Self::HerbCleaner),
            "herb_cleaner_named" => Ok(Self::HerbCleanerNamed),
            "herb_cleaner_empty_bank" => Ok(Self::HerbCleanerEmptyBank),
            "gem_cutter" => Ok(Self::GemCutter),
            "gem_cutter_named" => Ok(Self::GemCutterNamed),
            "door_opener" => Ok(Self::DoorOpener),
            "door_opener_gate" => Ok(Self::DoorOpenerGate),
            "gnome_course" => Ok(Self::GnomeCourse),
            "gnome_course_radius" => Ok(Self::GnomeCourseRadius),
            "wildy_agility" => Ok(Self::WildyAgility),
            "brimhaven_agility" => Ok(Self::BrimhavenAgility),
            "flax_picker" => Ok(Self::FlaxPicker),
            "superheater" => Ok(Self::Superheater),
            "superheater_steel" => Ok(Self::SuperheaterSteel),
            "superheater_fire_battlestaff" => Ok(Self::SuperheaterFireBattlestaff),
            "superheater_silver_low_natures" => Ok(Self::SuperheaterSilverLowNatures),
            "vial_filler" => Ok(Self::VialFiller),
            "vial_filler_east" => Ok(Self::VialFillerEast),
            "potion_maker" => Ok(Self::PotionMaker),
            "potion_maker_named" => Ok(Self::PotionMakerNamed),
            "tanner_bot" => Ok(Self::TannerBot),
            "tanner_bot_hard" => Ok(Self::TannerBotHard),
            "rune_crafter" => Ok(Self::RuneCrafter),
            "rune_crafter_earth" => Ok(Self::RuneCrafterEarth),
            "mule_crafter" => Ok(Self::MuleCrafter),
            "ardy_cakes" => Ok(Self::ArdyCakes),
            "ardy_cakes_fight" => Ok(Self::ArdyCakesFight),
            "ardy_thiever" => Ok(Self::ArdyThiever),
            "ardy_thiever_fight" => Ok(Self::ArdyThieverFight),
            "ardy_thiever_knight" => Ok(Self::ArdyThieverKnight),
            "gnome_chop" => Ok(Self::GnomeChop),
            "gnome_fletch_short" => Ok(Self::GnomeFletchShort),
            "gnome_fletch_long" => Ok(Self::GnomeFletchLong),
            "coal_trucks" => Ok(Self::CoalTrucks),
            "cook_bot" => Ok(Self::CookBot),
            "cook_bot_lobster" => Ok(Self::CookBotLobster),
            "smelter_bot" => Ok(Self::SmelterBot),
            "smelter_bot_steel" => Ok(Self::SmelterBotSteel),
            "flax_spinner" => Ok(Self::FlaxSpinner),
            "flax_aio" => Ok(Self::FlaxAio),
            "flax_aio_pick" => Ok(Self::FlaxAioPick),
            "flax_aio_spin" => Ok(Self::FlaxAioSpin),
            "herblore_secondaries" => Ok(Self::HerbloreSecondaries),
            "herblore_secondaries_newt" => Ok(Self::HerbloreSecondariesNewt),
            "chaos_druid" => Ok(Self::ChaosDruid),
            "chaos_druid_tower" => Ok(Self::ChaosDruidTower),
            "chaos_druid_yanille" => Ok(Self::ChaosDruidYanille),
            "moss_giant" => Ok(Self::MossGiant),
            "moss_giant_prepared" => Ok(Self::MossGiantPrepared),
            "moss_giant_dart" => Ok(Self::MossGiantDart),
            "hill_giant" => Ok(Self::HillGiant),
            "auto_fighter" => Ok(Self::AutoFighter),
            "auto_fighter_mage" => Ok(Self::AutoFighterMage),
            "auto_fighter_range" => Ok(Self::AutoFighterRange),
            "rock_crab" => Ok(Self::RockCrab),
            "rock_crab_range" => Ok(Self::RockCrabRange),
            "green_dragon" => Ok(Self::GreenDragon),
            "green_dragon_prepared" => Ok(Self::GreenDragonPrepared),
            "green_dragon_mage_prepared" => Ok(Self::GreenDragonMagePrepared),
            "green_dragon_special" => Ok(Self::GreenDragonSpecial),
            "green_dragon_special_prepared" => Ok(Self::GreenDragonSpecialPrepared),
            "green_dragon_potions" => Ok(Self::GreenDragonPotions),
            "green_dragon_potions_prepared" => Ok(Self::GreenDragonPotionsPrepared),
            "fire_giant" => Ok(Self::FireGiant),
            "fire_giant_prepared" => Ok(Self::FireGiantPrepared),
            "ardy_fighter" => Ok(Self::ArdyFighter),
            "auto_fighter_bank" => Ok(Self::AutoFighterBank),
            "moss_giant_bank" => Ok(Self::MossGiantBank),
            "hill_giant_bank" => Ok(Self::HillGiantBank),
            "hill_giant_bank_prepared" => Ok(Self::HillGiantBankPrepared),
            "chaos_druid_bank" => Ok(Self::ChaosDruidBank),
            "ardy_fighter_bank" => Ok(Self::ArdyFighterBank),
            "rock_crab_bank" => Ok(Self::RockCrabBank),
            "green_dragon_bank" => Ok(Self::GreenDragonBank),
            "green_dragon_bank_prepared" => Ok(Self::GreenDragonBankPrepared),
            "green_dragon_bank_default_prepared" => Ok(Self::GreenDragonBankDefaultPrepared),
            "green_dragon_tele" => Ok(Self::GreenDragonTele),
            "green_dragon_tele_prepared" => Ok(Self::GreenDragonTelePrepared),
            "fire_giant_approach" => Ok(Self::FireGiantApproach),
            "fire_giant_bank" => Ok(Self::FireGiantBank),
            "fire_giant_bank_prepared" => Ok(Self::FireGiantBankPrepared),
            "fire_giant_camelot_prepared" => Ok(Self::FireGiantCamelotPrepared),
            "aio_teleport" => Ok(Self::AioTeleport),
            "aio_teleport_falador" => Ok(Self::AioTeleportFalador),
            "aio_teleport_no_staff" => Ok(Self::AioTeleportNoStaff),
            "shop_buyout" => Ok(Self::ShopBuyout),
            "shop_buyout_aubury" => Ok(Self::ShopBuyoutAubury),
            "shop_buyout_lowe" => Ok(Self::ShopBuyoutLowe),
            "shop_buyout_hickton" => Ok(Self::ShopBuyoutHickton),
            "shop_buyout_harry" => Ok(Self::ShopBuyoutHarry),
            "shop_buyout_betty" => Ok(Self::ShopBuyoutBetty),
            "shop_buyout_gerrant" => Ok(Self::ShopBuyoutGerrant),
            "smithing_bot" => Ok(Self::SmithingBot),
            "smithing_bot_platebody" => Ok(Self::SmithingBotPlatebody),
            "leather_crafter" => Ok(Self::LeatherCrafter),
            "leather_crafter_hard_body" => Ok(Self::LeatherCrafterHardBody),
            "firemaker" => Ok(Self::Firemaker),
            "firemaker_oak" => Ok(Self::FiremakerOak),
            "climbing_boots" => Ok(Self::ClimbingBoots),
            "climbing_boots_teleport" => Ok(Self::ClimbingBootsTeleport),
            "ranging_guild_round" => Ok(Self::RangingGuildRound),
            "ranging_guild_redeem" => Ok(Self::RangingGuildRedeem),
            "ranging_guild_bank" => Ok(Self::RangingGuildBank),
            "ranging_guild_full" => Ok(Self::RangingGuildFull),
            "brimhaven_moss_inspect_v1" => Ok(Self::BrimhavenMossInspectV1),
            "route_inspect_brimhaven_v2_ts" => Ok(Self::RouteInspectBrimhavenV2),
            "prayer_v2_ts" => Ok(Self::PrayerV2),
            "prayer_v1_ts" => Ok(Self::PrayerV1),
            "line_of_sight_v2_ts" => Ok(Self::LineOfSightV2),
            "actor_observation_v2_ts" => Ok(Self::ActorObservationV2),
            "fight_field_v2_ts" => Ok(Self::FightFieldV2),
            "hold_spot_v2_ts" => Ok(Self::HoldSpotV2),
            "retreat_spot_v2_ts" => Ok(Self::RetreatSpotV2),
            "walk_spot_v2_ts" => Ok(Self::WalkSpotV2),
            "enter_lair_v2_ts" => Ok(Self::EnterLairV2),
            "leave_lair_v2_ts" => Ok(Self::LeaveLairV2),
            "acquire_key_v2_ts" => Ok(Self::AcquireKeyV2),
            "cell_v2_ts" => Ok(Self::CellV2),
            "bank_v2_ts" => Ok(Self::BankV2),
            _ => Err(format!(
                "unknown CATALOG_SCENARIO {value:?}; expected {CORE_SCENARIOS}"
            )),
        }
    }

    pub fn scenario_name(self) -> &'static str {
        match self {
            Self::BoneBurier => "bone_burier",
            Self::ChickenKiller => "chicken_killer",
            Self::ChickenKillerBank => "chicken_killer_bank",
            Self::Thiever => "thiever",
            Self::Alcher => "alcher",
            Self::AlcherDefaults => "alcher_defaults",
            Self::AlcherCustom => "alcher_custom",
            Self::AlcherCustomAlias => "alcher_custom_alias",
            Self::AlcherCustomName => "alcher_custom_name",
            Self::AlcherOrdered => "alcher_ordered",
            Self::AlcherLargeBatch => "alcher_large_batch",
            Self::AlcherLow => "alcher_low",
            Self::AlcherFireBattlestaff => "alcher_fire_battlestaff",
            Self::AlcherSwarmDrain => "alcher_swarm_drain",
            Self::BankFletcher => "bank_fletcher",
            Self::BankFletcherShafts => "bank_fletcher_shafts",
            Self::BankFletcherHeadless => "bank_fletcher_headless",
            Self::BankFletcherString => "bank_fletcher_string",
            Self::BankFletcherCutString => "bank_fletcher_cut_string",
            Self::DartFletcher => "dart_fletcher",
            Self::DartFletcherIron => "dart_fletcher_iron",
            Self::HerbCleaner => "herb_cleaner",
            Self::HerbCleanerNamed => "herb_cleaner_named",
            Self::HerbCleanerEmptyBank => "herb_cleaner_empty_bank",
            Self::GemCutter => "gem_cutter",
            Self::GemCutterNamed => "gem_cutter_named",
            Self::DoorOpener => "door_opener",
            Self::DoorOpenerGate => "door_opener_gate",
            Self::GnomeCourse => "gnome_course",
            Self::GnomeCourseRadius => "gnome_course_radius",
            Self::WildyAgility => "wildy_agility",
            Self::BrimhavenAgility => "brimhaven_agility",
            Self::FlaxPicker => "flax_picker",
            Self::Superheater => "superheater",
            Self::SuperheaterSteel => "superheater_steel",
            Self::SuperheaterFireBattlestaff => "superheater_fire_battlestaff",
            Self::SuperheaterSilverLowNatures => "superheater_silver_low_natures",
            Self::VialFiller => "vial_filler",
            Self::VialFillerEast => "vial_filler_east",
            Self::PotionMaker => "potion_maker",
            Self::PotionMakerNamed => "potion_maker_named",
            Self::TannerBot => "tanner_bot",
            Self::TannerBotHard => "tanner_bot_hard",
            Self::RuneCrafter => "rune_crafter",
            Self::RuneCrafterEarth => "rune_crafter_earth",
            Self::MuleCrafter => "mule_crafter",
            Self::ArdyCakes => "ardy_cakes",
            Self::ArdyCakesFight => "ardy_cakes_fight",
            Self::ArdyThiever => "ardy_thiever",
            Self::ArdyThieverFight => "ardy_thiever_fight",
            Self::ArdyThieverKnight => "ardy_thiever_knight",
            Self::GnomeChop => "gnome_chop",
            Self::GnomeFletchShort => "gnome_fletch_short",
            Self::GnomeFletchLong => "gnome_fletch_long",
            Self::CoalTrucks => "coal_trucks",
            Self::CookBot => "cook_bot",
            Self::CookBotLobster => "cook_bot_lobster",
            Self::SmelterBot => "smelter_bot",
            Self::SmelterBotSteel => "smelter_bot_steel",
            Self::FlaxSpinner => "flax_spinner",
            Self::FlaxAio => "flax_aio",
            Self::FlaxAioPick => "flax_aio_pick",
            Self::FlaxAioSpin => "flax_aio_spin",
            Self::HerbloreSecondaries => "herblore_secondaries",
            Self::HerbloreSecondariesNewt => "herblore_secondaries_newt",
            Self::ChaosDruid => "chaos_druid",
            Self::ChaosDruidTower => "chaos_druid_tower",
            Self::ChaosDruidYanille => "chaos_druid_yanille",
            Self::MossGiant => "moss_giant",
            Self::MossGiantPrepared => "moss_giant_prepared",
            Self::MossGiantDart => "moss_giant_dart",
            Self::HillGiant => "hill_giant",
            Self::AutoFighter => "auto_fighter",
            Self::AutoFighterMage => "auto_fighter_mage",
            Self::AutoFighterRange => "auto_fighter_range",
            Self::RockCrab => "rock_crab",
            Self::RockCrabRange => "rock_crab_range",
            Self::GreenDragon => "green_dragon",
            Self::GreenDragonPrepared => "green_dragon_prepared",
            Self::GreenDragonMagePrepared => "green_dragon_mage_prepared",
            Self::GreenDragonSpecial => "green_dragon_special",
            Self::GreenDragonSpecialPrepared => "green_dragon_special_prepared",
            Self::GreenDragonPotions => "green_dragon_potions",
            Self::GreenDragonPotionsPrepared => "green_dragon_potions_prepared",
            Self::FireGiant => "fire_giant",
            Self::FireGiantPrepared => "fire_giant_prepared",
            Self::ArdyFighter => "ardy_fighter",
            Self::AutoFighterBank => "auto_fighter_bank",
            Self::MossGiantBank => "moss_giant_bank",
            Self::HillGiantBank => "hill_giant_bank",
            Self::HillGiantBankPrepared => "hill_giant_bank_prepared",
            Self::ChaosDruidBank => "chaos_druid_bank",
            Self::ArdyFighterBank => "ardy_fighter_bank",
            Self::RockCrabBank => "rock_crab_bank",
            Self::GreenDragonBank => "green_dragon_bank",
            Self::GreenDragonBankPrepared => "green_dragon_bank_prepared",
            Self::GreenDragonBankDefaultPrepared => "green_dragon_bank_default_prepared",
            Self::GreenDragonTele => "green_dragon_tele",
            Self::GreenDragonTelePrepared => "green_dragon_tele_prepared",
            Self::FireGiantApproach => "fire_giant_approach",
            Self::FireGiantBank => "fire_giant_bank",
            Self::FireGiantBankPrepared => "fire_giant_bank_prepared",
            Self::FireGiantCamelotPrepared => "fire_giant_camelot_prepared",
            Self::AioTeleport => "aio_teleport",
            Self::AioTeleportFalador => "aio_teleport_falador",
            Self::AioTeleportNoStaff => "aio_teleport_no_staff",
            Self::ShopBuyout => "shop_buyout",
            Self::ShopBuyoutAubury => "shop_buyout_aubury",
            Self::ShopBuyoutLowe => "shop_buyout_lowe",
            Self::ShopBuyoutHickton => "shop_buyout_hickton",
            Self::ShopBuyoutHarry => "shop_buyout_harry",
            Self::ShopBuyoutBetty => "shop_buyout_betty",
            Self::ShopBuyoutGerrant => "shop_buyout_gerrant",
            Self::SmithingBot => "smithing_bot",
            Self::SmithingBotPlatebody => "smithing_bot_platebody",
            Self::LeatherCrafter => "leather_crafter",
            Self::LeatherCrafterHardBody => "leather_crafter_hard_body",
            Self::Firemaker => "firemaker",
            Self::FiremakerOak => "firemaker_oak",
            Self::ClimbingBoots => "climbing_boots",
            Self::ClimbingBootsTeleport => "climbing_boots_teleport",
            Self::RangingGuildRound => "ranging_guild_round",
            Self::RangingGuildRedeem => "ranging_guild_redeem",
            Self::RangingGuildBank => "ranging_guild_bank",
            Self::RangingGuildFull => "ranging_guild_full",
            Self::BrimhavenMossInspectV1 => "brimhaven_moss_inspect_v1",
            Self::RouteInspectBrimhavenV2 => "route_inspect_brimhaven_v2_ts",
            Self::PrayerV2 => "prayer_v2_ts",
            Self::PrayerV1 => "prayer_v1_ts",
            Self::LineOfSightV2 => "line_of_sight_v2_ts",
            Self::ActorObservationV2 => "actor_observation_v2_ts",
            Self::FightFieldV2 => "fight_field_v2_ts",
            Self::HoldSpotV2 => "hold_spot_v2_ts",
            Self::RetreatSpotV2 => "retreat_spot_v2_ts",
            Self::WalkSpotV2 => "walk_spot_v2_ts",
            Self::EnterLairV2 => "enter_lair_v2_ts",
            Self::LeaveLairV2 => "leave_lair_v2_ts",
            Self::AcquireKeyV2 => "acquire_key_v2_ts",
            Self::CellV2 => "cell_v2_ts",
            Self::BankV2 => "bank_v2_ts",
        }
    }

    pub fn card_name(self) -> &'static str {
        match self {
            Self::BoneBurier => "BoneBurier",
            Self::ChickenKiller | Self::ChickenKillerBank => "ChickenKiller",
            Self::Thiever => "Thiever",
            Self::Alcher | Self::AlcherDefaults => "Alcher",
            Self::AlcherCustom
            | Self::AlcherCustomAlias
            | Self::AlcherCustomName
            | Self::AlcherOrdered
            | Self::AlcherLargeBatch
            | Self::AlcherLow
            | Self::AlcherFireBattlestaff
            | Self::AlcherSwarmDrain => "Alcher",
            Self::BankFletcher
            | Self::BankFletcherShafts
            | Self::BankFletcherHeadless
            | Self::BankFletcherString
            | Self::BankFletcherCutString => "BankFletcher",
            Self::DartFletcher | Self::DartFletcherIron => "DartFletcher",
            Self::HerbCleaner | Self::HerbCleanerNamed | Self::HerbCleanerEmptyBank => {
                "HerbCleaner"
            }
            Self::GemCutter | Self::GemCutterNamed => "GemCutter",
            Self::DoorOpener | Self::DoorOpenerGate => "DoorOpener",
            Self::GnomeCourse | Self::GnomeCourseRadius => "GnomeCourse",
            Self::WildyAgility => "WildyAgility",
            Self::BrimhavenAgility => "BrimhavenAgility",
            Self::FlaxPicker => "FlaxPicker",
            Self::Superheater
            | Self::SuperheaterSteel
            | Self::SuperheaterFireBattlestaff
            | Self::SuperheaterSilverLowNatures => "Superheater",
            Self::VialFiller | Self::VialFillerEast => "VialFiller",
            Self::PotionMaker | Self::PotionMakerNamed => "PotionMaker",
            Self::TannerBot | Self::TannerBotHard => "TannerBot",
            Self::RuneCrafter | Self::RuneCrafterEarth => "RuneCrafter",
            Self::MuleCrafter => "MuleCrafter",
            Self::ArdyCakes | Self::ArdyCakesFight => "ArdyCakes",
            Self::ArdyThiever | Self::ArdyThieverFight | Self::ArdyThieverKnight => "ArdyThiever",
            Self::GnomeChop | Self::GnomeFletchShort | Self::GnomeFletchLong => "GnomeMagicChopper",
            Self::CoalTrucks => "CoalTrucks",
            Self::CookBot | Self::CookBotLobster => "CookBot",
            Self::SmelterBot | Self::SmelterBotSteel => "SmelterBot",
            Self::FlaxSpinner => "FlaxSpinner",
            Self::FlaxAio | Self::FlaxAioPick | Self::FlaxAioSpin => "FlaxAIO",
            Self::HerbloreSecondaries | Self::HerbloreSecondariesNewt => "HerbloreSecondaries",
            Self::ChaosDruid
            | Self::ChaosDruidTower
            | Self::ChaosDruidYanille
            | Self::ChaosDruidBank => "ChaosDruidKiller",
            Self::MossGiant
            | Self::MossGiantPrepared
            | Self::MossGiantDart
            | Self::MossGiantBank => "MossGiant",
            Self::HillGiant | Self::HillGiantBank | Self::HillGiantBankPrepared => "HillGiant",
            Self::AutoFighter
            | Self::AutoFighterMage
            | Self::AutoFighterRange
            | Self::AutoFighterBank => "AutoFighter",
            Self::RockCrab | Self::RockCrabRange | Self::RockCrabBank => "RockCrab",
            Self::GreenDragon
            | Self::GreenDragonPrepared
            | Self::GreenDragonMagePrepared
            | Self::GreenDragonSpecial
            | Self::GreenDragonSpecialPrepared
            | Self::GreenDragonPotions
            | Self::GreenDragonPotionsPrepared
            | Self::GreenDragonBank
            | Self::GreenDragonBankPrepared
            | Self::GreenDragonBankDefaultPrepared
            | Self::GreenDragonTele
            | Self::GreenDragonTelePrepared => "GreenDragon",
            Self::FireGiant
            | Self::FireGiantPrepared
            | Self::FireGiantApproach
            | Self::FireGiantBank
            | Self::FireGiantBankPrepared
            | Self::FireGiantCamelotPrepared => "FireGiant",
            Self::ArdyFighter | Self::ArdyFighterBank => "ArdyFighter",
            Self::AioTeleport | Self::AioTeleportFalador | Self::AioTeleportNoStaff => {
                "AIO Teleport"
            }
            Self::ShopBuyout
            | Self::ShopBuyoutAubury
            | Self::ShopBuyoutLowe
            | Self::ShopBuyoutHickton
            | Self::ShopBuyoutHarry
            | Self::ShopBuyoutBetty
            | Self::ShopBuyoutGerrant => "ShopBuyout",
            Self::SmithingBot | Self::SmithingBotPlatebody => "SmithingBot",
            Self::LeatherCrafter | Self::LeatherCrafterHardBody => "LeatherCrafter",
            Self::Firemaker | Self::FiremakerOak => "Firemaker",
            Self::ClimbingBoots | Self::ClimbingBootsTeleport => "ClimbingBoots",
            Self::RangingGuildRound
            | Self::RangingGuildRedeem
            | Self::RangingGuildBank
            | Self::RangingGuildFull => "RangingGuild",
            Self::BrimhavenMossInspectV1 => "BrimhavenMossGiants",
            Self::RouteInspectBrimhavenV2 => "route_inspect_brimhaven_v2",
            Self::PrayerV2 => "prayer_v2",
            Self::PrayerV1 => "prayer_v1",
            Self::LineOfSightV2 => "line_of_sight_v2",
            Self::ActorObservationV2 => "actor_observation_v2",
            Self::FightFieldV2 => "fight_field_v2",
            Self::HoldSpotV2 => "hold_spot_v2",
            Self::RetreatSpotV2 => "retreat_spot_v2",
            Self::WalkSpotV2 => "walk_spot_v2",
            Self::EnterLairV2 => "enter_lair_v2",
            Self::LeaveLairV2 => "leave_lair_v2",
            Self::AcquireKeyV2 => "acquire_key_v2",
            Self::CellV2 => "cell_v2",
            Self::BankV2 => "bank_v2",
        }
    }

    pub fn copies_route_inspect(self) -> bool {
        matches!(
            self,
            Self::BrimhavenMossInspectV1 | Self::RouteInspectBrimhavenV2
        )
    }

    pub fn copies_prayer_varps(self) -> bool {
        matches!(self, Self::PrayerV2 | Self::PrayerV1)
    }

    pub fn copies_line_of_sight(self) -> bool {
        matches!(self, Self::LineOfSightV2)
    }

    pub fn copies_actor_observation(self) -> bool {
        matches!(self, Self::ActorObservationV2)
    }

    pub fn copies_fight_field(self) -> bool {
        matches!(self, Self::FightFieldV2)
    }

    /// The v2 hunt File cell this case witnesses, if any. Callers attach
    /// paint and the slot's act ledger only then.
    pub fn hunt_cell(self) -> Option<HuntCell> {
        Some(match self {
            Self::HoldSpotV2 => HuntCell::Hold,
            Self::RetreatSpotV2 => HuntCell::Retreat,
            Self::WalkSpotV2 => HuntCell::WalkSpot,
            Self::EnterLairV2 => HuntCell::Enter,
            Self::LeaveLairV2 => HuntCell::Leave,
            Self::AcquireKeyV2 => HuntCell::Key,
            Self::CellV2 => HuntCell::Cell,
            Self::BankV2 => HuntCell::Bank,
            _ => return None,
        })
    }

    pub fn prayer_stop_reason(self) -> Option<&'static str> {
        match self {
            Self::PrayerV2 => Some(PRAYER_V2_STOP),
            Self::PrayerV1 => Some(PRAYER_V1_STOP),
            _ => None,
        }
    }

    pub fn los_stop_reason(self) -> Option<&'static str> {
        match self {
            Self::LineOfSightV2 => Some(LOS_V2_STOP),
            _ => None,
        }
    }

    pub fn actor_stop_reason(self) -> Option<&'static str> {
        match self {
            Self::ActorObservationV2 => Some(ACTOR_OBSERVATION_V2_STOP),
            _ => None,
        }
    }

    pub fn fight_field_stop_reason(self) -> Option<&'static str> {
        match self {
            Self::FightFieldV2 => Some(FIGHT_FIELD_V2_STOP),
            _ => None,
        }
    }
}

pub fn validate_case_catalog(case: CoreCase, commit: &str) -> Result<(), String> {
    if case == CoreCase::BankFletcherCutString && commit == CATALOG_COMMIT_A {
        return Err(format!(
            "catalog {CATALOG_COMMIT_A} BankFletcher has no mode setting; bank_fletcher_cut_string is supported only by {CATALOG_COMMIT_B}"
        ));
    }
    if case == CoreCase::SuperheaterFireBattlestaff && commit == CATALOG_COMMIT_A {
        return Err(format!(
            "catalog {CATALOG_COMMIT_A} Superheater requires Staff of fire; superheater_fire_battlestaff is supported only by {CATALOG_COMMIT_B}"
        ));
    }
    if matches!(case, CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff)
        && (commit == CATALOG_COMMIT_A || commit == CATALOG_COMMIT_B)
    {
        return Err(format!(
            "catalog {commit} Alcher is High-only and withdraws Staff of fire; the Low spell and the \
             alternative-staff branch are supported only by the current 96410ec5 Alcher (spell setting \
             and pickFireStaff over FIRE_STAVES)"
        ));
    }
    if case == CoreCase::HerbCleanerEmptyBank
        && (commit == CATALOG_COMMIT_A || commit == CATALOG_COMMIT_B)
    {
        return Err(format!(
            "catalog {commit} HerbCleaner does not carry the frozen 96410ec5 eventual empty-bank Stop behavior"
        ));
    }
    if case == CoreCase::AlcherSwarmDrain
        && (commit == CATALOG_COMMIT_A || commit == CATALOG_COMMIT_B)
    {
        return Err(format!(
            "catalog {commit} Alcher may batch a whole trip; alcher_swarm_drain is supported only by \
             the current 96410ec5 Alcher (single-cast yield between loop iterations)"
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Observation {
    pub ingame: bool,
    pub scene_state: i32,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub combat_level: i32,
    pub tick: u32,
    pub items: BTreeMap<String, i32>,
    pub item_ids: BTreeMap<i32, i32>,
    pub bank: BTreeMap<String, i32>,
    pub bank_ids: BTreeMap<i32, i32>,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    /// Bounded native receipt for ScriptRunner.stop from this Start. The
    /// panel log queue is not consumed to populate it.
    pub script_lifecycle: Option<script::ScriptLifecycleReceipt>,
    /// Native base skill levels from the snapshot stat table.
    pub levels: BTreeMap<String, i32>,
    /// Native effective skill levels, retained separately for fixture gates.
    pub effective_levels: BTreeMap<String, i32>,
    pub xp: BTreeMap<String, i32>,
    /// Only varps explicitly used by a core witness; do not clone the full table.
    pub varps: BTreeMap<i32, i32>,
    pub chat: Vec<(i32, String)>,
    pub loc_facts: Vec<BoundedLoc>,
    pub npc_facts: Vec<BoundedNpc>,
    pub magic_tree_ready: bool,
    pub dormant_rocks_seen: bool,
    pub ground_loot: Vec<BoundedGround>,
    pub local_in_combat: bool,
    /// Frozen snapshot positive-hit predicate. Zero/blocked/expired hits do not qualify.
    pub taking_damage: bool,
    pub local_target_npc: Option<usize>,
    pub local_health: i32,
    pub local_animation: i32,
    /// Prior-frame guardian publication. Snapshot observe runs before this
    /// frame's status-row copy of `RandomStatus`.
    pub guardian: BoundedGuardian,
    pub equipment_ids: BTreeMap<i32, i32>,
    pub main_modal: i32,
    pub widget_ids: BTreeSet<i32>,
    /// Compact open-shop stock (empty while the shop is down). Not a full
    /// shop clone.
    pub shop_open: bool,
    pub shop_stock: Vec<BoundedShopItem>,
    /// Posted anvil/main-skill-multi row ids this frame (empty if the panel
    /// was not decoded). Chat `make_products` does not fill this.
    pub main_make_ids: BTreeSet<i32>,
    /// Already-published inspect terminal, attached only for inspect Core
    /// cases. Default empty; `from_snapshot` does not copy hops.
    pub route_inspect_seq: u64,
    pub route_inspect_generation: u64,
    pub route_inspect_request_id: u64,
    pub route_inspect_ok: bool,
    pub route_inspect_reason: String,
    #[serde(rename = "route_inspect_hops")]
    pub route_inspect_hops: Vec<RouteInspectHopFact>,
    /// `InspectNav.generation` even when no terminal is published.
    /// Missing-terminal Observation defaults are 0 and are not this value.
    pub route_inspect_live_generation: u64,
    pub route_inspect_has_terminal: bool,
    /// Compact current-plane identity and selected pair cells. Empty unless
    /// the active Core case asked for collision. Never a whole-grid dump.
    pub los: LineOfSightObservation,
    /// Compact chosen NPC + LOS helper result. Empty unless the active Core
    /// case asked for actor observation. Never a world or NPC-table copy.
    pub actor: ActorObservation,
    /// Compact fight-field NPC + both LOS helper results. Empty unless the
    /// active Core case asked for fight field. Never a world or NPC-table copy.
    pub fight: FightFieldObservation,
    /// Compact hunt witness: the slot's act ledger and the cell's paint
    /// receipt. Empty unless the active Core case is a hunt cell.
    pub hunt: HuntObservation,
}

/// Compact hop projection for Core JSON. Only `locName` is copied from the
/// already-published host terminal.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteInspectHopFact {
    pub loc_name: String,
}

/// Identity/freshness/ok/hop names already published by the host inspect
/// terminal. Copied only when the active Core case asks for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteInspectPublished {
    pub live_generation: u64,
    pub has_terminal: bool,
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub ok: bool,
    pub reason: String,
    pub hop_loc_names: Vec<String>,
}

/// Compact proof that this run observed the exact HerbCleaner seed in a
/// loaded bank before the bank was closed for Start. Closed snapshots
/// intentionally do not retain bank contents.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HerbCleanerStartPreparationReceipt {
    pub player: String,
    pub tick: u32,
    pub bank_generation: u64,
    pub guam_count: i32,
    pub marrentill_count: i32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbCleanerStartPreparation {
    loaded: Option<HerbCleanerStartPreparationReceipt>,
    closed_after_loaded: bool,
}

impl HerbCleanerStartPreparation {
    pub fn observe(&mut self, account: &str, observation: &Observation) {
        let expected = client::util::jstring::JString::to_screen_name(account);
        if !observation.ingame
            || observation.scene_state != 2
            || !observation
                .player
                .as_deref()
                .is_some_and(|player| player.eq_ignore_ascii_case(&expected))
        {
            *self = Self::default();
            return;
        }

        if observation.bank_open && observation.bank_loaded {
            let guam_count = observation.bank_item_id(UNIDENTIFIED_GUAM_ID);
            let marrentill_count = observation.bank_item_id(UNIDENTIFIED_MARENTILL_ID);
            self.loaded = (guam_count == 20 && marrentill_count == 0).then(|| {
                HerbCleanerStartPreparationReceipt {
                    player: observation.player.clone().expect("player checked above"),
                    tick: observation.tick,
                    bank_generation: observation.bank_generation,
                    guam_count,
                    marrentill_count,
                }
            });
            self.closed_after_loaded = false;
            return;
        }

        if observation.bank_open || observation.bank_loaded {
            *self = Self::default();
            return;
        }

        self.closed_after_loaded = self.loaded.as_ref().is_some_and(|loaded| {
            observation.bank_generation > loaded.bank_generation
                && observation.tick >= loaded.tick
                && observation.bank.is_empty()
                && observation.bank_ids.is_empty()
        });
    }

    pub fn receipt_for(
        &self,
        account: &str,
        baseline: &Observation,
    ) -> Option<HerbCleanerStartPreparationReceipt> {
        if !self.closed_after_loaded || baseline.bank_open || baseline.bank_loaded {
            return None;
        }
        let expected = client::util::jstring::JString::to_screen_name(account);
        let loaded = self.loaded.as_ref()?;
        (baseline.bank.is_empty()
            && baseline.bank_ids.is_empty()
            && baseline.bank_generation > loaded.bank_generation
            && baseline.tick >= loaded.tick
            && baseline
                .player
                .as_deref()
                .is_some_and(|player| player.eq_ignore_ascii_case(&expected))
            && loaded.player.eq_ignore_ascii_case(&expected))
        .then(|| loaded.clone())
    }
}

/// One loc retained for these named cases. The live loc sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedLoc {
    pub id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub name: Option<String>,
    pub open: bool,
}

/// Compact prior-frame guardian fact. Not a history buffer and not the chrome row.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct BoundedGuardian {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub ours: bool,
    pub hold: bool,
}

/// Compact NPC identity used by combat cores. The live NPC sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedNpc {
    pub index: usize,
    pub name: Option<String>,
    pub health: i32,
    pub total_health: i32,
    pub animation: i32,
    pub in_combat: bool,
    pub targeting_local: bool,
    pub tile: (i32, i32, i32),
    pub distance: i32,
}

/// Compact ground loot of combat-core item ids. The live ground sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedGround {
    pub id: i32,
    pub count: i32,
    pub tile: (i32, i32, i32),
    pub distance: i32,
}

/// One posted shop-stock row retained for shop-buyout cores.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedShopItem {
    pub id: i32,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FlaxLocFailureFact {
    pub id: i32,
    pub tile: (i32, i32, i32),
    pub name: Option<String>,
    pub actions: Vec<String>,
    pub field_distance: i32,
    pub player_distance: i32,
    pub reachable: bool,
    pub reachable_adj: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FlaxAioPickFailureFacts {
    pub player_tile: Option<(i32, i32, i32)>,
    pub field_center: (i32, i32, i32),
    pub field_scope: i32,
    pub at_field: bool,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    pub reachability_source: &'static str,
    pub adapter_query: &'static str,
    pub reachability_available: bool,
    pub relevant_locs: Vec<FlaxLocFailureFact>,
    pub nearest_reachable: Option<(i32, i32, i32)>,
}

pub fn flax_aio_pick_failure_facts(
    case: CoreCase,
    observation: &Observation,
    scene: &SceneView,
    locs: &[LocView],
) -> Option<FlaxAioPickFailureFacts> {
    if case != CoreCase::FlaxAioPick {
        return None;
    }
    let player_tile = observation.tile;
    let player_world = player_tile.map(|(x, z, level)| WorldTile { x, z, level });
    let flood = api::query::SceneQuery::new(scene, player_world).flood_reach();
    let field_world = WorldTile {
        x: FLAX_FIELD.0,
        z: FLAX_FIELD.1,
        level: FLAX_FIELD.2,
    };
    let distance = |a: WorldTile, b: WorldTile| {
        let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
        if a.level == b.level {
            xz
        } else {
            1_000_000 + xz
        }
    };
    let mut relevant_locs = locs
        .iter()
        .filter(|loc| {
            loc.name
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("Flax"))
                && loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case("Pick"))
                && distance(loc.tile, field_world) <= FLAX_FIELD_SCOPE
        })
        .map(|loc| {
            let (reachable, reachable_adj) = flood
                .as_ref()
                .map(|flood| flood.at(&loc.tile))
                .unwrap_or((false, false));
            FlaxLocFailureFact {
                id: loc.id,
                tile: (loc.tile.x, loc.tile.z, loc.tile.level),
                name: loc.name.clone(),
                actions: loc.actions.iter().flatten().cloned().collect(),
                field_distance: distance(loc.tile, field_world),
                player_distance: player_world
                    .map(|player| distance(loc.tile, player))
                    .unwrap_or(1_000_000),
                reachable,
                reachable_adj,
            }
        })
        .collect::<Vec<_>>();
    relevant_locs.sort_by_key(|loc| loc.player_distance);
    let nearest_reachable = relevant_locs
        .iter()
        .find(|loc| loc.reachable_adj)
        .map(|loc| loc.tile);

    Some(FlaxAioPickFailureFacts {
        player_tile,
        field_center: FLAX_FIELD,
        field_scope: FLAX_FIELD_SCOPE,
        at_field: player_world
            .is_some_and(|player| distance(player, field_world) <= FLAX_FIELD_SCOPE),
        bank_open: observation.bank_open,
        bank_loaded: observation.bank_loaded,
        bank_generation: observation.bank_generation,
        reachability_source: "api::query::SceneQuery::flood_reach().at(tile)",
        adapter_query: "Reachability.canReach(tile,{adjacentOk:true,maxSteps:400})",
        reachability_available: flood.is_some(),
        relevant_locs,
        nearest_reachable,
    })
}

pub fn failure_diagnostic(case: CoreCase, snapshot: &GameSnapshot, names: &ObjNames) -> Value {
    let observation = Observation::from_snapshot(snapshot, names);
    json!({
        "observation": &observation,
        "flax_aio_pick": flax_aio_pick_failure_facts(
            case,
            &observation,
            snapshot.scene(),
            snapshot.locs(),
        ),
    })
}

impl Observation {
    pub fn from_snapshot(snapshot: &GameSnapshot, names: &ObjNames) -> Self {
        let mut items = BTreeMap::new();
        for (id, count) in snapshot.inv() {
            let name = names
                .name(*id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("obj#{id}"));
            *items.entry(name).or_insert(0) += *count;
        }
        let mut item_ids = BTreeMap::new();
        for (id, count) in snapshot.inv() {
            *item_ids.entry(*id).or_insert(0) += *count;
        }
        let xp = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.xp))
            .collect();
        let mut bank = BTreeMap::new();
        for row in snapshot.bank() {
            let name = names
                .name(row.def.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("obj#{}", row.def.id));
            *bank.entry(name).or_insert(0) += row.count;
        }
        let mut bank_ids = BTreeMap::new();
        for row in snapshot.bank() {
            *bank_ids.entry(row.def.id).or_insert(0) += row.count;
        }
        let levels = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.base))
            .collect();
        let effective_levels = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.effective))
            .collect();
        let varps = snapshot
            .varps()
            .iter()
            .filter(|varp| {
                matches!(
                    varp.index,
                    BRIMHAVEN_ARENA_VARP
                        | AUTOCAST_MAGIC_VARP
                        | COMBAT_MODE_VARP
                        | SA_ENERGY_VARP
                        | SA_ARMED_VARP
                        | VARP_TARGET_COUNT
                        | VARP_TARGET_SCORE
                        | VARP_TARGET_HIT
                )
            })
            .map(|varp| (varp.index, varp.value))
            .collect();
        let chat = snapshot
            .chat_lines()
            .iter()
            .map(|line| (line.sequence, line.text.clone()))
            .collect();
        let player = snapshot
            .local_player()
            .and_then(|local| local.player.actor.name.clone());
        let loc_facts = snapshot
            .locs()
            .iter()
            .filter(|loc| loc.distance <= 8 && keep_bounded_loc(loc.id, loc.name.as_deref()))
            .take(16)
            .map(|loc| BoundedLoc {
                id: loc.id,
                x: loc.tile.x,
                z: loc.tile.z,
                level: loc.tile.level,
                name: loc.name.clone(),
                open: loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|op| op.trim().to_ascii_lowercase().starts_with("open")),
            })
            .collect();
        let magic_tree_ready = snapshot.locs().iter().any(|loc| {
            loc.id == MAGIC_TREE_ID
                && (loc.tile.x, loc.tile.z, loc.tile.level) == GNOME_SOUTH_BANK_MAGIC_TREE
                && loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.trim().eq_ignore_ascii_case("Chop down"))
        });
        let mut equipment_ids = BTreeMap::new();
        for item in snapshot.equipment() {
            if item.count > 0 && item.def.id >= 0 {
                *equipment_ids.entry(item.def.id).or_insert(0) += item.count;
            }
        }
        let self_slot = snapshot.self_slot();
        let local = snapshot.local_player();
        let local_in_combat = local.is_some_and(|player| player.player.actor.in_combat);
        let local_target_npc = local
            .and_then(|player| player.player.actor.target)
            .and_then(|target| (target.kind == ActorKind::Npc).then_some(target.index));
        let local_health = local.map(|player| player.player.actor.health).unwrap_or(0);
        let local_animation = local
            .map(|player| player.player.actor.animation)
            .unwrap_or(0);
        let npc_facts = snapshot
            .npcs()
            .iter()
            .filter(|npc| {
                npc.distance <= 8
                    && (npc.in_combat
                        || combat_npc_name(npc.name.as_deref())
                        || npc.target.is_some_and(|target| {
                            target.kind == ActorKind::Player
                                && self_slot >= 0
                                && target.index == self_slot as usize
                        })
                        || local_target_npc == Some(npc.index))
            })
            .take(8)
            .map(|npc| bounded_combat_npc(npc, self_slot))
            .collect::<Vec<_>>();
        // Dormant RockCrab `Rocks` are outside that combat window by
        // construction (no target, not in combat, parked far from the player),
        // yet their real index/tile identity is what the activation witness
        // matches: a `Rock Crab` on one of those witnesses is the source's own
        // Rocks->crab transition, not a separately spawned crab. Project the
        // nearest dormant Rocks inside the supported field, bounded, so the
        // cycle can see that identity without the world copy.
        let mut dormant_rocks = snapshot
            .npcs()
            .iter()
            .filter(|npc| dormant_rock_in_field(npc))
            .collect::<Vec<_>>();
        dormant_rocks.sort_by_key(|npc| (npc.distance, npc.index));
        dormant_rocks.truncate(DORMANT_ROCK_FACTS_MAX);
        // The field boolean is this projection, not a second sweep.
        let dormant_rocks_seen = !dormant_rocks.is_empty();
        let npc_facts = npc_facts
            .into_iter()
            .chain(
                dormant_rocks
                    .into_iter()
                    .map(|npc| bounded_combat_npc(npc, self_slot)),
            )
            .collect();
        let ground_loot = snapshot
            .ground_items()
            .iter()
            .filter(|item| item.distance <= 8 && combat_ground_id(item.def.id))
            .take(8)
            .map(|item| BoundedGround {
                id: item.def.id,
                count: item.count,
                tile: (item.tile.x, item.tile.z, item.tile.level),
                distance: item.distance,
            })
            .collect();
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            player,
            tile: snapshot.tile(),
            combat_level: snapshot
                .local_player()
                .map(|local| local.player.combat_level)
                .unwrap_or(0),
            tick: snapshot.tick(),
            items,
            item_ids,
            bank,
            bank_ids,
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_generation: snapshot.bank_session_generation(),
            script_lifecycle: None,
            levels,
            effective_levels,
            xp,
            varps,
            chat,
            loc_facts,
            npc_facts,
            magic_tree_ready,
            dormant_rocks_seen,
            ground_loot,
            local_in_combat,
            taking_damage: snapshot.taking_damage(),
            local_target_npc,
            local_health,
            local_animation,
            guardian: BoundedGuardian::default(),
            equipment_ids,
            main_modal: snapshot.modals().main,
            widget_ids: snapshot
                .widgets()
                .iter()
                .map(|widget| widget.component_id)
                .collect(),
            shop_open: snapshot.shop().open,
            shop_stock: snapshot
                .shop()
                .stock
                .iter()
                .filter(|item| item.count > 0 && item.def.id >= 0)
                .take(16)
                .map(|item| BoundedShopItem {
                    id: item.def.id,
                    count: item.count,
                })
                .collect(),
            main_make_ids: snapshot
                .main_make()
                .iter()
                .filter(|item| item.def.id >= 0)
                .take(16)
                .map(|item| item.def.id)
                .collect(),
            route_inspect_seq: 0,
            route_inspect_generation: 0,
            route_inspect_request_id: 0,
            route_inspect_ok: false,
            route_inspect_reason: String::new(),
            route_inspect_hops: Vec::new(),
            route_inspect_live_generation: 0,
            route_inspect_has_terminal: false,
            los: LineOfSightObservation::default(),
            actor: ActorObservation::default(),
            fight: FightFieldObservation::default(),
            hunt: HuntObservation::default(),
        }
    }

    pub fn attach_route_inspect(&mut self, published: RouteInspectPublished) {
        self.route_inspect_live_generation = published.live_generation;
        self.route_inspect_has_terminal = published.has_terminal;
        self.route_inspect_seq = published.seq;
        self.route_inspect_generation = published.generation;
        self.route_inspect_request_id = published.request_id;
        self.route_inspect_ok = published.ok;
        self.route_inspect_reason = published.reason;
        self.route_inspect_hops = published
            .hop_loc_names
            .into_iter()
            .map(|loc_name| RouteInspectHopFact { loc_name })
            .collect();
    }

    pub fn item(&self, name: &str) -> i32 {
        self.items.get(name).copied().unwrap_or(0)
    }

    pub fn skill_xp(&self, name: &str) -> i32 {
        self.xp.get(name).copied().unwrap_or(0)
    }

    pub fn item_id(&self, id: i32) -> i32 {
        self.item_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn bank_item(&self, name: &str) -> i32 {
        self.bank.get(name).copied().unwrap_or(0)
    }

    pub fn bank_item_id(&self, id: i32) -> i32 {
        self.bank_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn level(&self, name: &str) -> i32 {
        self.levels.get(name).copied().unwrap_or(0)
    }

    pub fn effective_level(&self, name: &str) -> i32 {
        self.effective_levels.get(name).copied().unwrap_or(0)
    }

    pub fn varp(&self, index: i32) -> i32 {
        self.varps.get(&index).copied().unwrap_or(0)
    }

    /// Copy varps 83..=97 only. Absent snapshot rows stay absent (not 0).
    pub fn attach_prayer_varps(&mut self, snapshot: &GameSnapshot) {
        let first = api::prayer::PRAYER_VARP0;
        let last = first + api::prayer::PRAYER_COUNT as i32 - 1;
        for varp in snapshot.varps() {
            if (first..=last).contains(&varp.index) {
                self.varps.insert(varp.index, varp.value);
            }
        }
    }

    /// Compact current-plane identity, independently selected one-step pairs,
    /// and the script paint receipt. Never copies the collision grid.
    pub fn attach_line_of_sight(
        &mut self,
        snapshot: &GameSnapshot,
        paint: Option<&script::shim::ScriptPaint>,
    ) {
        let scene = snapshot.scene();
        let identity = LineOfSightIdentity {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
        };
        let here = self
            .tile
            .map(|(x, z, level)| LineOfSightTile { x, z, level });
        let here_flag = here.and_then(|tile| collision_flag_at(scene, tile));
        let (host_open, host_blocked, fixture_failure) = match here {
            Some(tile) if scene.available => match select_line_of_sight_pairs(tile, |x, z| {
                collision_flag_at(
                    scene,
                    LineOfSightTile {
                        x,
                        z,
                        level: tile.level,
                    },
                )
            }) {
                Ok((open, blocked)) => (Some(open), Some(blocked), None),
                Err(msg) => (None, None, Some(msg)),
            },
            _ => (None, None, None),
        };
        self.los = LineOfSightObservation {
            available: scene.available,
            identity,
            here,
            here_flag,
            host_open,
            host_blocked,
            fixture_failure,
            receipt: paint.and_then(parse_los_receipt_from_paint),
        };
    }

    /// Compact host identity, one size>=1 NPC, existing LOS helper, and the
    /// script paint receipt. Never copies the NPC table or collision grid.
    pub fn attach_actor_observation(
        &mut self,
        snapshot: &GameSnapshot,
        paint: Option<&script::shim::ScriptPaint>,
    ) {
        let scene = snapshot.scene();
        let identity = LineOfSightIdentity {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
        };
        let here = self
            .tile
            .map(|(x, z, level)| LineOfSightTile { x, z, level });
        let receipt = paint.and_then(parse_actor_receipt_from_paint);
        let npc = choose_actor_observation_npc(snapshot.npcs(), receipt.as_ref()).map(|row| {
            ActorObservationNpc {
                index: row.index as i32,
                name: row.name.clone(),
                size: row.size,
                tile_x: row.tile.x,
                tile_z: row.tile.z,
                nx: row.network.x,
                nz: row.network.z,
                level: row.tile.level,
            }
        });
        let (self_target_kind, self_target_index) = packed_self_target(snapshot);
        let host_los = match (here, npc.as_ref()) {
            (Some(from), Some(npc)) if scene.available => {
                let query = CollisionQuery {
                    available: scene.available,
                    base_x: scene.base_x,
                    base_z: scene.base_z,
                    level: scene.level,
                    width: scene.width,
                    height: scene.height,
                    flags: Arc::from(scene.collision_flags.as_slice()),
                };
                line_of_sight_v2(
                    Some(&query),
                    WorldTile {
                        x: from.x,
                        z: from.z,
                        level: from.level,
                    },
                    WorldTile {
                        x: npc.nx,
                        z: npc.nz,
                        level: npc.level,
                    },
                    Some(npc.size),
                )
                .ok()
            }
            _ => None,
        };
        self.actor = ActorObservation {
            available: scene.available,
            identity,
            here,
            npc,
            host_los,
            self_target_kind,
            self_target_index,
            receipt,
        };
    }

    /// Compact host identity, one size>=1 NPC, both existing LOS helper
    /// results, and the script paint receipt. Never copies the NPC table.
    pub fn attach_fight_field(
        &mut self,
        snapshot: &GameSnapshot,
        paint: Option<&script::shim::ScriptPaint>,
    ) {
        let scene = snapshot.scene();
        let identity = LineOfSightIdentity {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
        };
        let here = self
            .tile
            .map(|(x, z, level)| LineOfSightTile { x, z, level });
        let receipt = paint.and_then(parse_fight_field_receipt_from_paint);
        let npc =
            choose_fight_field_npc(snapshot.npcs(), receipt.as_ref()).map(|row| FightFieldNpc {
                index: row.index as i32,
                size: row.size,
                tile_x: row.tile.x,
                tile_z: row.tile.z,
                nx: row.network.x,
                nz: row.network.z,
                level: row.tile.level,
            });
        let query = if scene.available {
            Some(CollisionQuery {
                available: scene.available,
                base_x: scene.base_x,
                base_z: scene.base_z,
                level: scene.level,
                width: scene.width,
                height: scene.height,
                flags: Arc::from(scene.collision_flags.as_slice()),
            })
        } else {
            None
        };
        let (host_los_network, host_los_tile) = match (here, npc.as_ref(), query.as_ref()) {
            (Some(from), Some(npc), Some(query)) => {
                let from_tile = WorldTile {
                    x: from.x,
                    z: from.z,
                    level: from.level,
                };
                let network = line_of_sight_v2(
                    Some(query),
                    from_tile,
                    WorldTile {
                        x: npc.nx,
                        z: npc.nz,
                        level: npc.level,
                    },
                    Some(npc.size),
                )
                .ok();
                let tile = line_of_sight_v2(
                    Some(query),
                    from_tile,
                    WorldTile {
                        x: npc.tile_x,
                        z: npc.tile_z,
                        level: npc.level,
                    },
                    Some(npc.size),
                )
                .ok();
                (network, tile)
            }
            _ => (None, None),
        };
        self.fight = FightFieldObservation {
            available: scene.available,
            identity,
            here,
            npc,
            host_los_network,
            host_los_tile,
            receipt,
        };
    }

    pub fn equipment_id(&self, id: i32) -> i32 {
        self.equipment_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn has_widget(&self, id: i32) -> bool {
        self.widget_ids.contains(&id)
    }

    pub fn shop_item_id(&self, id: i32) -> i32 {
        self.shop_stock
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.count)
            .unwrap_or(0)
    }

    pub fn has_main_make(&self, id: i32) -> bool {
        self.main_make_ids.contains(&id)
    }
}

pub fn combat_npc_name(name: Option<&str>) -> bool {
    matches!(
        name.map(str::trim),
        Some("Chaos druid" | "Moss giant" | "Giant" | "Guard")
    )
}

/// One compact combat/fact record from the published npc family. The
/// `targeting_local` flag is recomputed here rather than copied, so a decoded
/// face target is the only source of that claim.
fn bounded_combat_npc(npc: &NpcView, self_slot: i32) -> BoundedNpc {
    BoundedNpc {
        index: npc.index,
        name: npc.name.clone(),
        health: npc.health,
        total_health: npc.total_health,
        animation: npc.animation,
        in_combat: npc.in_combat,
        targeting_local: npc.target.is_some_and(|target| {
            target.kind == ActorKind::Player && self_slot >= 0 && target.index == self_slot as usize
        }),
        tile: (npc.tile.x, npc.tile.z, npc.tile.level),
        distance: npc.distance,
    }
}

/// A dormant RockCrab `Rocks` parked inside the supported field. This is the
/// field predicate the scoped stand is audited against; it is identity
/// evidence for the activation witness, never combat evidence on its own.
fn dormant_rock_in_field(npc: &NpcView) -> bool {
    npc.name.as_deref() == Some(ROCK_CRAB_DORMANT_NAME)
        && npc.tile.level == ROCK_CRAB_SPOT.2
        && (npc.tile.x - ROCK_CRAB_SPOT.0)
            .abs()
            .max((npc.tile.z - ROCK_CRAB_SPOT.1).abs())
            <= ROCK_CRAB_FIELD_RADIUS
}

pub fn unidentified_herb_id(id: i32) -> bool {
    matches!(
        id,
        199 | 201 | 203 | 205 | 207 | 209 | 211 | 213 | 215 | 217 | 219 | LANTADYME_HERB_ID
    )
}

pub fn noted_herb_id(id: i32) -> bool {
    matches!(
        id,
        200 | 202 | 204 | 206 | 208 | 210 | 212 | 214 | 216 | 218 | 220 | NOTED_LANTADYME_HERB_ID
    )
}

pub fn combat_ground_id(id: i32) -> bool {
    unidentified_herb_id(id)
        || noted_herb_id(id)
        || matches!(
            id,
            NATURE_RUNE_ID
                | LAW_RUNE_ID
                | BIG_BONES_ID
                | NOTED_BIG_BONES_ID
                | LIMPWURT_ROOT_ID
                | NOTED_LIMPWURT_ROOT_ID
                | BONES_ID
                | NOTED_BONES_ID
        )
}

pub fn keep_bounded_loc(id: i32, name: Option<&str>) -> bool {
    matches!(
        id,
        WOODEN_DOOR_CLOSED_ID | WOODEN_DOOR_OPEN_ID | WOODEN_GATE_CLOSED_ID | WOODEN_GATE_OPEN_ID
    ) || name.is_some_and(|name| {
        let n = name.trim().to_ascii_lowercase();
        n == "door" || n.ends_with(" door") || n.contains("gate") || n == "fire"
    })
}

pub fn loc_name_matches(name: Option<&str>, gate: bool) -> bool {
    let n = name.unwrap_or("").trim().to_ascii_lowercase();
    if n.is_empty() {
        return false;
    }
    if gate {
        n.contains("gate")
    } else {
        n == "door" || n.ends_with(" door") || n.contains("gate")
    }
}

pub fn loc_at(
    observation: &Observation,
    id: i32,
    tile: (i32, i32, i32),
    radius: i32,
) -> Option<&BoundedLoc> {
    observation.loc_facts.iter().find(|loc| {
        loc.id == id
            && loc.level == tile.2
            && (loc.x - tile.0).abs().max((loc.z - tile.1).abs()) <= radius
    })
}

pub fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

pub fn empty_pack(observation: &Observation) -> bool {
    observation.item_ids.values().copied().sum::<i32>() == 0
}

/// Frozen `config.ts` @ rs2b0t `beecd912` Ardougne SE bank / Barnaby pier / field.
pub const BRIMHAVEN_INSPECT_BANK: (i32, i32, i32) = (2655, 3283, 0);
pub const BRIMHAVEN_INSPECT_PIER: (i32, i32, i32) = (2683, 3272, 0);
pub const BRIMHAVEN_INSPECT_FIELD: (i32, i32, i32) = (2698, 3206, 0);
pub const BRIMHAVEN_INSPECT_BANK_RADIUS: i32 = 6;
pub const BRIMHAVEN_INSPECT_PIER_RADIUS: i32 = 8;
pub const BRIMHAVEN_INSPECT_FOOD_WITHDRAW: i32 = 20;
pub const BRIMHAVEN_INSPECT_BOAT_FARE_ROUNDTRIP: i32 = 60;

fn chebyshev(a: (i32, i32, i32), b: (i32, i32, i32)) -> i32 {
    if a.2 != b.2 {
        i32::MAX
    } else {
        (a.0 - b.0).abs().max((a.1 - b.1).abs())
    }
}

fn hop_loc_has(hops: &[RouteInspectHopFact], needle: &str) -> bool {
    hops.iter()
        .any(|hop| hop.loc_name.to_ascii_lowercase().contains(needle))
}

fn hop_loc_wrong_boat(hops: &[RouteInspectHopFact]) -> bool {
    hops.iter().any(|hop| {
        let name = hop.loc_name.to_ascii_lowercase();
        name.contains("thresnor") || name.contains("musa") || name.contains("port sarim")
    })
}

/// Authoritative inspect freshness for Core (not a new product policy).
///
/// `InspectNav.generation` starts at 0 and is the live generation even when
/// no terminal is published. `reset_inspect` (session nav reset, not catalog
/// Start) does `wrapping_add(1)` and `clear_published`; a later publish uses
/// `next_seq` from an empty latest (seq 1) stamped with the new generation.
/// `Observation` defaults (`generation`/`seq`/`live_generation` = 0,
/// `has_terminal` = false) are missing-projection placeholders, not "live
/// generation is 0". The narrow inspect projection therefore copies
/// `live_generation` even without a terminal so Start baseline can store the
/// real ring. Generation 0 is a legitimate first-session value.
///
/// Current publish: `has_terminal` and `terminal.generation == live_generation`.
/// Same live generation as the Start baseline: `seq` must advance past that
/// baseline seq. After `reset_inspect` the live generation changes; the new
/// ring's seq 1 is fresh even if the previous ring ended at seq 8. Old
/// generation / stale seq / unpublished seq 0 are rejected. v1 also requires
/// a registered `request_id != 0` at the cycle; v2 token uses `!= 0` then a
/// later distinct `request_id == 0` snapshot.
fn fresh_barnaby_inspect(now: &Observation, prior_live_generation: u64, prior_seq: u64) -> bool {
    if !now.route_inspect_has_terminal
        || now.route_inspect_generation != now.route_inspect_live_generation
        || !now.route_inspect_ok
        || !hop_loc_has(&now.route_inspect_hops, "barnaby")
        || hop_loc_wrong_boat(&now.route_inspect_hops)
    {
        return false;
    }
    if now.route_inspect_live_generation == prior_live_generation {
        now.route_inspect_seq > prior_seq
    } else {
        now.route_inspect_seq > 0
    }
}

pub fn brimhaven_moss_inspect_v1_baseline_ready(baseline: &Observation) -> bool {
    near(
        baseline.tile,
        BRIMHAVEN_INSPECT_BANK,
        BRIMHAVEN_INSPECT_BANK_RADIUS,
    ) && empty_pack(baseline)
        && baseline.item_id(LOBSTER_ID) == 0
        && baseline.item_id(COINS_ID) == 0
        && baseline.level("agility") >= 30
        && baseline.ingame
        && baseline.scene_state == 2
}

pub fn route_inspect_brimhaven_v2_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, BRIMHAVEN_INSPECT_PIER, 4) && baseline.ingame && baseline.scene_state == 2
}

/// Ordered v1 witness: restock after empty-pack Start, then a fresh accepted
/// Barnaby inspect (`request_id != 0`), then a later observation with an
/// actual tile change toward/at the pier. First `accepted_tile` is kept.
/// Seed, fallback-without-accept, same-frame pier, wrong-boat, and stale
/// generation cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BrimhavenMossInspectCycle {
    pub restocked: bool,
    pub accepted_seq: Option<u64>,
    pub accepted_tile: Option<(i32, i32, i32)>,
    pub walk_progress: bool,
}

impl BrimhavenMossInspectCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if !self.restocked
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(LOBSTER_ID) >= BRIMHAVEN_INSPECT_FOOD_WITHDRAW
            && now.item_id(COINS_ID) >= BRIMHAVEN_INSPECT_BOAT_FARE_ROUNDTRIP
            && baseline.item_id(LOBSTER_ID) == 0
            && baseline.item_id(COINS_ID) == 0
        {
            self.restocked = true;
        }
        if self.restocked
            && self.accepted_seq.is_none()
            && now.route_inspect_request_id != 0
            && fresh_barnaby_inspect(
                now,
                baseline.route_inspect_live_generation,
                baseline.route_inspect_seq,
            )
        {
            self.accepted_seq = Some(now.route_inspect_seq);
            // First accepted tile only; repeated later terminals must not refresh it.
            self.accepted_tile = now.tile;
        }
        if let (Some(_), Some(from)) = (self.accepted_seq, self.accepted_tile) {
            let later_tile = now.tile.filter(|tile| *tile != from);
            if let Some(tile) = later_tile {
                self.walk_progress |= near(
                    Some(tile),
                    BRIMHAVEN_INSPECT_PIER,
                    BRIMHAVEN_INSPECT_PIER_RADIUS,
                ) || chebyshev(tile, BRIMHAVEN_INSPECT_PIER)
                    < chebyshev(from, BRIMHAVEN_INSPECT_PIER);
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.restocked && self.accepted_seq.is_some() && self.walk_progress
    }
}

/// Ordered v2 witness: consumed token result, then a later request_id 0
/// result, then ordinary arrival on a distinct bank tile.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RouteInspectBrimhavenV2Cycle {
    pub token_seq: Option<u64>,
    pub snap0_seq: Option<u64>,
    pub walked: bool,
}

impl RouteInspectBrimhavenV2Cycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.token_seq.is_none()
            && now.route_inspect_request_id != 0
            && fresh_barnaby_inspect(
                now,
                baseline.route_inspect_live_generation,
                baseline.route_inspect_seq,
            )
        {
            self.token_seq = Some(now.route_inspect_seq);
        }
        if let Some(token_seq) = self.token_seq {
            if self.snap0_seq.is_none()
                && now.route_inspect_request_id == 0
                && fresh_barnaby_inspect(
                    now,
                    // id0 is a later publish on this same live ring; seq must
                    // advance past the token. v2 request_id 0 is distinct from
                    // a v1 registered identity.
                    now.route_inspect_live_generation,
                    token_seq,
                )
            {
                self.snap0_seq = Some(now.route_inspect_seq);
            }
        }
        if self.snap0_seq.is_some() {
            self.walked |= near(
                now.tile,
                BRIMHAVEN_INSPECT_BANK,
                BRIMHAVEN_INSPECT_BANK_RADIUS,
            ) && !near(now.tile, BRIMHAVEN_INSPECT_PIER, 4);
        }
    }

    pub fn qualified(&self) -> bool {
        self.token_seq.is_some() && self.snap0_seq.is_some() && self.walked
    }
}

pub const PRAYER_V2_STOP: &str = "prayer v2 qualification complete";
pub const PRAYER_V1_STOP: &str = "prayer v1 qualification complete";
pub const PRAYER_BASE_MIN: i32 = 43;
pub const PROTECT_FROM_MELEE_VARP: i32 = 97;

pub fn prayer_varp_indexes() -> impl Iterator<Item = i32> {
    let start = api::prayer::PRAYER_VARP0;
    (0..api::prayer::PRAYER_COUNT as i32).map(move |i| start + i)
}

/// All 15 overlay keys present and zero. Missing keys are not off.
pub fn prayer_varps_all_present_off(observation: &Observation) -> bool {
    prayer_varp_indexes().all(|index| observation.varps.get(&index) == Some(&0))
}

pub fn prayer_delivery_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.level("prayer") >= PRAYER_BASE_MIN
        && baseline.effective_level("prayer") > 0
        && prayer_varps_all_present_off(baseline)
}

/// Ordered witness: seeded all-off, then a latched Protect from Melee ON
/// (varp 97==1, even if later cleared the same/later tick), then all 15
/// present-and-off, then the exact File-card stop reason.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PrayerDeliveryCycle {
    pub saw_on: bool,
    pub later_all_off: bool,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl PrayerDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if now.varps.get(&PROTECT_FROM_MELEE_VARP) == Some(&1) {
            self.saw_on = true;
        }
        if self.saw_on && prayer_varps_all_present_off(now) {
            self.later_all_off = true;
        }
    }

    pub fn observe_script_lifecycle(
        &mut self,
        receipt: script::ScriptLifecycleReceipt,
        expected: &str,
    ) {
        if self.later_all_off
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && receipt.reason == expected
        {
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        self.saw_on && self.later_all_off && self.stopped.is_some()
    }
}

pub const LOS_V2_STOP: &str = "line of sight qualification complete";
pub const LOS_RECEIPT_PREFIX: &str = "los-receipt:";
pub const ACTOR_OBSERVATION_V2_STOP: &str = "actor observation qualification complete";
pub const ACTOR_RECEIPT_PREFIX: &str = "actor-receipt:";
pub const FIGHT_FIELD_V2_STOP: &str = "fight field qualification complete";
pub const FIGHT_FIELD_RECEIPT_PREFIX: &str = "fight-field-receipt:";
pub const LOS_WALK_SCENERY: i32 = 0x100;
pub const LOS_V_N: i32 = 0x400;
pub const LOS_V_E: i32 = 0x1000;
pub const LOS_V_S: i32 = 0x4000;
pub const LOS_V_W: i32 = 0x10000;
pub const LOS_VIS_SCENERY: i32 = 0x20000;
pub const LOS_PAIR_RADIUS: i32 = 8;
const LOS_DIRS: [(i32, i32, i32); 4] = [
    (1, 0, LOS_V_W),
    (-1, 0, LOS_V_E),
    (0, 1, LOS_V_S),
    (0, -1, LOS_V_N),
];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightIdentity {
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightPair {
    pub from: LineOfSightTile,
    pub to: LineOfSightTile,
    pub src: i32,
    pub dst: i32,
    pub mask: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightPairResult {
    pub from: LineOfSightTile,
    pub to: LineOfSightTile,
    pub src: i32,
    pub dst: i32,
    pub mask: i32,
    pub v2: bool,
    pub v1: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightHere {
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub flag: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightScriptReceipt {
    pub identity: LineOfSightIdentity,
    pub here: LineOfSightHere,
    pub open: LineOfSightPairResult,
    pub blocked: LineOfSightPairResult,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LineOfSightObservation {
    pub available: bool,
    pub identity: LineOfSightIdentity,
    pub here: Option<LineOfSightTile>,
    pub here_flag: Option<i32>,
    pub host_open: Option<LineOfSightPair>,
    pub host_blocked: Option<LineOfSightPair>,
    pub fixture_failure: Option<String>,
    pub receipt: Option<LineOfSightScriptReceipt>,
}

impl LineOfSightPairResult {
    pub fn pair(self) -> LineOfSightPair {
        LineOfSightPair {
            from: self.from,
            to: self.to,
            src: self.src,
            dst: self.dst,
            mask: self.mask,
        }
    }
}

fn collision_flag_at(scene: &SceneView, tile: LineOfSightTile) -> Option<i32> {
    if !scene.available || tile.level != scene.level {
        return None;
    }
    let lx = tile.x - scene.base_x;
    let lz = tile.z - scene.base_z;
    if lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height {
        return None;
    }
    scene
        .collision_flags
        .get((lx * scene.height + lz) as usize)
        .copied()
}

pub fn los_entering_mask(dx: i32, dz: i32) -> Option<i32> {
    match (dx, dz) {
        (1, 0) => Some(LOS_V_W),
        (-1, 0) => Some(LOS_V_E),
        (0, 1) => Some(LOS_V_S),
        (0, -1) => Some(LOS_V_N),
        _ => None,
    }
}

pub fn line_of_sight_pair_is_open(pair: &LineOfSightPair) -> bool {
    let dx = pair.to.x - pair.from.x;
    let dz = pair.to.z - pair.from.z;
    pair.from.level == pair.to.level
        && los_entering_mask(dx, dz) == Some(pair.mask)
        && pair.src & LOS_WALK_SCENERY == 0
        && pair.dst & pair.mask == 0
}

pub fn line_of_sight_pair_is_blocked(pair: &LineOfSightPair) -> bool {
    let dx = pair.to.x - pair.from.x;
    let dz = pair.to.z - pair.from.z;
    // Source WALK_SCENERY must be clear: the LOS helper returns false on
    // source scenery before ray tracing, so a scenery-sourced negative is
    // not attributable to the entering V-wall.
    pair.from.level == pair.to.level
        && los_entering_mask(dx, dz) == Some(pair.mask)
        && pair.src & LOS_WALK_SCENERY == 0
        && pair.dst & pair.mask != 0
}

pub fn line_of_sight_dest_vis_alone(pair: &LineOfSightPair) -> bool {
    pair.dst & LOS_VIS_SCENERY != 0 && pair.dst & pair.mask == 0
}

/// Deterministic Chebyshev 0..=8 cardinal scan. Expected answers come from
/// one-step raw V-mask facts, not from calling the LOS helper.
pub fn select_line_of_sight_pairs(
    here: LineOfSightTile,
    flag_at: impl Fn(i32, i32) -> Option<i32>,
) -> Result<(LineOfSightPair, LineOfSightPair), String> {
    let mut open = None;
    let mut blocked = None;
    for r in 0..=LOS_PAIR_RADIUS {
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let from = LineOfSightTile {
                    x: here.x + dx,
                    z: here.z + dz,
                    level: here.level,
                };
                if (from.x - here.x).abs().max((from.z - here.z).abs()) > LOS_PAIR_RADIUS {
                    continue;
                }
                let Some(src) = flag_at(from.x, from.z) else {
                    continue;
                };
                for (sx, sz, mask) in LOS_DIRS {
                    let to = LineOfSightTile {
                        x: from.x + sx,
                        z: from.z + sz,
                        level: here.level,
                    };
                    if (to.x - here.x).abs().max((to.z - here.z).abs()) > LOS_PAIR_RADIUS {
                        continue;
                    }
                    let Some(dst) = flag_at(to.x, to.z) else {
                        continue;
                    };
                    let pair = LineOfSightPair {
                        from,
                        to,
                        src,
                        dst,
                        mask,
                    };
                    if blocked.is_none() && line_of_sight_pair_is_blocked(&pair) {
                        blocked = Some(pair);
                    } else if open.is_none() && line_of_sight_pair_is_open(&pair) {
                        open = Some(pair);
                    }
                    if let (Some(open), Some(blocked)) = (open, blocked) {
                        return Ok((open, blocked));
                    }
                }
            }
        }
    }
    Err("los fixture failure: no cardinal open+blocked V-wall pair within 8".into())
}

pub fn parse_los_receipt_from_paint(
    paint: &script::shim::ScriptPaint,
) -> Option<LineOfSightScriptReceipt> {
    paint.lines.iter().find_map(|line| {
        line.strip_prefix(LOS_RECEIPT_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

pub fn line_of_sight_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.los.available
        && baseline.los.here_flag.is_some()
}

fn los_receipt_joined(now: &LineOfSightObservation) -> bool {
    let (Some(host_open), Some(host_blocked), Some(receipt), Some(here), Some(here_flag)) = (
        now.host_open,
        now.host_blocked,
        now.receipt.as_ref(),
        now.here,
        now.here_flag,
    ) else {
        return false;
    };
    now.available
        && now.fixture_failure.is_none()
        && receipt.identity == now.identity
        && receipt.here.x == here.x
        && receipt.here.z == here.z
        && receipt.here.level == here.level
        && receipt.here.flag == here_flag
        && line_of_sight_pair_is_open(&host_open)
        && line_of_sight_pair_is_blocked(&host_blocked)
        && !line_of_sight_dest_vis_alone(&host_blocked)
        && !line_of_sight_dest_vis_alone(&receipt.blocked.pair())
        && receipt.open.pair() == host_open
        && receipt.blocked.pair() == host_blocked
        && receipt.open.v2
        && !receipt.blocked.v2
        && receipt.open.v1
        && !receipt.blocked.v1
}

/// Post-Start witness: host-selected one-step pairs, user-script receipt
/// joined to the same SceneView identity/flags, then the named helper stop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LineOfSightDeliveryCycle {
    pub host_open: Option<LineOfSightPair>,
    pub host_blocked: Option<LineOfSightPair>,
    pub identity: Option<LineOfSightIdentity>,
    pub receipt: Option<LineOfSightScriptReceipt>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
    pub fixture_failure: Option<String>,
}

impl LineOfSightDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if let Some(msg) = now.los.fixture_failure.clone() {
            self.fixture_failure = Some(msg);
        }
        // Once a same-observation host/receipt join is latched, freeze host
        // pairs and identity with that historical witness. Later host drift
        // must not overwrite evidence that still backs a valid receipt.
        if self.receipt.is_some() {
            return;
        }
        if now.los.available {
            if let (Some(open), Some(blocked)) = (now.los.host_open, now.los.host_blocked) {
                if line_of_sight_pair_is_open(&open) && line_of_sight_pair_is_blocked(&blocked) {
                    self.host_open = Some(open);
                    self.host_blocked = Some(blocked);
                    self.identity = Some(now.los.identity);
                }
            }
        }
        if los_receipt_joined(&now.los) {
            // Latch host + receipt from the same joined observation.
            self.host_open = now.los.host_open;
            self.host_blocked = now.los.host_blocked;
            self.identity = Some(now.los.identity);
            self.receipt = now.los.receipt;
        }
    }

    pub fn observe_script_lifecycle(
        &mut self,
        receipt: script::ScriptLifecycleReceipt,
        expected: &str,
    ) {
        if self.receipt.is_some()
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && receipt.reason == expected
        {
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        if self.fixture_failure.is_some() || self.stopped.is_none() {
            return false;
        }
        let (Some(host_open), Some(host_blocked), Some(identity), Some(receipt)) = (
            self.host_open,
            self.host_blocked,
            self.identity,
            self.receipt.as_ref(),
        ) else {
            return false;
        };
        // Re-assert stored host still matches the latched receipt (fail-closed).
        identity == receipt.identity
            && host_open == receipt.open.pair()
            && host_blocked == receipt.blocked.pair()
            && line_of_sight_pair_is_open(&host_open)
            && line_of_sight_pair_is_blocked(&host_blocked)
            && !line_of_sight_dest_vis_alone(&host_blocked)
            && receipt.open.v2
            && !receipt.blocked.v2
            && receipt.open.v1
            && !receipt.blocked.v1
    }
}

/// File and Core share this rule: min `(distance, index)` among `size >= 1`.
/// After the script posts a receipt, look that index up so headed join verifies
/// the observed row instead of independently picking another NPC.
fn choose_actor_observation_npc<'a>(
    npcs: &'a [NpcView],
    receipt: Option<&ActorObservationScriptReceipt>,
) -> Option<&'a NpcView> {
    if let Some(index) = receipt.map(|row| row.npc.index) {
        return npcs
            .iter()
            .find(|row| row.size >= 1 && row.index as i32 == index);
    }
    npcs.iter()
        .filter(|row| row.size >= 1)
        .min_by_key(|row| (row.distance, row.index))
}

fn packed_self_target(snapshot: &GameSnapshot) -> (i32, i32) {
    match snapshot
        .local_player()
        .and_then(|player| player.player.actor.target)
    {
        None => (0, -1),
        Some(target) => match target.kind {
            ActorKind::Npc => (1, target.index as i32),
            ActorKind::Player => (2, target.index as i32),
        },
    }
}

fn parse_actor_receipt_from_paint(
    paint: &script::shim::ScriptPaint,
) -> Option<ActorObservationScriptReceipt> {
    paint.lines.iter().find_map(|line| {
        line.strip_prefix(ACTOR_RECEIPT_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationPoint {
    pub x: i32,
    pub z: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationNpcFact {
    pub index: i32,
    pub name: Option<String>,
    pub size: i32,
    pub tile: ActorObservationPoint,
    pub network: ActorObservationPoint,
    pub level: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationPacked {
    pub size: i32,
    pub nx: i32,
    pub nz: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationSelfTarget {
    pub kind: i32,
    pub index: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationLos {
    pub v2: bool,
    pub v1: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationScriptReceipt {
    pub identity: LineOfSightIdentity,
    pub here: LineOfSightTile,
    pub npc: ActorObservationNpcFact,
    pub packed: ActorObservationPacked,
    pub rendered: ActorObservationPoint,
    pub self_target: ActorObservationSelfTarget,
    pub los: ActorObservationLos,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ActorObservationNpc {
    pub index: i32,
    pub name: Option<String>,
    pub size: i32,
    pub tile_x: i32,
    pub tile_z: i32,
    pub nx: i32,
    pub nz: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActorObservation {
    pub available: bool,
    pub identity: LineOfSightIdentity,
    pub here: Option<LineOfSightTile>,
    pub npc: Option<ActorObservationNpc>,
    pub host_los: Option<bool>,
    pub self_target_kind: i32,
    pub self_target_index: i32,
    pub receipt: Option<ActorObservationScriptReceipt>,
}

impl Default for ActorObservation {
    fn default() -> Self {
        Self {
            available: false,
            identity: LineOfSightIdentity::default(),
            here: None,
            npc: None,
            host_los: None,
            self_target_kind: 0,
            self_target_index: -1,
            receipt: None,
        }
    }
}

pub fn actor_observation_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.actor.available
        && baseline.actor.here.is_some()
}

fn actor_receipt_joined(now: &ActorObservation) -> bool {
    let (Some(npc), Some(here), Some(host_los), Some(receipt)) = (
        now.npc.as_ref(),
        now.here,
        now.host_los,
        now.receipt.as_ref(),
    ) else {
        return false;
    };
    now.available
        && npc.size >= 1
        && receipt.identity == now.identity
        && receipt.here == here
        && receipt.npc.index == npc.index
        && receipt.npc.name == npc.name
        && receipt.npc.size == npc.size
        && receipt.npc.level == npc.level
        && receipt.npc.tile.x == npc.tile_x
        && receipt.npc.tile.z == npc.tile_z
        && receipt.npc.network.x == npc.nx
        && receipt.npc.network.z == npc.nz
        && receipt.packed.size == npc.size
        && receipt.packed.nx == npc.nx
        && receipt.packed.nz == npc.nz
        && receipt.rendered.x == npc.tile_x
        && receipt.rendered.z == npc.tile_z
        && receipt.self_target.kind == now.self_target_kind
        && receipt.self_target.index == now.self_target_index
        && receipt.los.v2 == host_los
        && receipt.los.v1 == host_los
}

/// Post-Start witness: one packed size>=1 NPC, joined script receipt, named stop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ActorObservationDeliveryCycle {
    pub identity: Option<LineOfSightIdentity>,
    pub npc: Option<ActorObservationNpc>,
    pub host_los: Option<bool>,
    pub receipt: Option<ActorObservationScriptReceipt>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl ActorObservationDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if self.receipt.is_some() {
            return;
        }
        if now.actor.available {
            if let Some(npc) = now.actor.npc.clone() {
                if npc.size >= 1 {
                    self.identity = Some(now.actor.identity);
                    self.npc = Some(npc);
                    self.host_los = now.actor.host_los;
                }
            }
        }
        if actor_receipt_joined(&now.actor) {
            self.identity = Some(now.actor.identity);
            self.npc = now.actor.npc.clone();
            self.host_los = now.actor.host_los;
            self.receipt = now.actor.receipt.clone();
        }
    }

    pub fn observe_script_lifecycle(
        &mut self,
        receipt: script::ScriptLifecycleReceipt,
        expected: &str,
    ) {
        if self.receipt.is_some()
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && receipt.reason == expected
        {
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        if self.stopped.is_none() {
            return false;
        }
        let (Some(identity), Some(npc), Some(host_los), Some(receipt)) = (
            self.identity,
            self.npc.as_ref(),
            self.host_los,
            self.receipt.as_ref(),
        ) else {
            return false;
        };
        identity == receipt.identity
            && npc.size >= 1
            && npc.index == receipt.npc.index
            && npc.name == receipt.npc.name
            && npc.size == receipt.npc.size
            && npc.level == receipt.npc.level
            && npc.tile_x == receipt.npc.tile.x
            && npc.tile_z == receipt.npc.tile.z
            && npc.nx == receipt.npc.network.x
            && npc.nz == receipt.npc.network.z
            && npc.size == receipt.packed.size
            && npc.nx == receipt.packed.nx
            && npc.nz == receipt.packed.nz
            && npc.tile_x == receipt.rendered.x
            && npc.tile_z == receipt.rendered.z
            && receipt.los.v2 == host_los
            && receipt.los.v1 == host_los
    }
}

/// File and Core share this rule: min `(distance, index)` among `size >= 1`.
/// After the script posts a receipt, look that index up so headed join verifies
/// the observed row instead of independently picking another NPC.
fn choose_fight_field_npc<'a>(
    npcs: &'a [NpcView],
    receipt: Option<&FightFieldScriptReceipt>,
) -> Option<&'a NpcView> {
    if let Some(index) = receipt.map(|row| row.index) {
        return npcs
            .iter()
            .find(|row| row.size >= 1 && row.index as i32 == index);
    }
    npcs.iter()
        .filter(|row| row.size >= 1)
        .min_by_key(|row| (row.distance, row.index))
}

fn parse_fight_field_receipt_from_paint(
    paint: &script::shim::ScriptPaint,
) -> Option<FightFieldScriptReceipt> {
    paint.lines.iter().find_map(|line| {
        line.strip_prefix(FIGHT_FIELD_RECEIPT_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

fn fight_field_effect_is_attack(receipt: &FightFieldScriptReceipt) -> bool {
    fn is_attack(value: &str) -> bool {
        value.eq_ignore_ascii_case("npc") || value.eq_ignore_ascii_case("attack")
    }
    receipt.kind.as_deref().is_some_and(is_attack)
        || receipt.effect.as_deref().is_some_and(is_attack)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FightFieldScriptReceipt {
    pub index: i32,
    pub size: i32,
    pub tile: ActorObservationPoint,
    pub network_origin: ActorObservationPoint,
    pub los_network: bool,
    pub los_tile: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FightFieldNpc {
    pub index: i32,
    pub size: i32,
    pub tile_x: i32,
    pub tile_z: i32,
    pub nx: i32,
    pub nz: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FightFieldObservation {
    pub available: bool,
    pub identity: LineOfSightIdentity,
    pub here: Option<LineOfSightTile>,
    pub npc: Option<FightFieldNpc>,
    pub host_los_network: Option<bool>,
    pub host_los_tile: Option<bool>,
    pub receipt: Option<FightFieldScriptReceipt>,
}

impl Default for FightFieldObservation {
    fn default() -> Self {
        Self {
            available: false,
            identity: LineOfSightIdentity::default(),
            here: None,
            npc: None,
            host_los_network: None,
            host_los_tile: None,
            receipt: None,
        }
    }
}

pub fn fight_field_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.fight.available
        && baseline.fight.here.is_some()
}

fn fight_field_receipt_joined(now: &FightFieldObservation) -> bool {
    let (Some(npc), Some(here), Some(host_los_network), Some(host_los_tile), Some(receipt)) = (
        now.npc.as_ref(),
        now.here,
        now.host_los_network,
        now.host_los_tile,
        now.receipt.as_ref(),
    ) else {
        return false;
    };
    now.available
        && npc.size >= 1
        && here.level == npc.level
        && receipt.index == npc.index
        && receipt.size == npc.size
        && receipt.tile.x == npc.tile_x
        && receipt.tile.z == npc.tile_z
        && receipt.network_origin.x == npc.nx
        && receipt.network_origin.z == npc.nz
        && receipt.los_network == host_los_network
        && receipt.los_tile == host_los_tile
        && !fight_field_effect_is_attack(receipt)
}

/// Post-Start witness: one packed size>=1 NPC, joined script receipt, named stop, no Attack.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FightFieldDeliveryCycle {
    pub identity: Option<LineOfSightIdentity>,
    pub here: Option<LineOfSightTile>,
    pub npc: Option<FightFieldNpc>,
    pub host_los_network: Option<bool>,
    pub host_los_tile: Option<bool>,
    pub receipt: Option<FightFieldScriptReceipt>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl FightFieldDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if self.receipt.is_some() {
            return;
        }
        if now.fight.available {
            if let Some(npc) = now.fight.npc.clone() {
                if npc.size >= 1 {
                    self.identity = Some(now.fight.identity);
                    self.here = now.fight.here;
                    self.npc = Some(npc);
                    self.host_los_network = now.fight.host_los_network;
                    self.host_los_tile = now.fight.host_los_tile;
                }
            }
        }
        if fight_field_receipt_joined(&now.fight) {
            self.identity = Some(now.fight.identity);
            self.here = now.fight.here;
            self.npc = now.fight.npc.clone();
            self.host_los_network = now.fight.host_los_network;
            self.host_los_tile = now.fight.host_los_tile;
            self.receipt = now.fight.receipt.clone();
        }
    }

    pub fn observe_script_lifecycle(
        &mut self,
        receipt: script::ScriptLifecycleReceipt,
        expected: &str,
    ) {
        if self.receipt.is_some()
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && receipt.reason == expected
        {
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        if self.stopped.is_none() {
            return false;
        }
        let (
            Some(identity),
            Some(here),
            Some(npc),
            Some(host_los_network),
            Some(host_los_tile),
            Some(receipt),
        ) = (
            self.identity,
            self.here,
            self.npc.as_ref(),
            self.host_los_network,
            self.host_los_tile,
            self.receipt.as_ref(),
        )
        else {
            return false;
        };
        identity.level == here.level
            && npc.size >= 1
            && npc.index == receipt.index
            && npc.size == receipt.size
            && npc.tile_x == receipt.tile.x
            && npc.tile_z == receipt.tile.z
            && npc.nx == receipt.network_origin.x
            && npc.nz == receipt.network_origin.z
            && receipt.los_network == host_los_network
            && receipt.los_tile == host_los_tile
            && !fight_field_effect_is_attack(receipt)
    }
}

fn empty_worn(observation: &Observation) -> bool {
    observation.equipment_ids.values().copied().sum::<i32>() == 0
}

/// Bank-only MossGiant dart Start. Worn-gear combat_baseline_ready(Ranged) is
/// the wrong predicate here: the script has to withdraw and equip 806.
pub fn moss_giant_dart_baseline_ready(baseline: &Observation) -> bool {
    let bank_ready = baseline.bank_open
        && baseline.bank_loaded
        && baseline.bank_item_id(BRONZE_DART_ID) == MOSS_GIANT_DART_SUPPLY
        && baseline.bank_item_id(LOBSTER_ID) == MOSS_GIANT_DART_BANK_FOOD;
    near(baseline.tile, MOSS_GIANT_BANK, 2)
        && empty_pack(baseline)
        && empty_worn(baseline)
        && baseline.level("ranged") >= MOSS_GIANT_DART_RANGED
        && baseline.level("defence") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && held_id(baseline, BRONZE_DART_ID) == 0
        && held_id(baseline, RUNE_ARROW_ID) == 0
        && baseline.item_id(LOBSTER_ID) == 0
        && baseline.bank_item_id(RUNE_ARROW_ID) == 0
        && bank_ready
}

pub fn fire_in_varrock_east_plot(observation: &Observation) -> bool {
    observation.loc_facts.iter().any(|loc| {
        loc.level == 0
            && loc.x >= FIRE_PLOT_VARROCK_EAST_X0
            && loc.x <= FIRE_PLOT_VARROCK_EAST_X1
            && loc.z >= FIRE_PLOT_VARROCK_EAST_Z0
            && loc.z <= FIRE_PLOT_VARROCK_EAST_Z1
            && loc
                .name
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("fire"))
    })
}

pub fn stall_food(observation: &Observation) -> i32 {
    observation.item_id(CAKE_ID)
        + observation.item_id(BREAD_ID)
        + observation.item_id(CHOCOLATE_SLICE_ID)
}

pub fn bank_stall_food(observation: &Observation) -> i32 {
    observation.bank_item_id(CAKE_ID)
        + observation.bank_item_id(BREAD_ID)
        + observation.bank_item_id(CHOCOLATE_SLICE_ID)
}

pub fn noted_stall_food(observation: &Observation) -> i32 {
    observation.item_id(NOTED_CAKE_ID)
        + observation.item_id(NOTED_BREAD_ID)
        + observation.item_id(NOTED_CHOCOLATE_SLICE_ID)
        + observation.bank_item_id(NOTED_CAKE_ID)
        + observation.bank_item_id(NOTED_BREAD_ID)
        + observation.bank_item_id(NOTED_CHOCOLATE_SLICE_ID)
}

pub fn ardy_thiever_baseline_ready(baseline: &Observation, thieving: i32) -> bool {
    near(baseline.tile, ARDY_THIEVER_STAND, 8)
        && baseline.level("thieving") >= thieving
        && baseline.item_id(COINS_ID) == 0
        && baseline.item_id(CAKE_ID) == 0
}

/// Baker's stall Flee cell prep plus combat stats and a weapon so FightBack can kill.
pub fn ardy_cakes_fight_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, ARDY_CAKES_STAND, 6)
        && baseline.level("thieving") >= 5
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && baseline.item_id(KNIFE_ID) == ARDY_CAKES_BALLAST_KNIVES
        && held_id(baseline, ADAMANT_SCIMITAR_ID) >= 1
        && stall_food(baseline) == 0
        && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
        && noted_stall_food(baseline) == 0
}

/// Guard pickpocket Fight cell: same empty-pack thieving prep plus combat kit.
pub fn ardy_thiever_fight_baseline_ready(baseline: &Observation) -> bool {
    ardy_thiever_baseline_ready(baseline, 40)
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && held_id(baseline, ADAMANT_SCIMITAR_ID) >= 1
}

pub fn held_id(observation: &Observation, id: i32) -> i32 {
    observation.item_id(id) + observation.equipment_id(id)
}

pub fn gnome_wrong_bows(observation: &Observation) -> bool {
    observation.item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(MAGIC_LONGBOW_ID) > 0
}

pub fn gnome_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.bank_item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
}

pub fn gnome_resource_tools(observation: &Observation) -> bool {
    held_id(observation, RUNE_AXE_ID) == 1 && observation.item_id(KNIFE_ID) == GNOME_BALLAST_KNIVES
}

pub fn gnome_chop_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, GNOME_SOUTH_BANK_MAGIC_STAND, 1)
        && baseline.magic_tree_ready
        && baseline.level("woodcutting") >= 75
        && gnome_resource_tools(baseline)
        && baseline.item_id(MAGIC_LOGS_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) == 0
        && !gnome_wrong_bows(baseline)
        && !gnome_noted(baseline)
}

pub fn gnome_fletch_baseline_ready(
    baseline: &Observation,
    fletching: i32,
    max: Option<i32>,
) -> bool {
    gnome_chop_baseline_ready(baseline)
        && baseline.level("fletching") >= fletching
        && max
            .map(|max| baseline.level("fletching") <= max)
            .unwrap_or(true)
}

pub fn coal_trucks_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, COAL_MINE, 8)
        && baseline.level("mining") >= 60
        && baseline.effective_level("mining") >= 60
        && baseline.item_id(RUNE_PICKAXE_ID) == 1
        && baseline.item_id(KNIFE_ID) == COAL_BALLAST_KNIVES
        && baseline.item_ids.values().copied().sum::<i32>() == 27
        && baseline.item_id(COAL_ID) == 0
        && baseline.item_id(NOTED_COAL_ID) == 0
        && baseline.bank_item_id(COAL_ID) == 0
}

pub fn cook_wrong_or_burnt(observation: &Observation, wrong: i32) -> bool {
    observation.item_id(wrong) > 0
        || observation.bank_item_id(wrong) > 0
        || observation.item_id(BURNT_FISH_1_ID) > 0
        || observation.bank_item_id(BURNT_FISH_1_ID) > 0
        || observation.item_id(BURNT_FISH_2_ID) > 0
        || observation.bank_item_id(BURNT_FISH_2_ID) > 0
        || observation.item_id(BURNT_LOBSTER_ID) > 0
        || observation.bank_item_id(BURNT_LOBSTER_ID) > 0
}

pub fn cook_noted(observation: &Observation, noted_raw: i32, noted_product: i32) -> bool {
    observation.item_id(noted_raw) > 0
        || observation.bank_item_id(noted_raw) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

pub fn cook_bot_baseline_ready(
    baseline: &Observation,
    raw: i32,
    product: i32,
    wrong: i32,
    noted_raw: i32,
    noted_product: i32,
) -> bool {
    near(baseline.tile, CATHERBY_BANK, 6)
        && baseline.level("cooking") >= COOKING_FIXTURE_LEVEL
        && baseline.item_id(raw) == 0
        && baseline.item_id(product) == 0
        && !cook_wrong_or_burnt(baseline, wrong)
        && !cook_noted(baseline, noted_raw, noted_product)
}

pub fn smelter_noted(
    observation: &Observation,
    noted_primary: i32,
    noted_secondary: i32,
    noted_product: i32,
) -> bool {
    observation.item_id(noted_primary) > 0
        || observation.bank_item_id(noted_primary) > 0
        || observation.item_id(noted_secondary) > 0
        || observation.bank_item_id(noted_secondary) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

pub fn smelter_bot_baseline_ready(
    baseline: &Observation,
    primary: i32,
    secondary: i32,
    product: i32,
    wrong: i32,
    smithing: i32,
) -> bool {
    near(baseline.tile, AL_KHARID_BANK, 6)
        && baseline.level("smithing") >= smithing
        && baseline.item_id(primary) == 0
        && baseline.item_id(secondary) == 0
        && baseline.item_id(product) == 0
        && baseline.item_id(wrong) == 0
        && baseline.item_id(IRON_BAR_ID) == 0
        && !smelter_noted(baseline, primary + 1, secondary + 1, product + 1)
}

pub fn flax_spinner_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_FLAX_ID) > 0
        || observation.bank_item_id(NOTED_FLAX_ID) > 0
        || observation.item_id(NOTED_BOW_STRING_ID) > 0
        || observation.bank_item_id(NOTED_BOW_STRING_ID) > 0
}

pub fn flax_spinner_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_SPINNER_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

pub fn flax_aio_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_FIELD, 6)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

pub fn flax_aio_pick_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_FIELD, 6)
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && !flax_spinner_noted(baseline)
}

pub fn flax_aio_spin_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_AIO_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

pub fn herblore_eggs_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_RED_SPIDERS_EGGS_ID) > 0
        || observation.bank_item_id(NOTED_RED_SPIDERS_EGGS_ID) > 0
}

pub fn herblore_newt_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_EYE_OF_NEWT_ID) > 0
        || observation.bank_item_id(NOTED_EYE_OF_NEWT_ID) > 0
}

pub fn herblore_eggs_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, EGG_FIELD, 8)
        && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
        && baseline.item_id(EYE_OF_NEWT_ID) == 0
        && !herblore_eggs_noted(baseline)
}

pub fn herblore_newt_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, BETTY_SHOP, 6)
        && baseline.item_id(EYE_OF_NEWT_ID) == 0
        && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
        && !herblore_newt_noted(baseline)
}

pub fn superheater_baseline_ready(
    baseline: &Observation,
    bar: i32,
    primary: i32,
    secondary: i32,
    staff: i32,
    smithing: i32,
) -> bool {
    near(baseline.tile, (3185, 3440, 0), 6)
        && baseline.level("magic") >= 43
        && baseline.level("smithing") >= smithing
        && baseline.item_id(bar) == 0
        && baseline.item_id(primary) == 0
        && baseline.item_id(secondary) == 0
        && baseline.item_id(NATURE_RUNE_ID) == 0
        && baseline.item_id(staff) == 0
        && baseline.item_id(IRON_BAR_ID) == 0
        && baseline.equipment_id(staff) == 0
}

pub fn in_temple(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 > TEMPLE_Z)
}

pub fn overworld(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 <= TEMPLE_Z)
}

pub fn runecraft_baseline_ready(
    baseline: &Observation,
    bank: (i32, i32, i32),
    rune: i32,
    wrong_rune: i32,
    talisman: i32,
    rc_level: i32,
) -> bool {
    near(baseline.tile, bank, 6)
        && baseline.level("runecraft") >= rc_level
        && baseline.item_id(RUNE_ESSENCE_ID) == 0
        && baseline.item_id(NOTED_ESSENCE_ID) == 0
        && baseline.item_id(rune) == 0
        && baseline.item_id(wrong_rune) == 0
        && baseline.item_id(talisman) == 0
}

pub fn validate_case_baseline(case: CoreCase, baseline: &Observation) -> Result<(), String> {
    validate_case_baseline_with_preparation(case, baseline, None)
}

pub fn validate_case_baseline_with_preparation(
    case: CoreCase,
    baseline: &Observation,
    start_preparation: Option<&HerbCleanerStartPreparationReceipt>,
) -> Result<(), String> {
    let ready = match case {
        CoreCase::BoneBurier => {
            near(baseline.tile, (3220, 3212, 0), 8) && baseline.item("Bones") >= 5
        }
        CoreCase::ChickenKiller => near(baseline.tile, (3235, 3295, 0), 8),
        CoreCase::ChickenKillerBank => {
            near(baseline.tile, FALADOR_CHICKENS, 8)
                && baseline.level("attack") >= 30
                && baseline.level("strength") >= 30
                && baseline.item_id(FEATHER_ID) == 0
        }
        CoreCase::Thiever => {
            near(baseline.tile, (2661, 3306, 0), 10)
                && baseline.item("Lobster") >= 10
                && baseline.level("thieving") >= 50
                && baseline.level("hitpoints") >= 50
        }
        CoreCase::Alcher
        | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => {
            near(baseline.tile, (3185, 3440, 0), 6) && baseline.level("magic") >= 55
        }
        CoreCase::AlcherLow => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= LOW_ALCH_LEVEL
                && baseline.item_id(RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(CERT_RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.item_id(FIRE_BATTLESTAFF_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(FIRE_BATTLESTAFF_ID) == 0
        }
        CoreCase::AlcherFireBattlestaff => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= HIGH_ALCH_LEVEL
                && baseline.level("attack") >= FIRE_BATTLESTAFF_WIELD_ATTACK
                && baseline.item_id(RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(CERT_RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.item_id(FIRE_BATTLESTAFF_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(FIRE_BATTLESTAFF_ID) == 0
        }
        CoreCase::AlcherSwarmDrain => {
            near(baseline.tile, VARROCK_WEST_BANK, 6)
                && baseline.level("magic") >= HIGH_ALCH_LEVEL
                && baseline.item_id(RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(CERT_RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(YEW_LONGBOW_ID) == 0
                && baseline.item_id(CERT_YEW_LONGBOW_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.item_id(FIRE_BATTLESTAFF_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(FIRE_BATTLESTAFF_ID) == 0
        }
        CoreCase::AlcherDefaults => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= 55
                && baseline.item_id(YEW_LONGBOW_ID) == 0
                && baseline.item_id(CERT_YEW_LONGBOW_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item("Rune chainbody") == 0
        }
        CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= 55
                && baseline.item_id(ADAMANT_SCIMITAR_ID) == 0
                && baseline.item_id(CERT_ADAMANT_SCIMITAR_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item("Coins") == 0
                && baseline.item("Rune chainbody") == 0
        }
        CoreCase::BankFletcher => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item("Knife") >= 1
                && baseline.item("Willow logs") >= 27
                && baseline.level("fletching") >= 35
        }
        CoreCase::BankFletcherShafts => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && (held_id(baseline, KNIFE_ID) == 1 || baseline.item("Knife") == 1)
                && baseline.item_id(LOGS_ID) == 27
                && baseline.item_id(ARROW_SHAFT_ID) == 0
                && baseline.level("fletching") >= 1
        }
        CoreCase::BankFletcherHeadless => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(FEATHER_ID) == 30
                && baseline.item_id(ARROW_SHAFT_ID) == 30
                && baseline.item_id(HEADLESS_ARROW_ID) == 0
                && baseline.level("fletching") >= 1
        }
        CoreCase::BankFletcherString => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(60) == 2
                && baseline.item_id(1777) == 2
                && baseline.item_id(849) == 0
                && baseline.level("fletching") >= 35
        }
        CoreCase::BankFletcherCutString => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item("Knife") == 1
                && baseline.item_id(1519) == 2
                && baseline.item_id(60) == 0
                && baseline.item_id(849) == 0
                && baseline.item_id(1777) == 0
                && baseline.level("fletching") >= 35
        }
        CoreCase::DartFletcher => {
            near(baseline.tile, (3220, 3212, 0), 8)
                && baseline.item_id(BRONZE_DART_TIP_ID) == 100
                && baseline.item_id(FEATHER_ID) == 100
                && baseline.item_id(BRONZE_DART_ID) == 0
                && baseline.item_id(IRON_DART_ID) == 0
                && baseline.level("fletching") >= 1
        }
        CoreCase::DartFletcherIron => {
            near(baseline.tile, (3220, 3212, 0), 8)
                && baseline.item_id(IRON_DART_TIP_ID) == 100
                && baseline.item_id(FEATHER_ID) == 100
                && baseline.item_id(IRON_DART_ID) == 0
                && baseline.item_id(BRONZE_DART_ID) == 0
                && baseline.level("fletching") >= 22
        }
        CoreCase::HerbCleaner => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(UNIDENTIFIED_GUAM_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.level("herblore") >= 3
        }
        CoreCase::HerbCleanerNamed => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(UNIDENTIFIED_GUAM_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.item_id(UNIDENTIFIED_MARENTILL_ID) == 0
                && baseline.level("herblore") >= 5
        }
        CoreCase::HerbCleanerEmptyBank => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(UNIDENTIFIED_GUAM_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.item_id(UNIDENTIFIED_MARENTILL_ID) == 0
                && baseline.item_id(MARRENTILL_ID) == 0
                && baseline.level("herblore") >= 20
                && match start_preparation {
                    Some(receipt) => {
                        !baseline.bank_open
                            && !baseline.bank_loaded
                            && baseline.bank.is_empty()
                            && baseline.bank_ids.is_empty()
                            && baseline
                                .player
                                .as_deref()
                                .is_some_and(|player| player.eq_ignore_ascii_case(&receipt.player))
                            && receipt.guam_count == 20
                            && receipt.marrentill_count == 0
                            && baseline.bank_generation > receipt.bank_generation
                            && baseline.tick >= receipt.tick
                    }
                    None => {
                        baseline.bank_open
                            && baseline.bank_loaded
                            && baseline.bank_item_id(UNIDENTIFIED_GUAM_ID) == 20
                            && baseline.bank_item_id(UNIDENTIFIED_MARENTILL_ID) == 0
                    }
                }
        }
        CoreCase::GemCutter => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(CHISEL_ID) == 0
                && baseline.item_id(UNCUT_SAPPHIRE_ID) == 0
                && baseline.item_id(SAPPHIRE_ID) == 0
                && baseline.item_id(CRUSHED_GEMSTONE_ID) == 0
                && baseline.level("crafting") >= 20
        }
        CoreCase::GemCutterNamed => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(CHISEL_ID) == 0
                && baseline.item_id(UNCUT_SAPPHIRE_ID) == 0
                && baseline.item_id(SAPPHIRE_ID) == 0
                && baseline.item_id(UNCUT_OPAL_ID) == 0
                && baseline.item_id(CRUSHED_GEMSTONE_ID) == 0
                && baseline.level("crafting") >= 20
        }
        CoreCase::DoorOpener => {
            near(baseline.tile, LUMBRIDGE_DOOR_STAND, 1)
                && loc_at(baseline, WOODEN_DOOR_CLOSED_ID, LUMBRIDGE_DOOR, 1)
                    .is_some_and(|loc| loc.open)
        }
        CoreCase::DoorOpenerGate => {
            near(baseline.tile, LUMBRIDGE_GATE_STAND, 1)
                && loc_at(baseline, WOODEN_GATE_CLOSED_ID, LUMBRIDGE_GATE, 1)
                    .is_some_and(|loc| loc.open)
        }
        CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => near(baseline.tile, GNOME_START, 2),
        CoreCase::WildyAgility => {
            near(baseline.tile, WILDY_START, 2)
                && baseline.level("agility") >= 52
                && baseline.level("hitpoints") >= 40
                && baseline.effective_level("hitpoints") >= 40
                && baseline.item_id(LOBSTER_ID) >= 5
        }
        CoreCase::BrimhavenAgility => {
            near(baseline.tile, BRIMHAVEN_START, 2)
                && baseline.level("agility") >= 52
                && baseline.effective_level("agility") >= 52
                && baseline.item_id(COINS_ID) >= 200
                && baseline.item_id(LOBSTER_ID) >= 10
                && baseline.item_id(BRIMHAVEN_TICKET_ID) == 0
                && baseline.varp(BRIMHAVEN_ARENA_VARP) & 0b10 == 0
        }
        CoreCase::FlaxPicker => {
            near(baseline.tile, FLAX_FIELD, 6) && baseline.item_id(FLAX_ID) == 0
        }
        CoreCase::Superheater => superheater_baseline_ready(
            baseline,
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            TIN_ORE_ID,
            STAFF_OF_FIRE_ID,
            1,
        ),
        CoreCase::SuperheaterSteel => superheater_baseline_ready(
            baseline,
            STEEL_BAR_ID,
            IRON_ORE_ID,
            COAL_ID,
            STAFF_OF_FIRE_ID,
            30,
        ),
        CoreCase::SuperheaterFireBattlestaff => {
            superheater_baseline_ready(
                baseline,
                BRONZE_BAR_ID,
                COPPER_ORE_ID,
                TIN_ORE_ID,
                FIRE_BATTLESTAFF_ID,
                1,
            ) && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.level("attack") >= 30
        }
        CoreCase::SuperheaterSilverLowNatures => superheater_baseline_ready(
            baseline,
            SILVER_BAR_ID,
            SILVER_ORE_ID,
            SILVER_ORE_ID,
            STAFF_OF_FIRE_ID,
            20,
        ),
        CoreCase::VialFiller => {
            near(baseline.tile, FALADOR_WEST_BANK, 6)
                && baseline.item_id(EMPTY_VIAL_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
        }
        CoreCase::VialFillerEast => {
            near(baseline.tile, FALADOR_EAST_BANK, 6)
                && baseline.item_id(EMPTY_VIAL_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
        }
        CoreCase::PotionMaker => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
                && baseline.item_id(EYE_OF_NEWT_ID) == 0
                && baseline.item_id(GUAM_UNF_ID) == 0
                && baseline.item_id(ATTACK_POTION_3_ID) == 0
                && baseline.level("herblore") >= 3
        }
        CoreCase::PotionMakerNamed => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(RANARR_WEED_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
                && baseline.item_id(SNAPE_GRASS_ID) == 0
                && baseline.item_id(RANARR_UNF_ID) == 0
                && baseline.item_id(PRAYER_POTION_3_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.level("herblore") >= 38
        }
        CoreCase::TannerBot | CoreCase::TannerBotHard => {
            near(baseline.tile, AL_KHARID_BANK, 6)
                && baseline.item_id(COW_HIDE_ID) == 0
                && baseline.item_id(SOFT_LEATHER_ID) == 0
                && baseline.item_id(HARD_LEATHER_ID) == 0
                && baseline.item_id(COINS_ID) == 0
        }
        CoreCase::RuneCrafter => runecraft_baseline_ready(
            baseline,
            FALADOR_EAST_BANK,
            AIR_RUNE_ID,
            EARTH_RUNE_ID,
            AIR_TALISMAN_ID,
            1,
        ),
        CoreCase::RuneCrafterEarth => runecraft_baseline_ready(
            baseline,
            VARROCK_EAST_BANK,
            EARTH_RUNE_ID,
            AIR_RUNE_ID,
            EARTH_TALISMAN_ID,
            9,
        ),
        CoreCase::MuleCrafter => runecraft_baseline_ready(
            baseline,
            FALADOR_EAST_BANK,
            AIR_RUNE_ID,
            EARTH_RUNE_ID,
            AIR_TALISMAN_ID,
            1,
        ),
        CoreCase::ArdyCakes => {
            near(baseline.tile, ARDY_CAKES_STAND, 6)
                && baseline.level("thieving") >= 5
                && baseline.item_id(KNIFE_ID) == ARDY_CAKES_BALLAST_KNIVES
                && baseline.item_ids.values().copied().sum::<i32>() == ARDY_CAKES_BALLAST_KNIVES
                && stall_food(baseline) == 0
                && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
                && noted_stall_food(baseline) == 0
        }
        CoreCase::ArdyCakesFight => ardy_cakes_fight_baseline_ready(baseline),
        CoreCase::ArdyThiever => ardy_thiever_baseline_ready(baseline, 40),
        CoreCase::ArdyThieverFight => ardy_thiever_fight_baseline_ready(baseline),
        CoreCase::ArdyThieverKnight => ardy_thiever_baseline_ready(baseline, 55),
        CoreCase::GnomeChop => gnome_chop_baseline_ready(baseline),
        CoreCase::GnomeFletchShort => gnome_fletch_baseline_ready(baseline, 80, Some(84)),
        CoreCase::GnomeFletchLong => gnome_fletch_baseline_ready(baseline, 85, None),
        CoreCase::CoalTrucks => coal_trucks_baseline_ready(baseline) && baseline.combat_level >= 55,
        CoreCase::CookBot => cook_bot_baseline_ready(
            baseline,
            RAW_SALMON_ID,
            SALMON_ID,
            LOBSTER_ID,
            NOTED_RAW_SALMON_ID,
            NOTED_SALMON_ID,
        ),
        CoreCase::CookBotLobster => cook_bot_baseline_ready(
            baseline,
            RAW_LOBSTER_ID,
            LOBSTER_ID,
            SALMON_ID,
            NOTED_RAW_LOBSTER_ID,
            NOTED_LOBSTER_ID,
        ),
        CoreCase::SmelterBot => smelter_bot_baseline_ready(
            baseline,
            COPPER_ORE_ID,
            TIN_ORE_ID,
            BRONZE_BAR_ID,
            STEEL_BAR_ID,
            1,
        ),
        CoreCase::SmelterBotSteel => smelter_bot_baseline_ready(
            baseline,
            IRON_ORE_ID,
            COAL_ID,
            STEEL_BAR_ID,
            BRONZE_BAR_ID,
            30,
        ),
        CoreCase::FlaxSpinner => flax_spinner_baseline_ready(baseline),
        CoreCase::FlaxAio => flax_aio_baseline_ready(baseline),
        CoreCase::FlaxAioPick => flax_aio_pick_baseline_ready(baseline),
        CoreCase::FlaxAioSpin => flax_aio_spin_baseline_ready(baseline),
        CoreCase::HerbloreSecondaries => herblore_eggs_baseline_ready(baseline),
        CoreCase::HerbloreSecondariesNewt => herblore_newt_baseline_ready(baseline),
        // Bank-only dart Start is empty pack/worn. Do not route this through
        // combat_baseline_ready(Ranged), which requires worn weapon+projectile.
        CoreCase::MossGiantDart => moss_giant_dart_baseline_ready(baseline),
        CoreCase::ChaosDruid
        | CoreCase::ChaosDruidTower
        | CoreCase::ChaosDruidYanille
        | CoreCase::MossGiant
        | CoreCase::MossGiantPrepared
        | CoreCase::HillGiant
        | CoreCase::AutoFighter
        | CoreCase::AutoFighterMage
        | CoreCase::AutoFighterRange
        | CoreCase::RockCrab
        | CoreCase::RockCrabRange
        | CoreCase::GreenDragon
        | CoreCase::GreenDragonPrepared
        | CoreCase::GreenDragonMagePrepared
        | CoreCase::GreenDragonSpecial
        | CoreCase::GreenDragonSpecialPrepared
        | CoreCase::GreenDragonPotions
        | CoreCase::GreenDragonPotionsPrepared
        | CoreCase::FireGiant
        | CoreCase::FireGiantPrepared
        | CoreCase::ArdyFighter
        | CoreCase::AutoFighterBank
        | CoreCase::MossGiantBank
        | CoreCase::HillGiantBank
        | CoreCase::HillGiantBankPrepared
        | CoreCase::ChaosDruidBank
        | CoreCase::ArdyFighterBank
        | CoreCase::RockCrabBank
        | CoreCase::GreenDragonBank
        | CoreCase::GreenDragonBankPrepared
        | CoreCase::GreenDragonBankDefaultPrepared
        | CoreCase::GreenDragonTele
        | CoreCase::GreenDragonTelePrepared
        | CoreCase::FireGiantApproach
        | CoreCase::FireGiantBank
        | CoreCase::FireGiantBankPrepared
        | CoreCase::FireGiantCamelotPrepared => combat_spec(case).is_some_and(|spec| {
            combat_baseline_ready(baseline, spec)
                && match case {
                    CoreCase::ChaosDruidTower => baseline.level("thieving") >= 46,
                    CoreCase::ChaosDruidYanille => baseline.level("agility") >= 40,
                    // Bank cells start from a wielded melee weapon: every one of
                    // these cards deposits (or is told to deposit) the pack, so a
                    // carried weapon would be stashed instead of used. Each
                    // card's loot class is refused by `combat_baseline_ready` itself
                    // (`combat_loot_count == 0`): the cell's deposit class must
                    // not be seeded before Start.
                    CoreCase::AutoFighterBank => baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1,
                    CoreCase::MossGiantBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(LOBSTER_ID) == MOSS_GIANT_BANK_FOOD
                    }
                    CoreCase::HillGiantBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(TROUT_ID) == HILL_GIANT_FOOD
                    }
                    CoreCase::ChaosDruidBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(LOBSTER_ID) == CHAOS_DRUID_BANK_FOOD
                    }
                    CoreCase::ArdyFighterBank => {
                        // Guard-drop emptiness is `combat_baseline_ready`'s
                        // `CombatLoot::GuardDrop` count == 0.
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(CAKE_ID) == 0
                            && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
                    }
                    CoreCase::RockCrabBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.dormant_rocks_seen
                    }
                    // The melee RockCrab core fights with the scoped weapon:
                    // the frozen card's own GearEquip refuses the carried
                    // fixture, so the cell has to arrive already wearing 1331
                    // (the fixture's own pre-Start wear is the native proof).
                    CoreCase::RockCrab => baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1,
                    CoreCase::GreenDragonPrepared
                    | CoreCase::GreenDragonSpecialPrepared
                    | CoreCase::GreenDragonPotionsPrepared
                    | CoreCase::GreenDragonBankPrepared
                    | CoreCase::GreenDragonBankDefaultPrepared
                    | CoreCase::GreenDragonTelePrepared
                    | CoreCase::FireGiantPrepared
                    | CoreCase::FireGiantBankPrepared
                    | CoreCase::FireGiantCamelotPrepared
                    | CoreCase::MossGiantPrepared
                    | CoreCase::HillGiantBankPrepared
                    | CoreCase::GreenDragonMagePrepared => {
                        prepared_combat_baseline_ready(case, baseline)
                    }
                    CoreCase::GreenDragonBank => baseline.equipment_id(RUNE_SCIMITAR_ID) == 1,
                    CoreCase::GreenDragonTele => {
                        baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                            && baseline.level("magic") >= VARROCK_TELE_MAGIC
                            && baseline.item_id(LAW_RUNE_ID) >= 1
                            && baseline.item_id(AIR_RUNE_ID) >= 3
                            && baseline.item_id(FIRE_RUNE_ID) >= 1
                    }
                    CoreCase::FireGiantBank => baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1,
                    _ => true,
                }
        }),
        CoreCase::AioTeleport => {
            near(baseline.tile, LUMBRIDGE_BANK, 8)
                && baseline.level("magic") >= VARROCK_TELE_MAGIC
                && baseline.equipment_id(STAFF_OF_AIR_ID) == 1
                && baseline.item_id(LAW_RUNE_ID) >= AIO_LAW_PACK
                && baseline.item_id(FIRE_RUNE_ID) >= 1
                && !near(baseline.tile, VARROCK_TELE_LAND, 8)
        }
        CoreCase::AioTeleportFalador => {
            near(baseline.tile, LUMBRIDGE_BANK, 8)
                && baseline.level("magic") >= FALADOR_TELE_MAGIC
                && baseline.equipment_id(STAFF_OF_WATER_ID) == 1
                && baseline.item_id(LAW_RUNE_ID) >= AIO_LAW_PACK
                && baseline.item_id(AIR_RUNE_ID) >= 3
                && !near(baseline.tile, FALADOR_TELE_LAND, 8)
        }
        CoreCase::AioTeleportNoStaff => {
            near(baseline.tile, LUMBRIDGE_BANK, 8)
                && baseline.level("magic") >= VARROCK_TELE_MAGIC
                && baseline.equipment_id(STAFF_OF_AIR_ID) == 0
                && baseline.equipment_id(STAFF_OF_WATER_ID) == 0
                && baseline.item_id(LAW_RUNE_ID) >= AIO_LAW_PACK
                && baseline.item_id(AIR_RUNE_ID) >= 3
                && baseline.item_id(FIRE_RUNE_ID) >= 1
                && !near(baseline.tile, VARROCK_TELE_LAND, 8)
        }
        CoreCase::ShopBuyout => {
            near(baseline.tile, AEMAD_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(EMPTY_VIAL_ID) == 0
        }
        CoreCase::ShopBuyoutAubury => {
            near(baseline.tile, AUBURY_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(AIR_RUNE_ID) == 0
        }
        CoreCase::ShopBuyoutLowe => {
            near(baseline.tile, LOWE_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_ARROW_ID) == 0
        }
        CoreCase::ShopBuyoutHickton => {
            near(baseline.tile, HICKTON_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_ARROW_ID) == 0
        }
        CoreCase::ShopBuyoutHarry => {
            near(baseline.tile, HARRY_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(FISHING_BAIT_ID) == 0
        }
        CoreCase::ShopBuyoutBetty => {
            near(baseline.tile, BETTY_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(FIRE_RUNE_ID) == 0
        }
        CoreCase::ShopBuyoutGerrant => {
            near(baseline.tile, GERRANT_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(FEATHER_ID) == 0
        }
        CoreCase::SmithingBot => {
            near(baseline.tile, VARROCK_WEST_BANK, 8)
                && baseline.level("smithing") >= 1
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_DAGGER_ID) == 0
                && baseline.item_id(BRONZE_BAR_ID) == 0
        }
        CoreCase::SmithingBotPlatebody => {
            near(baseline.tile, VARROCK_WEST_BANK, 8)
                && baseline.level("smithing") >= BRONZE_PLATEBODY_SMITHING
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_PLATEBODY_ID) == 0
                && baseline.item_id(BRONZE_DAGGER_ID) == 0
                && baseline.item_id(BRONZE_BAR_ID) == 0
        }
        CoreCase::LeatherCrafter => {
            near(baseline.tile, AL_KHARID_BANK, 8)
                && baseline.level("crafting") >= 1
                && empty_pack(baseline)
                && baseline.item_id(LEATHER_GLOVES_ID) == 0
                && baseline.item_id(SOFT_LEATHER_ID) == 0
        }
        CoreCase::LeatherCrafterHardBody => {
            near(baseline.tile, AL_KHARID_BANK, 8)
                && baseline.level("crafting") >= HARD_LEATHER_CRAFTING
                && empty_pack(baseline)
                && baseline.item_id(HARDLEATHER_BODY_ID) == 0
                && baseline.item_id(HARD_LEATHER_ID) == 0
                && baseline.item_id(LEATHER_GLOVES_ID) == 0
        }
        CoreCase::Firemaker => {
            near(baseline.tile, VARROCK_EAST_BANK, 8)
                && baseline.level("firemaking") >= 1
                && empty_pack(baseline)
                && baseline.item_id(LOGS_ID) == 0
                && !fire_in_varrock_east_plot(baseline)
        }
        CoreCase::FiremakerOak => {
            near(baseline.tile, VARROCK_EAST_BANK, 8)
                && baseline.level("firemaking") >= OAK_FIREMAKING
                && empty_pack(baseline)
                && baseline.item_id(OAK_LOGS_ID) == 0
                && baseline.item_id(LOGS_ID) == 0
                && !fire_in_varrock_east_plot(baseline)
        }
        CoreCase::ClimbingBoots => {
            near(baseline.tile, TENZING_HUT_DOOR, 12)
                && baseline.item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.bank_item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.item_id(COINS_ID) == CLIMBING_BOOTS_WALK_PACK_COINS
                && baseline.item_id(LAW_RUNE_ID) == 0
                && baseline.item_id(AIR_RUNE_ID) == 0
                && baseline.item_id(WATER_RUNE_ID) == 0
        }
        CoreCase::ClimbingBootsTeleport => {
            near(baseline.tile, TENZING_HUT_DOOR, 12)
                && baseline.item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.bank_item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.item_id(COINS_ID) == CLIMBING_BOOTS_TELE_PACK_COINS
                && baseline.item_id(LAW_RUNE_ID) == CLIMBING_BOOTS_RUNE_STOCK_MIN
                && baseline.item_id(AIR_RUNE_ID) == 3
                && baseline.item_id(WATER_RUNE_ID) == 1
                && baseline.level("magic") >= FALADOR_TELE_MAGIC
        }
        CoreCase::RangingGuildRound => ranging_guild_round_baseline_ready(baseline),
        CoreCase::RangingGuildRedeem => ranging_guild_redeem_baseline_ready(baseline),
        CoreCase::RangingGuildBank => ranging_guild_bank_baseline_ready(baseline),
        CoreCase::RangingGuildFull => ranging_guild_full_baseline_ready(baseline),
        CoreCase::BrimhavenMossInspectV1 => brimhaven_moss_inspect_v1_baseline_ready(baseline),
        CoreCase::RouteInspectBrimhavenV2 => route_inspect_brimhaven_v2_baseline_ready(baseline),
        CoreCase::PrayerV2 | CoreCase::PrayerV1 => prayer_delivery_baseline_ready(baseline),
        CoreCase::LineOfSightV2 => line_of_sight_baseline_ready(baseline),
        CoreCase::ActorObservationV2 => actor_observation_baseline_ready(baseline),
        CoreCase::FightFieldV2 => fight_field_baseline_ready(baseline),
        CoreCase::HoldSpotV2
        | CoreCase::RetreatSpotV2
        | CoreCase::WalkSpotV2
        | CoreCase::EnterLairV2
        | CoreCase::LeaveLairV2
        | CoreCase::AcquireKeyV2
        | CoreCase::CellV2
        | CoreCase::BankV2 => case
            .hunt_cell()
            .is_some_and(|cell| hunt_baseline_ready(cell, baseline)),
    };
    if ready {
        return Ok(());
    }
    let requirement = match case {
        CoreCase::BoneBurier => "Lumbridge mainland and five Bones",
        CoreCase::ChickenKiller => "Lumbridge chicken pen",
        CoreCase::ChickenKillerBank => {
            "Falador south chickens (3029,3294,0), Attack/Strength 30 and no Feather 314"
        }
        CoreCase::Thiever => "Ardougne guard stand, ten Lobsters, and prepared stats",
        CoreCase::Alcher | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => "Varrock West bank and Magic 55",
        CoreCase::AlcherLow => {
            "Varrock West bank, Magic 21+, and no seeded chainbody note, coins, Nature rune, or staff"
        }
        CoreCase::AlcherFireBattlestaff => {
            "Varrock West bank, Magic 55+ and Attack 30+, no staff worn or seeded, and no seeded \
             chainbody note, coins, or Nature rune"
        }
        CoreCase::AlcherSwarmDrain => {
            "Varrock West bank, Magic 55+, and no seeded chainbody/yew-longbow note, coins, Nature rune, or staff"
        }
        CoreCase::AlcherDefaults => {
            "Varrock West bank, Magic 55, and no seeded yew-longbow note, coins, Nature rune, or chainbody"
        }
        CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
            "Varrock West bank, Magic 55, and no seeded coins/custom-target outcome"
        }
        CoreCase::BankFletcher => {
            "Varrock West bank, Knife, twenty-seven Willow logs, and Fletching 35"
        }
        CoreCase::BankFletcherShafts => {
            "Varrock West bank, Knife, exact id 1511x27, no id 52, and Fletching 1"
        }
        CoreCase::BankFletcherHeadless => {
            "Varrock West bank, exact ids 314x30 and 52x30, no id 53, and Fletching 1"
        }
        CoreCase::BankFletcherString => {
            "Varrock West bank, exact ids 60x2 and 1777x2, no id 849, and Fletching 35"
        }
        CoreCase::BankFletcherCutString => {
            "Varrock West bank, Knife, exact id 1519x2, no bow/string ids, and Fletching 35"
        }
        CoreCase::DartFletcher => {
            "Lumbridge courtyard, exact ids 819x100 and 314x100, no darts, and Fletching 1"
        }
        CoreCase::DartFletcherIron => {
            "Lumbridge courtyard, exact ids 820x100 and 314x100, no darts, and Fletching 22"
        }
        CoreCase::HerbCleaner => "Varrock West bank, empty pack of 199/249, and Herblore 3",
        CoreCase::HerbCleanerNamed => {
            "Varrock West bank, empty pack of 199/201/249, and Herblore 5"
        }
        CoreCase::HerbCleanerEmptyBank => {
            "Varrock West bank, Herblore 20, empty pack of 199/201/249/251, and this run's loaded bank id 199x20/id 201x0 acknowledged before a closed-bank Start"
        }
        CoreCase::GemCutter => {
            "Varrock West bank, empty pack of 1755/1623/1607/1633, and Crafting 20"
        }
        CoreCase::GemCutterNamed => {
            "Varrock West bank, empty pack of 1755/1623/1625/1607/1633, and Crafting 20"
        }
        CoreCase::DoorOpener => {
            "adjacent stand (3208,3212,0) and shut wooden door 1530 offering Open"
        }
        CoreCase::DoorOpenerGate => {
            "adjacent stand (3213,3260,0) and shut wooden gate 1551 offering Open"
        }
        CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => {
            "Gnome Stronghold start (2474,3436,0)"
        }
        CoreCase::WildyAgility => {
            "south ridge stand (2998,3916,0), Agility 52, base/effective Hitpoints 40, and five Lobsters 379"
        }
        CoreCase::BrimhavenAgility => {
            "surface entrance (2809,3194,0), Agility 52, at least 200 coins, ten Lobsters 379, no ticket 2996, and unpaid varp 309"
        }
        CoreCase::FlaxPicker => "Seers flax field (2741,3444,0) with empty pack of 1779",
        CoreCase::Superheater => {
            "Varrock West bank, Magic 43, Smithing 1, empty pack of 436/438/2349/561/1387"
        }
        CoreCase::SuperheaterSteel => {
            "Varrock West bank, Magic 43, Smithing 30, empty pack of 440/453/2353/561/1387"
        }
        CoreCase::SuperheaterFireBattlestaff => {
            "Varrock West bank, Magic 43, Attack 30, empty pack of 1393 and no 1387"
        }
        CoreCase::SuperheaterSilverLowNatures => {
            "Varrock West bank, Magic 43, Smithing 20, empty pack of 442/2355/561/1387"
        }
        CoreCase::VialFiller => "Falador West bank (2946,3369,0) and empty pack of 229/227",
        CoreCase::VialFillerEast => "Falador East bank (3013,3355,0) and empty pack of 229/227",
        CoreCase::PotionMaker => "Varrock West bank, Herblore 3, empty pack of 249/227/221/91/121",
        CoreCase::PotionMakerNamed => {
            "Varrock West bank, Herblore 38, empty pack of 257/227/231/99/139/249"
        }
        CoreCase::TannerBot | CoreCase::TannerBotHard => {
            "Al-Kharid bank (3269,3167,0) and empty pack of 1739/1741/1743/995"
        }
        CoreCase::RuneCrafter => {
            "Falador East bank (3013,3355,0), Runecraft 1, and empty pack of 1436/1437/556/557"
        }
        CoreCase::RuneCrafterEarth => {
            "Varrock East bank (3253,3420,0), Runecraft 9, and empty pack of 1436/1437/557/556"
        }
        CoreCase::MuleCrafter => {
            "Falador East bank (3013,3355,0), Runecraft 1, blank partner, and empty pack of 1436/1437/556/557"
        }
        CoreCase::ArdyCakes => {
            "Baker's stall stand (2668,3312,0), Thieving 5, exactly 22 Knives 946, and no 1891/2309/1901/1897"
        }
        CoreCase::ArdyCakesFight => {
            "Baker's stall stand (2668,3312,0), Thieving 5, Attack/Strength/Hitpoints 40, scimitar 1331, 22 Knives 946, and no stall food"
        }
        CoreCase::ArdyThiever => {
            "Ardougne Guard stand (2661,3306,0), Thieving 40, and empty pack of 995/1891"
        }
        CoreCase::ArdyThieverFight => {
            "Ardougne Guard stand (2661,3306,0), Thieving 40, Attack/Strength/Hitpoints 40, scimitar 1331, and empty pack of 995/1891"
        }
        CoreCase::ArdyThieverKnight => {
            "Ardougne Knight stand (2661,3306,0), Thieving 55, and empty pack of 995/1891"
        }
        CoreCase::GnomeChop => {
            "south-bank Magic tree stand (2433,3409,0), exact Chop-down tree 1306 at (2432,3410,0), Woodcutting 75, one Rune axe 1359, exactly 26 Knives 946, and no seeded 1513/72/70"
        }
        CoreCase::GnomeFletchShort => {
            "south-bank Magic tree, Woodcutting 75, Fletching 80-84, one Rune axe 1359, exactly 26 Knives 946, and no seeded 1513/72"
        }
        CoreCase::GnomeFletchLong => {
            "south-bank Magic tree, Woodcutting 75, Fletching 85, one Rune axe 1359, exactly 26 Knives 946, and no seeded 1513/70"
        }
        CoreCase::CoalTrucks => {
            "coal mine (2582,3481,0), native combat level >=55, base/effective Mining 60, Rune pickaxe 1275, exactly 26 nonproduct Knives 946 (one free slot), and zero 453/454"
        }
        CoreCase::CookBot => {
            "Catherby bank (2809,3441,0), Cooking 80, empty pack of 331/329"
        }
        CoreCase::CookBotLobster => {
            "Catherby bank (2809,3441,0), Cooking 80, empty pack of 377/379"
        }
        CoreCase::SmelterBot => {
            "Al-Kharid bank (3269,3167,0), Smithing 1, empty pack of 436/438/2349"
        }
        CoreCase::SmelterBotSteel => {
            "Al-Kharid bank (3269,3167,0), Smithing 30, empty pack of 440/453/2353"
        }
        CoreCase::FlaxSpinner => {
            "Seers flax bank (2722,3493,0), Crafting 1, empty pack of 1779/1777"
        }
        CoreCase::FlaxAio => {
            "Seers flax field (2741,3444,0), Crafting 1, empty pack of 1779/1777"
        }
        CoreCase::FlaxAioPick => {
            "Seers flax field (2741,3444,0) with empty pack of 1779/1777"
        }
        CoreCase::FlaxAioSpin => {
            "FlaxAIO Seers bank (2725,3493,0), Crafting 1, empty pack of 1779/1777"
        }
        CoreCase::HerbloreSecondaries => {
            "Edgeville dungeon eggs (3120,9952,0) with empty pack of 223/221"
        }
        CoreCase::HerbloreSecondariesNewt => {
            "Betty shop (3012,3259,0) with empty pack of 221/223"
        }
        CoreCase::ChaosDruid => {
            "Edgeville dungeon (3110,9936,0), Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, empty herb/law/nature"
        }
        CoreCase::ChaosDruidTower => {
            "Chaos Druid Tower (2562,3356,0) r4, Thieving 46, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, empty herb/law/nature"
        }
        CoreCase::ChaosDruidYanille => {
            "Yanille Dungeon warrior room (2580,9501,0) r8, Agility 40, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, empty herb/law/nature"
        }
        CoreCase::MossGiant => {
            "Moss safespot (2553,3406,0), Attack/Strength/Hitpoints 40, lobster 10, scimitar 1331, empty big bones 532"
        }
        CoreCase::MossGiantPrepared => {
            "Moss safespot (2553,3406,0), exact Attack/Strength/Defence/Hitpoints 70, exact lobster 10, worn rune scimitar 1333 and 1113/1079/1163, empty big bones 532"
        }
        CoreCase::MossGiantDart => {
            "Ardougne North bank (2615,3332,0) r2 with the booth still open, empty pack and worn, Ranged 50 / Defence 40 / Hitpoints 40, bank bronze dart 806x80 and lobster 15, no pack/worn/bank rune arrow 892"
        }
        CoreCase::HillGiant => {
            "Giant pit (3110,9832,0), Attack/Strength/Hitpoints 40, trout 8, scimitar 1331, empty 532/225"
        }
        CoreCase::AutoFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, trout 8, scimitar 1331, banking None"
        }
        CoreCase::AutoFighterMage => {
            "Ardougne Guard (2661,3306,0), Magic 13, Hitpoints 40, trout 8, worn Staff of fire 1387, exact Mind rune 558x150 and Air rune 556x300, banking None"
        }
        CoreCase::RockCrab => {
            "safe stand (2712,3707,0), dormant Rocks observed in the supported field, Attack/Strength/Hitpoints 40, lobster 8, scimitar 1331, bank Off"
        }
        CoreCase::AutoFighterRange => {
            "Ardougne Guard (2661,3306,0), Hitpoints 40, Ranged 40, Maple shortbow 853 worn, Bronze arrow 882 x200 worn, Trout 8, bank None"
        }
        CoreCase::GreenDragon => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, lobster 20, rune scimitar 1333, worn shield 1540, empty 536/1753"
        }
        CoreCase::GreenDragonPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Defence/Hitpoints 40, exact lobster 20, carried rune scimitar 1333, worn 1113/1079/1163/1540, empty 536/1753"
        }
        CoreCase::GreenDragonMagePrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Magic/Defence/Hitpoints 70, exact lobster 12, worn Staff of fire 1387 and shield 1540, exact Mind rune 558x150 and Air rune 556x300, no rune armour or scimitar"
        }
        CoreCase::GreenDragonSpecial => {
            "Wilderness field (3096,3814,0) z>=3520, Attack 60, Hitpoints 40, worn dragon dagger 1215 and shield 1540, unarmed spec bar, lobster 12"
        }
        CoreCase::GreenDragonSpecialPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 70, exact lobster 12, worn dragon dagger 1215, shield 1540 and 1113/1079/1163, unarmed spec bar with at least 250 energy"
        }
        CoreCase::GreenDragonPotions => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, worn shield 1540, super attack(3) 145 and super strength(3) 157 with no two-dose flask and no live boost, lobster 12"
        }
        CoreCase::GreenDragonPotionsPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 70, exact lobster 12, carried rune scimitar 1333, worn shield 1540 and 1113/1079/1163, fresh 3-dose attack/strength flasks with no two-dose result or live boost"
        }
        CoreCase::RockCrabRange => {
            "safe stand (2712,3707,0), dormant Rocks observed in the supported field, Hitpoints and Ranged 40, Maple shortbow 853 worn, Bronze arrow 882 x200 worn, lobster 8, bank Off"
        }
        CoreCase::FireGiant => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, amulet 295, rope 954, empty 532"
        }
        CoreCase::FireGiantPrepared => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Defence/Hitpoints 40, exact lobster 12, worn rune scimitar 1333 and 1113/1079/1163, amulet 295, rope 954, empty 532"
        }
        CoreCase::ArdyFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, Thieving 5, scimitar 1331, empty cake/bread/slice, bank Off"
        }
        CoreCase::AutoFighterBank => {
            "Ardougne Guard (2661,3306,0) r8, Attack/Strength/Hitpoints 40, trout 8, worn scimitar 1331, banking Auto, loot Bones, buryBones off, empty 526"
        }
        CoreCase::MossGiantBank => {
            "Moss safespot (2553,3406,0) r10, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 2 (below its restock line), empty 532/225"
        }
        CoreCase::HillGiantBank => {
            "Giant pit (3110,9832,0) r16, Attack/Strength/Hitpoints 40, worn scimitar 1331, trout 8, brass key 983, lootSlots 1, empty 532/225"
        }
        CoreCase::HillGiantBankPrepared => {
            "Giant pit (3110,9832,0) r16, exact Attack/Strength/Defence/Hitpoints 70, exact trout 8, worn adamant scimitar 1331 and 1113/1079/1163, brass key 983, lootSlots 1, empty 532/225"
        }
        CoreCase::ChaosDruidBank => {
            "Edgeville dungeon (3110,9936,0) r14, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 8 (under foodWithdraw 12)"
        }
        CoreCase::ArdyFighterBank => {
            "Ardougne Guard (2661,3306,0) r12, Attack/Strength/Hitpoints 40, Thieving 5, worn scimitar 1331, empty cake pack, empty Guard-drop class 440/886/1446/565/562/561, bankStrategy Loot count"
        }
        CoreCase::RockCrabBank => {
            "safe stand (2712,3707,0), dormant Rocks, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 8, empty 1623/405, bankStrategy Loot count"
        }
        CoreCase::GreenDragonBank => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, worn rune scimitar 1333 and shield 1540, lobster 12, empty 536/1753, escape Flee to bank"
        }
        CoreCase::GreenDragonBankPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 99, exact lobster 26, worn rune scimitar 1333, shield 1540 and 1113/1079/1163, empty 536/1753, stock-selected inventory-pressure Flee to bank"
        }
        CoreCase::GreenDragonBankDefaultPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 99, exact lobster 26, worn rune scimitar 1333, shield 1540 and 1113/1079/1163, empty 536/1753, source-default loot inventory-pressure Flee to bank"
        }
        CoreCase::GreenDragonTele => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, Magic 25, worn rune scimitar 1333 and shield 1540, lobster 12, Law/Air/Fire runes, escape Teleport to Varrock"
        }
        CoreCase::GreenDragonTelePrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 99 at exactly 70 current HP, zero lobster, Magic 25, worn rune scimitar 1333, shield 1540 and 1113/1079/1163, Law/Air/Fire runes, panicHp 98, escape Teleport to Varrock, no death recovery"
        }
        CoreCase::FireGiantApproach => {
            "raft stand (2510,3493,0) z<9000, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, amulet 295, rope 954, Waterfall Quest"
        }
        CoreCase::FireGiantBank => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 12, amulet 295, rope 954, empty 532, escapeTele Barrel"
        }
        CoreCase::FireGiantBankPrepared => {
            "Fire giant room (2575,9893,0) z>=9000, exact Attack/Strength/Defence/Hitpoints 99, exact initial lobster 1, worn rune scimitar 1333 and 1113/1079/1163, amulet 295, rope 954, empty earned Big bones 532, stock-selected one-food escapeTele Barrel and exact restock 25"
        }
        CoreCase::FireGiantCamelotPrepared => {
            "Fire giant room (2575,9893,0) z>=9000, exact Attack/Strength/Defence/Hitpoints 99, Magic 45, exact initial lobster 1, worn rune scimitar 1333 and 1113/1079/1163, worn amulet 295, rope 954, Air 556x15 and Law 563x3, empty earned Big bones 532, escapeTele Camelot land 2757,3478 then Seers 24-lobster restock"
        }
        CoreCase::AioTeleport => {
            "Lumbridge bank (3092,3245,0) r8, Magic 25, worn staff of air 1381, pack law 563x2 and fire 554, not already at Varrock land"
        }
        CoreCase::AioTeleportFalador => {
            "Lumbridge bank (3092,3245,0) r8, Magic 37, worn staff of water 1383, pack law 563x2 and air 556, not already at Falador land"
        }
        CoreCase::AioTeleportNoStaff => {
            "Lumbridge bank (3092,3245,0) r8, Magic 25, pack law/air/fire, no covering air/water staff, not already at Varrock land"
        }
        CoreCase::ShopBuyout => {
            "Aemad stand (2613,3294,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutAubury => {
            "Aubury stand (3253,3401,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutLowe => {
            "Lowe stand (3231,3421,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutHickton => {
            "Hickton stand (2821,3442,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutHarry => {
            "Harry stand (2833,3443,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutBetty => {
            "Betty stand (3012,3258,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutGerrant => {
            "Gerrant stand (3013,3224,0) r6 and empty pack of coins/stock"
        }
        CoreCase::SmithingBot => {
            "Varrock West bank (3185,3440,0), Smithing 1, empty pack of 2347/2349/1205"
        }
        CoreCase::SmithingBotPlatebody => {
            "Varrock West bank (3185,3440,0), Smithing 18, empty pack of 2347/2349/1117/1205"
        }
        CoreCase::LeatherCrafter => {
            "Al-Kharid bank (3269,3167,0), Crafting 1, empty pack of 1733/1734/1741/1059"
        }
        CoreCase::LeatherCrafterHardBody => {
            "Al-Kharid bank (3269,3167,0), Crafting 28, empty pack of 1733/1734/1743/1131/1059"
        }
        CoreCase::Firemaker => {
            "Varrock East bank (3253,3420,0), Firemaking 1, empty pack of 590/1511, no Fire loc in posted plot"
        }
        CoreCase::FiremakerOak => {
            "Varrock East bank (3253,3420,0), Firemaking 15, empty pack of 590/1521/1511, no Fire loc in posted plot"
        }
        CoreCase::ClimbingBoots => {
            "Tenzing hut door (2823,3555,0) r12, zero boots 3105 carried and banked, exact carried 336 coins (28 pair), no runes, Death Plateau complete"
        }
        CoreCase::ClimbingBootsTeleport => {
            "Tenzing hut door (2823,3555,0) r12, zero boots 3105 carried and banked, exact carried 300 coins (25 pair), Law 563x1/Air 556x3/Water 555x1, Magic 37, Death Plateau complete"
        }
        CoreCase::RangingGuildRound => {
            "range stand (2672,3419,0) r2, Ranged 70, Magic shortbow 861, 400 coins, no ticket 1464, no rune arrow 892, unpaid targetcount 156"
        }
        CoreCase::RangingGuildRedeem => {
            "merchant stand (2659,3430,0) r3, Ranged 70, Magic shortbow 861, seeded ticket 1464x2000, no rune arrow 892"
        }
        CoreCase::RangingGuildBank => {
            "Seers bank (2725,3491,0) r6, Ranged 70, seeded KEEP ticket 1464x1999, seeded rune arrow 892x50, no pack coins 995, no pack/worn Magic shortbow 861, unpaid targetcount 156"
        }
        CoreCase::RangingGuildFull => {
            "Seers bank (2725,3491,0) r6, Ranged 70, seeded KEEP ticket 1464x1999, no rune arrow 892 packed or banked, no pack coins 995, no pack/worn Magic shortbow 861, unpaid targetcount 156"
        }
        CoreCase::BrimhavenMossInspectV1 => {
            "Ardougne SE bank (2655,3283,0) r6, empty pack, Agility 30, no carried lobster/coins"
        }
        CoreCase::RouteInspectBrimhavenV2 => {
            "Captain Barnaby pier (2683,3272,0) r4, ingame && scene_state==2"
        }
        CoreCase::PrayerV2 | CoreCase::PrayerV1 => {
            "ingame && scene_state==2, prayer base>=43, positive points, varps 83..97 present and 0"
        }
        CoreCase::LineOfSightV2 => {
            "ingame && scene_state==2 && SceneView.available, here in published collision bounds"
        }
        CoreCase::ActorObservationV2 => {
            "ingame && scene_state==2 && SceneView.available, here on the published plane"
        }
        CoreCase::FightFieldV2 => {
            "ingame && scene_state==2 && SceneView.available, here on the published plane"
        }
        CoreCase::HoldSpotV2 | CoreCase::RetreatSpotV2 | CoreCase::WalkSpotV2 => {
            "ingame && scene_state==2 && SceneView.available, host here on the published plane"
        }
        CoreCase::EnterLairV2 => {
            "ingame && scene_state==2 && SceneView.available, host here not the KBD tile"
        }
        CoreCase::LeaveLairV2 => {
            "ingame && scene_state==2 && SceneView.available, host here not the KBD, Edgeville or Varrock tile"
        }
        CoreCase::AcquireKeyV2 => {
            "ingame && scene_state==2 && SceneView.available, host here outside the jail cell and the projected lair box, Chebyshev > 1 from the corridor (2931,9690,0), no Jail key held"
        }
        CoreCase::CellV2 => {
            "ingame && scene_state==2 && SceneView.available, host here outside the jail cell and the projected lair box, Chebyshev > 1 from the jail door (2931,9690,0), no Dusty key held"
        }
        CoreCase::BankV2 => {
            "ingame && scene_state==2 && SceneView.available, host here Chebyshev > 3 from (2946,3369,0) and outside the projected lair box"
        }
    };
    Err(format!(
        "{} Start baseline lacks required preparation ({requirement}): {baseline:?}",
        case.scenario_name()
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct CoreWitness {
    pub case: CoreCase,
    pub baseline: Observation,
    pub start_preparation: Option<HerbCleanerStartPreparationReceipt>,
    pub latest: Observation,
    pub max_items: BTreeMap<String, i32>,
    pub max_xp: BTreeMap<String, i32>,
    pub saw_bury_chat: bool,
    pub post_start_observations: u64,
    pub bone_bank_cycle: BoneBankCycle,
    pub bank_fletcher_cycle: BankFletcherCycle,
    pub bank_fletcher_option_cycle: BankFletcherOptionCycle,
    pub bank_fletcher_string_cycle: BankFletcherStringCycle,
    pub bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle,
    pub alcher_defaults_cycle: AlcherGeneratedCustomCycle,
    pub alcher_generated_custom_cycle: AlcherGeneratedCustomCycle,
    pub alcher_spell_cycle: AlcherGeneratedCustomCycle,
    pub alcher_swarm_cycle: AlcherSwarmDrainCycle,
    pub dart_fletcher_cycle: DartFletcherCycle,
    pub herb_cleaner_cycle: HerbCleanerCycle,
    pub herb_cleaner_empty_cycle: HerbCleanerEmptyCycle,
    pub gem_cutter_cycle: GemCutterCycle,
    pub door_opener_cycle: DoorOpenerCycle,
    pub gnome_course_cycle: GnomeCourseCycle,
    pub wildy_agility_cycle: WildyAgilityCycle,
    pub brimhaven_agility_cycle: BrimhavenAgilityCycle,
    pub flax_picker_cycle: FlaxPickerCycle,
    pub superheater_cycle: SuperheaterCycle,
    pub chicken_killer_bank_cycle: ChickenKillerBankCycle,
    pub vial_filler_cycle: VialFillerCycle,
    pub potion_maker_cycle: PotionMakerCycle,
    pub tanner_bot_cycle: TannerBotCycle,
    pub rune_crafter_cycle: RuneCrafterCycle,
    pub ardy_cakes_cycle: ArdyCakesCycle,
    pub ardy_cakes_fight_cycle: ArdyCakesFightCycle,
    pub ardy_thiever_cycle: ArdyThieverCycle,
    pub ardy_thiever_fight_cycle: ArdyThieverFightCycle,
    pub gnome_chop_cycle: GnomeChopCycle,
    pub gnome_fletch_cycle: GnomeFletchCycle,
    pub coal_trucks_cycle: CoalTrucksCycle,
    pub station_production_cycle: StationProductionCycle,
    pub flax_aio_cycle: FlaxAioCycle,
    pub flax_aio_pick_cycle: FlaxAioPickCycle,
    pub herblore_eggs_cycle: HerbloreEggsCycle,
    pub herblore_newt_cycle: HerbloreNewtCycle,
    pub combat_core_cycle: CombatCoreCycle,
    pub combat_dart_branch_cycle: CombatDartBranchCycle,
    pub combat_bank_cycle: CombatBankCycle,
    pub combat_approach_cycle: CombatApproachCycle,
    pub aio_teleport_cycle: AioTeleportCycle,
    pub shop_buyout_cycle: ShopBuyoutCycle,
    pub smithing_bot_cycle: SmithingBotCycle,
    pub leather_crafter_cycle: LeatherCrafterCycle,
    pub firemaker_cycle: FiremakerCycle,
    pub climbing_boots_cycle: ClimbingBootsCycle,
    pub ranging_guild_round_cycle: RangingGuildRoundCycle,
    pub ranging_guild_redeem_cycle: RangingGuildRedeemCycle,
    pub ranging_guild_bank_cycle: RangingGuildBankCycle,
    pub ranging_guild_full_cycle: RangingGuildFullCycle,
    pub brimhaven_moss_inspect_cycle: BrimhavenMossInspectCycle,
    pub route_inspect_brimhaven_v2_cycle: RouteInspectBrimhavenV2Cycle,
    pub prayer_delivery_cycle: PrayerDeliveryCycle,
    pub line_of_sight_cycle: LineOfSightDeliveryCycle,
    pub actor_observation_cycle: ActorObservationDeliveryCycle,
    pub fight_field_cycle: FightFieldDeliveryCycle,
    pub hunt_cycle: HuntDeliveryCycle,
    pub ordered_first_exhausted: bool,
}

/// Ordered observations: seed depletion alone must never qualify this card.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BoneBankCycle {
    pub first_batch_buried: bool,
    pub opened: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub buried_after_withdrawal: bool,
}

impl BoneBankCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_batch_buried |=
            now.item("Bones") == 0 && now.skill_xp("prayer") - baseline.skill_xp("prayer") >= 22;
        if self.first_batch_buried
            && self.opened.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item("Bones") == 0
            && now.bank_item("Bones") == 28
            && now.tile != baseline.tile
        {
            self.opened = Some(now.clone());
        }
        if let Some(opened) = &self.opened {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == opened.bank_generation
                && now.item("Bones") == 28
                && now.bank_item("Bones") == 0
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.buried_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item("Bones") < withdrawn.item("Bones")
                && now.skill_xp("prayer") > withdrawn.skill_xp("prayer");
        }
    }
}

/// A first product or an empty-stock stop cannot qualify the bank loop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherCycle {
    pub first_pack_created: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub crafted_after_withdrawal: bool,
}

impl BankFletcherCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_pack_created |= now.item("Willow logs") == 0
            && now.item("Willow shortbow") == 27
            && now.skill_xp("fletching") > baseline.skill_xp("fletching");
        if self.first_pack_created
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item("Knife") == 1
            && now.item("Willow logs") == 0
            && now.item("Willow shortbow") == 0
            && now.bank_item("Willow shortbow") == 27
            && now.bank_item("Willow logs") == 54
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item("Knife") == 1
                && now.item("Willow logs") == 27
                && now.item("Willow shortbow") == 0
                && now.bank_item("Willow logs") == 27
                && now.bank_item("Willow shortbow") == 27
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.crafted_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item("Willow logs") < withdrawn.item("Willow logs")
                && now.item("Willow shortbow") > 0
                && now.skill_xp("fletching") > withdrawn.skill_xp("fletching");
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BankFletcherOptionSpec {
    pub primary: i32,
    pub secondary: Option<i32>,
    pub product: i32,
    pub first_product_count: i32,
}

pub fn bank_fletcher_option_spec(case: CoreCase) -> Option<BankFletcherOptionSpec> {
    match case {
        CoreCase::BankFletcherShafts => Some(BankFletcherOptionSpec {
            primary: LOGS_ID,
            secondary: None,
            product: ARROW_SHAFT_ID,
            first_product_count: 405,
        }),
        CoreCase::BankFletcherHeadless => Some(BankFletcherOptionSpec {
            primary: FEATHER_ID,
            secondary: Some(ARROW_SHAFT_ID),
            product: HEADLESS_ARROW_ID,
            first_product_count: 30,
        }),
        _ => None,
    }
}

/// Exact option output, fresh-bank deposit/restock, closed return, and further
/// production. A seeded product or a first batch alone cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherOptionCycle {
    pub first: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub further: bool,
}

impl BankFletcherOptionCycle {
    pub fn observe(
        &mut self,
        spec: BankFletcherOptionSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        if self.first.is_none()
            && now.item_id(spec.product) >= spec.first_product_count
            && now.item_id(spec.primary) < baseline.item_id(spec.primary)
            && spec
                .secondary
                .is_none_or(|id| now.item_id(id) < baseline.item_id(id))
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
        {
            self.first = Some(now.clone());
        }
        if self.first.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(spec.product) == 0
            && now.bank_item_id(spec.product) >= spec.first_product_count
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(spec.primary) > 0
                && now.bank_item_id(spec.primary) < deposited.bank_item_id(spec.primary)
                && spec.secondary.is_none_or(|id| {
                    now.item_id(id) > 0 && now.bank_item_id(id) < deposited.bank_item_id(id)
                })
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(restocked) = &self.restocked {
            self.further |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(spec.product) >= 1
                && now.item_id(spec.primary) < restocked.item_id(spec.primary)
                && spec
                    .secondary
                    .is_none_or(|id| now.item_id(id) < restocked.item_id(id))
                && now.skill_xp("fletching") > restocked.skill_xp("fletching");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.first.is_some() && self.deposited.is_some() && self.restocked.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherStringCycle {
    pub initial_pairs_strung: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub strung_after_withdrawal: bool,
}

impl BankFletcherStringCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.initial_pairs_strung |= now.item_id(60) == 0
            && now.item_id(1777) == 0
            && now.item_id(849) == 2
            && now.skill_xp("fletching") - baseline.skill_xp("fletching") >= 66;
        if self.initial_pairs_strung
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(849) == 0
            && now.bank_item_id(849) == 2
            && now.bank_item_id(60) == 28
            && now.bank_item_id(1777) == 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(60) == 14
                && now.item_id(1777) == 14
                && now.bank_item_id(60) == 14
                && now.bank_item_id(1777) == 14
                && now.bank_item_id(849) == 2
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.strung_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(60) < withdrawn.item_id(60)
                && now.item_id(1777) < withdrawn.item_id(1777)
                && now.item_id(849) > 0
                && now.skill_xp("fletching") > withdrawn.skill_xp("fletching");
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherCutStringCycle {
    pub cut_pair_created: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub string_pair_created: bool,
}

impl BankFletcherCutStringCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.cut_pair_created |= now.item_id(1519) == 0
            && now.item_id(60) == 2
            && now.item_id(849) == 0
            && now.skill_xp("fletching") - baseline.skill_xp("fletching") >= 66;
        if self.cut_pair_created
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(60) == 0
            && now.bank_item_id(60) == 2
            && now.bank_item_id(1777) == 28
            && now.bank_item_id(849) == 0
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item("Knife") == 0
                && now.bank_item("Knife") >= 1
                && now.item_id(60) == 2
                && now.item_id(1777) == 14
                && now.bank_item_id(60) == 0
                && now.bank_item_id(1777) == 14
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.string_pair_created |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(60) == 0
                && now.item_id(849) == 2
                && now.item_id(1777) == withdrawn.item_id(1777) - 2
                && now.skill_xp("fletching") > withdrawn.skill_xp("fletching");
        }
    }
}

/// Noted-id withdrawal then Nature-rune consumption. Display name
/// "Adamant scimitar" is shared by unnoted 1331 and certificate 1332.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AlcherGeneratedCustomCycle {
    pub withdrawn: Option<Observation>,
    pub consumed: bool,
    /// Selected spell a spelled row demanded (`None` on the High/default rows).
    pub spell: Option<AlcherSpell>,
    /// Client-scale Magic XP one cast of that spell credits.
    pub magic_xp_per_cast: i32,
    /// Coins one cast pays for the selected item at that spell's rate.
    pub coins_per_cast: i32,
    /// Casts the noted-stack, rune and XP deltas agreed on.
    pub casts: i32,
    /// Fire staff worn on the baseline frame (`None`: no staff before Start).
    pub baseline_staff: Option<i32>,
    /// Fire staff worn on the frame that proved the cast.
    pub cast_staff: Option<i32>,
    /// Fire staff a spelled row required; `None` when the row claims none.
    pub required_staff: Option<i32>,
    /// A fire staff other than the required one was worn after Start.
    pub wrong_staff: bool,
}

/// The Alcher's selected spell: Low needs 21 Magic and pays 40% of shop cost,
/// High needs 55 and pays 60%.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlcherSpell {
    Low,
    High,
}

/// One selected spell's per-cast expectations for the selected item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlcherSpellExpectation {
    pub spell: AlcherSpell,
    /// Client-scale Magic XP one cast credits (Low 31 / High 65).
    pub magic_xp: i32,
    /// Coins one cast pays: floor(canonical item cost * the spell's rate).
    pub coins_per_cast: i32,
    /// Fire staff the fixture's gear step must actually wear.
    pub staff: i32,
}

/// Low Level Alchemy on noted rune chainbodies under a Staff of fire (#744).
pub const ALCHER_LOW_EXPECTATION: AlcherSpellExpectation = AlcherSpellExpectation {
    spell: AlcherSpell::Low,
    magic_xp: LOW_ALCH_MAGIC_XP,
    coins_per_cast: RUNE_CHAINBODY_LOW_ALCH_COINS,
    staff: STAFF_OF_FIRE_ID,
};

/// High Level Alchemy on noted rune chainbodies under a Fire battlestaff.
pub const ALCHER_FIRE_BATTLESTAFF_EXPECTATION: AlcherSpellExpectation = AlcherSpellExpectation {
    spell: AlcherSpell::High,
    magic_xp: HIGH_ALCH_MAGIC_XP,
    coins_per_cast: RUNE_CHAINBODY_HIGH_ALCH_COINS,
    staff: FIRE_BATTLESTAFF_ID,
};

/// The fire staff worn right now, if any.
fn worn_fire_staff(observation: &Observation) -> Option<i32> {
    ALCHER_FIRE_STAFFS
        .iter()
        .copied()
        .find(|id| observation.equipment_id(*id) > 0)
}

impl AlcherGeneratedCustomCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.observe_target(
            baseline,
            now,
            ADAMANT_SCIMITAR_ID,
            CERT_ADAMANT_SCIMITAR_ID,
            ADAMANT_SCIMITAR_ALCH_COINS,
        );
    }

    /// A fresh bank generation handed over the noted stack, with no unnoted copy
    /// in the pack and at least one cast's fuel.
    fn withdrawal_ready(
        &self,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
    ) -> bool {
        self.withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(noted) >= 1
            && now.item_id(unnoted) == 0
            && now.item_id(noted) > baseline.item_id(noted)
            && now.item_id(NATURE_RUNE_ID) >= 1
    }

    /// The High/default rows keep their own name-based guard on top of the
    /// shared withdrawal, exactly as before.
    pub fn observe_target(
        &mut self,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
        alch_coins: i32,
    ) {
        if self.withdrawal_ready(baseline, now, unnoted, noted) && now.item("Rune chainbody") == 0 {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.consumed |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(noted) < withdrawn.item_id(noted)
                && now.item_id(NATURE_RUNE_ID) < withdrawn.item_id(NATURE_RUNE_ID)
                && now.item_id(COINS_ID) - baseline.item_id(COINS_ID) == alch_coins
                && now.skill_xp("magic") - baseline.skill_xp("magic") >= HIGH_ALCH_MAGIC_XP
                && now.item("Rune chainbody") == 0;
        }
    }

    /// The selected-spell rows: the ordered withdrawal, then one frame where the
    /// noted stack, the Nature runes, the Magic XP and the coins agree on the
    /// same number of casts, with the selected fire staff actually worn after a
    /// baseline that wore no staff at all.
    pub fn observe_spelled(
        &mut self,
        expectation: AlcherSpellExpectation,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
    ) {
        self.spell = Some(expectation.spell);
        self.magic_xp_per_cast = expectation.magic_xp;
        self.coins_per_cast = expectation.coins_per_cast;
        self.required_staff = Some(expectation.staff);
        if self.baseline_staff.is_none() {
            self.baseline_staff = worn_fire_staff(baseline);
        }
        self.wrong_staff |= worn_fire_staff(now).is_some_and(|worn| worn != expectation.staff);
        if self.withdrawal_ready(baseline, now, unnoted, noted) {
            self.withdrawn = Some(now.clone());
        }
        let withdrawn = match &self.withdrawn {
            Some(withdrawn) => withdrawn,
            None => return,
        };
        if now.bank_open || now.bank_loaded || self.consumed {
            return;
        }
        let casts = withdrawn.item_id(noted) - now.item_id(noted);
        if casts < 1 {
            return;
        }
        let coins = expectation.coins_per_cast * casts;
        let xp = expectation.magic_xp * casts;
        let consumed = withdrawn.item_id(NATURE_RUNE_ID) - now.item_id(NATURE_RUNE_ID) == casts
            && now.item_id(COINS_ID) - withdrawn.item_id(COINS_ID) == coins
            && now.skill_xp("magic") - withdrawn.skill_xp("magic") == xp
            && now.item_id(COINS_ID) - baseline.item_id(COINS_ID) == coins
            && now.skill_xp("magic") - baseline.skill_xp("magic") == xp
            && worn_fire_staff(now) == Some(expectation.staff);
        if consumed {
            self.casts = casts;
            self.cast_staff = worn_fire_staff(now);
            self.consumed = true;
        }
    }

    /// Qualification for a spelled row: the exact cast arithmetic proved, the
    /// demanded staff worn, no staff in the baseline and no other fire staff
    /// seen anywhere after Start.
    pub fn qualified_spell(&self) -> bool {
        self.consumed
            && self.casts >= 1
            && !self.wrong_staff
            && self.baseline_staff.is_none()
            && self.required_staff.is_some()
            && self.cast_staff == self.required_staff
    }
}

fn swarm_targeting_local(observation: &Observation) -> bool {
    observation.npc_facts.iter().any(|npc| {
        npc.name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("Swarm"))
            && npc.targeting_local
    })
}

fn evade_hold(observation: &Observation) -> bool {
    observation.guardian.kind.as_deref() == Some("evade")
        && observation.guardian.hold
        && observation.guardian.ours
        && observation
            .guardian
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("Swarm"))
}

fn high_cast(from: &Observation, now: &Observation, noted: i32, coins_per_cast: i32) -> bool {
    if now.bank_open || now.bank_loaded {
        return false;
    }
    let casts = from.item_id(noted) - now.item_id(noted);
    if casts < 1 {
        return false;
    }
    from.item_id(NATURE_RUNE_ID) - now.item_id(NATURE_RUNE_ID) == casts
        && now.item_id(COINS_ID) - from.item_id(COINS_ID) == coins_per_cast * casts
        && now.skill_xp("magic") - from.skill_xp("magic") == HIGH_ALCH_MAGIC_XP * casts
        && worn_fire_staff(now) == Some(STAFF_OF_FIRE_ID)
}

fn rich_chainbody_exhausted(observation: &Observation) -> bool {
    observation.item_id(CERT_RUNE_CHAINBODY_ID) == 0 && observation.item_id(RUNE_CHAINBODY_ID) == 0
}

/// Ordered High-alch interruption: first cast, targeted Swarm + positive hit,
/// native guardian hold, actual flee under that hold, native hold release
/// independent of bank proximity, further rich cast from that release baseline
/// that exhausts remaining notes, bank return and loaded rich retirement near
/// Varrock West, then a complete poor restock (noted poor and Nature fuel)
/// before bank-closed consumption.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AlcherSwarmDrainCycle {
    pub withdrawn: Option<Observation>,
    pub first_cast: Option<Observation>,
    pub swarm_hit: Option<Observation>,
    pub guardian_hold: Option<Observation>,
    pub fled: Option<Observation>,
    pub released: Option<Observation>,
    pub further_cast: Option<Observation>,
    pub rich_retired: Option<Observation>,
    pub poor_withdrawn: Option<Observation>,
    pub poor_consumed: bool,
    pub baseline_staff: Option<i32>,
    pub wrong_staff: bool,
}

impl AlcherSwarmDrainCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.baseline_staff.is_none() {
            self.baseline_staff = worn_fire_staff(baseline);
        }
        self.wrong_staff |= worn_fire_staff(now).is_some_and(|worn| worn != STAFF_OF_FIRE_ID);
        if self.withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(CERT_RUNE_CHAINBODY_ID) >= 1
            && now.item_id(RUNE_CHAINBODY_ID) == 0
            && now.item_id(CERT_RUNE_CHAINBODY_ID) > baseline.item_id(CERT_RUNE_CHAINBODY_ID)
            && now.item_id(NATURE_RUNE_ID) >= 1
        {
            self.withdrawn = Some(now.clone());
        }
        if self.first_cast.is_none() {
            if let Some(withdrawn) = &self.withdrawn {
                if high_cast(
                    withdrawn,
                    now,
                    CERT_RUNE_CHAINBODY_ID,
                    RUNE_CHAINBODY_HIGH_ALCH_COINS,
                ) {
                    self.first_cast = Some(now.clone());
                }
            }
        }
        if self.first_cast.is_some()
            && self.swarm_hit.is_none()
            && swarm_targeting_local(now)
            && now.taking_damage
        {
            self.swarm_hit = Some(now.clone());
        }
        if self.swarm_hit.is_some() && self.guardian_hold.is_none() && evade_hold(now) {
            self.guardian_hold = Some(now.clone());
        }
        if let Some(held) = &self.guardian_hold {
            if self.fled.is_none()
                && evade_hold(now)
                && now.tile.is_some()
                && held.tile.is_some()
                && now.tile != held.tile
            {
                self.fled = Some(now.clone());
            }
        }
        if self.fled.is_some()
            && self.released.is_none()
            && !now.guardian.hold
            && now.guardian.kind.as_deref() != Some("evade")
        {
            self.released = Some(now.clone());
        }
        if self.further_cast.is_none() {
            if let Some(released) = &self.released {
                if high_cast(
                    released,
                    now,
                    CERT_RUNE_CHAINBODY_ID,
                    RUNE_CHAINBODY_HIGH_ALCH_COINS,
                ) && rich_chainbody_exhausted(now)
                {
                    self.further_cast = Some(now.clone());
                }
            }
        }
        if self
            .further_cast
            .as_ref()
            .is_some_and(rich_chainbody_exhausted)
            && self.rich_retired.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.bank_item_id(RUNE_CHAINBODY_ID) == 0
            && now.bank_item_id(CERT_RUNE_CHAINBODY_ID) == 0
            && rich_chainbody_exhausted(now)
            && near(now.tile, VARROCK_WEST_BANK, 6)
        {
            self.rich_retired = Some(now.clone());
        }
        if self.rich_retired.is_some()
            && self.poor_withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(CERT_YEW_LONGBOW_ID) >= 1
            && now.item_id(YEW_LONGBOW_ID) == 0
            && now.item_id(CERT_YEW_LONGBOW_ID) > baseline.item_id(CERT_YEW_LONGBOW_ID)
            && now.item_id(NATURE_RUNE_ID) >= 1
        {
            self.poor_withdrawn = Some(now.clone());
        }
        if let Some(poor) = &self.poor_withdrawn {
            self.poor_consumed |= high_cast(poor, now, CERT_YEW_LONGBOW_ID, YEW_LONGBOW_ALCH_COINS);
        }
    }

    pub fn qualified(&self) -> bool {
        self.first_cast.is_some()
            && self.swarm_hit.is_some()
            && self.guardian_hold.is_some()
            && self.fled.is_some()
            && self.released.is_some()
            && self.further_cast.is_some()
            && self.rich_retired.is_some()
            && self.poor_consumed
            && !self.wrong_staff
            && self.baseline_staff.is_none()
    }

    /// Compact named-phase receipt for timeout/stop diagnosis. Does not
    /// serialize world snapshots or grow an observation history.
    pub fn phase_status(&self) -> Value {
        json!({
            "firstcast": self.first_cast.is_some(),
            "swarm_hit": self.swarm_hit.is_some(),
            "guardianhold": self.guardian_hold.is_some(),
            "fled": self.fled.is_some(),
            "released": self.released.is_some(),
            "further": self.further_cast.is_some(),
            "richretired": self.rich_retired.is_some(),
            "poor": self.poor_consumed,
        })
    }
}

/// Two observed dart actions: exact product id, both inputs down, XP, no wrong tier.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DartFletcherCycle {
    pub first: Option<Observation>,
    pub further: bool,
    pub wrong_tier: bool,
}

impl DartFletcherCycle {
    pub fn observe(
        &mut self,
        tips: i32,
        feathers: i32,
        product: i32,
        wrong: i32,
        baseline: &Observation,
        now: &Observation,
    ) {
        self.wrong_tier |= now.item_id(wrong) > 0;
        if self.first.is_none()
            && now.item_id(product) >= 10
            && now.item_id(tips) < baseline.item_id(tips)
            && now.item_id(feathers) < baseline.item_id(feathers)
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
            && now.item_id(wrong) == 0
        {
            self.first = Some(now.clone());
        }
        if let Some(first) = &self.first {
            self.further |= now.item_id(product) > first.item_id(product)
                && now.item_id(tips) < first.item_id(tips)
                && now.item_id(feathers) < first.item_id(feathers)
                && now.skill_xp("fletching") > first.skill_xp("fletching")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_tier
    }
}

/// Identify a full pack, deposit script-created output, restock, then identify again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbCleanerCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub cleaned_after_withdrawal: bool,
    pub filter_violated: bool,
}

impl HerbCleanerCycle {
    pub fn observe(&mut self, named: bool, baseline: &Observation, now: &Observation) {
        if named
            && (now.item_id(UNIDENTIFIED_MARENTILL_ID) > 0
                || now.bank_item_id(UNIDENTIFIED_MARENTILL_ID) < 4
                    && now.bank_open
                    && now.bank_loaded)
        {
            self.filter_violated = true;
        }
        self.first_pack |= now.item_id(GUAM_LEAF_ID) >= 28
            && now.item_id(UNIDENTIFIED_GUAM_ID) == 0
            && now.skill_xp("herblore") > baseline.skill_xp("herblore")
            && baseline.item_id(GUAM_LEAF_ID) == 0;
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(GUAM_LEAF_ID) == 0
            && now.bank_item_id(GUAM_LEAF_ID) == 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(UNIDENTIFIED_GUAM_ID) >= 1
                && now.bank_item_id(UNIDENTIFIED_GUAM_ID)
                    < deposited.bank_item_id(UNIDENTIFIED_GUAM_ID)
                && (!named || now.bank_item_id(UNIDENTIFIED_MARENTILL_ID) == 4)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.cleaned_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(GUAM_LEAF_ID) > 0
                && now.item_id(UNIDENTIFIED_GUAM_ID) < withdrawn.item_id(UNIDENTIFIED_GUAM_ID)
                && now.skill_xp("herblore") > withdrawn.skill_xp("herblore");
        }
    }

    pub fn qualified(&self) -> bool {
        self.cleaned_after_withdrawal && !self.filter_violated
    }
}

pub const HERB_CLEANER_EMPTY_STOP_REASON: &str = "every selected herb is empty in the bank";
pub const HERB_CLEANER_BANK_TRIP_STOP_REASON: &str = "bank has no eligible herbs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HerbCleanerEmptyStopKind {
    LoopExhausted,
    BankTripNoEligibleHerbs,
}

impl HerbCleanerEmptyStopKind {
    fn from_reason(reason: &str) -> Option<Self> {
        match reason {
            HERB_CLEANER_EMPTY_STOP_REASON => Some(Self::LoopExhausted),
            HERB_CLEANER_BANK_TRIP_STOP_REASON => Some(Self::BankTripNoEligibleHerbs),
            _ => None,
        }
    }
}

/// Frozen 20-guam/two-selected-herb terminal fixture. Work, a fresh loaded
/// empty bank and the script's own Stop receipt must occur in that order.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbCleanerEmptyCycle {
    pub cleaned: bool,
    pub exhausted_bank: Option<Observation>,
    pub stop_kind: Option<HerbCleanerEmptyStopKind>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl HerbCleanerEmptyCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.cleaned |= baseline.item_id(GUAM_LEAF_ID) == 0
            && now.item_id(GUAM_LEAF_ID) > 0
            && now.item_id(UNIDENTIFIED_GUAM_ID) == 0
            && now.skill_xp("herblore") > baseline.skill_xp("herblore");
        if self.cleaned
            && self.exhausted_bank.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.bank_item_id(UNIDENTIFIED_GUAM_ID) == 0
            && now.bank_item_id(UNIDENTIFIED_MARENTILL_ID) == 0
        {
            self.exhausted_bank = Some(now.clone());
        }
    }

    pub fn observe_script_lifecycle(&mut self, receipt: script::ScriptLifecycleReceipt) {
        let stop_kind = HerbCleanerEmptyStopKind::from_reason(&receipt.reason);
        if self.exhausted_bank.is_some()
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && stop_kind.is_some()
        {
            self.stop_kind = stop_kind;
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        self.cleaned
            && self.exhausted_bank.is_some()
            && self.stop_kind.is_some()
            && self.stopped.is_some()
    }
}

/// Cut a chisel-kept pack, deposit except chisel, restock, then cut again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GemCutterCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub cut_after_withdrawal: bool,
    pub filter_violated: bool,
}

impl GemCutterCycle {
    pub fn observe(&mut self, named: bool, baseline: &Observation, now: &Observation) {
        if now.item_id(CRUSHED_GEMSTONE_ID) > 0 || now.bank_item_id(CRUSHED_GEMSTONE_ID) > 0 {
            self.filter_violated = true;
        }
        if named
            && (now.item_id(UNCUT_OPAL_ID) > 0
                || (now.bank_open
                    && now.bank_loaded
                    && now.bank_item_id(UNCUT_OPAL_ID) < 4
                    && now.bank_generation > baseline.bank_generation))
        {
            self.filter_violated = true;
        }
        self.first_pack |= now.item_id(SAPPHIRE_ID) >= 27
            && now.item_id(UNCUT_SAPPHIRE_ID) == 0
            && now.item_id(CHISEL_ID) == 1
            && now.skill_xp("crafting") > baseline.skill_xp("crafting")
            && baseline.item_id(SAPPHIRE_ID) == 0;
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(SAPPHIRE_ID) == 0
            && now.item_id(CHISEL_ID) == 1
            && now.bank_item_id(SAPPHIRE_ID) == 27
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(UNCUT_SAPPHIRE_ID) >= 1
                && now.item_id(CHISEL_ID) == 1
                && now.bank_item_id(UNCUT_SAPPHIRE_ID) < deposited.bank_item_id(UNCUT_SAPPHIRE_ID)
                && (!named || now.bank_item_id(UNCUT_OPAL_ID) == 4)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.cut_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(SAPPHIRE_ID) > 0
                && now.item_id(CHISEL_ID) == 1
                && now.item_id(UNCUT_SAPPHIRE_ID) < withdrawn.item_id(UNCUT_SAPPHIRE_ID)
                && now.skill_xp("crafting") > withdrawn.skill_xp("crafting");
        }
    }

    pub fn qualified(&self) -> bool {
        self.cut_after_withdrawal && !self.filter_violated
    }
}

/// Selected shut loc must lose Open through a same-session world change.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DoorOpenerCycle {
    pub selected: Option<BoundedLoc>,
    pub opened: bool,
}

impl DoorOpenerCycle {
    pub fn observe(
        &mut self,
        gate: bool,
        packed: (i32, i32, i32),
        closed_id: i32,
        open_id: i32,
        baseline: &Observation,
        now: &Observation,
    ) {
        if self.selected.is_none() {
            self.selected = baseline
                .loc_facts
                .iter()
                .find(|loc| {
                    loc.id == closed_id
                        && loc.open
                        && loc.level == packed.2
                        && (loc.x - packed.0).abs().max((loc.z - packed.1).abs()) <= 1
                        && loc_name_matches(loc.name.as_deref(), gate)
                })
                .cloned();
        }
        let Some(selected) = &self.selected else {
            return;
        };
        let still_shut = loc_at(
            now,
            selected.id,
            (selected.x, selected.z, selected.level),
            1,
        )
        .is_some_and(|loc| loc.open);
        let opened_leaf =
            loc_at(now, open_id, (selected.x, selected.z, selected.level), 3).is_some();
        self.opened |= !still_shut && opened_leaf;
    }

    pub fn qualified(&self) -> bool {
        self.selected.as_ref().is_some_and(|loc| loc.open) && self.opened
    }
}

/// Ordered plane/tile/XP milestones from selected gnome_course.rs2 dests.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeCourseCycle {
    pub log: Option<Observation>,
    pub ground_return: Option<Observation>,
    pub pipe: Option<Observation>,
    pub second_lap: bool,
}

impl GnomeCourseCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let xp = now.skill_xp("agility");
        if self.log.is_none()
            && xp > baseline.skill_xp("agility")
            && near(now.tile, GNOME_AFTER_LOG, 3)
        {
            self.log = Some(now.clone());
        }
        if let Some(log) = &self.log {
            if self.ground_return.is_none()
                && xp > log.skill_xp("agility")
                && near(now.tile, GNOME_GROUND_RETURN, 3)
            {
                self.ground_return = Some(now.clone());
            }
        }
        if let Some(ground) = &self.ground_return {
            if self.pipe.is_none()
                && xp > ground.skill_xp("agility")
                && near(now.tile, GNOME_PIPE, 6)
            {
                self.pipe = Some(now.clone());
            }
        }
        if let Some(pipe) = &self.pipe {
            self.second_lap |= xp > pipe.skill_xp("agility")
                && xp - baseline.skill_xp("agility") >= GNOME_SECOND_LOG_XP
                && near(now.tile, GNOME_AFTER_LOG, 3);
        }
    }

    pub fn qualified(&self) -> bool {
        self.second_lap
    }
}

/// Ordered ridge, five-obstacle lap bonus, and next-pipe milestones from the
/// selected m46_61 loc destinations. A queued click or aggregate XP alone
/// cannot advance the chain.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WildyAgilityCycle {
    pub ridge: Option<Observation>,
    pub pipe: Option<Observation>,
    pub rope: Option<Observation>,
    pub stone: Option<Observation>,
    pub log: Option<Observation>,
    pub rocks: Option<Observation>,
    pub further_pipe: bool,
}

impl WildyAgilityCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let xp = now.skill_xp("agility");
        let baseline_sequence = baseline
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(i32::MIN);
        let fresh_chat = |needle: &str| {
            now.chat.iter().any(|(sequence, text)| {
                *sequence > baseline_sequence
                    && text
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
            })
        };
        let on_course = now.tile.is_some_and(|(x, z, level)| {
            level == 0 && (3932..=3967).contains(&z) && (x - 2998).abs() <= 24
        });
        let ridge_failed = fresh_chat("lose your footing and fall into the wolf pit");
        if self.ridge.is_none()
            && on_course
            && !ridge_failed
            && (xp - baseline.skill_xp("agility") >= 15
                || fresh_chat("skillfully balance across the ridge"))
        {
            self.ridge = Some(now.clone());
        }
        if let Some(ridge) = &self.ridge {
            if self.pipe.is_none()
                && xp > ridge.skill_xp("agility")
                && near(now.tile, WILDY_PIPE_DEST, 3)
            {
                self.pipe = Some(now.clone());
            }
        }
        if let Some(pipe) = &self.pipe {
            if self.rope.is_none()
                && xp > pipe.skill_xp("agility")
                && near(now.tile, WILDY_ROPE_DEST, 3)
            {
                self.rope = Some(now.clone());
            }
        }
        if let Some(rope) = &self.rope {
            if self.stone.is_none()
                && xp > rope.skill_xp("agility")
                && near(now.tile, WILDY_STONE_DEST, 3)
            {
                self.stone = Some(now.clone());
            }
        }
        if let Some(stone) = &self.stone {
            if self.log.is_none()
                && xp > stone.skill_xp("agility")
                && near(now.tile, WILDY_LOG_DEST, 3)
            {
                self.log = Some(now.clone());
            }
        }
        if let Some(log) = &self.log {
            if self.rocks.is_none()
                && xp > log.skill_xp("agility")
                && xp - baseline.skill_xp("agility") >= WILDY_LAP_XP
                && near(now.tile, WILDY_ROCKS_DEST, 3)
            {
                self.rocks = Some(now.clone());
            }
        }
        if let Some(rocks) = &self.rocks {
            self.further_pipe |= xp > rocks.skill_xp("agility")
                && xp - baseline.skill_xp("agility") >= WILDY_FURTHER_XP
                && near(now.tile, WILDY_PIPE_DEST, 3);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further_pipe
    }
}

pub fn brimhaven_platform(tile: Option<(i32, i32, i32)>) -> Option<usize> {
    let (x, z, level) = tile?;
    if level != 3 {
        return None;
    }
    let xs = [2761, 2772, 2783, 2794, 2805];
    let zs = [9546, 9557, 9568, 9579, 9590];
    for (row, center_z) in zs.into_iter().enumerate() {
        for (column, center_x) in xs.into_iter().enumerate() {
            if (x - center_x).abs() <= 4 && (z - center_z).abs() <= 4 {
                return Some(row * xs.len() + column);
            }
        }
    }
    None
}

/// Ordered natural entrance fee, arena movement, first tag, first ticket, and
/// work after the ticket. Platform indexing only recognizes the selected
/// 5-by-5 arena centers and does not reproduce the foreign route planner.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BrimhavenAgilityCycle {
    pub paid: bool,
    pub entered: Option<(usize, Observation)>,
    pub moved: Option<(usize, Observation)>,
    pub first_tag: bool,
    pub ticket: Option<(usize, Observation)>,
    pub subsequent_work: bool,
}

impl BrimhavenAgilityCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let varp = now.varp(BRIMHAVEN_ARENA_VARP);
        self.paid |= baseline.item_id(COINS_ID) - now.item_id(COINS_ID) >= 200 && varp & 0b10 != 0;
        if !self.paid {
            return;
        }

        let platform = brimhaven_platform(now.tile);
        if self.entered.is_none() {
            if let Some(platform) = platform {
                self.entered = Some((platform, now.clone()));
            }
            return;
        }
        if self.moved.is_none() {
            let (entered_platform, entered) = self.entered.as_ref().unwrap();
            if platform.is_some_and(|platform| platform != *entered_platform)
                && now.skill_xp("agility") > entered.skill_xp("agility")
            {
                self.moved = Some((platform.unwrap(), now.clone()));
            }
            return;
        }
        if !self.first_tag {
            let baseline_sequence = baseline
                .chat
                .iter()
                .map(|(sequence, _)| *sequence)
                .max()
                .unwrap_or(i32::MIN);
            let fresh_next_pillar = now.chat.iter().any(|(sequence, text)| {
                *sequence > baseline_sequence && text.to_ascii_lowercase().contains("tag the next")
            });
            if varp & 0b1111 == 0b1111 && now.item_id(BRIMHAVEN_TICKET_ID) == 0 && fresh_next_pillar
            {
                self.first_tag = true;
            }
            return;
        }
        if self.ticket.is_none() {
            if now.item_id(BRIMHAVEN_TICKET_ID) >= 1 {
                if let Some(platform) = platform {
                    self.ticket = Some((platform, now.clone()));
                }
            }
            return;
        }
        let (ticket_platform, ticket) = self.ticket.as_ref().unwrap();
        self.subsequent_work |= platform.is_some_and(|platform| platform != *ticket_platform)
            || now.skill_xp("agility") > ticket.skill_xp("agility");
    }

    pub fn qualified(&self) -> bool {
        self.subsequent_work
    }
}

/// Full pack of exact flax 1779, Seers deposit, return, further pick.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxPickerCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl FlaxPickerCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_pack |= now.item_id(FLAX_ID) >= 28
            && baseline.item_id(FLAX_ID) == 0
            && now.item_id(FLAX_ID) > baseline.item_id(FLAX_ID);
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(FLAX_ID) == 0
            && now.bank_item_id(FLAX_ID) >= 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FLAX_FIELD, 12);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(FLAX_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Pick 1779 at the field, convert at the wheel with Crafting XP, deposit 1777,
/// closed return to the field, further Pick. Wool or noted ids fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxAioCycle {
    pub picked: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl FlaxAioCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(BALL_OF_WOOL_ID) > 0 || now.bank_item_id(BALL_OF_WOOL_ID) > 0;
        self.noted |= flax_spinner_noted(now);
        if self.picked.is_none()
            && now.item_id(FLAX_ID) >= 1
            && baseline.item_id(FLAX_ID) == 0
            && now.item_id(BOW_STRING_ID) == 0
            && !self.noted
            && !self.wrong_product
        {
            self.picked = Some(now.clone());
        }
        if let Some(picked) = &self.picked {
            if self.produced.is_none()
                && now.item_id(BOW_STRING_ID) >= 1
                && now.item_id(FLAX_ID) < picked.item_id(FLAX_ID)
                && now.skill_xp("crafting") > baseline.skill_xp("crafting")
                && near(now.tile, FLAX_SPINNER_WHEEL, 8)
                && now.item_id(BALL_OF_WOOL_ID) == 0
                && !self.noted
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(BOW_STRING_ID) == 0
            && now.bank_item_id(BOW_STRING_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FLAX_FIELD, 12);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(FLAX_ID) >= 1 && now.item_id(BALL_OF_WOOL_ID) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.picked.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Full pack of exact flax 1779, deposit, return, further pick. Bow string 1777
/// must never qualify this pick-only cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxAioPickCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl FlaxAioPickCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |= now.item_id(BOW_STRING_ID) > 0
            || now.bank_item_id(BOW_STRING_ID) > 0
            || now.item_id(BALL_OF_WOOL_ID) > 0
            || now.bank_item_id(BALL_OF_WOOL_ID) > 0;
        self.noted |= flax_spinner_noted(now);
        self.first_pack |= now.item_id(FLAX_ID) >= 28
            && baseline.item_id(FLAX_ID) == 0
            && now.item_id(FLAX_ID) > baseline.item_id(FLAX_ID)
            && !self.wrong_product;
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(FLAX_ID) == 0
            && now.bank_item_id(FLAX_ID) >= 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FLAX_FIELD, 12);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(FLAX_ID) >= 1 && !self.wrong_product;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.noted
    }
}

/// Ground Take 223, deposit, closed return to the egg field, further Take.
/// Eye of newt 221 must not mix into this cell. Ground spawns are not given.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbloreEggsCycle {
    pub taken: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl HerbloreEggsCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(EYE_OF_NEWT_ID) > 0 || now.bank_item_id(EYE_OF_NEWT_ID) > 0;
        self.noted |= herblore_eggs_noted(now);
        self.taken |= now.item_id(RED_SPIDERS_EGGS_ID) >= 1
            && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
            && !self.wrong_product
            && !self.noted;
        if self.taken
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(RED_SPIDERS_EGGS_ID) == 0
            && now.bank_item_id(RED_SPIDERS_EGGS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, EGG_FIELD, 14);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(RED_SPIDERS_EGGS_ID) >= 1 && !self.wrong_product;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.noted
    }
}

/// Betty shop purchase 221 with coins 995, deposit, return, further buy.
/// Eggs 223 must not mix into this cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbloreNewtCycle {
    pub bought: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub coins_spent: bool,
    pub coins_peak: i32,
    pub wrong_product: bool,
    pub noted: bool,
}

impl HerbloreNewtCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(RED_SPIDERS_EGGS_ID) > 0 || now.bank_item_id(RED_SPIDERS_EGGS_ID) > 0;
        self.noted |= herblore_newt_noted(now);
        let previous_coins_peak = self.coins_peak;
        self.coins_peak = previous_coins_peak.max(now.item_id(COINS_ID));
        self.bought |= now.item_id(EYE_OF_NEWT_ID) >= 1
            && baseline.item_id(EYE_OF_NEWT_ID) == 0
            && !self.wrong_product
            && !self.noted;
        if self.bought
            && previous_coins_peak > 0
            && now.item_id(EYE_OF_NEWT_ID) >= 1
            && now.item_id(COINS_ID) < previous_coins_peak
        {
            self.coins_spent = true;
        }
        if self.bought
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(EYE_OF_NEWT_ID) == 0
            && now.bank_item_id(EYE_OF_NEWT_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, BETTY_SHOP, 6);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(EYE_OF_NEWT_ID) >= 1 && !self.wrong_product;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.coins_spent && !self.wrong_product && !self.noted
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CombatSpec {
    pub target: &'static str,
    pub stand: (i32, i32, i32),
    pub radius: i32,
    pub food_id: i32,
    pub food_count: i32,
    pub weapon_id: i32,
    pub style: CombatStyleWitness,
    pub loot: CombatLoot,
    pub extra: CombatExtra,
    /// Worn projectile the ranged branch consumes (arrows in the ammo slot).
    pub projectile: Option<i32>,
    /// The extra consumable transition this branch must actually execute.
    pub consumable: CombatConsumable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatStyleWitness {
    Strength,
    FireStrike,
    Ranged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatConsumable {
    None,
    /// `useSpecial=true`: the spec bar arms and `%sa_energy` pays the cost.
    Special,
    /// `usePotions=true`: a dose leaves the flask and the boost lands mid-fight.
    Potions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLoot {
    HerbLawNature,
    BigBones,
    BigBonesOrLimpwurt,
    DragonBonesOrHide,
    /// Both guaranteed Green dragon drops must increase after Start.
    /// `DragonBonesOrHide` cannot distinguish that pair from two of one id.
    DragonBonesAndHide,
    /// Guard-drop pack stock: any one of the six verifiable Guard drops
    /// ([`GUARD_DROP_IDS`]) landing in the pack. ArdyFighter lists the same
    /// names in `DEFAULT_LOOT`.
    GuardDrop,
    /// Plain Bones (526) in the pack: the Guard's guaranteed death drop that
    /// AutoFighter bank injects as its only `loot` with burial off.
    Bones,
    /// RockCrab PeriodicBank listed loot the bank cell can verify by id.
    SapphireOrCasket,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatExtra {
    None,
    DungeonKey,
    RockActivation,
    WornShield,
    DungeonAmulet,
    StolenFood,
    /// FireGiant `EnterDungeon` from the raft: Start is on the surface.
    DungeonApproach,
}

pub fn combat_spec(case: CoreCase) -> Option<CombatSpec> {
    match case {
        CoreCase::ChaosDruid => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_FIELD,
            radius: 14,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `location=Chaos Druid Tower`: surface camp r4, Thieving 46 door, same
        // Chaos druid display name and Herb/Law/Nature pickup as Edgeville.
        CoreCase::ChaosDruidTower => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_TOWER_FIELD,
            radius: 4,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `location=Yanille Dungeon`: warrior room target identity differs; loot
        // still the card's selected Herb/Law/Nature set.
        CoreCase::ChaosDruidYanille => Some(CombatSpec {
            target: "Chaos druid warrior",
            stand: CHAOS_DRUID_YANILLE_FIELD,
            radius: 8,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::MossGiant => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::MossGiantPrepared => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // Observe reuse only. Bank-only dart Start is empty; do not feed this
        // spec to combat_baseline_ready(Ranged) or CombatCoreCycle::qualified().
        CoreCase::MossGiantDart => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: MOSS_GIANT_DART_FIELD_RADIUS,
            food_id: LOBSTER_ID,
            food_count: 0,
            weapon_id: BRONZE_DART_ID,
            style: CombatStyleWitness::Ranged,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: Some(BRONZE_DART_ID),
            consumable: CombatConsumable::None,
        }),
        CoreCase::HillGiant => Some(CombatSpec {
            target: "Giant",
            stand: HILL_GIANT_PIT,
            radius: 16,
            food_id: TROUT_ID,
            food_count: HILL_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBonesOrLimpwurt,
            extra: CombatExtra::DungeonKey,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::AutoFighter => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::AutoFighterMage => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: STAFF_OF_FIRE_ID,
            style: CombatStyleWitness::FireStrike,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `combatStyle=range`: the source selects the rapid combat mode itself
        // and fires the worn Bronze arrow stack, so both transitions are the
        // witnessed outcome instead of a prepared state.
        CoreCase::AutoFighterRange => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: MAPLE_SHORTBOW_ID,
            style: CombatStyleWitness::Ranged,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: Some(BRONZE_ARROW_ID),
            consumable: CombatConsumable::None,
        }),
        CoreCase::RockCrab => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SAFE_STAND,
            radius: 2,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::RockActivation,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // RockCrab's own `AmmoLogic.sweepPlan` branch: dormant Rocks into a crab,
        // rapid combat mode, and a worn projectile stack that actually shrinks.
        CoreCase::RockCrabRange => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SAFE_STAND,
            radius: 2,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: MAPLE_SHORTBOW_ID,
            style: CombatStyleWitness::Ranged,
            loot: CombatLoot::None,
            extra: CombatExtra::RockActivation,
            projectile: Some(BRONZE_ARROW_ID),
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragon | CoreCase::GreenDragonPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BASE_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // Fire Strike branch on the enabled GreenDragon card. Loot stays None:
        // this cell does not claim a full kill unless a later hop proves one.
        CoreCase::GreenDragonMagePrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: STAFF_OF_FIRE_ID,
            style: CombatStyleWitness::FireStrike,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `useSpecial=true` with the dragon dagger: the bar arms and `%sa_energy`
        // pays the 250 cost, so a queued-but-unspent arming cannot qualify.
        CoreCase::GreenDragonSpecial | CoreCase::GreenDragonSpecialPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: DRAGON_DAGGER_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::Special,
        }),
        // `usePotions=true`: a three-dose flask becomes its two-dose form and the
        // native boost has to land on the matching skill during the fight.
        CoreCase::GreenDragonPotions | CoreCase::GreenDragonPotionsPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::Potions,
        }),
        CoreCase::FireGiant => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantPrepared => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::ArdyFighter => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 12,
            food_id: CAKE_ID,
            food_count: 0,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::StolenFood,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // The five bank cells keep their card's combat shape; the bank trip
        // itself is declared in `combat_bank_spec`. Prepared food differs from
        // the plain core where the card's own trip rule needs a shortfall
        // (MossGiant banks when the pack runs dry, ChaosDruidKiller when the
        // carried food is under `foodWithdraw`). AutoFighter finishes its
        // fights instantly at these levels, so the cell's trip end is the
        // injected `bankAtLootSlots=1`: the loot class is the card's injected
        // `loot=[Bones]` (the Guard's guaranteed drop, burial off), and the
        // pack starts empty of it.
        CoreCase::AutoFighterBank => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::Bones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::MossGiantBank => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_BANK_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::HillGiantBank | CoreCase::HillGiantBankPrepared => Some(CombatSpec {
            target: "Giant",
            stand: HILL_GIANT_PIT,
            radius: 16,
            food_id: TROUT_ID,
            food_count: HILL_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBonesOrLimpwurt,
            extra: CombatExtra::DungeonKey,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::ChaosDruidBank => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_FIELD,
            radius: 14,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_BANK_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::ArdyFighterBank => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 12,
            food_id: CAKE_ID,
            food_count: 0,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            // The card's own DEFAULT_LOOT is already the Guard-reachable
            // names; `bankEveryItems=1` ends the trip on the first of those
            // drops. Pack starts empty of the class so the deposit cannot be
            // a pre-Start seed. StolenFood extra stays: the Baker's stall is
            // this card's restock.
            loot: CombatLoot::GuardDrop,
            extra: CombatExtra::StolenFood,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::RockCrabBank => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SAFE_STAND,
            radius: 2,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::SapphireOrCasket,
            extra: CombatExtra::RockActivation,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonBank => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonBankPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BANK_PREPARED_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonBankDefaultPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BANK_PREPARED_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesAndHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // Teleport escape is Magic XP + Varrock land, then the Edgeville bank
        // trip. Combat kills are not this cell: a flee-walk to Edgeville
        // without Magic XP / Varrock land fails it.
        CoreCase::GreenDragonTele => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonTelePrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: 0,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantApproach => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_RAFT,
            radius: 5,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::DungeonApproach,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantBank => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantBankPrepared | CoreCase::FireGiantCamelotPrepared => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        _ => None,
    }
}

/// The declared bank trip a bank cell has to execute: the pack stock that has
/// to land in the bank, the card's own booth stand, the food the card restocks
/// there (when it restocks at all) and the tile the trip returns to before
/// further work. Values are the frozen cards' own tiles and keep-lists; see
/// `docs/compat/05-options-149.md` and `docs/compat/05-options-150.md`.
#[derive(Debug, Clone, Copy)]
pub struct CombatBankSpec {
    /// Pack-to-bank stock: the card's own loot or carried-food class. Any one
    /// of these ids counts, because the source's matcher is per item.
    pub deposit: &'static [i32],
    /// The bank the card's own walk opens a booth at.
    pub stand: (i32, i32, i32),
    pub stand_radius: i32,
    /// Food the trip withdraws back into the pack, when the card restocks.
    pub restock: Option<i32>,
    /// Exact post-withdrawal pack count when this fixture selects one.
    pub restock_count: Option<i32>,
    /// Where the trip returns before further work.
    pub ret: (i32, i32, i32),
    pub ret_radius: i32,
    /// Optional waypoint the trip has to visit before the booth: Varrock land
    /// for GreenDragon tele, barrel wash-up for FireGiant barrel exit.
    pub via: Option<(i32, i32, i32)>,
    pub via_radius: i32,
    /// The via tile is a spellbook teleport: Magic XP has to land with it.
    pub via_magic: bool,
    /// When false, the bank trip itself is the cell (tele escape). The
    /// reviewed two-engagement combat core is not required.
    pub require_combat: bool,
    /// A death/recovery path cannot substitute for this fixture's intended
    /// escape route.
    pub forbid_death: bool,
}

pub fn combat_bank_spec(case: CoreCase) -> Option<CombatBankSpec> {
    match case {
        // `banking=Auto`: BankRun walks to the nearest bank from the anchor and
        // deposits everything its keep-list does not hold, then restocks food.
        // The deposit class is the card's own injected `loot=[Bones]`: burial
        // is off, so the pack only ever holds Bones because a Guard dropped
        // them and the script looted them.
        CoreCase::AutoFighterBank => Some(CombatBankSpec {
            deposit: &[BONES_ID],
            stand: ARDOUGNE_EAST_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            restock_count: None,
            ret: ARDY_THIEVER_STAND,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Script-owned Ardougne West trip: deposits everything but
        // food/runes/ammo/weapon and withdraws lobster back to the safespot.
        CoreCase::MossGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: MOSS_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: MOSS_GIANT_SAFESPOT,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Always-on trip end: Varrock West keeps only trout and the Brass key.
        CoreCase::HillGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: HILL_GIANT_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            restock_count: None,
            ret: HILL_GIANT_PIT,
            ret_radius: 16,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        CoreCase::HillGiantBankPrepared => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: HILL_GIANT_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            restock_count: Some(HILL_GIANT_BANK_PREPARED_RESTOCK),
            ret: HILL_GIANT_PIT,
            ret_radius: 16,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Edgeville trip end: `depositInventory` empties the pack, then the
        // card withdraws exactly its food back and returns through the trapdoor.
        CoreCase::ChaosDruidBank => Some(CombatBankSpec {
            deposit: &[LOBSTER_ID],
            stand: CHAOS_DRUID_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: CHAOS_DRUID_FIELD,
            ret_radius: 14,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // `bankStrategy=Loot count`: PeriodicBank deposits the card's own
        // Guard-reachable loot list and walks back to the market anchor. No
        // food restock (the Baker's stall is this card's food). The class
        // only ever enters the pack as a Guard drop.
        CoreCase::ArdyFighterBank => Some(CombatBankSpec {
            deposit: &GUARD_DROP_IDS,
            stand: ARDY_BANK,
            stand_radius: 6,
            restock: None,
            restock_count: None,
            ret: ARDY_THIEVER_STAND,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // RockCrab PeriodicBank Loot-count at Seers. The task deposits listed
        // loot and returns to `currentSpot()`; it does not restock food
        // (`BankRun` is the food-gone restock, a different trip).
        CoreCase::RockCrabBank => Some(CombatBankSpec {
            deposit: &[UNCUT_SAPPHIRE_ID, CASKET_ID],
            stand: SEERS_BANK,
            stand_radius: 6,
            restock: None,
            restock_count: None,
            ret: ROCK_CRAB_BANK_RET,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // GreenDragon BankRun to Edgeville: deposit except keep-list, withdraw
        // food to `foodWithdraw` 20, walk back past the ditch.
        CoreCase::GreenDragonBank | CoreCase::GreenDragonBankPrepared => Some(CombatBankSpec {
            deposit: &[DRAGON_BONES_ID, GREEN_DRAGONHIDE_ID],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        CoreCase::GreenDragonBankDefaultPrepared => Some(CombatBankSpec {
            deposit: &[GREEN_DRAGONHIDE_ID],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: Some(GREEN_DRAGON_BANK_PREPARED_RESTOCK),
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // `escape=Teleport to Varrock`: Magic XP and a Varrock land, then the
        // Edgeville booth. A south-walk flee without the teleport fails.
        CoreCase::GreenDragonTele => Some(CombatBankSpec {
            deposit: &[],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: Some(VARROCK_TELE_LAND),
            via_radius: 8,
            via_magic: true,
            require_combat: false,
            forbid_death: false,
        }),
        CoreCase::GreenDragonTelePrepared => Some(CombatBankSpec {
            deposit: &[],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: Some(VARROCK_TELE_LAND),
            via_radius: 8,
            via_magic: true,
            require_combat: false,
            forbid_death: true,
        }),
        // Barrel exit to (2527,3413,0) then Ardougne West restock and re-entry.
        CoreCase::FireGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID],
            stand: FIRE_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: FIRE_GIANT_ROOM,
            ret_radius: 10,
            via: Some(FIRE_GIANT_WASH),
            via_radius: 6,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        CoreCase::FireGiantBankPrepared => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID],
            stand: FIRE_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: Some(FIRE_GIANT_BANK_PREPARED_RESTOCK),
            ret: FIRE_GIANT_ROOM,
            ret_radius: 10,
            via: Some(FIRE_GIANT_WASH),
            via_radius: 6,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Source-specific Camelot escape: land + Magic XP, then Seers restock.
        // The Seers endpoint alone is not this via.
        CoreCase::FireGiantCamelotPrepared => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID],
            stand: SEERS_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: Some(FIRE_GIANT_CAMELOT_PREPARED_RESTOCK),
            ret: FIRE_GIANT_ROOM,
            ret_radius: 10,
            via: Some(CAMELOT_TELE_LAND),
            via_radius: 8,
            via_magic: true,
            require_combat: true,
            forbid_death: false,
        }),
        _ => None,
    }
}

pub fn combat_noted(observation: &Observation) -> bool {
    noted_herb_id_held(observation)
        || observation.item_id(NOTED_BIG_BONES_ID) > 0
        || observation.bank_item_id(NOTED_BIG_BONES_ID) > 0
        || observation.item_id(NOTED_LIMPWURT_ROOT_ID) > 0
        || observation.bank_item_id(NOTED_LIMPWURT_ROOT_ID) > 0
        || observation.item_id(NOTED_BONES_ID) > 0
        || observation.bank_item_id(NOTED_BONES_ID) > 0
        || observation.item_id(NOTED_DRAGON_BONES_ID) > 0
        || observation.bank_item_id(NOTED_DRAGON_BONES_ID) > 0
        || observation.item_id(NOTED_GREEN_DRAGONHIDE_ID) > 0
        || observation.bank_item_id(NOTED_GREEN_DRAGONHIDE_ID) > 0
        || observation.item_id(NOTED_DRAGONFIRE_SHIELD_ID) > 0
        || observation.bank_item_id(NOTED_DRAGONFIRE_SHIELD_ID) > 0
        || observation.item_id(NOTED_CASKET_ID) > 0
        || observation.bank_item_id(NOTED_CASKET_ID) > 0
        || observation.item_id(NOTED_UNCUT_SAPPHIRE_ID) > 0
        || observation.bank_item_id(NOTED_UNCUT_SAPPHIRE_ID) > 0
        || noted_stall_food(observation) > 0
}

pub fn noted_herb_id_held(observation: &Observation) -> bool {
    observation
        .item_ids
        .keys()
        .copied()
        .chain(observation.bank_ids.keys().copied())
        .any(noted_herb_id)
}

pub fn combat_loot_count(observation: &Observation, loot: CombatLoot) -> i32 {
    match loot {
        CombatLoot::HerbLawNature => {
            observation.item_id(NATURE_RUNE_ID)
                + observation.item_id(LAW_RUNE_ID)
                + observation
                    .item_ids
                    .iter()
                    .filter(|(id, _)| unidentified_herb_id(**id))
                    .map(|(_, count)| *count)
                    .sum::<i32>()
        }
        CombatLoot::BigBones => observation.item_id(BIG_BONES_ID),
        CombatLoot::BigBonesOrLimpwurt => {
            observation.item_id(BIG_BONES_ID) + observation.item_id(LIMPWURT_ROOT_ID)
        }
        CombatLoot::DragonBonesOrHide | CombatLoot::DragonBonesAndHide => {
            observation.item_id(DRAGON_BONES_ID) + observation.item_id(GREEN_DRAGONHIDE_ID)
        }
        CombatLoot::GuardDrop => GUARD_DROP_IDS
            .iter()
            .map(|id| observation.item_id(*id))
            .sum(),
        CombatLoot::Bones => observation.item_id(BONES_ID),
        CombatLoot::SapphireOrCasket => {
            observation.item_id(UNCUT_SAPPHIRE_ID) + observation.item_id(CASKET_ID)
        }
        CombatLoot::None => 0,
    }
}

pub fn combat_baseline_ready(baseline: &Observation, spec: CombatSpec) -> bool {
    let food_ok = match spec.style {
        CombatStyleWitness::FireStrike => baseline.item_id(spec.food_id) == spec.food_count,
        CombatStyleWitness::Ranged | CombatStyleWitness::Strength if spec.food_count == 0 => true,
        CombatStyleWitness::Ranged | CombatStyleWitness::Strength => {
            baseline.item_id(spec.food_id) >= spec.food_count
        }
    };
    let style_ok = match spec.style {
        CombatStyleWitness::Strength => {
            baseline.level("attack") >= COMBAT_ATTACK_LEVEL
                && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
                && held_id(baseline, spec.weapon_id) >= 1
        }
        CombatStyleWitness::FireStrike => {
            baseline.level("magic") >= AUTO_FIGHTER_MAGE_LEVEL
                && baseline.effective_level("magic") >= AUTO_FIGHTER_MAGE_LEVEL
                && baseline.equipment_id(spec.weapon_id) == 1
                && baseline.item_id(spec.weapon_id) == 0
                && baseline.item_id(MIND_RUNE_ID) == AUTO_FIGHTER_MAGE_CASTS
                && baseline.item_id(AIR_RUNE_ID) == AUTO_FIGHTER_MAGE_AIR_RUNES
        }
        // Both ranged pieces worn: the bow in the weapon slot and the projectile
        // stack in the ammo slot the source fires from.
        CombatStyleWitness::Ranged => {
            baseline.level("ranged") >= RANGED_LEVEL
                && baseline.equipment_id(spec.weapon_id) >= 1
                && spec
                    .projectile
                    .is_some_and(|ammo| baseline.equipment_id(ammo) >= 1)
        }
    };
    let extra_ok = match spec.extra {
        CombatExtra::None => true,
        CombatExtra::RockActivation => baseline.dormant_rocks_seen,
        CombatExtra::DungeonKey => baseline.item_id(BRASS_KEY_ID) == 1,
        CombatExtra::WornShield => {
            baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
                && baseline.tile.is_some_and(|tile| tile.1 >= WILDERNESS_MIN_Z)
        }
        CombatExtra::DungeonAmulet => {
            held_id(baseline, GLARIALS_AMULET_ID) >= 1
                && held_id(baseline, ROPE_ID) >= 1
                && baseline.tile.is_some_and(|tile| tile.1 >= DUNGEON_MIN_Z)
        }
        CombatExtra::StolenFood => {
            baseline.level("thieving") >= 5
                && stall_food(baseline) == 0
                && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
        }
        CombatExtra::DungeonApproach => {
            held_id(baseline, GLARIALS_AMULET_ID) >= 1
                && held_id(baseline, ROPE_ID) >= 1
                && baseline.tile.is_some_and(|tile| tile.1 < DUNGEON_MIN_Z)
        }
    };
    // The consumable branch starts from an unspent, unboosted state: a seeded
    // boost or an already-armed bar would let the run "pass" without the sip
    // or the special it is meant to prove.
    let consumable_ok = match spec.consumable {
        CombatConsumable::None => true,
        CombatConsumable::Special => {
            baseline.equipment_id(spec.weapon_id) >= 1
                && baseline.varp(SA_ARMED_VARP) != SA_ARMED_VALUE
        }
        CombatConsumable::Potions => {
            baseline.item_id(SUPER_ATTACK_3_ID) >= 1
                && baseline.item_id(SUPER_ATTACK_2_ID) == 0
                && baseline.item_id(SUPER_STRENGTH_3_ID) >= 1
                && baseline.item_id(SUPER_STRENGTH_2_ID) == 0
                && baseline.effective_level("attack") <= baseline.level("attack")
                && baseline.effective_level("strength") <= baseline.level("strength")
        }
    };
    near(baseline.tile, spec.stand, spec.radius)
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && food_ok
        && style_ok
        && combat_loot_count(baseline, spec.loot) == 0
        && !combat_noted(baseline)
        && extra_ok
        && consumable_ok
}

fn prepared_combat_baseline_ready(case: CoreCase, baseline: &Observation) -> bool {
    // Mage is 70 Magic/Defence/HP with staff+shield. Rune armour and a
    // scimitar would falsify this branch; check it before armour_ready.
    if case == CoreCase::GreenDragonMagePrepared {
        return ["magic", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL)
            && baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_FOOD
            && baseline.equipment_id(STAFF_OF_FIRE_ID) == 1
            && baseline.item_id(STAFF_OF_FIRE_ID) == 0
            && baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
            && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
            && baseline.item_id(MIND_RUNE_ID) == AUTO_FIGHTER_MAGE_CASTS
            && baseline.item_id(AIR_RUNE_ID) == AUTO_FIGHTER_MAGE_AIR_RUNES
            && baseline.item_id(RUNE_SCIMITAR_ID) == 0
            && baseline.equipment_id(RUNE_SCIMITAR_ID) == 0
            && [RUNE_CHAINBODY_ID, RUNE_PLATELEGS_ID, RUNE_FULL_HELM_ID]
                .into_iter()
                .all(|id| baseline.equipment_id(id) == 0 && baseline.item_id(id) == 0);
    }
    let armour_ready = [RUNE_CHAINBODY_ID, RUNE_PLATELEGS_ID, RUNE_FULL_HELM_ID]
        .into_iter()
        .all(|id| baseline.equipment_id(id) == 1);
    let exact_stats = match case {
        CoreCase::GreenDragonSpecialPrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL),
        CoreCase::GreenDragonPrepared | CoreCase::FireGiantPrepared => {
            baseline.level("defence") == COMBAT_ATTACK_LEVEL
        }
        CoreCase::GreenDragonPotionsPrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL),
        CoreCase::GreenDragonTelePrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == GREEN_DRAGON_TELE_PREPARED_LEVEL),
        CoreCase::GreenDragonBankPrepared
        | CoreCase::GreenDragonBankDefaultPrepared
        | CoreCase::FireGiantBankPrepared
        | CoreCase::FireGiantCamelotPrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == BANK_PRESSURE_PREPARED_LEVEL),
        CoreCase::MossGiantPrepared | CoreCase::HillGiantBankPrepared => {
            ["attack", "strength", "defence", "hitpoints"]
                .into_iter()
                .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL)
        }
        _ => return false,
    };
    let exact_food = match case {
        CoreCase::GreenDragonPrepared => baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_BASE_FOOD,
        CoreCase::GreenDragonSpecialPrepared | CoreCase::GreenDragonPotionsPrepared => {
            baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_FOOD
        }
        CoreCase::GreenDragonBankPrepared | CoreCase::GreenDragonBankDefaultPrepared => {
            baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_BANK_PREPARED_FOOD
        }
        CoreCase::GreenDragonTelePrepared => baseline.item_id(LOBSTER_ID) == 0,
        CoreCase::FireGiantPrepared => baseline.item_id(LOBSTER_ID) == FIRE_GIANT_FOOD,
        CoreCase::FireGiantBankPrepared | CoreCase::FireGiantCamelotPrepared => {
            baseline.item_id(LOBSTER_ID) == FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD
        }
        CoreCase::MossGiantPrepared => baseline.item_id(LOBSTER_ID) == MOSS_GIANT_FOOD,
        CoreCase::HillGiantBankPrepared => baseline.item_id(TROUT_ID) == HILL_GIANT_FOOD,
        _ => return false,
    };
    let exact_profile = exact_stats && armour_ready && exact_food;
    exact_profile
        && match case {
            CoreCase::GreenDragonPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 0
            }
            CoreCase::GreenDragonSpecialPrepared => {
                baseline.item_id(DRAGON_DAGGER_ID) == 0
                    && baseline.equipment_id(DRAGON_DAGGER_ID) == 1
                    && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
                    && baseline.varp(SA_ENERGY_VARP) >= DRAGON_DAGGER_SPECIAL_COST
            }
            CoreCase::GreenDragonPotionsPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 0
            }
            CoreCase::GreenDragonBankPrepared | CoreCase::GreenDragonBankDefaultPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
                    && baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
            }
            CoreCase::GreenDragonTelePrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
                    && baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
                    && baseline.level("magic") >= VARROCK_TELE_MAGIC
                    && baseline.item_id(LAW_RUNE_ID) >= 1
                    && baseline.item_id(AIR_RUNE_ID) >= 3
                    && baseline.item_id(FIRE_RUNE_ID) >= 1
                    && baseline.effective_level("hitpoints") == GREEN_DRAGON_TELE_PREPARED_HP
            }
            CoreCase::FireGiantPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
            }
            CoreCase::FireGiantBankPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
            }
            CoreCase::FireGiantCamelotPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.equipment_id(GLARIALS_AMULET_ID) == 1
                    && baseline.item_id(GLARIALS_AMULET_ID) == 0
                    && baseline.level("magic") >= CAMELOT_TELE_MAGIC
                    && baseline.effective_level("magic") >= CAMELOT_TELE_MAGIC
                    && baseline.item_id(AIR_RUNE_ID) >= CAMELOT_AIR_CARRY
                    && baseline.item_id(LAW_RUNE_ID) >= CAMELOT_LAW_CARRY
            }
            CoreCase::MossGiantPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
            }
            CoreCase::HillGiantBankPrepared => {
                baseline.item_id(ADAMANT_SCIMITAR_ID) == 0
                    && baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                    && baseline.item_id(BRASS_KEY_ID) == 1
            }
            _ => false,
        }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatNpcLast {
    pub name: String,
    pub health: i32,
    pub animation: i32,
    pub tile: (i32, i32, i32),
    pub engaged: bool,
    pub defeated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CombatFact {
    pub kind: &'static str,
    pub index: usize,
    pub name: String,
    pub health: i32,
    pub animation: i32,
    pub loot_id: Option<i32>,
    pub zero_health: bool,
}

// JSON objects cannot encode tuple keys. Retain each ground-count fact as a
// named row while keeping the observer's keyed map unchanged.
pub fn serialize_ground_counts<S: serde::Serializer>(
    counts: &BTreeMap<(i32, i32, i32, i32), i32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct GroundCount {
        id: i32,
        x: i32,
        z: i32,
        level: i32,
        count: i32,
    }
    serializer.collect_seq(
        counts
            .iter()
            .map(|(&(id, x, z, level), &count)| GroundCount {
                id,
                x,
                z,
                level,
                count,
            }),
    )
}

/// Sustained selected-target combat: two engagements, a verified defeat
/// that is not mere despawn, fresh selected work after that defeat,
/// selected-style XP, and exact loot where required.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatCoreCycle {
    pub engagements: u32,
    pub defeats: u32,
    pub further_work: bool,
    pub style_xp: bool,
    pub autocast_armed: bool,
    pub mind_rune_consumed: bool,
    pub air_runes_consumed: bool,
    /// Rapid `com_mode` observed, the ranged branches' own style selection.
    pub combat_mode: bool,
    /// Highest worn projectile count seen; a later lower count means arrows fired.
    pub projectile_peak: i32,
    pub projectile_fired: bool,
    /// Spec bar armed (`%sa_attack`) and `%sa_energy` actually paid the cost.
    pub special_armed: bool,
    pub special_spent: bool,
    /// Highest `%sa_energy` seen; a later fall of a full cost means a real spend.
    pub special_energy_peak: i32,
    /// A three-dose flask became its two-dose form while in combat.
    pub potion_dose: bool,
    /// The matching native attack/strength boost landed after that dose.
    pub potion_boost: bool,
    pub looted: bool,
    pub noted: bool,
    pub activated: bool,
    pub shield_worn: bool,
    /// Mage-branch only: latched when Start wore 1540 and a later WornShield frame saw it off.
    /// Re-equipping does not clear this; `qualified()` still uses sticky `shield_worn` only.
    pub shield_continuity_break: bool,
    pub stolen_food: bool,
    pub wrong_item: bool,
    pub dormant_indexes: BTreeSet<usize>,
    pub dormant_tiles: BTreeMap<usize, (i32, i32, i32)>,
    pub last: BTreeMap<usize, CombatNpcLast>,
    pub currently_engaged: BTreeSet<usize>,
    #[serde(serialize_with = "serialize_ground_counts")]
    pub previous_ground: BTreeMap<(i32, i32, i32, i32), i32>,
    pub facts: Vec<CombatFact>,
}

impl CombatCoreCycle {
    pub fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        self.noted |= combat_noted(now);
        match spec.style {
            CombatStyleWitness::Strength => {
                self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength");
            }
            CombatStyleWitness::FireStrike => {
                self.style_xp |= now.skill_xp("magic") > baseline.skill_xp("magic");
                self.autocast_armed |= now.varp(AUTOCAST_MAGIC_VARP) == AUTOCAST_ARMED_VALUE;
                self.mind_rune_consumed |=
                    now.item_id(MIND_RUNE_ID) < baseline.item_id(MIND_RUNE_ID);
                self.air_runes_consumed |= baseline
                    .item_id(AIR_RUNE_ID)
                    .saturating_sub(now.item_id(AIR_RUNE_ID))
                    >= 2;
            }
            // `combatStyle=range`: the source picks the rapid combat mode and fires
            // the worn stack down, so both are observed transitions of this branch.
            CombatStyleWitness::Ranged => {
                self.style_xp |= now.skill_xp("ranged") > baseline.skill_xp("ranged");
                self.combat_mode |= now.varp(COMBAT_MODE_VARP) == RAPID_COMBAT_MODE;
                if let Some(ammo) = spec.projectile {
                    let worn = now.equipment_id(ammo);
                    self.projectile_fired |= self.projectile_peak > worn;
                    self.projectile_peak = self.projectile_peak.max(worn);
                }
            }
        }
        self.looted |= match spec.loot {
            CombatLoot::DragonBonesAndHide => {
                now.item_id(DRAGON_BONES_ID) > baseline.item_id(DRAGON_BONES_ID)
                    && now.item_id(GREEN_DRAGONHIDE_ID) > baseline.item_id(GREEN_DRAGONHIDE_ID)
            }
            _ => combat_loot_count(now, spec.loot) > combat_loot_count(baseline, spec.loot),
        };
        self.wrong_item |= now.item_id(BLACK_DRAGONHIDE_ID) > 0
            || now.item_id(RED_DRAGONHIDE_ID) > 0
            || now.item_id(BLUE_DRAGONHIDE_ID) > 0
            || now.item_id(CHOCOLATE_CAKE_ID) > 0;
        if spec.extra == CombatExtra::WornShield {
            self.shield_worn |= now.equipment_id(DRAGONFIRE_SHIELD_ID) >= 1;
            if baseline.equipment_id(DRAGONFIRE_SHIELD_ID) >= 1
                && now.equipment_id(DRAGONFIRE_SHIELD_ID) == 0
            {
                self.shield_continuity_break = true;
            }
        }
        if spec.extra == CombatExtra::StolenFood {
            self.stolen_food |= stall_food(now) > stall_food(baseline);
        }
        if spec.extra == CombatExtra::RockActivation {
            for npc in &now.npc_facts {
                let Some(name) = npc.name.as_deref().map(str::trim) else {
                    continue;
                };
                if name == ROCK_CRAB_DORMANT_NAME {
                    self.dormant_indexes.insert(npc.index);
                    self.dormant_tiles.insert(npc.index, npc.tile);
                }
                // Preserve the existing index OR tile identity match. An
                // unrelated index and tile cannot count as activation; this
                // predicate does not separately rule out reuse of an old slot.
                if name == spec.target
                    && (self.dormant_indexes.contains(&npc.index)
                        || self.dormant_tiles.values().any(|tile| *tile == npc.tile))
                {
                    self.activated = true;
                }
            }
        }
        match spec.consumable {
            CombatConsumable::None => {}
            // `useSpecial=true`: arming alone is not an execution, the pool has to
            // pay the wielded weapon's cost. A full cost below either the pre-Start
            // pool or the highest pool seen since is that payment; the energy only
            // ever regenerates, so a fall of this size cannot be anything else.
            CombatConsumable::Special => {
                self.special_armed |= now.varp(SA_ARMED_VARP) == SA_ARMED_VALUE;
                let energy = now.varp(SA_ENERGY_VARP);
                self.special_energy_peak = self.special_energy_peak.max(energy);
                self.special_spent |= baseline.varp(SA_ENERGY_VARP).saturating_sub(energy)
                    >= DRAGON_DAGGER_SPECIAL_COST
                    || self.special_energy_peak.saturating_sub(energy)
                        >= DRAGON_DAGGER_SPECIAL_COST;
            }
            // `usePotions=true`: the sip shows up as the three-dose flask becoming
            // its two-dose form in a fight, and the boost has to land afterwards.
            CombatConsumable::Potions => {
                if !self.potion_dose {
                    let attack_sip = baseline.item_id(SUPER_ATTACK_3_ID)
                        > now.item_id(SUPER_ATTACK_3_ID)
                        && now.item_id(SUPER_ATTACK_2_ID) > baseline.item_id(SUPER_ATTACK_2_ID);
                    let strength_sip = baseline.item_id(SUPER_STRENGTH_3_ID)
                        > now.item_id(SUPER_STRENGTH_3_ID)
                        && now.item_id(SUPER_STRENGTH_2_ID) > baseline.item_id(SUPER_STRENGTH_2_ID);
                    if (attack_sip || strength_sip) && now.local_in_combat {
                        self.potion_dose = true;
                    }
                }
                if self.potion_dose {
                    self.potion_boost |= now.effective_level("attack") > now.level("attack")
                        || now.effective_level("strength") > now.level("strength");
                }
            }
        }
        let mut seen = BTreeSet::new();
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != spec.target {
                continue;
            }
            seen.insert(npc.index);
            let prev = self.last.get(&npc.index).cloned();
            let selected_combat = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            let health_drop = prev.as_ref().is_some_and(|prev| {
                prev.health > 0 && prev.health > npc.health && npc.health >= 0 && prev.engaged
            });
            let new_spawn = prev
                .as_ref()
                .is_some_and(|prev| prev.defeated && npc.total_health > 0 && npc.health > 0);
            if new_spawn {
                self.currently_engaged.remove(&npc.index);
            }
            // Defeated indexes stay dead until a positive-health respawn. A known
            // zero-health bar (total_health > 0) is a corpse/death frame — selected
            // combat on it must not open a new engagement or mark further work.
            // total_health == 0 remains unknown (no bar), not universally dead.
            let stale_defeated = prev.as_ref().is_some_and(|prev| prev.defeated) && !new_spawn;
            let known_corpse = npc.total_health > 0 && npc.health == 0;
            let was_engaged = self.currently_engaged.contains(&npc.index);
            let actual_work = !stale_defeated && !known_corpse && (selected_combat || health_drop);
            if self.defeats > 0 && was_engaged && actual_work {
                self.further_work = true;
            }
            if actual_work && !was_engaged {
                self.engagements += 1;
                self.currently_engaged.insert(npc.index);
                if self.facts.len() < 16 {
                    self.facts.push(CombatFact {
                        kind: "engage",
                        index: npc.index,
                        name: name.to_string(),
                        health: npc.health,
                        animation: npc.animation,
                        loot_id: None,
                        zero_health: false,
                    });
                }
            }
            if npc.total_health > 0
                && npc.health == 0
                && prev
                    .as_ref()
                    .is_some_and(|prev| prev.engaged && prev.health > 0)
                && self.currently_engaged.contains(&npc.index)
            {
                self.record_defeat(npc.index, name, npc.health, npc.animation, None, true);
            }
            let defeated =
                self.last.get(&npc.index).is_some_and(|last| last.defeated) && !new_spawn;
            self.last.insert(
                npc.index,
                CombatNpcLast {
                    name: name.to_string(),
                    health: npc.health,
                    animation: npc.animation,
                    tile: npc.tile,
                    engaged: self.currently_engaged.contains(&npc.index),
                    defeated,
                },
            );
        }
        let missing: Vec<(usize, CombatNpcLast)> = self
            .last
            .iter()
            .filter(|(index, prev)| prev.engaged && !prev.defeated && !seen.contains(index))
            .map(|(index, prev)| (*index, prev.clone()))
            .collect();
        for (index, prev) in missing {
            let loot_id = now.ground_loot.iter().find_map(|item| {
                let key = (item.id, item.tile.0, item.tile.1, item.tile.2);
                let baseline_count = baseline
                    .ground_loot
                    .iter()
                    .filter(|old| old.id == item.id && old.tile == item.tile)
                    .map(|old| old.count)
                    .sum::<i32>();
                let previous_count = self.previous_ground.get(&key).copied().unwrap_or(0);
                (item.tile == prev.tile
                    && combat_loot_id(item.id, spec.loot)
                    && item.count > baseline_count.max(previous_count))
                .then_some(item.id)
            });
            if loot_id.is_some() {
                self.record_defeat(
                    index,
                    &prev.name,
                    prev.health,
                    prev.animation,
                    loot_id,
                    false,
                );
            }
        }
        self.previous_ground.clear();
        for item in &now.ground_loot {
            let key = (item.id, item.tile.0, item.tile.1, item.tile.2);
            *self.previous_ground.entry(key).or_insert(0) += item.count;
        }
    }

    pub fn record_defeat(
        &mut self,
        index: usize,
        name: &str,
        health: i32,
        animation: i32,
        loot_id: Option<i32>,
        zero_health: bool,
    ) {
        if self.last.get(&index).is_some_and(|prev| prev.defeated) {
            return;
        }
        self.defeats += 1;
        self.currently_engaged.remove(&index);
        if let Some(prev) = self.last.get_mut(&index) {
            prev.defeated = true;
            prev.engaged = false;
        }
        if self.facts.len() < 16 {
            self.facts.push(CombatFact {
                kind: "defeat",
                index,
                name: name.to_string(),
                health,
                animation,
                loot_id,
                zero_health,
            });
        }
    }

    pub fn qualified(&self, spec: CombatSpec) -> bool {
        let style_ok = match spec.style {
            CombatStyleWitness::Strength => self.style_xp,
            CombatStyleWitness::FireStrike => {
                self.style_xp
                    && self.autocast_armed
                    && self.mind_rune_consumed
                    && self.air_runes_consumed
            }
            // Ranged XP alone is not the branch: the source's rapid combat mode and
            // a projectile that actually left the quiver are both required.
            CombatStyleWitness::Ranged => {
                self.style_xp && self.combat_mode && self.projectile_fired
            }
        };
        let extra_ok = match spec.extra {
            CombatExtra::None | CombatExtra::DungeonKey | CombatExtra::DungeonAmulet => true,
            CombatExtra::RockActivation => self.activated,
            CombatExtra::WornShield => self.shield_worn,
            CombatExtra::StolenFood => self.stolen_food,
            CombatExtra::DungeonApproach => true,
        };
        let consumable_ok = match spec.consumable {
            CombatConsumable::None => true,
            CombatConsumable::Special => self.special_armed && self.special_spent,
            CombatConsumable::Potions => self.potion_dose && self.potion_boost,
        };
        self.engagements >= 2
            && self.defeats >= 1
            && self.further_work
            && style_ok
            && (spec.loot == CombatLoot::None || self.looted)
            && !self.noted
            && !self.wrong_item
            && extra_ok
            && consumable_ok
    }
}

/// Fire Strike prepared branch: one correct-target engagement, autocast,
/// Mind+Air spend, and the shield staying worn. Not a full-kill claim.
pub fn qualified_mage_branch(cycle: &CombatCoreCycle, spec: CombatSpec) -> bool {
    spec.style == CombatStyleWitness::FireStrike
        && spec.extra == CombatExtra::WornShield
        && cycle.engagements >= 1
        && cycle.style_xp
        && cycle.autocast_armed
        && cycle.mind_rune_consumed
        && cycle.air_runes_consumed
        && cycle.shield_worn
        && !cycle.shield_continuity_break
        && !cycle.noted
        && !cycle.wrong_item
}

/// Bank-only dart branch: script withdrawal/equip/travel plus one ranged
/// engagement. Never CombatCoreCycle::qualified() (2/1/further_work).
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatDartBranchCycle {
    pub combat: CombatCoreCycle,
    pub withdrawn: bool,
    pub worn: bool,
    pub arrived: bool,
    pub wrong_ammo: bool,
}

impl CombatDartBranchCycle {
    pub fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        self.combat.observe(spec, baseline, now);
        let bank_withdraw = baseline.bank_open
            && baseline.bank_loaded
            && now.bank_open
            && now.bank_loaded
            && baseline.bank_item_id(BRONZE_DART_ID) > 0
            && now.bank_item_id(BRONZE_DART_ID) < baseline.bank_item_id(BRONZE_DART_ID);
        self.withdrawn |= now.item_id(BRONZE_DART_ID) > 0 || bank_withdraw;
        self.worn |= now.equipment_id(BRONZE_DART_ID) >= 1;
        self.arrived |= near(now.tile, MOSS_GIANT_SAFESPOT, MOSS_GIANT_DART_FIELD_RADIUS);
        self.wrong_ammo |= held_id(now, RUNE_ARROW_ID) > 0 || now.bank_item_id(RUNE_ARROW_ID) > 0;
    }

    pub fn qualified(&self) -> bool {
        self.withdrawn
            && self.worn
            && self.arrived
            && !self.wrong_ammo
            && self.combat.engagements >= 1
            && self.combat.style_xp
            && self.combat.combat_mode
            && self.combat.projectile_fired
            && !self.combat.noted
            && !self.combat.wrong_item
    }
}

pub fn combat_loot_id(id: i32, loot: CombatLoot) -> bool {
    match loot {
        CombatLoot::HerbLawNature => {
            unidentified_herb_id(id) || id == NATURE_RUNE_ID || id == LAW_RUNE_ID
        }
        CombatLoot::BigBones => id == BIG_BONES_ID,
        CombatLoot::BigBonesOrLimpwurt => id == BIG_BONES_ID || id == LIMPWURT_ROOT_ID,
        CombatLoot::DragonBonesOrHide | CombatLoot::DragonBonesAndHide => {
            id == DRAGON_BONES_ID || id == GREEN_DRAGONHIDE_ID
        }
        CombatLoot::GuardDrop => GUARD_DROP_IDS.contains(&id),
        CombatLoot::Bones => id == BONES_ID,
        CombatLoot::SapphireOrCasket => id == UNCUT_SAPPHIRE_ID || id == CASKET_ID,
        CombatLoot::None => false,
    }
}

/// Bank/return cycle for the five bank cells: the reviewed combat witness plus
/// the declared bank trip the card itself has to execute — pack stock moves to
/// the booth's bank, the card's restock line (and selected exact count) is met
/// from that bank's own stock, the modal closes on a later bank session, the
/// trip returns to the card's tile, and work resumes there. A booth opened away
/// from the card's stand, a deposit the card never made, a return without
/// further work, or a bank that only ever opened (seed/readiness) cannot
/// qualify it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatBankCycle {
    /// The same reviewed combat witness the plain card core uses.
    pub combat: CombatCoreCycle,
    /// The observation that proved the trip's deposit: pack empty of the
    /// deposit class, bank's own stock above its pre-Start level.
    pub banked: Option<Observation>,
    /// Pack food count when the deposit was observed; the restock has to beat it.
    pub food_at_deposit: i32,
    /// The card's restock line was met from the bank's own stock.
    pub restocked: bool,
    /// The modal closed on a later bank session than the deposit.
    pub closed: bool,
    /// The trip returned to the card's own tile.
    pub returned: bool,
    /// Further work after that return.
    pub further: bool,
    /// Strength XP when the return landed; `further` must beat it.
    pub return_xp: Option<i32>,
    /// A loaded booth opened away from the card's declared stand.
    pub wrong_bank: bool,
    /// The optional via tile (Varrock land / barrel wash-up) was visited.
    pub via_seen: bool,
    /// Magic XP landed with the via tile when `via_magic` is set.
    pub via_magic: bool,
    /// A fresh post-Start player death line was observed.
    pub death_seen: bool,
}

impl CombatBankCycle {
    fn deposit_count(observation: &Observation, deposit: &[i32]) -> i32 {
        deposit.iter().map(|id| observation.item_id(*id)).sum()
    }

    fn bank_count(observation: &Observation, deposit: &[i32]) -> i32 {
        deposit.iter().map(|id| observation.bank_item_id(*id)).sum()
    }

    pub fn observe(
        &mut self,
        spec: CombatSpec,
        bank: CombatBankSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        self.combat.observe(spec, baseline, now);
        self.death_seen |= now.chat.iter().any(|line| {
            !baseline.chat.contains(line)
                && line.1.to_ascii_lowercase().contains("oh dear you are dead")
        });
        let open = now.bank_open && now.bank_loaded;
        let at_stand = near(now.tile, bank.stand, bank.stand_radius);
        if open && !at_stand {
            self.wrong_bank = true;
        }
        if let Some(via) = bank.via {
            if near(now.tile, via, bank.via_radius) {
                self.via_seen = true;
                if bank.via_magic {
                    self.via_magic |= now.skill_xp("magic") > baseline.skill_xp("magic");
                }
            }
        }
        let via_ready =
            bank.via.is_none() || (self.via_seen && (!bank.via_magic || self.via_magic));
        let deposited = if bank.deposit.is_empty() {
            true
        } else {
            Self::deposit_count(now, bank.deposit) == 0
                && Self::bank_count(now, bank.deposit) > Self::bank_count(baseline, bank.deposit)
        };
        if self.banked.is_none()
            && via_ready
            && open
            && at_stand
            // A fresh session: the pre-Start bank state cannot satisfy this.
            && now.bank_generation > baseline.bank_generation
            && deposited
        {
            self.food_at_deposit = bank.restock.map_or(0, |food| now.item_id(food));
            self.banked = Some(now.clone());
        }
        if let Some(banked) = &self.banked {
            if let Some(food) = bank.restock {
                self.restocked |= open
                    && now.bank_generation == banked.bank_generation
                    && now.item_id(food) > self.food_at_deposit
                    && now.bank_item_id(food) < banked.bank_item_id(food)
                    && bank
                        .restock_count
                        .is_none_or(|count| now.item_id(food) == count);
            }
            self.closed |=
                !now.bank_open && !now.bank_loaded && now.bank_generation > banked.bank_generation;
        }
        if self.closed {
            self.returned |= !now.bank_open && near(now.tile, bank.ret, bank.ret_radius);
            if self.returned {
                let xp = now.skill_xp("strength");
                let reference = *self.return_xp.get_or_insert(xp);
                self.further |= !now.bank_open && xp > reference;
            }
        }
    }

    pub fn qualified(&self, spec: CombatSpec, bank: CombatBankSpec) -> bool {
        let combat_ok = !bank.require_combat || self.combat.qualified(spec);
        combat_ok
            && (bank.via.is_none() || self.via_seen)
            && (!bank.via_magic || self.via_magic)
            && self.banked.is_some()
            && (bank.restock.is_none() || self.restocked)
            && self.closed
            && self.returned
            && self.further
            && !self.wrong_bank
            && (!bank.forbid_death || !self.death_seen)
    }
}

/// FireGiant `EnterDungeon` from the raft: Start is on the surface, the script
/// has to put the player at z>=9000, then Strength XP on a Fire giant. An
/// in-room core (already z>=9000 at Start) cannot qualify this cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatApproachCycle {
    pub entered: bool,
    pub style_xp: bool,
}

impl CombatApproachCycle {
    pub fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        let baseline_surface = baseline.tile.is_some_and(|tile| tile.1 < DUNGEON_MIN_Z);
        self.entered |= baseline_surface && now.tile.is_some_and(|tile| tile.1 >= DUNGEON_MIN_Z);
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            && spec.style == CombatStyleWitness::Strength;
    }

    pub fn qualified(&self) -> bool {
        self.entered && self.style_xp
    }
}

/// Superheat a trip, deposit bars except natures, restock ores, then smelt again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SuperheaterCycle {
    pub first_bars: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub produced_after_withdrawal: bool,
    pub wrong_product: bool,
    pub wrong_staff: bool,
}

pub struct SuperheaterSpec {
    pub bar: i32,
    pub primary: i32,
    pub secondary: i32,
    pub staff: i32,
    pub steel: bool,
}

impl SuperheaterCycle {
    pub fn observe(&mut self, spec: SuperheaterSpec, baseline: &Observation, now: &Observation) {
        let SuperheaterSpec {
            bar,
            primary,
            secondary,
            staff,
            steel,
        } = spec;
        let wrong_bars = [IRON_BAR_ID, BRONZE_BAR_ID, STEEL_BAR_ID, SILVER_BAR_ID]
            .into_iter()
            .filter(|id| *id != bar)
            .any(|id| now.item_id(id) > 0 || now.bank_item_id(id) > 0);
        self.wrong_product |= wrong_bars;
        if now.equipment_id(STAFF_OF_FIRE_ID) > 0 && staff != STAFF_OF_FIRE_ID {
            self.wrong_staff = true;
        }
        if now.equipment_id(FIRE_BATTLESTAFF_ID) > 0 && staff != FIRE_BATTLESTAFF_ID {
            self.wrong_staff = true;
        }
        self.first_bars |= now.item_id(bar) >= 1
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item_id(staff) == 0
            && now.equipment_id(staff) >= 1
            && now.skill_xp("magic") > baseline.skill_xp("magic")
            && now.skill_xp("smithing") > baseline.skill_xp("smithing")
            && baseline.item_id(bar) == 0
            && !wrong_bars;
        if self.first_bars
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(bar) == 0
            && now.bank_item_id(bar) >= 1
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item_id(primary) == 0
            && now.item_id(secondary) == 0
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(primary) >= 1
                && now.item_id(secondary) >= 1
                && now.bank_item_id(primary) < deposited.bank_item_id(primary)
                && now.bank_item_id(secondary) < deposited.bank_item_id(secondary)
                && now.item_id(NATURE_RUNE_ID) >= 1
                && if steel {
                    now.item_id(secondary) == 2 * now.item_id(primary)
                } else {
                    now.item_id(primary) == now.item_id(secondary)
                }
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.produced_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(bar) >= 1
                && now.item_id(primary) < withdrawn.item_id(primary)
                && now.item_id(secondary) < withdrawn.item_id(secondary)
                && now.item_id(NATURE_RUNE_ID) < withdrawn.item_id(NATURE_RUNE_ID)
                && now.skill_xp("magic") > withdrawn.skill_xp("magic")
                && now.skill_xp("smithing") > withdrawn.skill_xp("smithing")
                && now.equipment_id(staff) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.produced_after_withdrawal && !self.wrong_product && !self.wrong_staff
    }
}

/// Combat and exact feather loot, then a fresh deposit, return r6, further work.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChickenKillerBankCycle {
    pub combat_loot: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl ChickenKillerBankCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.combat_loot |= now.skill_xp("strength") > baseline.skill_xp("strength")
            && now.item_id(FEATHER_ID) >= 1
            && baseline.item_id(FEATHER_ID) == 0;
        if self.combat_loot
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(FEATHER_ID) == 0
            && now.bank_item_id(FEATHER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FALADOR_CHICKENS, 6);
        }
        if self.returned {
            if let Some(deposited) = &self.deposited {
                self.further |= now.skill_xp("strength") > deposited.skill_xp("strength")
                    || now.item_id(FEATHER_ID) >= 1;
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Fill at the fountain, deposit produced water vials, restock empties, fill again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VialFillerCycle {
    pub filled: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl VialFillerCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.filled.is_none()
            && now.item_id(VIAL_OF_WATER_ID) >= 1
            && baseline.item_id(VIAL_OF_WATER_ID) == 0
            && near(now.tile, FALADOR_FOUNTAIN, 4)
        {
            self.filled = Some(now.clone());
        }
        if self.filled.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(VIAL_OF_WATER_ID) == 0
            && now.bank_item_id(VIAL_OF_WATER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(EMPTY_VIAL_ID) >= 1
                && now.bank_item_id(EMPTY_VIAL_ID) < deposited.bank_item_id(EMPTY_VIAL_ID)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.withdrawn.is_some()
                && !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FALADOR_FOUNTAIN, 4);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(VIAL_OF_WATER_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Staged herb+water to unfinished, secondary to finished, deposit, restock, further product.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PotionMakerCycle {
    pub unfinished: Option<Observation>,
    pub finished: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub further: bool,
    pub wrong_product: bool,
    pub filter_violated: bool,
}

pub struct PotionMakerSpec {
    pub herb: i32,
    pub unf: i32,
    pub secondary: i32,
    pub finished: i32,
    pub wrong_unf: i32,
    pub wrong_finished: i32,
    pub named: bool,
}

impl PotionMakerCycle {
    pub fn observe(&mut self, spec: PotionMakerSpec, baseline: &Observation, now: &Observation) {
        let PotionMakerSpec {
            herb,
            unf,
            secondary,
            finished,
            wrong_unf,
            wrong_finished,
            named,
        } = spec;
        self.wrong_product |= now.item_id(wrong_unf) > 0
            || now.item_id(wrong_finished) > 0
            || now.bank_item_id(wrong_unf) > 0
            || now.bank_item_id(wrong_finished) > 0;
        if named
            && (now.item_id(GUAM_LEAF_ID) > 0
                || (now.bank_open
                    && now.bank_loaded
                    && now.bank_generation > baseline.bank_generation
                    && now.bank_item_id(GUAM_LEAF_ID) < 14))
        {
            self.filter_violated = true;
        }
        if self.unfinished.is_none()
            && now.item_id(unf) >= 1
            && now.item_id(herb) < 14
            && now.item_id(VIAL_OF_WATER_ID) < 14
            && now.item_id(finished) == 0
            && baseline.item_id(unf) == 0
            && baseline.item_id(finished) == 0
        {
            self.unfinished = Some(now.clone());
        }
        if let Some(unfinished) = &self.unfinished {
            if self.finished.is_none()
                && now.item_id(finished) >= 1
                && now.item_id(unf) < unfinished.item_id(unf)
                && now.item_id(secondary) < 14
                && now.skill_xp("herblore") > baseline.skill_xp("herblore")
                && !now.bank_open
            {
                self.finished = Some(now.clone());
            }
        }
        if self.finished.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(finished) == 0
            && now.bank_item_id(finished) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(herb) >= 1
                && now.item_id(VIAL_OF_WATER_ID) >= 1
                && now.bank_item_id(herb) < deposited.bank_item_id(herb)
                && now.bank_item_id(VIAL_OF_WATER_ID) < deposited.bank_item_id(VIAL_OF_WATER_ID)
                && (!named || now.bank_item_id(GUAM_LEAF_ID) == 14)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.further |= now.item_id(unf) >= 1
                && now.item_id(herb) < withdrawn.item_id(herb)
                && now.item_id(VIAL_OF_WATER_ID) < withdrawn.item_id(VIAL_OF_WATER_ID);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.finished.is_some() && !self.wrong_product && !self.filter_violated
    }
}

/// Tan at the Tanner widget, deposit produced leather, restock hides, tan again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TannerBotCycle {
    pub widget: Option<Observation>,
    pub tanned: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub shop: bool,
}

pub struct TannerBotSpec {
    pub product: i32,
    pub wrong_product: i32,
    pub tan_all: i32,
}

impl TannerBotCycle {
    pub fn observe(&mut self, spec: TannerBotSpec, baseline: &Observation, now: &Observation) {
        let TannerBotSpec {
            product,
            wrong_product,
            tan_all,
        } = spec;
        self.wrong_product |= now.item_id(wrong_product) > 0 || now.bank_item_id(wrong_product) > 0;
        self.shop |= now.main_modal == SHOPMAIN;
        if self.widget.is_none()
            && now.main_modal == TANNER_IF
            && now.has_widget(tan_all)
            && now.item_id(COW_HIDE_ID) >= 1
            && now.item_id(product) == 0
            && now.item_id(COINS_ID) >= 1
            && near(now.tile, TANNER_STAND, 4)
            && now.main_modal != SHOPMAIN
        {
            self.widget = Some(now.clone());
        }
        if let Some(widget) = &self.widget {
            if self.tanned.is_none()
                && near(now.tile, TANNER_STAND, 4)
                && now.item_id(product) >= 1
                && now.item_id(COW_HIDE_ID) == 0
                && now.item_id(COINS_ID) < widget.item_id(COINS_ID)
                && now.item_id(wrong_product) == 0
                && now.main_modal != SHOPMAIN
                && !near(now.tile, DOMMIK_STAND, 4)
            {
                self.tanned = Some(now.clone());
            }
        }
        if self.tanned.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(COW_HIDE_ID) >= 1
                && now.bank_item_id(COW_HIDE_ID) < deposited.bank_item_id(COW_HIDE_ID)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.withdrawn.is_some()
                && !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, TANNER_STAND, 4);
        }
        if self.returned {
            self.further |= !now.bank_open
                && now.item_id(product) >= 1
                && now.item_id(COW_HIDE_ID) == 0
                && now.item_id(wrong_product) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.shop && self.tanned.is_some()
    }
}

/// Withdraw unnoted essence, enter the selected altar, convert to the selected
/// rune with Runecraft XP, portal out, deposit, restock, and craft again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RuneCrafterCycle {
    pub withdrawn: Option<Observation>,
    pub entered: Option<Observation>,
    pub crafted: Option<Observation>,
    pub exited: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

pub struct RuneCrafterSpec {
    pub rune: i32,
    pub wrong_rune: i32,
    pub ruins: (i32, i32, i32),
    pub bank: (i32, i32, i32),
}

impl RuneCrafterCycle {
    pub fn observe(&mut self, spec: RuneCrafterSpec, baseline: &Observation, now: &Observation) {
        let RuneCrafterSpec {
            rune,
            wrong_rune,
            ruins,
            bank,
        } = spec;
        self.wrong_product |= now.item_id(wrong_rune) > 0 || now.bank_item_id(wrong_rune) > 0;
        self.noted |= now.item_id(NOTED_ESSENCE_ID) > 0;
        if self.withdrawn.is_none()
            && overworld(now.tile)
            && near(now.tile, bank, 8)
            && now.item_id(RUNE_ESSENCE_ID) >= 1
            && now.item_id(rune) == 0
            && now.item_id(NOTED_ESSENCE_ID) == 0
        {
            self.withdrawn = Some(now.clone());
        }
        if self.withdrawn.is_some()
            && self.entered.is_none()
            && in_temple(now.tile)
            && now.item_id(RUNE_ESSENCE_ID) >= 1
            && now.item_id(rune) == 0
        {
            self.entered = Some(now.clone());
        }
        if self.entered.is_some()
            && self.crafted.is_none()
            && in_temple(now.tile)
            && now.item_id(rune) >= 1
            && now.item_id(RUNE_ESSENCE_ID) == 0
            && now.skill_xp("runecraft") > baseline.skill_xp("runecraft")
            && now.item_id(wrong_rune) == 0
            && now.item_id(NOTED_ESSENCE_ID) == 0
        {
            self.crafted = Some(now.clone());
        }
        if self.crafted.is_some()
            && self.exited.is_none()
            && overworld(now.tile)
            && near(now.tile, ruins, 8)
            && now.item_id(rune) >= 1
        {
            self.exited = Some(now.clone());
        }
        if self.exited.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(rune) == 0
            && now.bank_item_id(rune) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(RUNE_ESSENCE_ID) >= 1
                && now.bank_item_id(RUNE_ESSENCE_ID) < deposited.bank_item_id(RUNE_ESSENCE_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && (near(now.tile, ruins, 8) || in_temple(now.tile));
        }
        if let Some(crafted) = &self.crafted {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(rune) >= 1
                && now.item_id(RUNE_ESSENCE_ID) == 0
                && now.item_id(wrong_rune) == 0
                && now.skill_xp("runecraft") > crafted.skill_xp("runecraft");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.entered.is_some()
            && self.crafted.is_some()
            && self.exited.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Start with exact 22-Knife ballast, steal six cake/bread/chocolate slices,
/// deposit both product and ballast in a fresh bank, return to STAND, steal
/// again. Chocolate cake 1897 and noted stall food fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyCakesCycle {
    pub stolen: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl ArdyCakesCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(CHOCOLATE_CAKE_ID) > 0 || now.bank_item_id(CHOCOLATE_CAKE_ID) > 0;
        self.noted |= noted_stall_food(now) > 0;
        if self.stolen.is_none()
            && stall_food(now) >= 1
            && stall_food(baseline) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
            && now.item_id(CHOCOLATE_CAKE_ID) == 0
            && noted_stall_food(now) == 0
        {
            self.stolen = Some(now.clone());
        }
        if self.stolen.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && stall_food(now) == 0
            && bank_stall_food(now) >= 1
            && now.item_id(KNIFE_ID) == 0
            && now.bank_item_id(KNIFE_ID)
                >= baseline.bank_item_id(KNIFE_ID) + ARDY_CAKES_BALLAST_KNIVES
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_CAKES_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && stall_food(now) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.stolen.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// `guardResponse=Fight`: stall steal then FightBack kill. Landing on the Flee
/// kite tile fails — that is the other branch.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyCakesFightCycle {
    pub stolen: Option<Observation>,
    pub killed: Option<Observation>,
    pub fled: bool,
    pub wrong_product: bool,
    pub noted: bool,
    pub engaged_guard: Option<usize>,
    pub style_xp: bool,
}

impl ArdyCakesFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(CHOCOLATE_CAKE_ID) > 0 || now.bank_item_id(CHOCOLATE_CAKE_ID) > 0;
        self.noted |= noted_stall_food(now) > 0;
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        if self.stolen.is_none()
            && stall_food(now) >= 1
            && stall_food(baseline) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
            && now.item_id(CHOCOLATE_CAKE_ID) == 0
            && noted_stall_food(now) == 0
        {
            self.stolen = Some(now.clone());
        }
        if self.stolen.is_some() && self.killed.is_none() {
            self.fled |= near(now.tile, ARDY_FLEE_TILE, 2);
        }
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != "Guard" {
                continue;
            }
            let selected = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            if selected {
                self.engaged_guard = Some(npc.index);
            }
            if self.stolen.is_some()
                && self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        // Despawn after engagement also counts once style XP landed.
        if self.stolen.is_some()
            && self.killed.is_none()
            && self.style_xp
            && self
                .engaged_guard
                .is_some_and(|index| !now.npc_facts.iter().any(|npc| npc.index == index))
        {
            self.killed = Some(now.clone());
        }
    }

    pub fn qualified(&self) -> bool {
        self.stolen.is_some()
            && self.killed.is_some()
            && self.style_xp
            && !self.fled
            && !self.wrong_product
            && !self.noted
    }
}

/// Pickpocket coins with Thieving XP, deposit coins at loot-count 1, return
/// to the market stand, pickpocket again. Stall food without coins cannot
/// qualify. Fight stays pending.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyThieverCycle {
    pub pickpocketed: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl ArdyThieverCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.pickpocketed.is_none()
            && now.item_id(COINS_ID) >= 1
            && baseline.item_id(COINS_ID) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
        {
            self.pickpocketed = Some(now.clone());
        }
        if self.pickpocketed.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(COINS_ID) == 0
            && now.bank_item_id(COINS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_THIEVER_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(COINS_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.pickpocketed.is_some() && self.deposited.is_some()
    }
}

/// `guardResponse=Fight` on ArdyThiever: a FightBack Guard kill plus the Flee
/// bank-cycle shape. The kill and the Flee-kite check count from Start, not
/// from the first coins: on 289 only a caught stall steal draws a Guard (a
/// failed pickpocket stuns but never starts combat), and ArdyThiever steals
/// from the stall only while its food is at `restockAtFood`, so the catch
/// comes from the opening restock. The deposit still needs both the coins
/// and the kill. The Flee kite tile fails this branch.
///
/// Coins count only as a pickpocket: a coin gain with a Thieving XP rise in
/// the same or the previous observation (the pickpocket script adds the
/// coins and the XP in one server tick). A dropped Guard's coins picked up by
/// `LootDrops` carry no Thieving XP, and neither does the stall steal that
/// raised it earlier.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyThieverFightCycle {
    pub pickpocketed: Option<Observation>,
    pub killed: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub fled: bool,
    pub engaged_guard: Option<usize>,
    pub style_xp: bool,
    /// Coins and Thieving XP at the previous observation, and whether the
    /// XP rose on it.
    #[serde(skip)]
    last: Option<(i32, i32, bool)>,
}

impl ArdyThieverFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        let (coins, thieving) = (now.item_id(COINS_ID), now.skill_xp("thieving"));
        let (last_coins, last_thieving, xp_rose_before) = self.last.unwrap_or((
            baseline.item_id(COINS_ID),
            baseline.skill_xp("thieving"),
            false,
        ));
        let xp_rose = thieving > last_thieving;
        let picked = coins > last_coins && (xp_rose || xp_rose_before);
        self.last = Some((coins, thieving, xp_rose));
        if self.pickpocketed.is_none() && picked && baseline.item_id(COINS_ID) == 0 {
            self.pickpocketed = Some(now.clone());
        }
        if self.killed.is_none() {
            self.fled |= near(now.tile, ARDY_FLEE_TILE, 2);
        }
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != "Guard" {
                continue;
            }
            let selected = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            if selected {
                self.engaged_guard = Some(npc.index);
            }
            if self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        if self.killed.is_none()
            && self.style_xp
            && self
                .engaged_guard
                .is_some_and(|index| !now.npc_facts.iter().any(|npc| npc.index == index))
        {
            self.killed = Some(now.clone());
        }
        if self.pickpocketed.is_some()
            && self.killed.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(COINS_ID) == 0
            && now.bank_item_id(COINS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_THIEVER_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && picked;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.pickpocketed.is_some()
            && self.killed.is_some()
            && self.deposited.is_some()
            && self.style_xp
            && !self.fled
    }
}

/// Chop magic logs 1513 with Woodcutting XP, deposit upstairs, return to
/// ground, chop again. Strung bows and noted logs fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeChopCycle {
    pub chopped: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl GnomeChopCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |= gnome_wrong_bows(now)
            || now.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
            || now.bank_item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
            || now.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) > 0
            || now.bank_item_id(UNSTRUNG_MAGIC_LONGBOW_ID) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.chopped = Some(now.clone());
        }
        if self.chopped.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(MAGIC_LOGS_ID) == 0
            && now.bank_item_id(MAGIC_LOGS_ID) >= 1
            && near(now.tile, GNOME_BANK_STAND, 8)
            && gnome_resource_tools(now)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30)
                && gnome_resource_tools(now);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1 && gnome_resource_tools(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.chopped.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Chop logs, consume them into exact unstrung 72 or 70 with Fletching XP,
/// deposit upstairs, return, chop again. The other unstrung id and strung
/// 861/859 fail. Missing knife cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeFletchCycle {
    pub chopped: Option<Observation>,
    pub fletched: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl GnomeFletchCycle {
    pub fn observe(&mut self, product: i32, other: i32, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            gnome_wrong_bows(now) || now.item_id(other) > 0 || now.bank_item_id(other) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && now.item_id(product) == 0
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.chopped = Some(now.clone());
        }
        if self.chopped.is_some()
            && self.fletched.is_none()
            && now.item_id(product) >= 1
            && now.item_id(MAGIC_LOGS_ID)
                < self
                    .chopped
                    .as_ref()
                    .map(|row| row.item_id(MAGIC_LOGS_ID))
                    .unwrap_or(0)
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
            && now.item_id(other) == 0
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.fletched = Some(now.clone());
        }
        if self.fletched.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && near(now.tile, GNOME_BANK_STAND, 8)
            && gnome_resource_tools(now)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30)
                && gnome_resource_tools(now);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1 && gnome_resource_tools(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.chopped.is_some()
            && self.fletched.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Mine coal 453 with Mining XP, deposit to the mine truck (pack empty at
/// the truck, bank closed, bank coal unchanged), then mine again. A Seers
/// bank deposit is not the truck proof.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CoalTrucksCycle {
    pub mined: Option<Observation>,
    pub trucked: Option<Observation>,
    pub further: bool,
    pub noted: bool,
    pub banked: bool,
    pub fixture_changed: bool,
}

impl CoalTrucksCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.noted |= now.item_id(NOTED_COAL_ID) > 0 || now.bank_item_id(NOTED_COAL_ID) > 0;
        self.banked |= now.bank_item_id(COAL_ID) > baseline.bank_item_id(COAL_ID);
        self.fixture_changed |=
            now.item_id(RUNE_PICKAXE_ID) != 1 || now.item_id(KNIFE_ID) != COAL_BALLAST_KNIVES;
        if self.mined.is_none()
            && now.item_id(COAL_ID) >= 1
            && baseline.item_id(COAL_ID) == 0
            && now.skill_xp("mining") > baseline.skill_xp("mining")
            && now.item_id(NOTED_COAL_ID) == 0
        {
            self.mined = Some(now.clone());
        }
        if self.mined.is_some()
            && self.trucked.is_none()
            && now.item_id(COAL_ID) == 0
            && !now.bank_open
            && now.bank_item_id(COAL_ID) == baseline.bank_item_id(COAL_ID)
            && near(now.tile, COAL_MINE_TRUCK_STAND, 4)
            && !near(now.tile, SEERS_BANK, 8)
        {
            self.trucked = Some(now.clone());
        }
        if self.trucked.is_some() {
            self.further |= !now.bank_open
                && now.item_id(COAL_ID) >= 1
                && now.skill_xp("mining") > baseline.skill_xp("mining");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.mined.is_some()
            && self.trucked.is_some()
            && !self.noted
            && !self.banked
            && !self.fixture_changed
    }
}

/// Produce exact unnoted output with relevant XP after Start, deposit into a
/// fresh loaded bank, restock the raw input, close, return to the station,
/// and produce again. Seeded/name-only/noted/wrong products fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StationProductionCycle {
    pub withdrawn: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

pub struct StationProductionSpec {
    pub product: i32,
    pub input: i32,
    pub extra_input: Option<i32>,
    pub wrong: i32,
    pub extra_wrong: Option<i32>,
    pub skill: &'static str,
    pub station: (i32, i32, i32),
    pub station_radius: i32,
    pub noted: bool,
}

impl StationProductionCycle {
    pub fn observe(
        &mut self,
        spec: StationProductionSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        let StationProductionSpec {
            product,
            input,
            extra_input,
            wrong,
            extra_wrong,
            skill,
            station,
            station_radius,
            noted,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0
            || now.bank_item_id(wrong) > 0
            || extra_wrong.is_some_and(|id| now.item_id(id) > 0 || now.bank_item_id(id) > 0);
        self.noted |= noted;
        if self.withdrawn.is_none()
            && now.item_id(input) >= 1
            && now.item_id(product) == 0
            && baseline.item_id(input) == 0
            && extra_input.is_none_or(|id| now.item_id(id) >= 1)
            && now.item_id(wrong) == 0
            && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
            && !noted
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            if self.produced.is_none()
                && now.item_id(product) >= 1
                && now.item_id(input) < withdrawn.item_id(input)
                && extra_input.is_none_or(|id| now.item_id(id) < withdrawn.item_id(id))
                && now.skill_xp(skill) > baseline.skill_xp(skill)
                && near(now.tile, station, station_radius)
                && now.item_id(wrong) == 0
                && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
                && !noted
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(input) >= 1
                && now.bank_item_id(input) < deposited.bank_item_id(input)
                && extra_input.is_none_or(|id| now.item_id(id) >= 1)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, station, station_radius);
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.item_id(wrong) == 0
                && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
                && now.skill_xp(skill) > produced.skill_xp(skill);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

pub fn station_production_spec(
    case: CoreCase,
    observation: &Observation,
) -> Option<StationProductionSpec> {
    match case {
        CoreCase::CookBot => Some(StationProductionSpec {
            product: SALMON_ID,
            input: RAW_SALMON_ID,
            extra_input: None,
            wrong: LOBSTER_ID,
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(observation, NOTED_RAW_SALMON_ID, NOTED_SALMON_ID)
                || cook_wrong_or_burnt(observation, LOBSTER_ID),
        }),
        CoreCase::CookBotLobster => Some(StationProductionSpec {
            product: LOBSTER_ID,
            input: RAW_LOBSTER_ID,
            extra_input: None,
            wrong: SALMON_ID,
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(observation, NOTED_RAW_LOBSTER_ID, NOTED_LOBSTER_ID)
                || cook_wrong_or_burnt(observation, SALMON_ID),
        }),
        CoreCase::SmelterBot => Some(StationProductionSpec {
            product: BRONZE_BAR_ID,
            input: COPPER_ORE_ID,
            extra_input: Some(TIN_ORE_ID),
            wrong: STEEL_BAR_ID,
            extra_wrong: Some(IRON_BAR_ID),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                NOTED_COPPER_ORE_ID,
                NOTED_TIN_ORE_ID,
                NOTED_BRONZE_BAR_ID,
            ),
        }),
        CoreCase::SmelterBotSteel => Some(StationProductionSpec {
            product: STEEL_BAR_ID,
            input: IRON_ORE_ID,
            extra_input: Some(COAL_ID),
            wrong: BRONZE_BAR_ID,
            extra_wrong: Some(IRON_BAR_ID),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                NOTED_IRON_ORE_ID,
                NOTED_COAL_ID,
                NOTED_STEEL_BAR_ID,
            ),
        }),
        CoreCase::FlaxSpinner => Some(StationProductionSpec {
            product: BOW_STRING_ID,
            input: FLAX_ID,
            extra_input: None,
            wrong: BALL_OF_WOOL_ID,
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        CoreCase::FlaxAioSpin => Some(StationProductionSpec {
            product: BOW_STRING_ID,
            input: FLAX_ID,
            extra_input: None,
            wrong: BALL_OF_WOOL_ID,
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        _ => None,
    }
}

/// Magic XP and destination tile, then native bank restock and a further
/// teleport. Walking or a queued if-button without XP/law spend fails.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AioTeleportCycle {
    pub teleported: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub closed: bool,
    pub further: bool,
}

pub struct AioTeleportSpec {
    pub landing: (i32, i32, i32),
    pub restock: (i32, i32, i32),
    pub staff: Option<i32>,
    pub air_from_pack: bool,
}

pub fn aio_teleport_spec(case: CoreCase) -> Option<AioTeleportSpec> {
    match case {
        CoreCase::AioTeleport => Some(AioTeleportSpec {
            landing: VARROCK_TELE_LAND,
            restock: VARROCK_EAST_BANK,
            staff: Some(STAFF_OF_AIR_ID),
            air_from_pack: false,
        }),
        CoreCase::AioTeleportFalador => Some(AioTeleportSpec {
            landing: FALADOR_TELE_LAND,
            restock: FALADOR_WEST_BANK,
            staff: Some(STAFF_OF_WATER_ID),
            air_from_pack: true,
        }),
        CoreCase::AioTeleportNoStaff => Some(AioTeleportSpec {
            landing: VARROCK_TELE_LAND,
            restock: VARROCK_EAST_BANK,
            staff: None,
            air_from_pack: true,
        }),
        _ => None,
    }
}

impl AioTeleportCycle {
    pub fn observe(&mut self, spec: AioTeleportSpec, baseline: &Observation, now: &Observation) {
        let AioTeleportSpec {
            landing,
            restock,
            staff,
            air_from_pack,
        } = spec;
        if self.teleported.is_none()
            && now.skill_xp("magic") > baseline.skill_xp("magic")
            && now.item_id(LAW_RUNE_ID) < baseline.item_id(LAW_RUNE_ID)
            && near(now.tile, landing, 8)
            && staff.is_none_or(|id| now.equipment_id(id) >= 1)
            && (!air_from_pack || now.item_id(AIR_RUNE_ID) < baseline.item_id(AIR_RUNE_ID))
        {
            self.teleported = Some(now.clone());
        }
        if self.teleported.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && near(now.tile, restock, 8)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(LAW_RUNE_ID) > deposited.item_id(LAW_RUNE_ID)
                && now.bank_item_id(LAW_RUNE_ID) < deposited.bank_item_id(LAW_RUNE_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.closed |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let Some(teleported) = &self.teleported {
            if let Some(restocked) = &self.restocked {
                self.further |= self.closed
                    && !now.bank_open
                    && now.skill_xp("magic") > teleported.skill_xp("magic")
                    && now.item_id(LAW_RUNE_ID) < restocked.item_id(LAW_RUNE_ID);
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.teleported.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
    }
}

/// Posted shop stock down, matching inv up, coins down, then deposit except
/// coins, retain or top up funding, and a second buy. Queued if-button without
/// stock movement fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ShopFunding {
    Retained {
        carried_coins: i32,
    },
    TopUp {
        carried_before: i32,
        carried_after: i32,
        bank_before: i32,
        bank_after: i32,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ShopBuyoutCycle {
    pub opened: Option<Observation>,
    pub bought: Option<Observation>,
    pub bought_id: Option<i32>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    /// Explicitly distinguishes already-retained coins from a bank withdrawal.
    pub funding: Option<ShopFunding>,
    pub returned: bool,
    pub reopened: Option<Observation>,
    pub further: bool,
    last_tick: Option<u32>,
    /// Peak carried product after the first purchase, until deposit. Deposit
    /// must bank this earned load, not the first 10/5/1 inventory delta.
    pub earned_quantity: i32,
    /// Product count on the first loaded bank session of this trip while the
    /// pack still held that product. None if that first loaded bank already
    /// had an empty pack: fail closed rather than invent a pre-deposit count.
    pub trip_bank_product: Option<i32>,
}

pub struct ShopBuyoutSpec {
    pub stand: (i32, i32, i32),
    pub stand_radius: i32,
    pub restock: (i32, i32, i32),
}

pub fn shop_buyout_spec(case: CoreCase) -> Option<ShopBuyoutSpec> {
    match case {
        CoreCase::ShopBuyout => Some(ShopBuyoutSpec {
            stand: AEMAD_STAND,
            stand_radius: 6,
            restock: ARDOUGNE_EAST_BANK,
        }),
        CoreCase::ShopBuyoutAubury => Some(ShopBuyoutSpec {
            stand: AUBURY_STAND,
            stand_radius: 6,
            restock: VARROCK_EAST_BANK,
        }),
        CoreCase::ShopBuyoutLowe => Some(ShopBuyoutSpec {
            stand: LOWE_STAND,
            stand_radius: 6,
            restock: VARROCK_EAST_BANK,
        }),
        CoreCase::ShopBuyoutHickton => Some(ShopBuyoutSpec {
            stand: HICKTON_STAND,
            stand_radius: 6,
            restock: CATHERBY_BANK,
        }),
        CoreCase::ShopBuyoutHarry => Some(ShopBuyoutSpec {
            stand: HARRY_STAND,
            stand_radius: 6,
            restock: CATHERBY_BANK,
        }),
        CoreCase::ShopBuyoutBetty => Some(ShopBuyoutSpec {
            stand: BETTY_STAND,
            stand_radius: 6,
            restock: FALADOR_WEST_BANK,
        }),
        CoreCase::ShopBuyoutGerrant => Some(ShopBuyoutSpec {
            stand: GERRANT_STAND,
            stand_radius: 6,
            restock: DRAYNOR_BANK,
        }),
        _ => None,
    }
}

fn shop_bought_id(
    before: &Observation,
    now: &Observation,
    stand: (i32, i32, i32),
    stand_radius: i32,
) -> Option<(i32, i32)> {
    if !now.shop_open
        || now.main_modal != SHOPMAIN
        || now.shop_stock.is_empty()
        || !near(now.tile, stand, stand_radius)
    {
        return None;
    }
    if now.item_id(COINS_ID) >= before.item_id(COINS_ID) {
        return None;
    }
    let mut found = None;
    for row in &before.shop_stock {
        if now.shop_item_id(row.id) < row.count && now.item_id(row.id) > before.item_id(row.id) {
            if row.id == EMPTY_VIAL_ID {
                return Some((
                    EMPTY_VIAL_ID,
                    now.item_id(EMPTY_VIAL_ID) - before.item_id(EMPTY_VIAL_ID),
                ));
            }
            if found.is_none() {
                found = Some((row.id, now.item_id(row.id) - before.item_id(row.id)));
            }
        }
    }
    found
}

/// First-purchase replay: equal shop stock, carried product, and coins.
fn shop_same_purchase(left: &Observation, right: &Observation, id: i32) -> bool {
    left.item_id(id) == right.item_id(id)
        && left.item_id(COINS_ID) == right.item_id(COINS_ID)
        && left.shop_stock == right.shop_stock
}

impl ShopBuyoutCycle {
    pub fn observe(&mut self, spec: ShopBuyoutSpec, baseline: &Observation, now: &Observation) {
        let ShopBuyoutSpec {
            stand,
            stand_radius,
            restock,
        } = spec;
        // Inv/shop/bank packets can publish later frames in the same game tick.
        // Reject only backwards ticks; identical no-delta replays must still
        // fail the stage predicates below rather than this cadence gate.
        if now.tick < self.last_tick.unwrap_or(baseline.tick) {
            return;
        }
        self.last_tick = Some(now.tick);
        if self.opened.is_none()
            && now.shop_open
            && now.main_modal == SHOPMAIN
            && near(now.tile, stand, stand_radius)
            && !now.shop_stock.is_empty()
        {
            self.opened = Some(now.clone());
        }
        if let Some(opened) = &self.opened {
            if self.bought.is_none() {
                if let Some((id, _)) = shop_bought_id(opened, now, stand, stand_radius) {
                    self.bought_id = Some(id);
                    self.earned_quantity = now.item_id(id);
                    self.bought = Some(now.clone());
                }
            }
        }
        if let Some(id) = self.bought_id {
            if self.deposited.is_none() {
                self.earned_quantity = self.earned_quantity.max(now.item_id(id));
            }
            // First loaded bank of this trip: latch the product count only
            // while the pack still holds the earned product.
            if self.trip_bank_product.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation > baseline.bank_generation
                && near(now.tile, restock, 8)
                && now.item_id(id) > 0
            {
                self.trip_bank_product = Some(now.bank_item_id(id));
            }
            if self.deposited.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation > baseline.bank_generation
                && near(now.tile, restock, 8)
                && now.item_id(id) == 0
                && self.earned_quantity >= 1
                && self
                    .trip_bank_product
                    .is_some_and(|before| now.bank_item_id(id) >= before + self.earned_quantity)
                && now.item_id(COINS_ID) >= 1
            {
                self.deposited = Some(now.clone());
            }
        }
        // Retained funding is recordable on the deposit observation when
        // carried and bank coins did not move, so a later open-bank tick is
        // not required. A real withdrawal in the same session overwrites as
        // TopUp so retained coins are not mislabeled as withdrawn.
        if let Some(deposited) = &self.deposited {
            if now.bank_open && now.bank_loaded && now.bank_generation == deposited.bank_generation
            {
                let carried_now = now.item_id(COINS_ID);
                let carried_then = deposited.item_id(COINS_ID);
                let bank_now = now.bank_item_id(COINS_ID);
                let bank_then = deposited.bank_item_id(COINS_ID);
                if carried_now > carried_then && bank_now < bank_then {
                    self.funding = Some(ShopFunding::TopUp {
                        carried_before: carried_then,
                        carried_after: carried_now,
                        bank_before: bank_then,
                        bank_after: bank_now,
                    });
                    self.restocked = Some(now.clone());
                } else if self.funding.is_none()
                    && carried_now == carried_then
                    && bank_now == bank_then
                    && carried_now >= 1
                {
                    self.funding = Some(ShopFunding::Retained {
                        carried_coins: carried_now,
                    });
                    self.restocked = Some(now.clone());
                }
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, stand, stand_radius);
        }
        if self.returned
            && self.reopened.is_none()
            && now.shop_open
            && now.main_modal == SHOPMAIN
            && near(now.tile, stand, stand_radius)
        {
            self.reopened = Some(now.clone());
        }
        if let Some(reopened) = &self.reopened {
            let stale = match (self.bought.as_ref(), self.bought_id) {
                (Some(bought), Some(id)) => shop_same_purchase(bought, now, id),
                _ => false,
            };
            self.further |= self.funding.is_some()
                && now.tick > reopened.tick
                && !stale
                && shop_bought_id(reopened, now, stand, stand_radius).is_some();
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.opened.is_some()
            && self.bought.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && self.funding.is_some()
    }
}

/// ClimbingBoots (frozen `ClimbingBoots.ts`) at Tenzing's hut. The buy is
/// only a purchase witness once the framed Tenzing projection and the sherpa
/// dialogue are in the observed window and carried boots rose while coins
/// fell by exactly 12 a pair. Walk and teleport are separate cells: the
/// teleport cell must show a real post-purchase return cast (magic XP, Law
/// spend, landing, still carrying the earned pack) and the walk cell must
/// not cast at all. Return, deposit, reopen and the further purchase stay
/// separate stages, so a failed full cycle keeps its purchase evidence
/// instead of silently downgrading to a smoke PASS.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClimbingBootsCycle {
    pub tenzing: Option<Observation>,
    pub cast: Option<Observation>,
    pub bought: Option<Observation>,
    /// Pairs gained at the first `bought` sample (each pair is exactly 12 coins).
    pub bought_pairs: i32,
    /// Peak carried boots after that purchase, until deposit. Deposit must
    /// bank this earned quantity, not the early 1-pair sample.
    pub earned_boots: i32,
    /// First loaded post-Start bank: `Some(true)` only when boots 3105 were 0.
    pub boot_bank_empty: Option<bool>,
    /// The sherpa sell/buy line seen in a bounded chat projection.
    pub dialogue: bool,
    pub returned: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub departed: Option<Observation>,
    pub further: bool,
}

pub struct ClimbingBootsSpec {
    /// Explicit typed fixture option, not the frozen default.
    pub use_teleport: bool,
    /// Exact `readyToBuy` trip money: 12 coins a pair for the whole pack.
    pub coins: i32,
    /// Falador landing the teleport branch must reach after a real cast.
    pub landing: (i32, i32, i32),
    /// Falador West bank stand the walk returns through.
    pub bank: (i32, i32, i32),
}

pub fn climbing_boots_spec(case: CoreCase) -> Option<ClimbingBootsSpec> {
    match case {
        // Walking cell: 28 boots * 12 coins, no runes, no Magic requirement.
        CoreCase::ClimbingBoots => Some(ClimbingBootsSpec {
            use_teleport: false,
            coins: CLIMBING_BOOTS_WALK_PACK_COINS,
            landing: FALADOR_TELE_LAND,
            bank: FALADOR_WEST_BANK,
        }),
        // Teleport cell: the `tripQty` is 28 minus the carried rune stacks
        // (Law, Air, Water), so the ready-to-buy money is 25 pairs.
        CoreCase::ClimbingBootsTeleport => Some(ClimbingBootsSpec {
            use_teleport: true,
            coins: CLIMBING_BOOTS_TELE_PACK_COINS,
            landing: FALADOR_TELE_LAND,
            bank: FALADOR_WEST_BANK,
        }),
        _ => None,
    }
}

/// The real framed Tenzing NPC near the hut inside tile. Route coordinates
/// and quest state never stand in for this projection.
fn tenzing_visible(now: &Observation) -> bool {
    now.npc_facts.iter().any(|npc| {
        npc.name.as_deref() == Some(TENZING_NAME) && near(Some(npc.tile), TENZING_INSIDE, 12)
    })
}

/// The sherpa sale line or the purchase confirmation, from the bounded chat
/// projection (`death_sherpa.rs2`: "for 12 gold" then "Tenzing has given you
/// some Climbing boots.").
fn climbing_boots_dialogue(now: &Observation) -> bool {
    now.chat
        .iter()
        .any(|(_, text)| text.contains("Climbing boots"))
}

impl ClimbingBootsCycle {
    pub fn observe(&mut self, spec: ClimbingBootsSpec, baseline: &Observation, now: &Observation) {
        let ClimbingBootsSpec {
            use_teleport,
            coins: _,
            landing,
            bank,
        } = spec;
        self.dialogue |= climbing_boots_dialogue(now);
        if self.tenzing.is_none() && tenzing_visible(now) {
            self.tenzing = Some(now.clone());
        }
        // The purchase: same player after Start, boots up, exactly 12 coins a
        // pair gone, with the framed Tenzing/dialogue evidence. The first
        // matching frame is typically one pair; earned_boots tracks the peak
        // pack after that until deposit.
        if self.tenzing.is_some() && self.dialogue {
            if self.bought.is_none() {
                let gained = now.item_id(CLIMBING_BOOTS_ID) - baseline.item_id(CLIMBING_BOOTS_ID);
                let spent = baseline.item_id(COINS_ID) - now.item_id(COINS_ID);
                if gained >= 1 && spent == gained * CLIMBING_BOOTS_PAIR_COINS {
                    self.bought_pairs = gained;
                    self.earned_boots = gained;
                    self.bought = Some(now.clone());
                }
            } else if self.deposited.is_none() {
                self.earned_boots = self.earned_boots.max(now.item_id(CLIMBING_BOOTS_ID));
            }
        }
        // A real cast, not the option: magic XP plus the whole Falador cost
        // (Law, Air and Water, no staff assumed) and the landing. Only after
        // the purchase, on a frame that still carries the earned pack, so a
        // pre-purchase Falador-shaped cast cannot satisfy the teleport cell.
        // Compare with the purchase snapshot: Start-relative deltas could
        // count an earlier cast when the earned pack later walks through here.
        // Recorded for either cell, so a walking cell that cast fails closed.
        if self.cast.is_none()
            && self.earned_boots >= 1
            && now.item_id(CLIMBING_BOOTS_ID) >= self.earned_boots
            && self.bought.as_ref().is_some_and(|bought| {
                now.skill_xp("magic") > bought.skill_xp("magic")
                    && now.item_id(LAW_RUNE_ID) < bought.item_id(LAW_RUNE_ID)
                    && now.item_id(AIR_RUNE_ID) < bought.item_id(AIR_RUNE_ID)
                    && now.item_id(WATER_RUNE_ID) < bought.item_id(WATER_RUNE_ID)
            })
            && near(now.tile, landing, 8)
        {
            self.cast = Some(now.clone());
        }
        // Return is genuine position after the purchase (the first arrival can
        // still be inside the baseline bank generation), never a generation
        // counter that would demand a second bank trip. Teleport return must
        // follow the post-purchase cast, still carrying the earned pack.
        if self.bought.is_some()
            && self.returned.is_none()
            && !now.bank_open
            && near(now.tile, bank, 8)
            && self.earned_boots >= 1
            && now.item_id(CLIMBING_BOOTS_ID) >= self.earned_boots
            && (!use_teleport || self.cast.is_some())
        {
            self.returned = Some(now.clone());
        }
        if now.bank_open && now.bank_loaded && self.boot_bank_empty.is_none() {
            self.boot_bank_empty = Some(now.bank_item_id(CLIMBING_BOOTS_ID) == 0);
        }
        // Deposit the earned peak against a verified empty boot bank, not the
        // first 1-pair bought sample and not leftover bank stock.
        if self.returned.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && self.boot_bank_empty == Some(true)
            && now.bank_generation > baseline.bank_generation
            && self.earned_boots >= 1
        {
            let carried_lost = self.earned_boots - now.item_id(CLIMBING_BOOTS_ID);
            let bank_gained = now.bank_item_id(CLIMBING_BOOTS_ID);
            if carried_lost >= self.earned_boots && bank_gained >= self.earned_boots {
                self.deposited = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(COINS_ID) > deposited.item_id(COINS_ID)
                && now.bank_item_id(COINS_ID) < deposited.bank_item_id(COINS_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        // Departure: a later bank session closed with the restocked pack.
        if let Some(restocked) = &self.restocked {
            if self.departed.is_none()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > restocked.bank_generation
                && now.item_id(COINS_ID) >= CLIMBING_BOOTS_PAIR_COINS
            {
                self.departed = Some(now.clone());
            }
        }
        // A further pair bought back at Tenzing's hut after the departure.
        if let Some(departed) = &self.departed {
            let gained = now.item_id(CLIMBING_BOOTS_ID) - departed.item_id(CLIMBING_BOOTS_ID);
            let spent = departed.item_id(COINS_ID) - now.item_id(COINS_ID);
            self.further |= gained >= 1
                && spent == gained * CLIMBING_BOOTS_PAIR_COINS
                && near(now.tile, TENZING_HUT_DOOR, 12);
        }
    }

    /// The honest intermediate state: a real purchase with its Tenzing and
    /// dialogue evidence but no claim of return, deposit or further work.
    pub fn purchase_smoke(&self) -> bool {
        self.tenzing.is_some() && self.dialogue && self.bought.is_some()
    }

    pub fn qualified(&self, spec: ClimbingBootsSpec) -> bool {
        let branch = if spec.use_teleport {
            self.cast.is_some()
        } else {
            // A walking cell must not silently run the teleport branch.
            self.cast.is_none()
        };
        branch
            && self.purchase_smoke()
            && self.returned.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && self.departed.is_some()
            && self.further
    }
}

/// Anvil main-panel production (not chat make), then deposit except hammer,
/// restock bars, and further smithing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SmithingBotCycle {
    pub panel: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
}

pub struct SmithingBotSpec {
    pub product: i32,
    pub wrong: i32,
    pub bars_per: i32,
}

pub fn smithing_bot_spec(case: CoreCase) -> Option<SmithingBotSpec> {
    match case {
        CoreCase::SmithingBot => Some(SmithingBotSpec {
            product: BRONZE_DAGGER_ID,
            wrong: BRONZE_PLATEBODY_ID,
            bars_per: 1,
        }),
        CoreCase::SmithingBotPlatebody => Some(SmithingBotSpec {
            product: BRONZE_PLATEBODY_ID,
            wrong: BRONZE_DAGGER_ID,
            bars_per: 5,
        }),
        _ => None,
    }
}

impl SmithingBotCycle {
    pub fn observe(&mut self, spec: SmithingBotSpec, baseline: &Observation, now: &Observation) {
        let SmithingBotSpec {
            product,
            wrong,
            bars_per,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if self.panel.is_none()
            && now.has_main_make(product)
            && near(now.tile, VARROCK_ANVIL, 8)
            && now.item_id(BRONZE_BAR_ID) >= bars_per
            && now.item_id(product) == 0
            && now.item_id(HAMMER_ID) >= 1
        {
            self.panel = Some(now.clone());
        }
        if let Some(panel) = &self.panel {
            if self.produced.is_none()
                && near(now.tile, VARROCK_ANVIL, 8)
                && now.item_id(product) >= 1
                && now.item_id(BRONZE_BAR_ID) <= panel.item_id(BRONZE_BAR_ID) - bars_per
                && now.skill_xp("smithing") > baseline.skill_xp("smithing")
                && now.item_id(wrong) == 0
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && now.item_id(HAMMER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(BRONZE_BAR_ID) >= bars_per
                && now.bank_item_id(BRONZE_BAR_ID) < deposited.bank_item_id(BRONZE_BAR_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, VARROCK_ANVIL, 8);
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.skill_xp("smithing") > produced.skill_xp("smithing")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.panel.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
    }
}

/// Soft-leather gloves wait the selected leather interface; hard-leather
/// body is the no-modal `flow=single` burst.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LeatherCrafterCycle {
    pub interface: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub used_interface: bool,
    pub wrong_product: bool,
}

pub struct LeatherCrafterSpec {
    pub product: i32,
    pub input: i32,
    pub wrong: i32,
    pub require_interface: bool,
}

pub fn leather_crafter_spec(case: CoreCase) -> Option<LeatherCrafterSpec> {
    match case {
        CoreCase::LeatherCrafter => Some(LeatherCrafterSpec {
            product: LEATHER_GLOVES_ID,
            input: SOFT_LEATHER_ID,
            wrong: HARDLEATHER_BODY_ID,
            require_interface: true,
        }),
        CoreCase::LeatherCrafterHardBody => Some(LeatherCrafterSpec {
            product: HARDLEATHER_BODY_ID,
            input: HARD_LEATHER_ID,
            wrong: LEATHER_GLOVES_ID,
            require_interface: false,
        }),
        _ => None,
    }
}

impl LeatherCrafterCycle {
    pub fn observe(&mut self, spec: LeatherCrafterSpec, baseline: &Observation, now: &Observation) {
        let LeatherCrafterSpec {
            product,
            input,
            wrong,
            require_interface,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if now.main_modal == LEATHER_IF && now.has_widget(LEATHER_GLOVES_MAKE10) {
            self.used_interface = true;
            if self.interface.is_none()
                && now.item_id(input) >= 1
                && now.item_id(product) == 0
                && now.item_id(NEEDLE_ID) >= 1
            {
                self.interface = Some(now.clone());
            }
        }
        if self.withdrawn.is_none()
            && now.item_id(input) >= 1
            && now.item_id(product) == 0
            && now.item_id(NEEDLE_ID) >= 1
            && baseline.item_id(input) == 0
        {
            self.withdrawn = Some(now.clone());
        }
        let prior = if require_interface {
            self.interface.as_ref()
        } else {
            self.withdrawn.as_ref().filter(|_| !self.used_interface)
        };
        if let Some(prior) = prior {
            if self.produced.is_none()
                && now.item_id(product) >= 1
                && now.item_id(input) < prior.item_id(input)
                && now.skill_xp("crafting") > baseline.skill_xp("crafting")
                && now.item_id(wrong) == 0
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && now.item_id(NEEDLE_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(input) >= 1
                && now.bank_item_id(input) < deposited.bank_item_id(input)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.skill_xp("crafting") > produced.skill_xp("crafting")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
    }
}

/// Firemaking XP plus a Fire loc inside the posted Varrock East AABB, then
/// deposit except tinderbox, restock logs, and another light.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FiremakerCycle {
    pub withdrawn: Option<Observation>,
    pub lit: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_log: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FiremakerSpec {
    pub log: i32,
    pub wrong: i32,
}

pub fn firemaker_spec(case: CoreCase) -> Option<FiremakerSpec> {
    match case {
        CoreCase::Firemaker => Some(FiremakerSpec {
            log: LOGS_ID,
            wrong: OAK_LOGS_ID,
        }),
        CoreCase::FiremakerOak => Some(FiremakerSpec {
            log: OAK_LOGS_ID,
            wrong: LOGS_ID,
        }),
        _ => None,
    }
}

impl FiremakerCycle {
    pub fn observe(&mut self, spec: FiremakerSpec, baseline: &Observation, now: &Observation) {
        let FiremakerSpec { log, wrong } = spec;
        self.wrong_log |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if self.withdrawn.is_none()
            && now.item_id(log) >= 1
            && now.item_id(TINDERBOX_ID) >= 1
            && baseline.item_id(log) == 0
            && !fire_in_varrock_east_plot(now)
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            if self.lit.is_none()
                && now.skill_xp("firemaking") > baseline.skill_xp("firemaking")
                && now.item_id(log) < withdrawn.item_id(log)
                && fire_in_varrock_east_plot(now)
            {
                self.lit = Some(now.clone());
            }
        }
        if self.lit.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(log) == 0
            && now.item_id(TINDERBOX_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(log) >= 1
                && now.bank_item_id(log) < deposited.bank_item_id(log)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let (Some(_lit), Some(restocked)) = (&self.lit, &self.restocked) {
            self.further |= self.returned
                && !now.bank_open
                && now.tick > restocked.tick
                && now.item_id(log) < restocked.item_id(log)
                && now.skill_xp("firemaking") > restocked.skill_xp("firemaking")
                && fire_in_varrock_east_plot(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.lit.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_log
    }
}

impl CoreWitness {
    pub fn new(case: CoreCase, baseline: Observation) -> Self {
        Self::new_with_start_preparation(case, baseline, None)
    }

    pub fn new_with_start_preparation(
        case: CoreCase,
        baseline: Observation,
        start_preparation: Option<HerbCleanerStartPreparationReceipt>,
    ) -> Self {
        Self {
            max_items: baseline.items.clone(),
            max_xp: baseline.xp.clone(),
            latest: baseline.clone(),
            baseline,
            start_preparation,
            case,
            saw_bury_chat: false,
            post_start_observations: 0,
            bone_bank_cycle: BoneBankCycle::default(),
            bank_fletcher_cycle: BankFletcherCycle::default(),
            bank_fletcher_option_cycle: BankFletcherOptionCycle::default(),
            bank_fletcher_string_cycle: BankFletcherStringCycle::default(),
            bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle::default(),
            alcher_defaults_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_generated_custom_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_spell_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_swarm_cycle: AlcherSwarmDrainCycle::default(),
            dart_fletcher_cycle: DartFletcherCycle::default(),
            herb_cleaner_cycle: HerbCleanerCycle::default(),
            herb_cleaner_empty_cycle: HerbCleanerEmptyCycle::default(),
            gem_cutter_cycle: GemCutterCycle::default(),
            door_opener_cycle: DoorOpenerCycle::default(),
            gnome_course_cycle: GnomeCourseCycle::default(),
            wildy_agility_cycle: WildyAgilityCycle::default(),
            brimhaven_agility_cycle: BrimhavenAgilityCycle::default(),
            flax_picker_cycle: FlaxPickerCycle::default(),
            superheater_cycle: SuperheaterCycle::default(),
            chicken_killer_bank_cycle: ChickenKillerBankCycle::default(),
            vial_filler_cycle: VialFillerCycle::default(),
            potion_maker_cycle: PotionMakerCycle::default(),
            tanner_bot_cycle: TannerBotCycle::default(),
            rune_crafter_cycle: RuneCrafterCycle::default(),
            ardy_cakes_cycle: ArdyCakesCycle::default(),
            ardy_cakes_fight_cycle: ArdyCakesFightCycle::default(),
            ardy_thiever_cycle: ArdyThieverCycle::default(),
            ardy_thiever_fight_cycle: ArdyThieverFightCycle::default(),
            gnome_chop_cycle: GnomeChopCycle::default(),
            gnome_fletch_cycle: GnomeFletchCycle::default(),
            coal_trucks_cycle: CoalTrucksCycle::default(),
            station_production_cycle: StationProductionCycle::default(),
            flax_aio_cycle: FlaxAioCycle::default(),
            flax_aio_pick_cycle: FlaxAioPickCycle::default(),
            herblore_eggs_cycle: HerbloreEggsCycle::default(),
            herblore_newt_cycle: HerbloreNewtCycle::default(),
            combat_core_cycle: CombatCoreCycle::default(),
            combat_dart_branch_cycle: CombatDartBranchCycle::default(),
            combat_bank_cycle: CombatBankCycle::default(),
            combat_approach_cycle: CombatApproachCycle::default(),
            aio_teleport_cycle: AioTeleportCycle::default(),
            shop_buyout_cycle: ShopBuyoutCycle::default(),
            smithing_bot_cycle: SmithingBotCycle::default(),
            leather_crafter_cycle: LeatherCrafterCycle::default(),
            firemaker_cycle: FiremakerCycle::default(),
            climbing_boots_cycle: ClimbingBootsCycle::default(),
            ranging_guild_round_cycle: RangingGuildRoundCycle::default(),
            ranging_guild_redeem_cycle: RangingGuildRedeemCycle::default(),
            ranging_guild_bank_cycle: RangingGuildBankCycle::default(),
            ranging_guild_full_cycle: RangingGuildFullCycle::default(),
            brimhaven_moss_inspect_cycle: BrimhavenMossInspectCycle::default(),
            route_inspect_brimhaven_v2_cycle: RouteInspectBrimhavenV2Cycle::default(),
            prayer_delivery_cycle: PrayerDeliveryCycle::default(),
            line_of_sight_cycle: LineOfSightDeliveryCycle::default(),
            actor_observation_cycle: ActorObservationDeliveryCycle::default(),
            fight_field_cycle: FightFieldDeliveryCycle::default(),
            hunt_cycle: HuntDeliveryCycle::default(),
            ordered_first_exhausted: false,
        }
    }

    pub fn observe(&mut self, observation: &Observation) {
        if !observation.ingame
            || observation.scene_state != 2
            || observation.player != self.baseline.player
        {
            return;
        }
        if matches!(self.case, CoreCase::BoneBurier) {
            self.bone_bank_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcher) {
            self.bank_fletcher_cycle
                .observe(&self.baseline, observation);
        }
        if let Some(spec) = bank_fletcher_option_spec(self.case) {
            self.bank_fletcher_option_cycle
                .observe(spec, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcherString) {
            self.bank_fletcher_string_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcherCutString) {
            self.bank_fletcher_cut_string_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(
            self.case,
            CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName
        ) {
            self.alcher_generated_custom_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::AlcherDefaults) {
            self.alcher_defaults_cycle.observe_target(
                &self.baseline,
                observation,
                YEW_LONGBOW_ID,
                CERT_YEW_LONGBOW_ID,
                YEW_LONGBOW_ALCH_COINS,
            );
        }
        if matches!(
            self.case,
            CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff
        ) {
            let expectation = match self.case {
                CoreCase::AlcherLow => ALCHER_LOW_EXPECTATION,
                _ => ALCHER_FIRE_BATTLESTAFF_EXPECTATION,
            };
            self.alcher_spell_cycle.observe_spelled(
                expectation,
                &self.baseline,
                observation,
                RUNE_CHAINBODY_ID,
                CERT_RUNE_CHAINBODY_ID,
            );
        }
        if matches!(self.case, CoreCase::AlcherSwarmDrain) {
            self.alcher_swarm_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::DartFletcher) {
            self.dart_fletcher_cycle.observe(
                BRONZE_DART_TIP_ID,
                FEATHER_ID,
                BRONZE_DART_ID,
                IRON_DART_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DartFletcherIron) {
            self.dart_fletcher_cycle.observe(
                IRON_DART_TIP_ID,
                FEATHER_ID,
                IRON_DART_ID,
                BRONZE_DART_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(
            self.case,
            CoreCase::HerbCleaner | CoreCase::HerbCleanerNamed
        ) {
            self.herb_cleaner_cycle.observe(
                matches!(self.case, CoreCase::HerbCleanerNamed),
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::HerbCleanerEmptyBank) {
            self.herb_cleaner_empty_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GemCutter | CoreCase::GemCutterNamed) {
            self.gem_cutter_cycle.observe(
                matches!(self.case, CoreCase::GemCutterNamed),
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DoorOpener) {
            self.door_opener_cycle.observe(
                false,
                LUMBRIDGE_DOOR,
                WOODEN_DOOR_CLOSED_ID,
                WOODEN_DOOR_OPEN_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DoorOpenerGate) {
            self.door_opener_cycle.observe(
                true,
                LUMBRIDGE_GATE,
                WOODEN_GATE_CLOSED_ID,
                WOODEN_GATE_OPEN_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(
            self.case,
            CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius
        ) {
            self.gnome_course_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::WildyAgility) {
            self.wildy_agility_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BrimhavenAgility) {
            self.brimhaven_agility_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxPicker) {
            self.flax_picker_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxAio) {
            self.flax_aio_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxAioPick) {
            self.flax_aio_pick_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::HerbloreSecondaries) {
            self.herblore_eggs_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::HerbloreSecondariesNewt) {
            self.herblore_newt_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::MossGiantDart) {
            if let Some(spec) = combat_spec(self.case) {
                self.combat_dart_branch_cycle
                    .observe(spec, &self.baseline, observation);
            }
        } else if let Some(spec) = combat_spec(self.case) {
            self.combat_core_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let (Some(spec), Some(bank)) = (combat_spec(self.case), combat_bank_spec(self.case)) {
            self.combat_bank_cycle
                .observe(spec, bank, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FireGiantApproach) {
            if let Some(spec) = combat_spec(self.case) {
                self.combat_approach_cycle
                    .observe(spec, &self.baseline, observation);
            }
        }
        if let Some(spec) = aio_teleport_spec(self.case) {
            self.aio_teleport_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = shop_buyout_spec(self.case) {
            self.shop_buyout_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = smithing_bot_spec(self.case) {
            self.smithing_bot_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = leather_crafter_spec(self.case) {
            self.leather_crafter_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = firemaker_spec(self.case) {
            self.firemaker_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = climbing_boots_spec(self.case) {
            self.climbing_boots_cycle
                .observe(spec, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildRound) {
            self.ranging_guild_round_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildRedeem) {
            self.ranging_guild_redeem_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildBank) {
            self.ranging_guild_bank_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildFull) {
            self.ranging_guild_full_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BrimhavenMossInspectV1) {
            self.brimhaven_moss_inspect_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RouteInspectBrimhavenV2) {
            self.route_inspect_brimhaven_v2_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::PrayerV2 | CoreCase::PrayerV1) {
            self.prayer_delivery_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::LineOfSightV2) {
            self.line_of_sight_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::ActorObservationV2) {
            self.actor_observation_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::FightFieldV2) {
            self.fight_field_cycle.observe(observation);
        }
        if let Some(cell) = self.case.hunt_cell() {
            self.hunt_cycle.observe(cell, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::Superheater) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: BRONZE_BAR_ID,
                    primary: COPPER_ORE_ID,
                    secondary: TIN_ORE_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterSteel) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: STEEL_BAR_ID,
                    primary: IRON_ORE_ID,
                    secondary: COAL_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: true,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterFireBattlestaff) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: BRONZE_BAR_ID,
                    primary: COPPER_ORE_ID,
                    secondary: TIN_ORE_ID,
                    staff: FIRE_BATTLESTAFF_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterSilverLowNatures) {
            // Single-ore: primary == secondary so the pair-ratio check is a no-op.
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: SILVER_BAR_ID,
                    primary: SILVER_ORE_ID,
                    secondary: SILVER_ORE_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::ChickenKillerBank) {
            self.chicken_killer_bank_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::VialFiller | CoreCase::VialFillerEast) {
            self.vial_filler_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::PotionMaker) {
            self.potion_maker_cycle.observe(
                PotionMakerSpec {
                    herb: GUAM_LEAF_ID,
                    unf: GUAM_UNF_ID,
                    secondary: EYE_OF_NEWT_ID,
                    finished: ATTACK_POTION_3_ID,
                    wrong_unf: RANARR_UNF_ID,
                    wrong_finished: PRAYER_POTION_3_ID,
                    named: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::PotionMakerNamed) {
            self.potion_maker_cycle.observe(
                PotionMakerSpec {
                    herb: RANARR_WEED_ID,
                    unf: RANARR_UNF_ID,
                    secondary: SNAPE_GRASS_ID,
                    finished: PRAYER_POTION_3_ID,
                    wrong_unf: GUAM_UNF_ID,
                    wrong_finished: ATTACK_POTION_3_ID,
                    named: true,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::TannerBot) {
            self.tanner_bot_cycle.observe(
                TannerBotSpec {
                    product: SOFT_LEATHER_ID,
                    wrong_product: HARD_LEATHER_ID,
                    tan_all: SOFT_TAN_ALL_COM,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::TannerBotHard) {
            self.tanner_bot_cycle.observe(
                TannerBotSpec {
                    product: HARD_LEATHER_ID,
                    wrong_product: SOFT_LEATHER_ID,
                    tan_all: HARD_TAN_ALL_COM,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::RuneCrafter) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: AIR_RUNE_ID,
                    wrong_rune: EARTH_RUNE_ID,
                    ruins: RUNECRAFTER_AIR_RUINS,
                    bank: FALADOR_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::RuneCrafterEarth) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: EARTH_RUNE_ID,
                    wrong_rune: AIR_RUNE_ID,
                    ruins: RUNECRAFTER_EARTH_RUINS,
                    bank: VARROCK_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::MuleCrafter) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: AIR_RUNE_ID,
                    wrong_rune: EARTH_RUNE_ID,
                    ruins: MULECRAFTER_AIR_RUINS,
                    bank: FALADOR_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::ArdyCakes) {
            self.ardy_cakes_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::ArdyCakesFight) {
            self.ardy_cakes_fight_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(
            self.case,
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight
        ) {
            self.ardy_thiever_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::ArdyThieverFight) {
            self.ardy_thiever_fight_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GnomeChop) {
            self.gnome_chop_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GnomeFletchShort) {
            self.gnome_fletch_cycle.observe(
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                UNSTRUNG_MAGIC_LONGBOW_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::GnomeFletchLong) {
            self.gnome_fletch_cycle.observe(
                UNSTRUNG_MAGIC_LONGBOW_ID,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::CoalTrucks) {
            self.coal_trucks_cycle.observe(&self.baseline, observation);
        }
        if let Some(spec) = station_production_spec(self.case, observation) {
            self.station_production_cycle
                .observe(spec, &self.baseline, observation);
        }
        let baseline_sequence = self
            .baseline
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(i32::MIN);
        self.saw_bury_chat |= observation.chat.iter().any(|(sequence, text)| {
            *sequence > baseline_sequence && text.to_ascii_lowercase().contains("bury the bones")
        });
        for (name, count) in &observation.items {
            let peak = self.max_items.entry(name.clone()).or_insert(*count);
            *peak = (*peak).max(*count);
        }
        for (name, xp) in &observation.xp {
            let peak = self.max_xp.entry(name.clone()).or_insert(*xp);
            *peak = (*peak).max(*xp);
        }
        self.ordered_first_exhausted |= self.max_items.get("Rune platebody").copied().unwrap_or(0)
            > 0
            && observation.item("Rune platebody") == 0
            && observation.item("Rune chainbody") > 0;
        self.latest = observation.clone();
        if let Some(receipt) = observation.script_lifecycle.clone() {
            self.observe_script_lifecycle(receipt);
        }
        self.post_start_observations += 1;
    }

    /// Observe one compact, non-consuming lifecycle value at the same native
    /// publication boundary as the latest snapshot.
    pub fn observe_script_lifecycle(&mut self, receipt: script::ScriptLifecycleReceipt) {
        match self.case {
            CoreCase::HerbCleanerEmptyBank => self
                .herb_cleaner_empty_cycle
                .observe_script_lifecycle(receipt),
            CoreCase::RangingGuildFull => self
                .ranging_guild_full_cycle
                .observe_script_lifecycle(receipt),
            CoreCase::PrayerV2 | CoreCase::PrayerV1 => {
                if let Some(expected) = self.case.prayer_stop_reason() {
                    self.prayer_delivery_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::LineOfSightV2 => {
                if let Some(expected) = self.case.los_stop_reason() {
                    self.line_of_sight_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::ActorObservationV2 => {
                if let Some(expected) = self.case.actor_stop_reason() {
                    self.actor_observation_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::FightFieldV2 => {
                if let Some(expected) = self.case.fight_field_stop_reason() {
                    self.fight_field_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::HoldSpotV2
            | CoreCase::RetreatSpotV2
            | CoreCase::WalkSpotV2
            | CoreCase::EnterLairV2
            | CoreCase::LeaveLairV2
            | CoreCase::AcquireKeyV2
            | CoreCase::CellV2
            | CoreCase::BankV2 => {
                if let Some(cell) = self.case.hunt_cell() {
                    self.hunt_cycle
                        .observe_script_lifecycle(receipt, cell.stop_reason());
                }
            }
            _ => {}
        }
    }

    fn incomplete_delta_error(&self) -> String {
        if self.case == CoreCase::AlcherSwarmDrain {
            format!(
                "{} core post-Start delta incomplete {}",
                self.case.scenario_name(),
                self.alcher_swarm_cycle.phase_status()
            )
        } else {
            format!(
                "{} core post-Start delta incomplete",
                self.case.scenario_name()
            )
        }
    }

    fn swarm_stopped_without_recovery(&self, observation: &Observation) -> Option<String> {
        if self.case != CoreCase::AlcherSwarmDrain || self.qualified() {
            return None;
        }
        let stopped = observation
            .script_lifecycle
            .as_ref()
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Stopped);
        stopped.then(|| {
            format!(
                "alcher_swarm_drain stopped without ordered recovery {}",
                self.alcher_swarm_cycle.phase_status()
            )
        })
    }

    pub fn peak_item(&self, name: &str) -> i32 {
        self.max_items.get(name).copied().unwrap_or(0)
    }

    pub fn xp_gained(&self, skill: &str) -> bool {
        self.latest.skill_xp(skill) > self.baseline.skill_xp(skill)
    }

    pub fn item_increased(&self, name: &str) -> bool {
        self.peak_item(name) > self.baseline.item(name)
    }

    pub fn item_consumed_from_baseline(&self, name: &str) -> bool {
        self.latest.item(name) < self.baseline.item(name)
    }

    pub fn acquired_then_consumed(&self, name: &str) -> bool {
        self.item_increased(name) && self.latest.item(name) < self.peak_item(name)
    }

    /// Cheap predicate used by headed UI polling. Evidence is serialized only
    /// once after this becomes true.
    pub fn qualified(&self) -> bool {
        if self.post_start_observations == 0 {
            return false;
        }
        match self.case {
            CoreCase::BoneBurier => {
                self.bone_bank_cycle.buried_after_withdrawal && self.saw_bury_chat
            }
            CoreCase::ChickenKiller => {
                self.xp_gained("strength")
                    && self.acquired_then_consumed("Bones")
                    && self.xp_gained("prayer")
                    && self.saw_bury_chat
            }
            CoreCase::ChickenKillerBank => self.chicken_killer_bank_cycle.qualified(),
            CoreCase::Thiever => self.xp_gained("thieving") && self.item_increased("Coins"),
            CoreCase::Alcher
            | CoreCase::AlcherCustom
            | CoreCase::AlcherOrdered
            | CoreCase::AlcherLargeBatch => {
                self.xp_gained("magic")
                    && self.acquired_then_consumed("Nature rune")
                    && self.item_increased("Coins")
                    && match self.case {
                        CoreCase::AlcherOrdered => {
                            self.peak_item("Rune platebody") >= 1
                                && self.peak_item("Rune chainbody") >= 1
                                && self.ordered_first_exhausted
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        CoreCase::AlcherLargeBatch => {
                            self.peak_item("Rune chainbody") >= 1000
                                && self.peak_item("Nature rune") >= 1000
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        _ => self.acquired_then_consumed("Rune chainbody"),
                    }
            }
            CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
                self.alcher_generated_custom_cycle.consumed
            }
            CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff => {
                self.alcher_spell_cycle.qualified_spell()
            }
            CoreCase::AlcherSwarmDrain => self.alcher_swarm_cycle.qualified(),
            CoreCase::AlcherDefaults => self.alcher_defaults_cycle.consumed,
            CoreCase::BankFletcher => {
                self.bank_fletcher_cycle.crafted_after_withdrawal
                    && self.xp_gained("fletching")
                    && self.item_consumed_from_baseline("Willow logs")
                    && self.item_increased("Willow shortbow")
            }
            CoreCase::BankFletcherShafts | CoreCase::BankFletcherHeadless => {
                self.bank_fletcher_option_cycle.qualified()
            }
            CoreCase::BankFletcherString => self.bank_fletcher_string_cycle.strung_after_withdrawal,
            CoreCase::BankFletcherCutString => {
                self.bank_fletcher_cut_string_cycle.string_pair_created
            }
            CoreCase::DartFletcher | CoreCase::DartFletcherIron => {
                self.dart_fletcher_cycle.qualified()
            }
            CoreCase::HerbCleaner | CoreCase::HerbCleanerNamed => {
                self.herb_cleaner_cycle.qualified()
            }
            CoreCase::HerbCleanerEmptyBank => self.herb_cleaner_empty_cycle.qualified(),
            CoreCase::GemCutter | CoreCase::GemCutterNamed => self.gem_cutter_cycle.qualified(),
            CoreCase::DoorOpener | CoreCase::DoorOpenerGate => self.door_opener_cycle.qualified(),
            CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => {
                self.gnome_course_cycle.qualified()
            }
            CoreCase::WildyAgility => self.wildy_agility_cycle.qualified(),
            CoreCase::BrimhavenAgility => self.brimhaven_agility_cycle.qualified(),
            CoreCase::FlaxPicker => self.flax_picker_cycle.qualified(),
            CoreCase::Superheater
            | CoreCase::SuperheaterSteel
            | CoreCase::SuperheaterFireBattlestaff
            | CoreCase::SuperheaterSilverLowNatures => self.superheater_cycle.qualified(),
            CoreCase::VialFiller | CoreCase::VialFillerEast => self.vial_filler_cycle.qualified(),
            CoreCase::PotionMaker | CoreCase::PotionMakerNamed => {
                self.potion_maker_cycle.qualified()
            }
            CoreCase::TannerBot | CoreCase::TannerBotHard => self.tanner_bot_cycle.qualified(),
            CoreCase::RuneCrafter | CoreCase::RuneCrafterEarth | CoreCase::MuleCrafter => {
                self.rune_crafter_cycle.qualified()
            }
            CoreCase::ArdyCakes => self.ardy_cakes_cycle.qualified(),
            CoreCase::ArdyCakesFight => self.ardy_cakes_fight_cycle.qualified(),
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight => {
                self.ardy_thiever_cycle.qualified()
            }
            CoreCase::ArdyThieverFight => self.ardy_thiever_fight_cycle.qualified(),
            CoreCase::GnomeChop => self.gnome_chop_cycle.qualified(),
            CoreCase::GnomeFletchShort | CoreCase::GnomeFletchLong => {
                self.gnome_fletch_cycle.qualified()
            }
            CoreCase::CoalTrucks => self.coal_trucks_cycle.qualified(),
            CoreCase::CookBot
            | CoreCase::CookBotLobster
            | CoreCase::SmelterBot
            | CoreCase::SmelterBotSteel
            | CoreCase::FlaxSpinner
            | CoreCase::FlaxAioSpin => self.station_production_cycle.qualified(),
            CoreCase::FlaxAio => self.flax_aio_cycle.qualified(),
            CoreCase::FlaxAioPick => self.flax_aio_pick_cycle.qualified(),
            CoreCase::HerbloreSecondaries => self.herblore_eggs_cycle.qualified(),
            CoreCase::HerbloreSecondariesNewt => self.herblore_newt_cycle.qualified(),
            CoreCase::AutoFighterBank
            | CoreCase::MossGiantBank
            | CoreCase::HillGiantBank
            | CoreCase::HillGiantBankPrepared
            | CoreCase::ChaosDruidBank
            | CoreCase::ArdyFighterBank
            | CoreCase::RockCrabBank
            | CoreCase::GreenDragonBank
            | CoreCase::GreenDragonBankPrepared
            | CoreCase::GreenDragonBankDefaultPrepared
            | CoreCase::GreenDragonTele
            | CoreCase::GreenDragonTelePrepared
            | CoreCase::FireGiantBank
            | CoreCase::FireGiantBankPrepared
            | CoreCase::FireGiantCamelotPrepared => {
                match (combat_spec(self.case), combat_bank_spec(self.case)) {
                    (Some(spec), Some(bank)) => self.combat_bank_cycle.qualified(spec, bank),
                    _ => false,
                }
            }
            CoreCase::FireGiantApproach => self.combat_approach_cycle.qualified(),
            CoreCase::AioTeleport | CoreCase::AioTeleportFalador | CoreCase::AioTeleportNoStaff => {
                self.aio_teleport_cycle.qualified()
            }
            CoreCase::ShopBuyout
            | CoreCase::ShopBuyoutAubury
            | CoreCase::ShopBuyoutLowe
            | CoreCase::ShopBuyoutHickton
            | CoreCase::ShopBuyoutHarry
            | CoreCase::ShopBuyoutBetty
            | CoreCase::ShopBuyoutGerrant => self.shop_buyout_cycle.qualified(),
            CoreCase::SmithingBot | CoreCase::SmithingBotPlatebody => {
                self.smithing_bot_cycle.qualified()
            }
            CoreCase::LeatherCrafter | CoreCase::LeatherCrafterHardBody => {
                self.leather_crafter_cycle.qualified()
            }
            CoreCase::Firemaker | CoreCase::FiremakerOak => self.firemaker_cycle.qualified(),
            CoreCase::ClimbingBoots | CoreCase::ClimbingBootsTeleport => {
                match climbing_boots_spec(self.case) {
                    Some(spec) => self.climbing_boots_cycle.qualified(spec),
                    None => false,
                }
            }
            CoreCase::RangingGuildRound => self.ranging_guild_round_cycle.qualified(),
            CoreCase::RangingGuildRedeem => self.ranging_guild_redeem_cycle.qualified(),
            CoreCase::RangingGuildBank => self.ranging_guild_bank_cycle.qualified(),
            CoreCase::RangingGuildFull => self.ranging_guild_full_cycle.qualified(),
            CoreCase::BrimhavenMossInspectV1 => self.brimhaven_moss_inspect_cycle.qualified(),
            CoreCase::RouteInspectBrimhavenV2 => self.route_inspect_brimhaven_v2_cycle.qualified(),
            CoreCase::PrayerV2 | CoreCase::PrayerV1 => self.prayer_delivery_cycle.qualified(),
            CoreCase::LineOfSightV2 => self.line_of_sight_cycle.qualified(),
            CoreCase::ActorObservationV2 => self.actor_observation_cycle.qualified(),
            CoreCase::FightFieldV2 => self.fight_field_cycle.qualified(),
            CoreCase::HoldSpotV2
            | CoreCase::RetreatSpotV2
            | CoreCase::WalkSpotV2
            | CoreCase::EnterLairV2
            | CoreCase::LeaveLairV2
            | CoreCase::AcquireKeyV2
            | CoreCase::CellV2
            | CoreCase::BankV2 => self
                .case
                .hunt_cell()
                .is_some_and(|cell| self.hunt_cycle.qualified(cell)),
            CoreCase::ChaosDruid
            | CoreCase::ChaosDruidTower
            | CoreCase::ChaosDruidYanille
            | CoreCase::MossGiant
            | CoreCase::MossGiantPrepared
            | CoreCase::HillGiant
            | CoreCase::AutoFighter
            | CoreCase::AutoFighterMage
            | CoreCase::AutoFighterRange
            | CoreCase::RockCrab
            | CoreCase::RockCrabRange
            | CoreCase::GreenDragon
            | CoreCase::GreenDragonPrepared
            | CoreCase::GreenDragonSpecial
            | CoreCase::GreenDragonSpecialPrepared
            | CoreCase::GreenDragonPotions
            | CoreCase::GreenDragonPotionsPrepared
            | CoreCase::FireGiant
            | CoreCase::FireGiantPrepared
            | CoreCase::ArdyFighter => {
                combat_spec(self.case).is_some_and(|spec| self.combat_core_cycle.qualified(spec))
            }
            CoreCase::MossGiantDart => self.combat_dart_branch_cycle.qualified(),
            CoreCase::GreenDragonMagePrepared => combat_spec(self.case)
                .is_some_and(|spec| qualified_mage_branch(&self.combat_core_cycle, spec)),
        }
    }

    pub fn qualify(&self) -> Result<Value, String> {
        if self.post_start_observations == 0 {
            return Err("no post-Start observations".into());
        }
        let ok = self.qualified();
        if !ok {
            return Err(self.incomplete_delta_error());
        }
        Ok(json!({
            "case": self.case,
            "baseline": self.baseline,
            "latest": self.latest,
            "max_items": self.max_items,
            "max_xp": self.max_xp,
            "saw_bury_chat": self.saw_bury_chat,
            "post_start_observations": self.post_start_observations,
            "bone_bank_cycle": self.bone_bank_cycle,
            "bank_fletcher_cycle": self.bank_fletcher_cycle,
            "bank_fletcher_option_cycle": self.bank_fletcher_option_cycle,
            "bank_fletcher_string_cycle": self.bank_fletcher_string_cycle,
            "bank_fletcher_cut_string_cycle": self.bank_fletcher_cut_string_cycle,
            "alcher_defaults_cycle": self.alcher_defaults_cycle,
            "alcher_generated_custom_cycle": self.alcher_generated_custom_cycle,
            "alcher_spell_cycle": self.alcher_spell_cycle,
            "alcher_swarm_cycle": self.alcher_swarm_cycle,
            "dart_fletcher_cycle": self.dart_fletcher_cycle,
            "herb_cleaner_cycle": self.herb_cleaner_cycle,
            "herb_cleaner_empty_cycle": self.herb_cleaner_empty_cycle,
            "gem_cutter_cycle": self.gem_cutter_cycle,
            "door_opener_cycle": self.door_opener_cycle,
            "gnome_course_cycle": self.gnome_course_cycle,
            "wildy_agility_cycle": self.wildy_agility_cycle,
            "brimhaven_agility_cycle": self.brimhaven_agility_cycle,
            "flax_picker_cycle": self.flax_picker_cycle,
            "superheater_cycle": self.superheater_cycle,
            "chicken_killer_bank_cycle": self.chicken_killer_bank_cycle,
            "vial_filler_cycle": self.vial_filler_cycle,
            "potion_maker_cycle": self.potion_maker_cycle,
            "tanner_bot_cycle": self.tanner_bot_cycle,
            "rune_crafter_cycle": self.rune_crafter_cycle,
            "ardy_cakes_cycle": self.ardy_cakes_cycle,
            "ardy_cakes_fight_cycle": self.ardy_cakes_fight_cycle,
            "ardy_thiever_cycle": self.ardy_thiever_cycle,
            "ardy_thiever_fight_cycle": self.ardy_thiever_fight_cycle,
            "gnome_chop_cycle": self.gnome_chop_cycle,
            "gnome_fletch_cycle": self.gnome_fletch_cycle,
            "coal_trucks_cycle": self.coal_trucks_cycle,
            "station_production_cycle": self.station_production_cycle,
            "flax_aio_cycle": self.flax_aio_cycle,
            "flax_aio_pick_cycle": self.flax_aio_pick_cycle,
            "herblore_eggs_cycle": self.herblore_eggs_cycle,
            "herblore_newt_cycle": self.herblore_newt_cycle,
            "combat_core_cycle": self.combat_core_cycle,
            "combat_dart_branch_cycle": self.combat_dart_branch_cycle,
            "combat_bank_cycle": self.combat_bank_cycle,
            "combat_approach_cycle": self.combat_approach_cycle,
            "aio_teleport_cycle": self.aio_teleport_cycle,
            "shop_buyout_cycle": self.shop_buyout_cycle,
            "smithing_bot_cycle": self.smithing_bot_cycle,
            "leather_crafter_cycle": self.leather_crafter_cycle,
            "firemaker_cycle": self.firemaker_cycle,
            "climbing_boots_cycle": self.climbing_boots_cycle,
            "ranging_guild_round_cycle": self.ranging_guild_round_cycle,
            "ranging_guild_redeem_cycle": self.ranging_guild_redeem_cycle,
            "ranging_guild_bank_cycle": self.ranging_guild_bank_cycle,
            "ranging_guild_full_cycle": self.ranging_guild_full_cycle,
            "brimhaven_moss_inspect_cycle": self.brimhaven_moss_inspect_cycle,
            "route_inspect_brimhaven_v2_cycle": self.route_inspect_brimhaven_v2_cycle,
            "prayer_delivery_cycle": self.prayer_delivery_cycle,
            "line_of_sight_cycle": self.line_of_sight_cycle,
            "fight_field_cycle": self.fight_field_cycle,
            "hunt_cycle": self.hunt_cycle,
            "ordered_first_exhausted": self.ordered_first_exhausted,
        }))
    }
}

/// Opt-in bridge from the production slot snapshot publisher to the catalog
/// core witness. Disabled by default; ordinary Play/panel runs pay one mutex
/// check and never build an [`Observation`].
#[derive(Clone, Default)]
pub struct CoreWatch {
    active: Arc<AtomicBool>,
    inner: Arc<Mutex<CoreWatchState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreWatchStatus {
    Disabled,
    Ready,
    Running,
    Qualified,
    Failed,
}

#[derive(Debug, Default)]
enum CoreWatchState {
    #[default]
    Disabled,
    Ready {
        case: CoreCase,
        account: String,
        latest: Option<Observation>,
        start_preparation: HerbCleanerStartPreparation,
    },
    Running {
        account: String,
        witness: Box<CoreWitness>,
    },
    /// Immutable bounded receipt cached at the first terminal qualification.
    Qualified {
        account: String,
        evidence: Arc<Value>,
    },
    Failed {
        case: CoreCase,
        account: String,
        error: String,
        latest: Option<Observation>,
        witness: Option<Box<CoreWitness>>,
    },
}

impl CoreWatch {
    /// Replace any previous run with one ready to collect its pre-Start baseline.
    pub fn configure(&self, case: CoreCase, account: impl Into<String>) {
        *self.inner.lock().unwrap() = CoreWatchState::Ready {
            case,
            account: account.into(),
            latest: None,
            start_preparation: HerbCleanerStartPreparation::default(),
        };
        self.active.store(true, Ordering::Release);
    }

    /// Disable and drop all bounded witness state.
    pub fn clear(&self) {
        self.active.store(false, Ordering::Release);
        *self.inner.lock().unwrap() = CoreWatchState::Disabled;
    }

    /// Mark the exact instant immediately before the catalog isolate is started.
    /// The most recent observation from the production publisher becomes the
    /// immutable Start baseline; a later snapshot may already contain script work.
    pub fn begin_start(&self, account: &str) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            CoreWatchState::Disabled => {
                *state = CoreWatchState::Disabled;
                Ok(())
            }
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
                start_preparation,
            } if expected == account => match latest {
                None => {
                    *state = CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                        start_preparation,
                    };
                    Err("catalog core has no published pre-Start observation".into())
                }
                Some(observation) => {
                    let preparation = if case == CoreCase::HerbCleanerEmptyBank {
                        start_preparation.receipt_for(&expected, &observation)
                    } else {
                        None
                    };
                    match validate_start_baseline(
                        case,
                        &expected,
                        &observation,
                        preparation.as_ref(),
                    ) {
                        Ok(()) => {
                            *state = CoreWatchState::Running {
                                account: expected,
                                witness: Box::new(CoreWitness::new_with_start_preparation(
                                    case,
                                    observation,
                                    preparation,
                                )),
                            };
                            Ok(())
                        }
                        Err(error) => {
                            *state = CoreWatchState::Failed {
                                case,
                                account: expected,
                                error: error.clone(),
                                latest: Some(observation),
                                witness: None,
                            };
                            Err(error)
                        }
                    }
                }
            },
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
                start_preparation,
            } => {
                let error = format!(
                    "catalog core Start slot {account:?} is not configured account {expected:?}"
                );
                *state = CoreWatchState::Ready {
                    case,
                    account: expected,
                    latest,
                    start_preparation,
                };
                Err(error)
            }
            other => {
                *state = other;
                Err("catalog core Start was armed more than once".into())
            }
        }
    }

    /// Preserve an isolate-start refusal as a terminal core failure.
    pub fn fail_start(&self, account: &str, error: impl Into<String>) {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        *state = match current {
            CoreWatchState::Running {
                account: expected,
                witness,
            } if expected == account => CoreWatchState::Failed {
                case: witness.case,
                account: expected,
                error: error.into(),
                latest: None,
                witness: Some(witness),
            },
            other => other,
        };
    }

    /// Observe one compact record made from the already-published slot snapshot.
    /// A session boundary after Start is terminal: relogged/stale rows cannot be
    /// combined with the original baseline to manufacture progress.
    pub fn observe(&self, account: &str, observation: Observation, session_boundary: bool) {
        let mut state = self.inner.lock().unwrap();
        Self::observe_locked(&mut state, account, observation, session_boundary);
    }

    fn observe_locked(
        state: &mut CoreWatchState,
        account: &str,
        observation: Observation,
        session_boundary: bool,
    ) {
        let current = std::mem::take(&mut *state);
        *state = match current {
            CoreWatchState::Ready {
                case,
                account: expected,
                latest: _,
                mut start_preparation,
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                        start_preparation: HerbCleanerStartPreparation::default(),
                    }
                } else {
                    if case == CoreCase::HerbCleanerEmptyBank {
                        start_preparation.observe(&expected, &observation);
                    }
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: Some(observation),
                        start_preparation,
                    }
                }
            }
            CoreWatchState::Running {
                account: expected,
                mut witness,
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Failed {
                        case: witness.case,
                        account: expected,
                        error: "catalog core session boundary after Start".into(),
                        latest: Some(observation),
                        witness: Some(witness),
                    }
                } else {
                    witness.observe(&observation);
                    if let Some(error) = witness.swarm_stopped_without_recovery(&observation) {
                        CoreWatchState::Failed {
                            case: witness.case,
                            account: expected,
                            error,
                            latest: Some(observation),
                            witness: Some(witness),
                        }
                    } else {
                        CoreWatchState::Running {
                            account: expected,
                            witness,
                        }
                    }
                }
            }
            other => other,
        };
    }

    /// Convert only for the configured slot and only after Start has been armed.
    pub fn observe_snapshot(
        &self,
        account: &str,
        snapshot: &GameSnapshot,
        names: &ObjNames,
        session_boundary: bool,
    ) {
        self.observe_snapshot_with_lifecycle(
            account,
            snapshot,
            names,
            None,
            BoundedGuardian::default(),
            session_boundary,
            None,
            None,
            None,
        );
    }

    /// Convert the published snapshot and attach the slot's bounded native
    /// lifecycle receipt, prior-frame guardian fact and (hunt cards only)
    /// the slot's act ledger without touching the panel's pending-log
    /// consumer.
    pub fn observe_snapshot_with_lifecycle(
        &self,
        account: &str,
        snapshot: &GameSnapshot,
        names: &ObjNames,
        lifecycle: Option<script::ScriptLifecycleReceipt>,
        guardian: BoundedGuardian,
        session_boundary: bool,
        inspect: Option<RouteInspectPublished>,
        paint: Option<&script::shim::ScriptPaint>,
        acts: Option<ScriptActsPublished>,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        if matches!(
            &*state,
            CoreWatchState::Ready { account: expected, .. }
                | CoreWatchState::Running { account: expected, .. }
                if expected == account
        ) {
            // Convert while the lifecycle lock is held so a reconfiguration
            // cannot attach this snapshot to a later run of the same account.
            let copies_prayer = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_prayer_varps()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_prayer_varps(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_los = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_line_of_sight()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_line_of_sight(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_actor = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_actor_observation()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_actor_observation(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_fight = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_fight_field()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_fight_field(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let hunt = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.hunt_cell()
                }
                CoreWatchState::Running { witness, .. } => witness.case.hunt_cell(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => None,
            };
            let mut observation = Observation::from_snapshot(snapshot, names);
            observation.script_lifecycle = lifecycle;
            observation.guardian = guardian;
            if let Some(published) = inspect {
                observation.attach_route_inspect(published);
            }
            if copies_prayer {
                observation.attach_prayer_varps(snapshot);
            }
            if copies_los {
                observation.attach_line_of_sight(snapshot, paint);
            }
            if copies_actor {
                observation.attach_actor_observation(snapshot, paint);
            }
            if copies_fight {
                observation.attach_fight_field(snapshot, paint);
            }
            if let Some(cell) = hunt {
                observation.attach_hunt(cell, snapshot, paint, acts);
            }
            Self::observe_locked(&mut state, account, observation, session_boundary);
        }
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// True only for the active inspect cards. Callers copy published hops
    /// only then; other cases keep the empty default.
    pub fn copies_route_inspect(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_route_inspect()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_route_inspect(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the line-of-sight File card. Callers attach paint and
    /// compact collision identity only then.
    pub fn copies_line_of_sight(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_line_of_sight()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_line_of_sight(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the actor-observation File card. Callers attach paint and
    /// one packed NPC row only then.
    pub fn copies_actor_observation(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_actor_observation()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_actor_observation(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the fight-field File card. Callers attach paint and
    /// one packed NPC row only then.
    pub fn copies_fight_field(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_fight_field()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_fight_field(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the v2 hunt File cards. Callers attach paint and the
    /// slot's act ledger only then.
    pub fn copies_hunt(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.hunt_cell().is_some()
            }
            CoreWatchState::Running { witness, .. } => witness.case.hunt_cell().is_some(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// Allocation-free terminal polling for the headed UI. The full receipt
    /// remains behind [`CoreWatch::qualify`] and is built once on terminal I/O.
    pub fn status(&self) -> CoreWatchStatus {
        if !self.active.load(Ordering::Acquire) {
            return CoreWatchStatus::Disabled;
        }
        let mut state = self.inner.lock().unwrap();
        let should_cache = matches!(
            &*state,
            CoreWatchState::Running { witness, .. } if witness.qualified()
        );
        if should_cache {
            let current = std::mem::take(&mut *state);
            if let CoreWatchState::Running { account, witness } = current {
                let evidence = Arc::new(
                    witness
                        .qualify()
                        .expect("qualified predicate must produce a receipt"),
                );
                *state = CoreWatchState::Qualified { account, evidence };
            }
        }
        match &*state {
            CoreWatchState::Disabled => CoreWatchStatus::Disabled,
            CoreWatchState::Ready { .. } => CoreWatchStatus::Ready,
            CoreWatchState::Running { .. } => CoreWatchStatus::Running,
            CoreWatchState::Qualified { .. } => CoreWatchStatus::Qualified,
            CoreWatchState::Failed { .. } => CoreWatchStatus::Failed,
        }
    }

    pub fn failure(&self) -> Option<String> {
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Failed { error, .. } => Some(error.clone()),
            _ => None,
        }
    }

    /// Return full core evidence only after the unchanged witness qualifies.
    pub fn qualify(&self) -> Result<Arc<Value>, String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            CoreWatchState::Running { account, witness } => match witness.qualify() {
                Ok(value) => {
                    let evidence = Arc::new(value);
                    *state = CoreWatchState::Qualified {
                        account,
                        evidence: Arc::clone(&evidence),
                    };
                    Ok(evidence)
                }
                Err(error) => {
                    *state = CoreWatchState::Running { account, witness };
                    Err(error)
                }
            },
            CoreWatchState::Qualified { account, evidence } => {
                let receipt = Arc::clone(&evidence);
                *state = CoreWatchState::Qualified { account, evidence };
                Ok(receipt)
            }
            failed @ CoreWatchState::Failed { .. } => {
                let error = match &failed {
                    CoreWatchState::Failed { error, .. } => error.clone(),
                    _ => unreachable!(),
                };
                *state = failed;
                Err(error)
            }
            ready @ CoreWatchState::Ready { .. } => {
                *state = ready;
                Err("catalog core Start not observed".into())
            }
            CoreWatchState::Disabled => {
                *state = CoreWatchState::Disabled;
                Err("catalog core watch disabled".into())
            }
        }
    }

    /// Compact failure/timeout receipt; never serializes the world snapshot.
    pub fn evidence(&self) -> Value {
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Disabled => json!({"phase": "disabled"}),
            CoreWatchState::Ready {
                case,
                account,
                latest,
                start_preparation,
            } => json!({
                "phase": "ready",
                "case": case,
                "account": account,
                "latest": latest,
                "start_preparation": start_preparation,
            }),
            CoreWatchState::Running { account, witness } => json!({
                "phase": "running",
                "account": account,
                "witness": witness,
                "qualification": witness.qualify().err(),
            }),
            CoreWatchState::Qualified { account, evidence } => json!({
                "phase": "qualified",
                "account": account,
                "receipt": evidence,
            }),
            CoreWatchState::Failed {
                case,
                account,
                error,
                latest,
                witness,
            } => json!({
                "phase": "failed",
                "case": case,
                "account": account,
                "error": error,
                "latest": latest,
                "witness": witness,
            }),
        }
    }
}

fn validate_start_baseline(
    case: CoreCase,
    account: &str,
    observation: &Observation,
    start_preparation: Option<&HerbCleanerStartPreparationReceipt>,
) -> Result<(), String> {
    if !observation.ingame || observation.scene_state != 2 {
        return Err(format!(
            "Start baseline is not attached ingame scene2: {observation:?}"
        ));
    }
    let player = observation
        .player
        .as_deref()
        .ok_or_else(|| "Start baseline has no local player".to_string())?;
    let expected = client::util::jstring::JString::to_screen_name(account);
    if !player.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "Start baseline player {player:?} is not fresh account {account:?}"
        ));
    }
    if case == CoreCase::HerbCleanerEmptyBank && start_preparation.is_none() {
        return Err(format!(
            "{} Start baseline lacks this run's loaded-bank preparation receipt before closed-bank Start: {observation:?}",
            case.scenario_name()
        ));
    }
    validate_case_baseline_with_preparation(case, observation, start_preparation)
}

#[cfg(test)]
mod actor_observation_selection_tests {
    use super::*;
    use api::snapshot::{NpcView, WorldTile};

    fn npc(index: usize, distance: i32, size: i32, name: &str) -> NpcView {
        NpcView {
            index,
            r#type: Some(1),
            name: Some(name.into()),
            actions: vec![Some("Attack".into())],
            tile: WorldTile {
                x: 3201,
                z: 3205,
                level: 0,
            },
            distance,
            animation: 0,
            pose_animation: 0,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 5,
            total_health: 5,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 2,
            size,
            network: WorldTile {
                x: 3205,
                z: 3201,
                level: 0,
            },
            x: 3201,
            z: 3205,
            yaw: 0,
        }
    }

    fn receipt_for(index: i32) -> ActorObservationScriptReceipt {
        ActorObservationScriptReceipt {
            npc: ActorObservationNpcFact {
                index,
                name: Some("Near".into()),
                size: 2,
                ..ActorObservationNpcFact::default()
            },
            ..ActorObservationScriptReceipt::default()
        }
    }

    #[test]
    fn chooses_nearest_size_ge_1_not_array_first() {
        let rows = [
            npc(1, 0, 0, "Zero"),
            npc(3, 8, 1, "Far"),
            npc(11, 1, 2, "Near"),
        ];
        let chosen = choose_actor_observation_npc(&rows, None).expect("nearest size>=1");
        assert_eq!(chosen.index, 11);
        assert_eq!(chosen.name.as_deref(), Some("Near"));
        assert_eq!(chosen.size, 2);
    }

    #[test]
    fn receipt_index_selects_that_row_not_nearest() {
        let rows = [
            npc(1, 0, 0, "Zero"),
            npc(3, 8, 1, "Far"),
            npc(11, 1, 2, "Near"),
        ];
        let receipt = receipt_for(3);
        let chosen = choose_actor_observation_npc(&rows, Some(&receipt)).expect("receipt index 3");
        assert_eq!(chosen.index, 3);
        assert_eq!(chosen.name.as_deref(), Some("Far"));
        let nearest = receipt_for(11);
        let chosen = choose_actor_observation_npc(&rows, Some(&nearest)).expect("receipt index 11");
        assert_eq!(chosen.index, 11);
    }
}

#[cfg(test)]
mod fight_field_selection_tests {
    use super::*;
    use api::snapshot::{NpcView, WorldTile};

    fn npc(index: usize, distance: i32, size: i32) -> NpcView {
        NpcView {
            index,
            r#type: Some(1),
            name: Some("Npc".into()),
            actions: vec![Some("Attack".into())],
            tile: WorldTile {
                x: 3201,
                z: 3205,
                level: 0,
            },
            distance,
            animation: 0,
            pose_animation: 0,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 5,
            total_health: 5,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 2,
            size,
            network: WorldTile {
                x: 3205,
                z: 3201,
                level: 0,
            },
            x: 3201,
            z: 3205,
            yaw: 0,
        }
    }

    fn receipt_for(index: i32) -> FightFieldScriptReceipt {
        FightFieldScriptReceipt {
            index,
            size: 2,
            ..FightFieldScriptReceipt::default()
        }
    }

    #[test]
    fn chooses_nearest_size_ge_1_not_array_first() {
        let rows = [npc(1, 0, 0), npc(3, 8, 1), npc(11, 1, 2)];
        let chosen = choose_fight_field_npc(&rows, None).expect("nearest size>=1");
        assert_eq!(chosen.index, 11);
        assert_eq!(chosen.size, 2);
    }

    #[test]
    fn receipt_index_selects_that_row_not_nearest() {
        let rows = [npc(1, 0, 0), npc(3, 8, 1), npc(11, 1, 2)];
        let receipt = receipt_for(3);
        let chosen = choose_fight_field_npc(&rows, Some(&receipt)).expect("receipt index 3");
        assert_eq!(chosen.index, 3);
        let nearest = receipt_for(11);
        let chosen = choose_fight_field_npc(&rows, Some(&nearest)).expect("receipt index 11");
        assert_eq!(chosen.index, 11);
    }
}

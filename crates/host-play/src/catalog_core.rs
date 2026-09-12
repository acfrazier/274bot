//! Shared full-core catalog witness used by headless and headed proof paths.
//!
//! This module intentionally contains the exact compact observations, Start
//! predicates, cycle observers and qualification logic from the original
//! `catalog_boundary_live` harness. It is proof infrastructure, not gameplay.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::obj_names::ObjNames;
use api::snapshot::{ActorKind, GameSnapshot, LocView, SceneView, WorldTile};
use serde::Serialize;
use serde_json::{json, Value};

pub const CORE_SCENARIOS: &str = "bone_burier|chicken_killer|chicken_killer_bank|thiever|alcher|alcher_defaults|alcher_custom|alcher_custom_alias|alcher_custom_name|alcher_ordered|alcher_large_batch|bank_fletcher|bank_fletcher_shafts|bank_fletcher_headless|bank_fletcher_string|bank_fletcher_cut_string|dart_fletcher|dart_fletcher_iron|herb_cleaner|herb_cleaner_named|gem_cutter|gem_cutter_named|door_opener|door_opener_gate|gnome_course|gnome_course_radius|wildy_agility|brimhaven_agility|flax_picker|superheater|superheater_steel|superheater_fire_battlestaff|vial_filler|vial_filler_east|potion_maker|potion_maker_named|tanner_bot|tanner_bot_hard|rune_crafter|rune_crafter_earth|mule_crafter|ardy_cakes|ardy_cakes_fight|ardy_thiever|ardy_thiever_fight|ardy_thiever_knight|gnome_chop|gnome_fletch_short|gnome_fletch_long|coal_trucks|cook_bot|cook_bot_lobster|smelter_bot|smelter_bot_steel|flax_spinner|flax_aio|flax_aio_pick|flax_aio_spin|herblore_secondaries|herblore_secondaries_newt|chaos_druid|chaos_druid_tower|chaos_druid_yanille|moss_giant|hill_giant|auto_fighter|auto_fighter_mage|auto_fighter_range|rock_crab|rock_crab_range|green_dragon|green_dragon_special|green_dragon_potions|fire_giant|ardy_fighter|auto_fighter_bank|moss_giant_bank|hill_giant_bank|chaos_druid_bank|ardy_fighter_bank";
pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";
pub const ADAMANT_SCIMITAR_ID: i32 = 1331;
pub const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
pub const YEW_LONGBOW_ID: i32 = 855;
pub const CERT_YEW_LONGBOW_ID: i32 = 856;
pub const NATURE_RUNE_ID: i32 = 561;
pub const MIND_RUNE_ID: i32 = 558;
pub const COINS_ID: i32 = 995;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
pub const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
pub const YEW_LONGBOW_ALCH_COINS: i32 = 768;
pub const HIGH_ALCH_MAGIC_XP: i32 = 65;
pub const LOGS_ID: i32 = 1511;
pub const ARROW_SHAFT_ID: i32 = 52;
pub const HEADLESS_ARROW_ID: i32 = 53;
pub const BRONZE_DART_TIP_ID: i32 = 819;
pub const BRONZE_DART_ID: i32 = 806;
pub const IRON_DART_TIP_ID: i32 = 820;
pub const IRON_DART_ID: i32 = 807;
pub const FEATHER_ID: i32 = 314;
pub const UNIDENTIFIED_GUAM_ID: i32 = 199;
pub const GUAM_LEAF_ID: i32 = 249;
pub const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
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
pub const COAL_ID: i32 = 453;
pub const STAFF_OF_FIRE_ID: i32 = 1387;
pub const FIRE_BATTLESTAFF_ID: i32 = 1393;
pub const BRONZE_BAR_ID: i32 = 2349;
pub const IRON_BAR_ID: i32 = 2351;
pub const STEEL_BAR_ID: i32 = 2353;
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
pub const ROCK_CRAB_FOOD: i32 = 8;
pub const GREEN_DRAGON_FOOD: i32 = 12;
pub const FIRE_GIANT_FOOD: i32 = 12;
pub const COMBAT_ATTACK_LEVEL: i32 = 40;
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
pub const ROCK_CRAB_SAFE_STAND: (i32, i32, i32) = (2712, 3688, 0);
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
/// The six verifiable Guard drops: the loot list the AutoFighter bank cell
/// injects (`loot=[iron ore, steel arrow, body talisman, blood/chaos/nature
/// rune]`, whose item names resolve to exactly these ids) and the class its
/// bank trip deposits — the same ArdyFighter already lists. A clue-only or
/// junk drop is not one of these.
pub const GUARD_DROP_IDS: [i32; 6] = [
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];

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
    BankFletcher,
    BankFletcherShafts,
    BankFletcherHeadless,
    BankFletcherString,
    BankFletcherCutString,
    DartFletcher,
    DartFletcherIron,
    HerbCleaner,
    HerbCleanerNamed,
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
    HillGiant,
    AutoFighter,
    AutoFighterMage,
    AutoFighterRange,
    RockCrab,
    RockCrabRange,
    GreenDragon,
    GreenDragonSpecial,
    GreenDragonPotions,
    FireGiant,
    ArdyFighter,
    AutoFighterBank,
    MossGiantBank,
    HillGiantBank,
    ChaosDruidBank,
    ArdyFighterBank,
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
            "bank_fletcher" => Ok(Self::BankFletcher),
            "bank_fletcher_shafts" => Ok(Self::BankFletcherShafts),
            "bank_fletcher_headless" => Ok(Self::BankFletcherHeadless),
            "bank_fletcher_string" => Ok(Self::BankFletcherString),
            "bank_fletcher_cut_string" => Ok(Self::BankFletcherCutString),
            "dart_fletcher" => Ok(Self::DartFletcher),
            "dart_fletcher_iron" => Ok(Self::DartFletcherIron),
            "herb_cleaner" => Ok(Self::HerbCleaner),
            "herb_cleaner_named" => Ok(Self::HerbCleanerNamed),
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
            "hill_giant" => Ok(Self::HillGiant),
            "auto_fighter" => Ok(Self::AutoFighter),
            "auto_fighter_mage" => Ok(Self::AutoFighterMage),
            "auto_fighter_range" => Ok(Self::AutoFighterRange),
            "rock_crab" => Ok(Self::RockCrab),
            "rock_crab_range" => Ok(Self::RockCrabRange),
            "green_dragon" => Ok(Self::GreenDragon),
            "green_dragon_special" => Ok(Self::GreenDragonSpecial),
            "green_dragon_potions" => Ok(Self::GreenDragonPotions),
            "fire_giant" => Ok(Self::FireGiant),
            "ardy_fighter" => Ok(Self::ArdyFighter),
            "auto_fighter_bank" => Ok(Self::AutoFighterBank),
            "moss_giant_bank" => Ok(Self::MossGiantBank),
            "hill_giant_bank" => Ok(Self::HillGiantBank),
            "chaos_druid_bank" => Ok(Self::ChaosDruidBank),
            "ardy_fighter_bank" => Ok(Self::ArdyFighterBank),
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
            Self::BankFletcher => "bank_fletcher",
            Self::BankFletcherShafts => "bank_fletcher_shafts",
            Self::BankFletcherHeadless => "bank_fletcher_headless",
            Self::BankFletcherString => "bank_fletcher_string",
            Self::BankFletcherCutString => "bank_fletcher_cut_string",
            Self::DartFletcher => "dart_fletcher",
            Self::DartFletcherIron => "dart_fletcher_iron",
            Self::HerbCleaner => "herb_cleaner",
            Self::HerbCleanerNamed => "herb_cleaner_named",
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
            Self::HillGiant => "hill_giant",
            Self::AutoFighter => "auto_fighter",
            Self::AutoFighterMage => "auto_fighter_mage",
            Self::AutoFighterRange => "auto_fighter_range",
            Self::RockCrab => "rock_crab",
            Self::RockCrabRange => "rock_crab_range",
            Self::GreenDragon => "green_dragon",
            Self::GreenDragonSpecial => "green_dragon_special",
            Self::GreenDragonPotions => "green_dragon_potions",
            Self::FireGiant => "fire_giant",
            Self::ArdyFighter => "ardy_fighter",
            Self::AutoFighterBank => "auto_fighter_bank",
            Self::MossGiantBank => "moss_giant_bank",
            Self::HillGiantBank => "hill_giant_bank",
            Self::ChaosDruidBank => "chaos_druid_bank",
            Self::ArdyFighterBank => "ardy_fighter_bank",
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
            | Self::AlcherLargeBatch => "Alcher",
            Self::BankFletcher
            | Self::BankFletcherShafts
            | Self::BankFletcherHeadless
            | Self::BankFletcherString
            | Self::BankFletcherCutString => "BankFletcher",
            Self::DartFletcher | Self::DartFletcherIron => "DartFletcher",
            Self::HerbCleaner | Self::HerbCleanerNamed => "HerbCleaner",
            Self::GemCutter | Self::GemCutterNamed => "GemCutter",
            Self::DoorOpener | Self::DoorOpenerGate => "DoorOpener",
            Self::GnomeCourse | Self::GnomeCourseRadius => "GnomeCourse",
            Self::WildyAgility => "WildyAgility",
            Self::BrimhavenAgility => "BrimhavenAgility",
            Self::FlaxPicker => "FlaxPicker",
            Self::Superheater | Self::SuperheaterSteel | Self::SuperheaterFireBattlestaff => {
                "Superheater"
            }
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
            Self::MossGiant | Self::MossGiantBank => "MossGiant",
            Self::HillGiant | Self::HillGiantBank => "HillGiant",
            Self::AutoFighter
            | Self::AutoFighterMage
            | Self::AutoFighterRange
            | Self::AutoFighterBank => "AutoFighter",
            Self::RockCrab | Self::RockCrabRange => "RockCrab",
            Self::GreenDragon | Self::GreenDragonSpecial | Self::GreenDragonPotions => {
                "GreenDragon"
            }
            Self::FireGiant => "FireGiant",
            Self::ArdyFighter | Self::ArdyFighterBank => "ArdyFighter",
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
    pub local_target_npc: Option<usize>,
    pub local_health: i32,
    pub local_animation: i32,
    pub equipment_ids: BTreeMap<i32, i32>,
    pub main_modal: i32,
    pub widget_ids: BTreeSet<i32>,
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
            .map(|npc| BoundedNpc {
                index: npc.index,
                name: npc.name.clone(),
                health: npc.health,
                total_health: npc.total_health,
                animation: npc.animation,
                in_combat: npc.in_combat,
                targeting_local: npc.target.is_some_and(|target| {
                    target.kind == ActorKind::Player
                        && self_slot >= 0
                        && target.index == self_slot as usize
                }),
                tile: (npc.tile.x, npc.tile.z, npc.tile.level),
                distance: npc.distance,
            })
            .collect();
        let dormant_rocks_seen = snapshot.npcs().iter().any(|npc| {
            npc.name.as_deref() == Some("Rocks")
                && npc.tile.level == ROCK_CRAB_SPOT.2
                && (npc.tile.x - ROCK_CRAB_SPOT.0)
                    .abs()
                    .max((npc.tile.z - ROCK_CRAB_SPOT.1).abs())
                    <= 50
        });
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
            local_target_npc,
            local_health,
            local_animation,
            equipment_ids,
            main_modal: snapshot.modals().main,
            widget_ids: snapshot
                .widgets()
                .iter()
                .map(|widget| widget.component_id)
                .collect(),
        }
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

    pub fn equipment_id(&self, id: i32) -> i32 {
        self.equipment_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn has_widget(&self, id: i32) -> bool {
        self.widget_ids.contains(&id)
    }
}

pub fn combat_npc_name(name: Option<&str>) -> bool {
    matches!(
        name.map(str::trim),
        Some("Chaos druid" | "Moss giant" | "Giant" | "Guard")
    )
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
        n == "door" || n.ends_with(" door") || n.contains("gate")
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
        CoreCase::ChaosDruid
        | CoreCase::ChaosDruidTower
        | CoreCase::ChaosDruidYanille
        | CoreCase::MossGiant
        | CoreCase::HillGiant
        | CoreCase::AutoFighter
        | CoreCase::AutoFighterMage
        | CoreCase::AutoFighterRange
        | CoreCase::RockCrab
        | CoreCase::RockCrabRange
        | CoreCase::GreenDragon
        | CoreCase::GreenDragonSpecial
        | CoreCase::GreenDragonPotions
        | CoreCase::FireGiant
        | CoreCase::ArdyFighter
        | CoreCase::AutoFighterBank
        | CoreCase::MossGiantBank
        | CoreCase::HillGiantBank
        | CoreCase::ChaosDruidBank
        | CoreCase::ArdyFighterBank => combat_spec(case).is_some_and(|spec| {
            combat_baseline_ready(baseline, spec)
                && match case {
                    CoreCase::ChaosDruidTower => baseline.level("thieving") >= 46,
                    CoreCase::ChaosDruidYanille => baseline.level("agility") >= 40,
                    // Bank cells start from a wielded melee weapon: every one of
                    // these cards deposits (or is told to deposit) the pack, so a
                    // carried weapon would be stashed instead of used. The Guard
                    // loot class is refused by `combat_baseline_ready` itself
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
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(CAKE_ID) == 0
                            && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
                    }
                    _ => true,
                }
        }),
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
            "safe stand (2712,3688,0), dormant Rocks observed in the supported field, Attack/Strength/Hitpoints 40, lobster 8, scimitar 1331, bank Off"
        }
        CoreCase::AutoFighterRange => {
            "Ardougne Guard (2661,3306,0), Hitpoints 40, Ranged 40, Maple shortbow 853 worn, Bronze arrow 882 x200 worn, Trout 8, bank None"
        }
        CoreCase::GreenDragon => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, lobster 12, rune scimitar 1333, worn shield 1540, empty 536/1753"
        }
        CoreCase::GreenDragonSpecial => {
            "Wilderness field (3096,3814,0) z>=3520, Attack 60, Hitpoints 40, worn dragon dagger 1215 and shield 1540, unarmed spec bar, lobster 12"
        }
        CoreCase::GreenDragonPotions => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, worn shield 1540, super attack(3) 145 and super strength(3) 157 with no two-dose flask and no live boost, lobster 12"
        }
        CoreCase::RockCrabRange => {
            "safe stand (2712,3688,0), dormant Rocks observed in the supported field, Hitpoints and Ranged 40, Maple shortbow 853 worn, Bronze arrow 882 x200 worn, lobster 8, bank Off"
        }
        CoreCase::FireGiant => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, amulet 295, rope 954, empty 532"
        }
        CoreCase::ArdyFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, Thieving 5, scimitar 1331, empty cake/bread/slice, bank Off"
        }
        CoreCase::AutoFighterBank => {
            "Ardougne Guard (2661,3306,0) r8, Attack/Strength/Hitpoints 40, trout 8, worn scimitar 1331, banking Auto, empty Guard-drop class 440/886/1446/565/562/561"
        }
        CoreCase::MossGiantBank => {
            "Moss safespot (2553,3406,0) r10, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 2 (below its restock line), empty 532/225"
        }
        CoreCase::HillGiantBank => {
            "Giant pit (3110,9832,0) r16, Attack/Strength/Hitpoints 40, worn scimitar 1331, trout 8, brass key 983, lootSlots 1, empty 532/225"
        }
        CoreCase::ChaosDruidBank => {
            "Edgeville dungeon (3110,9936,0) r14, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 8 (under foodWithdraw 12)"
        }
        CoreCase::ArdyFighterBank => {
            "Ardougne Guard (2661,3306,0) r12, Attack/Strength/Hitpoints 40, Thieving 5, worn scimitar 1331, empty cake pack, bankStrategy Loot count"
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
    pub dart_fletcher_cycle: DartFletcherCycle,
    pub herb_cleaner_cycle: HerbCleanerCycle,
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
    pub combat_bank_cycle: CombatBankCycle,
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

    pub fn observe_target(
        &mut self,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
        alch_coins: i32,
    ) {
        if self.withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(noted) >= 1
            && now.item_id(unnoted) == 0
            && now.item_id(noted) > baseline.item_id(noted)
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item("Rune chainbody") == 0
        {
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
    /// The AutoFighter bank cell's injected Guard loot list: any one of the
    /// six verifiable Guard drops ([`GUARD_DROP_IDS`]) landing in the pack.
    GuardDrop,
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
        CoreCase::GreenDragon => Some(CombatSpec {
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
        // `useSpecial=true` with the dragon dagger: the bar arms and `%sa_energy`
        // pays the 250 cost, so a queued-but-unspent arming cannot qualify.
        CoreCase::GreenDragonSpecial => Some(CombatSpec {
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
        CoreCase::GreenDragonPotions => Some(CombatSpec {
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
        // injected `bankAtLootSlots=1`: the loot class is the card's own
        // injected Guard list, and the pack starts empty of it.
        CoreCase::AutoFighterBank => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::GuardDrop,
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
        CoreCase::HillGiantBank => Some(CombatSpec {
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
            loot: CombatLoot::None,
            extra: CombatExtra::StolenFood,
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
/// `docs/compat/05-options-149.md`.
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
    /// Where the trip returns before further work.
    pub ret: (i32, i32, i32),
    pub ret_radius: i32,
}

pub fn combat_bank_spec(case: CoreCase) -> Option<CombatBankSpec> {
    match case {
        // `banking=Auto`: BankRun walks to the nearest bank from the anchor and
        // deposits everything its keep-list does not hold, then restocks food.
        // The deposit class is the card's own injected Guard loot list: the
        // pack only ever holds it because a Guard dropped it.
        CoreCase::AutoFighterBank => Some(CombatBankSpec {
            deposit: &GUARD_DROP_IDS,
            stand: ARDOUGNE_EAST_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            ret: ARDY_THIEVER_STAND,
            ret_radius: 6,
        }),
        // Script-owned Ardougne West trip: deposits everything but
        // food/runes/ammo/weapon and withdraws lobster back to the safespot.
        CoreCase::MossGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: MOSS_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            ret: MOSS_GIANT_SAFESPOT,
            ret_radius: 6,
        }),
        // Always-on trip end: Varrock West keeps only trout and the Brass key.
        CoreCase::HillGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: HILL_GIANT_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            ret: HILL_GIANT_PIT,
            ret_radius: 16,
        }),
        // Edgeville trip end: `depositInventory` empties the pack, then the
        // card withdraws exactly its food back and returns through the trapdoor.
        CoreCase::ChaosDruidBank => Some(CombatBankSpec {
            deposit: &[LOBSTER_ID],
            stand: CHAOS_DRUID_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            ret: CHAOS_DRUID_FIELD,
            ret_radius: 14,
        }),
        // `bankStrategy=Loot count`: PeriodicBank deposits the card's own loot
        // list and walks back to the market anchor. No food restock (the
        // Baker's stall is this card's food).
        CoreCase::ArdyFighterBank => Some(CombatBankSpec {
            deposit: &GUARD_DROP_IDS,
            stand: ARDY_BANK,
            stand_radius: 6,
            restock: None,
            ret: ARDY_THIEVER_STAND,
            ret_radius: 6,
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
        CombatLoot::DragonBonesOrHide => {
            observation.item_id(DRAGON_BONES_ID) + observation.item_id(GREEN_DRAGONHIDE_ID)
        }
        CombatLoot::GuardDrop => GUARD_DROP_IDS
            .iter()
            .map(|id| observation.item_id(*id))
            .sum(),
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
        self.looted |= combat_loot_count(now, spec.loot) > combat_loot_count(baseline, spec.loot);
        self.wrong_item |= now.item_id(BLACK_DRAGONHIDE_ID) > 0
            || now.item_id(RED_DRAGONHIDE_ID) > 0
            || now.item_id(BLUE_DRAGONHIDE_ID) > 0
            || now.item_id(CHOCOLATE_CAKE_ID) > 0;
        if spec.extra == CombatExtra::WornShield {
            self.shield_worn |= now.equipment_id(DRAGONFIRE_SHIELD_ID) >= 1;
        }
        if spec.extra == CombatExtra::StolenFood {
            self.stolen_food |= stall_food(now) > stall_food(baseline);
        }
        if spec.extra == CombatExtra::RockActivation {
            for npc in &now.npc_facts {
                let Some(name) = npc.name.as_deref().map(str::trim) else {
                    continue;
                };
                if name == "Rocks" {
                    self.dormant_indexes.insert(npc.index);
                    self.dormant_tiles.insert(npc.index, npc.tile);
                }
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
            let was_engaged = self.currently_engaged.contains(&npc.index);
            let actual_work = selected_combat || health_drop;
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

pub fn combat_loot_id(id: i32, loot: CombatLoot) -> bool {
    match loot {
        CombatLoot::HerbLawNature => {
            unidentified_herb_id(id) || id == NATURE_RUNE_ID || id == LAW_RUNE_ID
        }
        CombatLoot::BigBones => id == BIG_BONES_ID,
        CombatLoot::BigBonesOrLimpwurt => id == BIG_BONES_ID || id == LIMPWURT_ROOT_ID,
        CombatLoot::DragonBonesOrHide => id == DRAGON_BONES_ID || id == GREEN_DRAGONHIDE_ID,
        CombatLoot::GuardDrop => GUARD_DROP_IDS.contains(&id),
        CombatLoot::None => false,
    }
}

/// Bank/return cycle for the five bank cells: the reviewed combat witness plus
/// the declared bank trip the card itself has to execute — pack stock moves to
/// the booth's bank, the card's restock line is met from that bank's own stock,
/// the modal closes on a later bank session, the trip returns to the card's
/// tile, and work resumes there. A booth opened away from the card's stand, a
/// deposit the card never made, a return without further work, or a bank that
/// only ever opened (seed/readiness) cannot qualify it.
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
        let open = now.bank_open && now.bank_loaded;
        let at_stand = near(now.tile, bank.stand, bank.stand_radius);
        if open && !at_stand {
            self.wrong_bank = true;
        }
        if self.banked.is_none()
            && open
            && at_stand
            // A fresh session: the pre-Start bank state cannot satisfy this.
            && now.bank_generation > baseline.bank_generation
            && Self::deposit_count(now, bank.deposit) == 0
            && Self::bank_count(now, bank.deposit) > Self::bank_count(baseline, bank.deposit)
        {
            self.food_at_deposit = bank.restock.map_or(0, |food| now.item_id(food));
            self.banked = Some(now.clone());
        }
        if let Some(banked) = &self.banked {
            if let Some(food) = bank.restock {
                self.restocked |= open
                    && now.item_id(food) > self.food_at_deposit
                    && now.bank_item_id(food) < baseline.bank_item_id(food);
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
        self.combat.qualified(spec)
            && self.banked.is_some()
            && (bank.restock.is_none() || self.restocked)
            && self.closed
            && self.returned
            && self.further
            && !self.wrong_bank
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
        let wrong_bars = [IRON_BAR_ID, BRONZE_BAR_ID, STEEL_BAR_ID]
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

/// `guardResponse=Fight` on ArdyThiever: the Flee bank-cycle shape plus a
/// FightBack Guard kill. The Flee kite tile fails this branch.
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
}

impl ArdyThieverFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        if self.pickpocketed.is_none()
            && now.item_id(COINS_ID) >= 1
            && baseline.item_id(COINS_ID) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
        {
            self.pickpocketed = Some(now.clone());
        }
        if self.pickpocketed.is_some() && self.killed.is_none() {
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
            if self.pickpocketed.is_some()
                && self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        if self.pickpocketed.is_some()
            && self.killed.is_none()
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
            self.further |= !now.bank_open && now.item_id(COINS_ID) >= 1;
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

impl CoreWitness {
    pub fn new(case: CoreCase, baseline: Observation) -> Self {
        Self {
            max_items: baseline.items.clone(),
            max_xp: baseline.xp.clone(),
            latest: baseline.clone(),
            baseline,
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
            dart_fletcher_cycle: DartFletcherCycle::default(),
            herb_cleaner_cycle: HerbCleanerCycle::default(),
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
            combat_bank_cycle: CombatBankCycle::default(),
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
        if let Some(spec) = combat_spec(self.case) {
            self.combat_core_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let (Some(spec), Some(bank)) = (combat_spec(self.case), combat_bank_spec(self.case)) {
            self.combat_bank_cycle
                .observe(spec, bank, &self.baseline, observation);
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
        self.post_start_observations += 1;
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
            | CoreCase::SuperheaterFireBattlestaff => self.superheater_cycle.qualified(),
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
            | CoreCase::ChaosDruidBank
            | CoreCase::ArdyFighterBank => {
                match (combat_spec(self.case), combat_bank_spec(self.case)) {
                    (Some(spec), Some(bank)) => self.combat_bank_cycle.qualified(spec, bank),
                    _ => false,
                }
            }
            CoreCase::ChaosDruid
            | CoreCase::ChaosDruidTower
            | CoreCase::ChaosDruidYanille
            | CoreCase::MossGiant
            | CoreCase::HillGiant
            | CoreCase::AutoFighter
            | CoreCase::AutoFighterMage
            | CoreCase::AutoFighterRange
            | CoreCase::RockCrab
            | CoreCase::RockCrabRange
            | CoreCase::GreenDragon
            | CoreCase::GreenDragonSpecial
            | CoreCase::GreenDragonPotions
            | CoreCase::FireGiant
            | CoreCase::ArdyFighter => {
                combat_spec(self.case).is_some_and(|spec| self.combat_core_cycle.qualified(spec))
            }
        }
    }

    pub fn qualify(&self) -> Result<Value, String> {
        if self.post_start_observations == 0 {
            return Err("no post-Start observations".into());
        }
        let ok = self.qualified();
        if !ok {
            return Err(format!(
                "{} core post-Start delta incomplete",
                self.case.scenario_name()
            ));
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
            "dart_fletcher_cycle": self.dart_fletcher_cycle,
            "herb_cleaner_cycle": self.herb_cleaner_cycle,
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
            "combat_bank_cycle": self.combat_bank_cycle,
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
            } if expected == account => match latest {
                None => {
                    *state = CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                    };
                    Err("catalog core has no published pre-Start observation".into())
                }
                Some(observation) => match validate_start_baseline(case, &expected, &observation) {
                    Ok(()) => {
                        *state = CoreWatchState::Running {
                            account: expected,
                            witness: Box::new(CoreWitness::new(case, observation)),
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
                },
            },
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
            } => {
                let error = format!(
                    "catalog core Start slot {account:?} is not configured account {expected:?}"
                );
                *state = CoreWatchState::Ready {
                    case,
                    account: expected,
                    latest,
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
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                    }
                } else {
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: Some(observation),
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
                    CoreWatchState::Running {
                        account: expected,
                        witness,
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
            Self::observe_locked(
                &mut state,
                account,
                Observation::from_snapshot(snapshot, names),
                session_boundary,
            );
        }
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire)
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
            } => json!({
                "phase": "ready",
                "case": case,
                "account": account,
                "latest": latest,
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
    validate_case_baseline(case, observation)
}

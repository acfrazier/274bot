//! Headless frozen-catalog proof through production Play.
//!
//! The ignored live cell starts one real catalog card only after its registered
//! scenario has completed all preparation waits. The production slot thread
//! remains the sole owner of gameplay actions; this harness observes snapshots.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::obj_names::ObjNames;
use api::snapshot::{ActorKind, GameSnapshot};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const SUPPORT_MATRIX: &str = include_str!("../../../docs/compat/support-matrix.json");
const CORE_SCENARIOS: &str = "bone_burier|chicken_killer|chicken_killer_bank|thiever|alcher|alcher_custom|alcher_custom_alias|alcher_custom_name|alcher_ordered|alcher_large_batch|bank_fletcher|bank_fletcher_string|bank_fletcher_cut_string|dart_fletcher|dart_fletcher_iron|herb_cleaner|herb_cleaner_named|gem_cutter|gem_cutter_named|door_opener|door_opener_gate|gnome_course|gnome_course_radius|flax_picker|superheater|superheater_steel|superheater_fire_battlestaff|vial_filler|vial_filler_east|potion_maker|potion_maker_named|tanner_bot|tanner_bot_hard|rune_crafter|rune_crafter_earth|mule_crafter|ardy_cakes|ardy_thiever|ardy_thiever_knight|gnome_chop|gnome_fletch_short|gnome_fletch_long|coal_trucks|cook_bot|cook_bot_lobster|smelter_bot|smelter_bot_steel|flax_spinner|flax_aio|flax_aio_pick|flax_aio_spin|herblore_secondaries|herblore_secondaries_newt|chaos_druid|moss_giant|hill_giant|auto_fighter|rock_crab|green_dragon|fire_giant|ardy_fighter";
const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";
const ADAMANT_SCIMITAR_ID: i32 = 1331;
const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
const NATURE_RUNE_ID: i32 = 561;
const COINS_ID: i32 = 995;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
const HIGH_ALCH_MAGIC_XP: i32 = 65;
const BRONZE_DART_TIP_ID: i32 = 819;
const BRONZE_DART_ID: i32 = 806;
const IRON_DART_TIP_ID: i32 = 820;
const IRON_DART_ID: i32 = 807;
const FEATHER_ID: i32 = 314;
const UNIDENTIFIED_GUAM_ID: i32 = 199;
const GUAM_LEAF_ID: i32 = 249;
const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
const UNCUT_SAPPHIRE_ID: i32 = 1623;
const SAPPHIRE_ID: i32 = 1607;
const UNCUT_OPAL_ID: i32 = 1625;
const CHISEL_ID: i32 = 1755;
const CRUSHED_GEMSTONE_ID: i32 = 1633;
const WOODEN_DOOR_CLOSED_ID: i32 = 1530;
const WOODEN_DOOR_OPEN_ID: i32 = 1531;
const WOODEN_GATE_CLOSED_ID: i32 = 1551;
const WOODEN_GATE_OPEN_ID: i32 = 1552;
const FLAX_ID: i32 = 1779;
const COPPER_ORE_ID: i32 = 436;
const TIN_ORE_ID: i32 = 438;
const IRON_ORE_ID: i32 = 440;
const COAL_ID: i32 = 453;
const STAFF_OF_FIRE_ID: i32 = 1387;
const FIRE_BATTLESTAFF_ID: i32 = 1393;
const BRONZE_BAR_ID: i32 = 2349;
const IRON_BAR_ID: i32 = 2351;
const STEEL_BAR_ID: i32 = 2353;
const LUMBRIDGE_DOOR: (i32, i32, i32) = (3208, 3211, 0);
const LUMBRIDGE_DOOR_STAND: (i32, i32, i32) = (3208, 3212, 0);
const LUMBRIDGE_GATE: (i32, i32, i32) = (3213, 3261, 0);
const LUMBRIDGE_GATE_STAND: (i32, i32, i32) = (3213, 3260, 0);
const GNOME_START: (i32, i32, i32) = (2474, 3436, 0);
const GNOME_AFTER_LOG: (i32, i32, i32) = (2474, 3429, 0);
const GNOME_GROUND_RETURN: (i32, i32, i32) = (2487, 3420, 0);
const GNOME_PIPE: (i32, i32, i32) = (2484, 3431, 0);
// Selected 274/289 content: 86.5 XP per full lap, then 7.5 for the next log.
const GNOME_SECOND_LOG_XP: i32 = 94;
const FLAX_FIELD: (i32, i32, i32) = (2741, 3444, 0);
const FALADOR_CHICKENS: (i32, i32, i32) = (3029, 3294, 0);
const EMPTY_VIAL_ID: i32 = 229;
const VIAL_OF_WATER_ID: i32 = 227;
const EYE_OF_NEWT_ID: i32 = 221;
const GUAM_UNF_ID: i32 = 91;
const ATTACK_POTION_3_ID: i32 = 121;
const RANARR_WEED_ID: i32 = 257;
const RANARR_UNF_ID: i32 = 99;
const SNAPE_GRASS_ID: i32 = 231;
const PRAYER_POTION_3_ID: i32 = 139;
const FALADOR_WEST_BANK: (i32, i32, i32) = (2946, 3369, 0);
const FALADOR_EAST_BANK: (i32, i32, i32) = (3013, 3355, 0);
const FALADOR_FOUNTAIN: (i32, i32, i32) = (2949, 3381, 0);
const COW_HIDE_ID: i32 = 1739;
const SOFT_LEATHER_ID: i32 = 1741;
const HARD_LEATHER_ID: i32 = 1743;
const TANNER_IF: i32 = 679;
const SOFT_TAN_ALL_COM: i32 = 8686;
const HARD_TAN_ALL_COM: i32 = 8690;
const SHOPMAIN: i32 = 3824;
const AL_KHARID_BANK: (i32, i32, i32) = (3269, 3167, 0);
const TANNER_STAND: (i32, i32, i32) = (3277, 3191, 0);
const DOMMIK_STAND: (i32, i32, i32) = (3316, 3192, 0);
const RUNE_ESSENCE_ID: i32 = 1436;
const NOTED_ESSENCE_ID: i32 = 1437;
const AIR_TALISMAN_ID: i32 = 1438;
const EARTH_TALISMAN_ID: i32 = 1440;
const AIR_RUNE_ID: i32 = 556;
const EARTH_RUNE_ID: i32 = 557;
const TEMPLE_Z: i32 = 4000;
const VARROCK_EAST_BANK: (i32, i32, i32) = (3253, 3420, 0);
const RUNECRAFTER_AIR_RUINS: (i32, i32, i32) = (2988, 3294, 0);
const RUNECRAFTER_EARTH_RUINS: (i32, i32, i32) = (3303, 3477, 0);
const MULECRAFTER_AIR_RUINS: (i32, i32, i32) = (2983, 3288, 0);
const AIR_ALTAR: (i32, i32, i32) = (2841, 4829, 0);
const EARTH_ALTAR: (i32, i32, i32) = (2655, 4830, 0);
const CAKE_ID: i32 = 1891;
const BREAD_ID: i32 = 2309;
const CHOCOLATE_SLICE_ID: i32 = 1901;
const CHOCOLATE_CAKE_ID: i32 = 1897;
const NOTED_CAKE_ID: i32 = 1892;
const NOTED_BREAD_ID: i32 = 2310;
const NOTED_CHOCOLATE_SLICE_ID: i32 = 1902;
const ARDY_CAKES_STAND: (i32, i32, i32) = (2668, 3312, 0);
const ARDY_THIEVER_STAND: (i32, i32, i32) = (2661, 3306, 0);
const ARDY_BANK: (i32, i32, i32) = (2655, 3286, 0);
const MAGIC_LOGS_ID: i32 = 1513;
const NOTED_MAGIC_LOGS_ID: i32 = 1514;
const UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 72;
const UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 70;
const MAGIC_SHORTBOW_ID: i32 = 861;
const MAGIC_LONGBOW_ID: i32 = 859;
const NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 73;
const NOTED_UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 71;
const KNIFE_ID: i32 = 946;
const STEEL_AXE_ID: i32 = 1353;
const STEEL_PICKAXE_ID: i32 = 1269;
const NOTED_COAL_ID: i32 = 454;
const GNOME_WEST_MAGICS: (i32, i32, i32) = (2372, 3425, 0);
const GNOME_BANK_STAND: (i32, i32, i32) = (2445, 3425, 1);
const GNOME_BANK_STAIR_SOUTH: (i32, i32, i32) = (2444, 3416, 0);
const COAL_MINE: (i32, i32, i32) = (2582, 3481, 0);
const COAL_MINE_TRUCK_STAND: (i32, i32, i32) = (2575, 3486, 0);
const SEERS_BANK: (i32, i32, i32) = (2725, 3491, 0);
const RAW_SALMON_ID: i32 = 331;
const SALMON_ID: i32 = 329;
const NOTED_RAW_SALMON_ID: i32 = 332;
const NOTED_SALMON_ID: i32 = 330;
const RAW_LOBSTER_ID: i32 = 377;
const LOBSTER_ID: i32 = 379;
const NOTED_RAW_LOBSTER_ID: i32 = 378;
const NOTED_LOBSTER_ID: i32 = 380;
const BURNT_FISH_1_ID: i32 = 323;
const BURNT_FISH_2_ID: i32 = 343;
const BURNT_LOBSTER_ID: i32 = 381;
const NOTED_COPPER_ORE_ID: i32 = 437;
const NOTED_TIN_ORE_ID: i32 = 439;
const NOTED_IRON_ORE_ID: i32 = 441;
const NOTED_BRONZE_BAR_ID: i32 = 2350;
const NOTED_STEEL_BAR_ID: i32 = 2354;
const BOW_STRING_ID: i32 = 1777;
const NOTED_FLAX_ID: i32 = 1780;
const NOTED_BOW_STRING_ID: i32 = 1778;
const BALL_OF_WOOL_ID: i32 = 1759;
const COOKING_FIXTURE_LEVEL: i32 = 80;
const CATHERBY_BANK: (i32, i32, i32) = (2809, 3441, 0);
const CATHERBY_RANGE_STAND: (i32, i32, i32) = (2817, 3443, 0);
const AL_KHARID_FURNACE: (i32, i32, i32) = (3275, 3185, 0);
const FLAX_SPINNER_BANK: (i32, i32, i32) = (2722, 3493, 0);
const FLAX_SPINNER_WHEEL: (i32, i32, i32) = (2711, 3471, 1);
const FLAX_AIO_BANK: (i32, i32, i32) = (2725, 3493, 0);
const RED_SPIDERS_EGGS_ID: i32 = 223;
const NOTED_RED_SPIDERS_EGGS_ID: i32 = 224;
const NOTED_EYE_OF_NEWT_ID: i32 = 222;
const EGG_FIELD: (i32, i32, i32) = (3120, 9952, 0);
const BETTY_SHOP: (i32, i32, i32) = (3012, 3259, 0);
const TROUT_ID: i32 = 333;
const BIG_BONES_ID: i32 = 532;
const NOTED_BIG_BONES_ID: i32 = 533;
const LIMPWURT_ROOT_ID: i32 = 225;
const NOTED_LIMPWURT_ROOT_ID: i32 = 226;
const LAW_RUNE_ID: i32 = 563;
const BONES_ID: i32 = 526;
const NOTED_BONES_ID: i32 = 527;
const NOTED_HERB_ID: i32 = 200;
const LANTADYME_HERB_ID: i32 = 2485;
const NOTED_LANTADYME_HERB_ID: i32 = 2486;
const CHAOS_DRUID_FIELD: (i32, i32, i32) = (3110, 9936, 0);
const MOSS_GIANT_SAFESPOT: (i32, i32, i32) = (2553, 3406, 0);
const HILL_GIANT_PIT: (i32, i32, i32) = (3110, 9832, 0);
const CHAOS_DRUID_FOOD: i32 = 12;
const MOSS_GIANT_FOOD: i32 = 10;
const HILL_GIANT_FOOD: i32 = 8;
const AUTO_FIGHTER_FOOD: i32 = 8;
const ROCK_CRAB_FOOD: i32 = 8;
const GREEN_DRAGON_FOOD: i32 = 12;
const FIRE_GIANT_FOOD: i32 = 12;
const COMBAT_ATTACK_LEVEL: i32 = 40;
const RUNE_SCIMITAR_ID: i32 = 1333;
const DRAGONFIRE_SHIELD_ID: i32 = 1540;
const NOTED_DRAGONFIRE_SHIELD_ID: i32 = 1541;
const DRAGON_BONES_ID: i32 = 536;
const NOTED_DRAGON_BONES_ID: i32 = 537;
const GREEN_DRAGONHIDE_ID: i32 = 1753;
const NOTED_GREEN_DRAGONHIDE_ID: i32 = 1754;
const BLACK_DRAGONHIDE_ID: i32 = 1747;
const RED_DRAGONHIDE_ID: i32 = 1749;
const BLUE_DRAGONHIDE_ID: i32 = 1751;
const GLARIALS_AMULET_ID: i32 = 295;
const ROPE_ID: i32 = 954;
const ROCK_CRAB_SPOT: (i32, i32, i32) = (2704, 3726, 0);
const GREEN_DRAGON_FIELD: (i32, i32, i32) = (3096, 3814, 0);
const FIRE_GIANT_ROOM: (i32, i32, i32) = (2575, 9893, 0);
const WILDERNESS_MIN_Z: i32 = 3520;
const DUNGEON_MIN_Z: i32 = 9000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CoreCase {
    BoneBurier,
    ChickenKiller,
    ChickenKillerBank,
    Thiever,
    Alcher,
    AlcherCustom,
    AlcherCustomAlias,
    AlcherCustomName,
    AlcherOrdered,
    AlcherLargeBatch,
    BankFletcher,
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
    ArdyThiever,
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
    MossGiant,
    HillGiant,
    AutoFighter,
    RockCrab,
    GreenDragon,
    FireGiant,
    ArdyFighter,
}

impl CoreCase {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "bone_burier" => Ok(Self::BoneBurier),
            "chicken_killer" => Ok(Self::ChickenKiller),
            "chicken_killer_bank" => Ok(Self::ChickenKillerBank),
            "thiever" => Ok(Self::Thiever),
            "alcher" => Ok(Self::Alcher),
            "alcher_custom" => Ok(Self::AlcherCustom),
            "alcher_custom_alias" => Ok(Self::AlcherCustomAlias),
            "alcher_custom_name" => Ok(Self::AlcherCustomName),
            "alcher_ordered" => Ok(Self::AlcherOrdered),
            "alcher_large_batch" => Ok(Self::AlcherLargeBatch),
            "bank_fletcher" => Ok(Self::BankFletcher),
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
            "ardy_thiever" => Ok(Self::ArdyThiever),
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
            "moss_giant" => Ok(Self::MossGiant),
            "hill_giant" => Ok(Self::HillGiant),
            "auto_fighter" => Ok(Self::AutoFighter),
            "rock_crab" => Ok(Self::RockCrab),
            "green_dragon" => Ok(Self::GreenDragon),
            "fire_giant" => Ok(Self::FireGiant),
            "ardy_fighter" => Ok(Self::ArdyFighter),
            _ => Err(format!(
                "unknown CATALOG_SCENARIO {value:?}; expected {CORE_SCENARIOS}"
            )),
        }
    }

    fn scenario_name(self) -> &'static str {
        match self {
            Self::BoneBurier => "bone_burier",
            Self::ChickenKiller => "chicken_killer",
            Self::ChickenKillerBank => "chicken_killer_bank",
            Self::Thiever => "thiever",
            Self::Alcher => "alcher",
            Self::AlcherCustom => "alcher_custom",
            Self::AlcherCustomAlias => "alcher_custom_alias",
            Self::AlcherCustomName => "alcher_custom_name",
            Self::AlcherOrdered => "alcher_ordered",
            Self::AlcherLargeBatch => "alcher_large_batch",
            Self::BankFletcher => "bank_fletcher",
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
            Self::ArdyThiever => "ardy_thiever",
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
            Self::MossGiant => "moss_giant",
            Self::HillGiant => "hill_giant",
            Self::AutoFighter => "auto_fighter",
            Self::RockCrab => "rock_crab",
            Self::GreenDragon => "green_dragon",
            Self::FireGiant => "fire_giant",
            Self::ArdyFighter => "ardy_fighter",
        }
    }

    fn card_name(self) -> &'static str {
        match self {
            Self::BoneBurier => "BoneBurier",
            Self::ChickenKiller | Self::ChickenKillerBank => "ChickenKiller",
            Self::Thiever => "Thiever",
            Self::Alcher => "Alcher",
            Self::AlcherCustom
            | Self::AlcherCustomAlias
            | Self::AlcherCustomName
            | Self::AlcherOrdered
            | Self::AlcherLargeBatch => "Alcher",
            Self::BankFletcher | Self::BankFletcherString | Self::BankFletcherCutString => {
                "BankFletcher"
            }
            Self::DartFletcher | Self::DartFletcherIron => "DartFletcher",
            Self::HerbCleaner | Self::HerbCleanerNamed => "HerbCleaner",
            Self::GemCutter | Self::GemCutterNamed => "GemCutter",
            Self::DoorOpener | Self::DoorOpenerGate => "DoorOpener",
            Self::GnomeCourse | Self::GnomeCourseRadius => "GnomeCourse",
            Self::FlaxPicker => "FlaxPicker",
            Self::Superheater | Self::SuperheaterSteel | Self::SuperheaterFireBattlestaff => {
                "Superheater"
            }
            Self::VialFiller | Self::VialFillerEast => "VialFiller",
            Self::PotionMaker | Self::PotionMakerNamed => "PotionMaker",
            Self::TannerBot | Self::TannerBotHard => "TannerBot",
            Self::RuneCrafter | Self::RuneCrafterEarth => "RuneCrafter",
            Self::MuleCrafter => "MuleCrafter",
            Self::ArdyCakes => "ArdyCakes",
            Self::ArdyThiever | Self::ArdyThieverKnight => "ArdyThiever",
            Self::GnomeChop | Self::GnomeFletchShort | Self::GnomeFletchLong => "GnomeMagicChopper",
            Self::CoalTrucks => "CoalTrucks",
            Self::CookBot | Self::CookBotLobster => "CookBot",
            Self::SmelterBot | Self::SmelterBotSteel => "SmelterBot",
            Self::FlaxSpinner => "FlaxSpinner",
            Self::FlaxAio | Self::FlaxAioPick | Self::FlaxAioSpin => "FlaxAIO",
            Self::HerbloreSecondaries | Self::HerbloreSecondariesNewt => "HerbloreSecondaries",
            Self::ChaosDruid => "ChaosDruidKiller",
            Self::MossGiant => "MossGiant",
            Self::HillGiant => "HillGiant",
            Self::AutoFighter => "AutoFighter",
            Self::RockCrab => "RockCrab",
            Self::GreenDragon => "GreenDragon",
            Self::FireGiant => "FireGiant",
            Self::ArdyFighter => "ArdyFighter",
        }
    }
}

fn validate_case_catalog(case: CoreCase, commit: &str) -> Result<(), String> {
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

#[derive(Debug, Deserialize)]
struct SupportMatrix {
    catalogs: Vec<CatalogLedger>,
    rows: Vec<CardLedger>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogLedger {
    commit: String,
    identity: String,
    read_only_path: String,
    registry_path: String,
    registry_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CardLedger {
    display_name: String,
    catalog_commit: String,
    source_path: String,
    source_sha256: String,
    revision: u16,
}

fn support_matrix() -> Result<SupportMatrix, String> {
    serde_json::from_str(SUPPORT_MATRIX).map_err(|error| format!("support matrix: {error}"))
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "CATALOG_COMMIT must be the exact lowercase 40-hex frozen commit, got {commit:?}"
        ));
    }
    Ok(())
}

fn catalog_ledger<'a>(
    matrix: &'a SupportMatrix,
    commit: &str,
) -> Result<&'a CatalogLedger, String> {
    validate_commit(commit)?;
    let rows = matrix
        .catalogs
        .iter()
        .filter(|catalog| catalog.commit == commit)
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [catalog] => Ok(*catalog),
        [] => Err(format!(
            "CATALOG_COMMIT {commit} is not frozen in support-matrix.json"
        )),
        _ => Err(format!(
            "CATALOG_COMMIT {commit} is duplicated in support-matrix.json"
        )),
    }
}

fn ledger_card(
    matrix: &SupportMatrix,
    commit: &str,
    revision: u16,
    case: CoreCase,
) -> Result<CardLedger, String> {
    let rows = matrix
        .rows
        .iter()
        .filter(|row| {
            row.catalog_commit == commit
                && row.revision == revision
                && row.display_name == case.card_name()
        })
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [row] => Ok(row.clone()),
        [] => Err(format!(
            "support matrix has no {} row for catalog {commit} revision {revision}",
            case.card_name()
        )),
        _ => Err(format!(
            "support matrix has duplicate {} rows for catalog {commit} revision {revision}",
            case.card_name()
        )),
    }
}

fn safe_catalog_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe support-matrix catalog path {relative:?}"));
    }
    Ok(root.join(relative))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(script::JsCache::origin_sha(&bytes))
}

fn verify_source_identity(root: &Path, row: &CardLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &row.source_path)?;
    let actual = hash_file(&path)?;
    if actual != row.source_sha256 {
        return Err(format!(
            "source SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            row.source_sha256,
            actual
        ));
    }
    Ok(path)
}

fn verify_registry_identity(root: &Path, catalog: &CatalogLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &catalog.registry_path)?;
    let actual = hash_file(&path)?;
    if actual != catalog.registry_sha256 {
        return Err(format!(
            "registry SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            catalog.registry_sha256,
            actual
        ));
    }
    Ok(path)
}

#[derive(Debug, Clone, Serialize)]
struct Observation {
    ingame: bool,
    scene_state: i32,
    player: Option<String>,
    tile: Option<(i32, i32, i32)>,
    tick: u32,
    items: BTreeMap<String, i32>,
    item_ids: BTreeMap<i32, i32>,
    bank: BTreeMap<String, i32>,
    bank_ids: BTreeMap<i32, i32>,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: u64,
    levels: BTreeMap<String, i32>,
    xp: BTreeMap<String, i32>,
    chat: Vec<(i32, String)>,
    loc_facts: Vec<BoundedLoc>,
    npc_facts: Vec<BoundedNpc>,
    ground_loot: Vec<BoundedGround>,
    local_in_combat: bool,
    local_target_npc: Option<usize>,
    local_health: i32,
    local_animation: i32,
    equipment_ids: BTreeMap<i32, i32>,
    main_modal: i32,
    widget_ids: BTreeSet<i32>,
}

/// One loc retained for these named cases. The live loc sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BoundedLoc {
    id: i32,
    x: i32,
    z: i32,
    level: i32,
    name: Option<String>,
    open: bool,
}

/// Compact NPC identity used by combat cores. The live NPC sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BoundedNpc {
    index: usize,
    name: Option<String>,
    health: i32,
    total_health: i32,
    animation: i32,
    in_combat: bool,
    targeting_local: bool,
    tile: (i32, i32, i32),
    distance: i32,
}

/// Compact ground loot of combat-core item ids. The live ground sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BoundedGround {
    id: i32,
    count: i32,
    tile: (i32, i32, i32),
    distance: i32,
}

impl Observation {
    fn from_snapshot(snapshot: &GameSnapshot, names: &ObjNames) -> Self {
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
            tick: snapshot.tick(),
            items,
            item_ids,
            bank,
            bank_ids,
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_generation: snapshot.bank_session_generation(),
            levels,
            xp,
            chat,
            loc_facts,
            npc_facts,
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

    fn item(&self, name: &str) -> i32 {
        self.items.get(name).copied().unwrap_or(0)
    }

    fn skill_xp(&self, name: &str) -> i32 {
        self.xp.get(name).copied().unwrap_or(0)
    }

    fn item_id(&self, id: i32) -> i32 {
        self.item_ids.get(&id).copied().unwrap_or(0)
    }

    fn bank_item(&self, name: &str) -> i32 {
        self.bank.get(name).copied().unwrap_or(0)
    }

    fn bank_item_id(&self, id: i32) -> i32 {
        self.bank_ids.get(&id).copied().unwrap_or(0)
    }

    fn level(&self, name: &str) -> i32 {
        self.levels.get(name).copied().unwrap_or(0)
    }

    fn equipment_id(&self, id: i32) -> i32 {
        self.equipment_ids.get(&id).copied().unwrap_or(0)
    }

    fn has_widget(&self, id: i32) -> bool {
        self.widget_ids.contains(&id)
    }
}

fn combat_npc_name(name: Option<&str>) -> bool {
    matches!(
        name.map(str::trim),
        Some("Chaos druid" | "Moss giant" | "Giant" | "Guard")
    )
}

fn unidentified_herb_id(id: i32) -> bool {
    matches!(
        id,
        199 | 201 | 203 | 205 | 207 | 209 | 211 | 213 | 215 | 217 | 219 | LANTADYME_HERB_ID
    )
}

fn noted_herb_id(id: i32) -> bool {
    matches!(
        id,
        200 | 202 | 204 | 206 | 208 | 210 | 212 | 214 | 216 | 218 | 220 | NOTED_LANTADYME_HERB_ID
    )
}

fn combat_ground_id(id: i32) -> bool {
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

fn keep_bounded_loc(id: i32, name: Option<&str>) -> bool {
    matches!(
        id,
        WOODEN_DOOR_CLOSED_ID | WOODEN_DOOR_OPEN_ID | WOODEN_GATE_CLOSED_ID | WOODEN_GATE_OPEN_ID
    ) || name.is_some_and(|name| {
        let n = name.trim().to_ascii_lowercase();
        n == "door" || n.ends_with(" door") || n.contains("gate")
    })
}

fn loc_name_matches(name: Option<&str>, gate: bool) -> bool {
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

fn loc_at(
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

fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

fn stall_food(observation: &Observation) -> i32 {
    observation.item_id(CAKE_ID)
        + observation.item_id(BREAD_ID)
        + observation.item_id(CHOCOLATE_SLICE_ID)
}

fn bank_stall_food(observation: &Observation) -> i32 {
    observation.bank_item_id(CAKE_ID)
        + observation.bank_item_id(BREAD_ID)
        + observation.bank_item_id(CHOCOLATE_SLICE_ID)
}

fn noted_stall_food(observation: &Observation) -> i32 {
    observation.item_id(NOTED_CAKE_ID)
        + observation.item_id(NOTED_BREAD_ID)
        + observation.item_id(NOTED_CHOCOLATE_SLICE_ID)
        + observation.bank_item_id(NOTED_CAKE_ID)
        + observation.bank_item_id(NOTED_BREAD_ID)
        + observation.bank_item_id(NOTED_CHOCOLATE_SLICE_ID)
}

fn ardy_thiever_baseline_ready(baseline: &Observation, thieving: i32) -> bool {
    near(baseline.tile, ARDY_THIEVER_STAND, 8)
        && baseline.level("thieving") >= thieving
        && baseline.item_id(COINS_ID) == 0
        && baseline.item_id(CAKE_ID) == 0
}

fn held_id(observation: &Observation, id: i32) -> i32 {
    observation.item_id(id) + observation.equipment_id(id)
}

fn gnome_wrong_bows(observation: &Observation) -> bool {
    observation.item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(MAGIC_LONGBOW_ID) > 0
}

fn gnome_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.bank_item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
}

fn gnome_chop_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, GNOME_WEST_MAGICS, 8)
        && baseline.level("woodcutting") >= 75
        && held_id(baseline, STEEL_AXE_ID) >= 1
        && baseline.item_id(MAGIC_LOGS_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) == 0
        && !gnome_wrong_bows(baseline)
        && !gnome_noted(baseline)
}

fn gnome_fletch_baseline_ready(baseline: &Observation, fletching: i32, max: Option<i32>) -> bool {
    gnome_chop_baseline_ready(baseline)
        && baseline.level("fletching") >= fletching
        && max
            .map(|max| baseline.level("fletching") <= max)
            .unwrap_or(true)
        && baseline.item_id(KNIFE_ID) >= 1
}

fn coal_trucks_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, COAL_MINE, 8)
        && baseline.level("mining") >= 30
        && held_id(baseline, STEEL_PICKAXE_ID) >= 1
        && baseline.item_id(COAL_ID) == 0
        && baseline.item_id(NOTED_COAL_ID) == 0
        && baseline.bank_item_id(COAL_ID) == 0
}

fn cook_wrong_or_burnt(observation: &Observation, wrong: i32) -> bool {
    observation.item_id(wrong) > 0
        || observation.bank_item_id(wrong) > 0
        || observation.item_id(BURNT_FISH_1_ID) > 0
        || observation.bank_item_id(BURNT_FISH_1_ID) > 0
        || observation.item_id(BURNT_FISH_2_ID) > 0
        || observation.bank_item_id(BURNT_FISH_2_ID) > 0
        || observation.item_id(BURNT_LOBSTER_ID) > 0
        || observation.bank_item_id(BURNT_LOBSTER_ID) > 0
}

fn cook_noted(observation: &Observation, noted_raw: i32, noted_product: i32) -> bool {
    observation.item_id(noted_raw) > 0
        || observation.bank_item_id(noted_raw) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

fn cook_bot_baseline_ready(
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

fn smelter_noted(
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

fn smelter_bot_baseline_ready(
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

fn flax_spinner_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_FLAX_ID) > 0
        || observation.bank_item_id(NOTED_FLAX_ID) > 0
        || observation.item_id(NOTED_BOW_STRING_ID) > 0
        || observation.bank_item_id(NOTED_BOW_STRING_ID) > 0
}

fn flax_spinner_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_SPINNER_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

fn flax_aio_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_FIELD, 6)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

fn flax_aio_pick_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_FIELD, 6)
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && !flax_spinner_noted(baseline)
}

fn flax_aio_spin_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_AIO_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

fn herblore_eggs_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_RED_SPIDERS_EGGS_ID) > 0
        || observation.bank_item_id(NOTED_RED_SPIDERS_EGGS_ID) > 0
}

fn herblore_newt_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_EYE_OF_NEWT_ID) > 0
        || observation.bank_item_id(NOTED_EYE_OF_NEWT_ID) > 0
}

fn herblore_eggs_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, EGG_FIELD, 8)
        && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
        && baseline.item_id(EYE_OF_NEWT_ID) == 0
        && !herblore_eggs_noted(baseline)
}

fn herblore_newt_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, BETTY_SHOP, 6)
        && baseline.item_id(EYE_OF_NEWT_ID) == 0
        && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
        && !herblore_newt_noted(baseline)
}

fn superheater_baseline_ready(
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

fn in_temple(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 > TEMPLE_Z)
}

fn overworld(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 <= TEMPLE_Z)
}

fn runecraft_baseline_ready(
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

fn validate_case_baseline(case: CoreCase, baseline: &Observation) -> Result<(), String> {
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
                && stall_food(baseline) == 0
                && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
                && noted_stall_food(baseline) == 0
        }
        CoreCase::ArdyThiever => ardy_thiever_baseline_ready(baseline, 40),
        CoreCase::ArdyThieverKnight => ardy_thiever_baseline_ready(baseline, 55),
        CoreCase::GnomeChop => gnome_chop_baseline_ready(baseline),
        CoreCase::GnomeFletchShort => gnome_fletch_baseline_ready(baseline, 80, Some(84)),
        CoreCase::GnomeFletchLong => gnome_fletch_baseline_ready(baseline, 85, None),
        CoreCase::CoalTrucks => coal_trucks_baseline_ready(baseline),
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
        | CoreCase::MossGiant
        | CoreCase::HillGiant
        | CoreCase::AutoFighter
        | CoreCase::RockCrab
        | CoreCase::GreenDragon
        | CoreCase::FireGiant
        | CoreCase::ArdyFighter => {
            combat_spec(case).is_some_and(|spec| combat_baseline_ready(baseline, spec))
        }
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
        CoreCase::Alcher
        | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => "Varrock West bank and Magic 55",
        CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
            "Varrock West bank, Magic 55, and no seeded coins/custom-target outcome"
        }
        CoreCase::BankFletcher => {
            "Varrock West bank, Knife, twenty-seven Willow logs, and Fletching 35"
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
            "Baker's stall stand (2668,3312,0), Thieving 5, and empty pack of 1891/2309/1901/1897"
        }
        CoreCase::ArdyThiever => {
            "Ardougne Guard stand (2661,3306,0), Thieving 40, and empty pack of 995/1891"
        }
        CoreCase::ArdyThieverKnight => {
            "Ardougne Knight stand (2661,3306,0), Thieving 55, and empty pack of 995/1891"
        }
        CoreCase::GnomeChop => {
            "west magics (2372,3425,0), Woodcutting 75, steel axe 1353, empty pack of 1513/72/70"
        }
        CoreCase::GnomeFletchShort => {
            "west magics, Woodcutting 75, Fletching 80-84, knife 946, steel axe, empty pack of 1513/72"
        }
        CoreCase::GnomeFletchLong => {
            "west magics, Woodcutting 75, Fletching 85, knife 946, steel axe, empty pack of 1513/70"
        }
        CoreCase::CoalTrucks => {
            "coal mine (2582,3481,0), Mining 30, steel pickaxe 1269, and empty pack of 453"
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
        CoreCase::MossGiant => {
            "Moss safespot (2553,3406,0), Attack/Strength/Hitpoints 40, lobster 10, scimitar 1331, empty big bones 532"
        }
        CoreCase::HillGiant => {
            "Giant pit (3110,9832,0), Attack/Strength/Hitpoints 40, trout 8, scimitar 1331, empty 532/225"
        }
        CoreCase::AutoFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, trout 8, scimitar 1331, banking None"
        }
        CoreCase::RockCrab => {
            "Rock crab spot (2704,3726,0), Attack/Strength/Hitpoints 40, lobster 8, scimitar 1331, bank Off"
        }
        CoreCase::GreenDragon => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, lobster 12, rune scimitar 1333, worn shield 1540, empty 536/1753"
        }
        CoreCase::FireGiant => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, amulet 295, rope 954, empty 532"
        }
        CoreCase::ArdyFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, Thieving 5, scimitar 1331, empty cake/bread/slice, bank Off"
        }
    };
    Err(format!(
        "{} Start baseline lacks required preparation ({requirement}): {baseline:?}",
        case.scenario_name()
    ))
}

#[derive(Debug, Clone, Serialize)]
struct CoreWitness {
    case: CoreCase,
    baseline: Observation,
    latest: Observation,
    max_items: BTreeMap<String, i32>,
    max_xp: BTreeMap<String, i32>,
    saw_bury_chat: bool,
    post_start_observations: u64,
    bone_bank_cycle: BoneBankCycle,
    bank_fletcher_cycle: BankFletcherCycle,
    bank_fletcher_string_cycle: BankFletcherStringCycle,
    bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle,
    alcher_generated_custom_cycle: AlcherGeneratedCustomCycle,
    dart_fletcher_cycle: DartFletcherCycle,
    herb_cleaner_cycle: HerbCleanerCycle,
    gem_cutter_cycle: GemCutterCycle,
    door_opener_cycle: DoorOpenerCycle,
    gnome_course_cycle: GnomeCourseCycle,
    flax_picker_cycle: FlaxPickerCycle,
    superheater_cycle: SuperheaterCycle,
    chicken_killer_bank_cycle: ChickenKillerBankCycle,
    vial_filler_cycle: VialFillerCycle,
    potion_maker_cycle: PotionMakerCycle,
    tanner_bot_cycle: TannerBotCycle,
    rune_crafter_cycle: RuneCrafterCycle,
    ardy_cakes_cycle: ArdyCakesCycle,
    ardy_thiever_cycle: ArdyThieverCycle,
    gnome_chop_cycle: GnomeChopCycle,
    gnome_fletch_cycle: GnomeFletchCycle,
    coal_trucks_cycle: CoalTrucksCycle,
    station_production_cycle: StationProductionCycle,
    flax_aio_cycle: FlaxAioCycle,
    flax_aio_pick_cycle: FlaxAioPickCycle,
    herblore_eggs_cycle: HerbloreEggsCycle,
    herblore_newt_cycle: HerbloreNewtCycle,
    combat_core_cycle: CombatCoreCycle,
    ordered_first_exhausted: bool,
}

/// Ordered observations: seed depletion alone must never qualify this card.
#[derive(Debug, Clone, Default, Serialize)]
struct BoneBankCycle {
    first_batch_buried: bool,
    opened: Option<Observation>,
    withdrawn: Option<Observation>,
    buried_after_withdrawal: bool,
}

impl BoneBankCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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
struct BankFletcherCycle {
    first_pack_created: bool,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    crafted_after_withdrawal: bool,
}

impl BankFletcherCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

#[derive(Debug, Clone, Default, Serialize)]
struct BankFletcherStringCycle {
    initial_pairs_strung: bool,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    strung_after_withdrawal: bool,
}

impl BankFletcherStringCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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
struct BankFletcherCutStringCycle {
    cut_pair_created: bool,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    string_pair_created: bool,
}

impl BankFletcherCutStringCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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
struct AlcherGeneratedCustomCycle {
    withdrawn: Option<Observation>,
    consumed: bool,
}

impl AlcherGeneratedCustomCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(CERT_ADAMANT_SCIMITAR_ID) >= 1
            && now.item_id(ADAMANT_SCIMITAR_ID) == 0
            && now.item_id(CERT_ADAMANT_SCIMITAR_ID) > baseline.item_id(CERT_ADAMANT_SCIMITAR_ID)
            && now.item_id(NATURE_RUNE_ID) >= 1
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.consumed |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(CERT_ADAMANT_SCIMITAR_ID)
                    < withdrawn.item_id(CERT_ADAMANT_SCIMITAR_ID)
                && now.item_id(NATURE_RUNE_ID) < withdrawn.item_id(NATURE_RUNE_ID)
                && now.item_id(COINS_ID) - baseline.item_id(COINS_ID)
                    == ADAMANT_SCIMITAR_ALCH_COINS
                && now.skill_xp("magic") - baseline.skill_xp("magic") >= HIGH_ALCH_MAGIC_XP
                && now.item("Rune chainbody") == withdrawn.item("Rune chainbody");
        }
    }
}

/// Two observed dart actions: exact product id, both inputs down, XP, no wrong tier.
#[derive(Debug, Clone, Default, Serialize)]
struct DartFletcherCycle {
    first: Option<Observation>,
    further: bool,
    wrong_tier: bool,
}

impl DartFletcherCycle {
    fn observe(
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

    fn qualified(&self) -> bool {
        self.further && !self.wrong_tier
    }
}

/// Identify a full pack, deposit script-created output, restock, then identify again.
#[derive(Debug, Clone, Default, Serialize)]
struct HerbCleanerCycle {
    first_pack: bool,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    cleaned_after_withdrawal: bool,
    filter_violated: bool,
}

impl HerbCleanerCycle {
    fn observe(&mut self, named: bool, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.cleaned_after_withdrawal && !self.filter_violated
    }
}

/// Cut a chisel-kept pack, deposit except chisel, restock, then cut again.
#[derive(Debug, Clone, Default, Serialize)]
struct GemCutterCycle {
    first_pack: bool,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    cut_after_withdrawal: bool,
    filter_violated: bool,
}

impl GemCutterCycle {
    fn observe(&mut self, named: bool, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.cut_after_withdrawal && !self.filter_violated
    }
}

/// Selected shut loc must lose Open through a same-session world change.
#[derive(Debug, Clone, Default, Serialize)]
struct DoorOpenerCycle {
    selected: Option<BoundedLoc>,
    opened: bool,
}

impl DoorOpenerCycle {
    fn observe(
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

    fn qualified(&self) -> bool {
        self.selected.as_ref().is_some_and(|loc| loc.open) && self.opened
    }
}

/// Ordered plane/tile/XP milestones from selected gnome_course.rs2 dests.
#[derive(Debug, Clone, Default, Serialize)]
struct GnomeCourseCycle {
    log: Option<Observation>,
    ground_return: Option<Observation>,
    pipe: Option<Observation>,
    second_lap: bool,
}

impl GnomeCourseCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.second_lap
    }
}

/// Full pack of exact flax 1779, Seers deposit, return, further pick.
#[derive(Debug, Clone, Default, Serialize)]
struct FlaxPickerCycle {
    first_pack: bool,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
}

impl FlaxPickerCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further
    }
}

/// Pick 1779 at the field, convert at the wheel with Crafting XP, deposit 1777,
/// closed return to the field, further Pick. Wool or noted ids fail.
#[derive(Debug, Clone, Default, Serialize)]
struct FlaxAioCycle {
    picked: Option<Observation>,
    produced: Option<Observation>,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

impl FlaxAioCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
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
struct FlaxAioPickCycle {
    first_pack: bool,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

impl FlaxAioPickCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.noted
    }
}

/// Ground Take 223, deposit, closed return to the egg field, further Take.
/// Eye of newt 221 must not mix into this cell. Ground spawns are not given.
#[derive(Debug, Clone, Default, Serialize)]
struct HerbloreEggsCycle {
    taken: bool,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

impl HerbloreEggsCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.noted
    }
}

/// Betty shop purchase 221 with coins 995, deposit, return, further buy.
/// Eggs 223 must not mix into this cell.
#[derive(Debug, Clone, Default, Serialize)]
struct HerbloreNewtCycle {
    bought: bool,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    coins_spent: bool,
    coins_peak: i32,
    wrong_product: bool,
    noted: bool,
}

impl HerbloreNewtCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further && self.coins_spent && !self.wrong_product && !self.noted
    }
}

#[derive(Debug, Clone, Copy)]
struct CombatSpec {
    target: &'static str,
    stand: (i32, i32, i32),
    radius: i32,
    food_id: i32,
    food_count: i32,
    weapon_id: i32,
    loot: CombatLoot,
    extra: CombatExtra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombatLoot {
    HerbLawNature,
    BigBones,
    BigBonesOrLimpwurt,
    DragonBonesOrHide,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombatExtra {
    None,
    RockActivation,
    WornShield,
    DungeonAmulet,
    StolenFood,
}

fn combat_spec(case: CoreCase) -> Option<CombatSpec> {
    match case {
        CoreCase::ChaosDruid => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_FIELD,
            radius: 14,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
        }),
        CoreCase::MossGiant => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
        }),
        CoreCase::HillGiant => Some(CombatSpec {
            target: "Giant",
            stand: HILL_GIANT_PIT,
            radius: 16,
            food_id: TROUT_ID,
            food_count: HILL_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::BigBonesOrLimpwurt,
            extra: CombatExtra::None,
        }),
        CoreCase::AutoFighter => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
        }),
        CoreCase::RockCrab => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SPOT,
            radius: 4,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::None,
            extra: CombatExtra::RockActivation,
        }),
        CoreCase::GreenDragon => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
        }),
        CoreCase::FireGiant => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
        }),
        CoreCase::ArdyFighter => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 12,
            food_id: CAKE_ID,
            food_count: 0,
            weapon_id: ADAMANT_SCIMITAR_ID,
            loot: CombatLoot::None,
            extra: CombatExtra::StolenFood,
        }),
        _ => None,
    }
}

fn combat_noted(observation: &Observation) -> bool {
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

fn noted_herb_id_held(observation: &Observation) -> bool {
    observation
        .item_ids
        .keys()
        .copied()
        .chain(observation.bank_ids.keys().copied())
        .any(noted_herb_id)
}

fn combat_loot_count(observation: &Observation, loot: CombatLoot) -> i32 {
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
        CombatLoot::None => 0,
    }
}

fn combat_baseline_ready(baseline: &Observation, spec: CombatSpec) -> bool {
    let food_ok = if spec.food_count == 0 {
        true
    } else {
        baseline.item_id(spec.food_id) >= spec.food_count
    };
    let extra_ok = match spec.extra {
        CombatExtra::None | CombatExtra::RockActivation => true,
        CombatExtra::WornShield => {
            baseline.equipment_id(DRAGONFIRE_SHIELD_ID) >= 1
                && held_id(baseline, DRAGONFIRE_SHIELD_ID) >= 1
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
    near(baseline.tile, spec.stand, spec.radius)
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && food_ok
        && held_id(baseline, spec.weapon_id) >= 1
        && combat_loot_count(baseline, spec.loot) == 0
        && !combat_noted(baseline)
        && extra_ok
}

#[derive(Debug, Clone, Default, Serialize)]
struct CombatNpcLast {
    name: String,
    health: i32,
    animation: i32,
    tile: (i32, i32, i32),
    engaged: bool,
    defeated: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CombatFact {
    kind: &'static str,
    index: usize,
    name: String,
    health: i32,
    animation: i32,
    loot_id: Option<i32>,
    zero_health: bool,
}

/// Sustained selected-target combat: two engagements, a verified defeat
/// that is not mere despawn, selected-style XP, and exact loot where required.
#[derive(Debug, Clone, Default, Serialize)]
struct CombatCoreCycle {
    engagements: u32,
    defeats: u32,
    style_xp: bool,
    looted: bool,
    noted: bool,
    activated: bool,
    shield_worn: bool,
    stolen_food: bool,
    wrong_item: bool,
    dormant_indexes: BTreeSet<usize>,
    dormant_tiles: BTreeMap<usize, (i32, i32, i32)>,
    last: BTreeMap<usize, CombatNpcLast>,
    currently_engaged: BTreeSet<usize>,
    facts: Vec<CombatFact>,
}

impl CombatCoreCycle {
    fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        self.noted |= combat_noted(now);
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength");
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
            let combat = npc.in_combat
                || npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            let health_drop = prev
                .as_ref()
                .is_some_and(|prev| prev.health > npc.health && npc.health >= 0 && prev.engaged);
            let new_spawn = prev
                .as_ref()
                .is_some_and(|prev| prev.defeated && npc.health > 0);
            if (combat || health_drop)
                && (!self.currently_engaged.contains(&npc.index) || new_spawn)
            {
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
            if npc.health == 0 && self.currently_engaged.contains(&npc.index) {
                self.record_defeat(npc.index, name, npc.health, npc.animation, None, true);
            }
            self.last.insert(
                npc.index,
                CombatNpcLast {
                    name: name.to_string(),
                    health: npc.health,
                    animation: npc.animation,
                    tile: npc.tile,
                    engaged: self.currently_engaged.contains(&npc.index),
                    defeated: self.last.get(&npc.index).is_some_and(|prev| prev.defeated)
                        || npc.health == 0,
                },
            );
            let _ = prev;
        }
        let missing: Vec<(usize, CombatNpcLast)> = self
            .last
            .iter()
            .filter(|(index, prev)| prev.engaged && !prev.defeated && !seen.contains(index))
            .map(|(index, prev)| (*index, prev.clone()))
            .collect();
        for (index, prev) in missing {
            let loot_id = now.ground_loot.iter().find_map(|item| {
                (item.tile == prev.tile && combat_loot_id(item.id, spec.loot)).then_some(item.id)
            });
            if self.looted || loot_id.is_some() {
                self.record_defeat(
                    index,
                    &prev.name,
                    prev.health,
                    prev.animation,
                    loot_id,
                    false,
                );
            } else {
                self.currently_engaged.remove(&index);
            }
        }
    }

    fn record_defeat(
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

    fn qualified(&self, spec: CombatSpec) -> bool {
        let extra_ok = match spec.extra {
            CombatExtra::None | CombatExtra::DungeonAmulet => true,
            CombatExtra::RockActivation => self.activated,
            CombatExtra::WornShield => self.shield_worn,
            CombatExtra::StolenFood => self.stolen_food,
        };
        self.engagements >= 2
            && self.defeats >= 1
            && self.style_xp
            && (spec.loot == CombatLoot::None || self.looted)
            && !self.noted
            && !self.wrong_item
            && extra_ok
    }
}

fn combat_loot_id(id: i32, loot: CombatLoot) -> bool {
    match loot {
        CombatLoot::HerbLawNature => {
            unidentified_herb_id(id) || id == NATURE_RUNE_ID || id == LAW_RUNE_ID
        }
        CombatLoot::BigBones => id == BIG_BONES_ID,
        CombatLoot::BigBonesOrLimpwurt => id == BIG_BONES_ID || id == LIMPWURT_ROOT_ID,
        CombatLoot::DragonBonesOrHide => id == DRAGON_BONES_ID || id == GREEN_DRAGONHIDE_ID,
        CombatLoot::None => false,
    }
}

/// Superheat a trip, deposit bars except natures, restock ores, then smelt again.
#[derive(Debug, Clone, Default, Serialize)]
struct SuperheaterCycle {
    first_bars: bool,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    produced_after_withdrawal: bool,
    wrong_product: bool,
    wrong_staff: bool,
}

struct SuperheaterSpec {
    bar: i32,
    primary: i32,
    secondary: i32,
    staff: i32,
    steel: bool,
}

impl SuperheaterCycle {
    fn observe(&mut self, spec: SuperheaterSpec, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.produced_after_withdrawal && !self.wrong_product && !self.wrong_staff
    }
}

/// Combat and exact feather loot, then a fresh deposit, return r6, further work.
#[derive(Debug, Clone, Default, Serialize)]
struct ChickenKillerBankCycle {
    combat_loot: bool,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
}

impl ChickenKillerBankCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further
    }
}

/// Fill at the fountain, deposit produced water vials, restock empties, fill again.
#[derive(Debug, Clone, Default, Serialize)]
struct VialFillerCycle {
    filled: Option<Observation>,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    returned: bool,
    further: bool,
}

impl VialFillerCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further
    }
}

/// Staged herb+water to unfinished, secondary to finished, deposit, restock, further product.
#[derive(Debug, Clone, Default, Serialize)]
struct PotionMakerCycle {
    unfinished: Option<Observation>,
    finished: Option<Observation>,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    further: bool,
    wrong_product: bool,
    filter_violated: bool,
}

struct PotionMakerSpec {
    herb: i32,
    unf: i32,
    secondary: i32,
    finished: i32,
    wrong_unf: i32,
    wrong_finished: i32,
    named: bool,
}

impl PotionMakerCycle {
    fn observe(&mut self, spec: PotionMakerSpec, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further && self.finished.is_some() && !self.wrong_product && !self.filter_violated
    }
}

/// Tan at the Tanner widget, deposit produced leather, restock hides, tan again.
#[derive(Debug, Clone, Default, Serialize)]
struct TannerBotCycle {
    widget: Option<Observation>,
    tanned: Option<Observation>,
    deposited: Option<Observation>,
    withdrawn: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    shop: bool,
}

struct TannerBotSpec {
    product: i32,
    wrong_product: i32,
    tan_all: i32,
}

impl TannerBotCycle {
    fn observe(&mut self, spec: TannerBotSpec, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.shop && self.tanned.is_some()
    }
}

/// Withdraw unnoted essence, enter the selected altar, convert to the selected
/// rune with Runecraft XP, portal out, deposit, restock, and craft again.
#[derive(Debug, Clone, Default, Serialize)]
struct RuneCrafterCycle {
    withdrawn: Option<Observation>,
    entered: Option<Observation>,
    crafted: Option<Observation>,
    exited: Option<Observation>,
    deposited: Option<Observation>,
    restocked: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

struct RuneCrafterSpec {
    rune: i32,
    wrong_rune: i32,
    ruins: (i32, i32, i32),
    bank: (i32, i32, i32),
}

impl RuneCrafterCycle {
    fn observe(&mut self, spec: RuneCrafterSpec, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further
            && self.entered.is_some()
            && self.crafted.is_some()
            && self.exited.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Steal cake/bread/chocolate slice with Thieving XP, deposit acquired stock
/// in a fresh bank, return to STAND, steal again. Chocolate cake 1897 and
/// noted stall food fail.
#[derive(Debug, Clone, Default, Serialize)]
struct ArdyCakesCycle {
    stolen: Option<Observation>,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

impl ArdyCakesCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further
            && self.stolen.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Pickpocket coins with Thieving XP, deposit coins at loot-count 1, return
/// to the market stand, pickpocket again. Stall food without coins cannot
/// qualify. Fight stays pending.
#[derive(Debug, Clone, Default, Serialize)]
struct ArdyThieverCycle {
    pickpocketed: Option<Observation>,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
}

impl ArdyThieverCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further && self.pickpocketed.is_some() && self.deposited.is_some()
    }
}

/// Chop magic logs 1513 with Woodcutting XP, deposit upstairs, return to
/// ground, chop again. Strung bows and noted logs fail.
#[derive(Debug, Clone, Default, Serialize)]
struct GnomeChopCycle {
    chopped: Option<Observation>,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

impl GnomeChopCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
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
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1;
        }
    }

    fn qualified(&self) -> bool {
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
struct GnomeFletchCycle {
    chopped: Option<Observation>,
    fletched: Option<Observation>,
    deposited: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

impl GnomeFletchCycle {
    fn observe(&mut self, product: i32, other: i32, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            gnome_wrong_bows(now) || now.item_id(other) > 0 || now.bank_item_id(other) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && now.item_id(product) == 0
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
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1;
        }
    }

    fn qualified(&self) -> bool {
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
struct CoalTrucksCycle {
    mined: Option<Observation>,
    trucked: Option<Observation>,
    further: bool,
    noted: bool,
    banked: bool,
}

impl CoalTrucksCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.noted |= now.item_id(NOTED_COAL_ID) > 0 || now.bank_item_id(NOTED_COAL_ID) > 0;
        self.banked |= now.bank_item_id(COAL_ID) > baseline.bank_item_id(COAL_ID);
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

    fn qualified(&self) -> bool {
        self.further && self.mined.is_some() && self.trucked.is_some() && !self.noted
    }
}

/// Produce exact unnoted output with relevant XP after Start, deposit into a
/// fresh loaded bank, restock the raw input, close, return to the station,
/// and produce again. Seeded/name-only/noted/wrong products fail.
#[derive(Debug, Clone, Default, Serialize)]
struct StationProductionCycle {
    withdrawn: Option<Observation>,
    produced: Option<Observation>,
    deposited: Option<Observation>,
    restocked: Option<Observation>,
    returned: bool,
    further: bool,
    wrong_product: bool,
    noted: bool,
}

struct StationProductionSpec {
    product: i32,
    input: i32,
    extra_input: Option<i32>,
    wrong: i32,
    extra_wrong: Option<i32>,
    skill: &'static str,
    station: (i32, i32, i32),
    station_radius: i32,
    noted: bool,
}

impl StationProductionCycle {
    fn observe(&mut self, spec: StationProductionSpec, baseline: &Observation, now: &Observation) {
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

    fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

fn station_production_spec(
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
    fn new(case: CoreCase, baseline: Observation) -> Self {
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
            bank_fletcher_string_cycle: BankFletcherStringCycle::default(),
            bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle::default(),
            alcher_generated_custom_cycle: AlcherGeneratedCustomCycle::default(),
            dart_fletcher_cycle: DartFletcherCycle::default(),
            herb_cleaner_cycle: HerbCleanerCycle::default(),
            gem_cutter_cycle: GemCutterCycle::default(),
            door_opener_cycle: DoorOpenerCycle::default(),
            gnome_course_cycle: GnomeCourseCycle::default(),
            flax_picker_cycle: FlaxPickerCycle::default(),
            superheater_cycle: SuperheaterCycle::default(),
            chicken_killer_bank_cycle: ChickenKillerBankCycle::default(),
            vial_filler_cycle: VialFillerCycle::default(),
            potion_maker_cycle: PotionMakerCycle::default(),
            tanner_bot_cycle: TannerBotCycle::default(),
            rune_crafter_cycle: RuneCrafterCycle::default(),
            ardy_cakes_cycle: ArdyCakesCycle::default(),
            ardy_thiever_cycle: ArdyThieverCycle::default(),
            gnome_chop_cycle: GnomeChopCycle::default(),
            gnome_fletch_cycle: GnomeFletchCycle::default(),
            coal_trucks_cycle: CoalTrucksCycle::default(),
            station_production_cycle: StationProductionCycle::default(),
            flax_aio_cycle: FlaxAioCycle::default(),
            flax_aio_pick_cycle: FlaxAioPickCycle::default(),
            herblore_eggs_cycle: HerbloreEggsCycle::default(),
            herblore_newt_cycle: HerbloreNewtCycle::default(),
            combat_core_cycle: CombatCoreCycle::default(),
            ordered_first_exhausted: false,
        }
    }

    fn observe(&mut self, observation: &Observation) {
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
        if matches!(
            self.case,
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight
        ) {
            self.ardy_thiever_cycle.observe(&self.baseline, observation);
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

    fn peak_item(&self, name: &str) -> i32 {
        self.max_items.get(name).copied().unwrap_or(0)
    }

    fn xp_gained(&self, skill: &str) -> bool {
        self.latest.skill_xp(skill) > self.baseline.skill_xp(skill)
    }

    fn item_increased(&self, name: &str) -> bool {
        self.peak_item(name) > self.baseline.item(name)
    }

    fn item_consumed_from_baseline(&self, name: &str) -> bool {
        self.latest.item(name) < self.baseline.item(name)
    }

    fn acquired_then_consumed(&self, name: &str) -> bool {
        self.item_increased(name) && self.latest.item(name) < self.peak_item(name)
    }

    fn qualify(&self) -> Result<Value, String> {
        if self.post_start_observations == 0 {
            return Err("no post-Start observations".into());
        }
        let ok = match self.case {
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
            CoreCase::BankFletcher => {
                self.bank_fletcher_cycle.crafted_after_withdrawal
                    && self.xp_gained("fletching")
                    && self.item_consumed_from_baseline("Willow logs")
                    && self.item_increased("Willow shortbow")
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
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight => {
                self.ardy_thiever_cycle.qualified()
            }
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
            CoreCase::ChaosDruid
            | CoreCase::MossGiant
            | CoreCase::HillGiant
            | CoreCase::AutoFighter
            | CoreCase::RockCrab
            | CoreCase::GreenDragon
            | CoreCase::FireGiant
            | CoreCase::ArdyFighter => {
                combat_spec(self.case).is_some_and(|spec| self.combat_core_cycle.qualified(spec))
            }
        };
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
            "bank_fletcher_string_cycle": self.bank_fletcher_string_cycle,
            "bank_fletcher_cut_string_cycle": self.bank_fletcher_cut_string_cycle,
            "alcher_generated_custom_cycle": self.alcher_generated_custom_cycle,
            "dart_fletcher_cycle": self.dart_fletcher_cycle,
            "herb_cleaner_cycle": self.herb_cleaner_cycle,
            "gem_cutter_cycle": self.gem_cutter_cycle,
            "door_opener_cycle": self.door_opener_cycle,
            "gnome_course_cycle": self.gnome_course_cycle,
            "flax_picker_cycle": self.flax_picker_cycle,
            "superheater_cycle": self.superheater_cycle,
            "chicken_killer_bank_cycle": self.chicken_killer_bank_cycle,
            "vial_filler_cycle": self.vial_filler_cycle,
            "potion_maker_cycle": self.potion_maker_cycle,
            "tanner_bot_cycle": self.tanner_bot_cycle,
            "rune_crafter_cycle": self.rune_crafter_cycle,
            "ardy_cakes_cycle": self.ardy_cakes_cycle,
            "ardy_thiever_cycle": self.ardy_thiever_cycle,
            "gnome_chop_cycle": self.gnome_chop_cycle,
            "gnome_fletch_cycle": self.gnome_fletch_cycle,
            "coal_trucks_cycle": self.coal_trucks_cycle,
            "station_production_cycle": self.station_production_cycle,
            "flax_aio_cycle": self.flax_aio_cycle,
            "flax_aio_pick_cycle": self.flax_aio_pick_cycle,
            "herblore_eggs_cycle": self.herblore_eggs_cycle,
            "herblore_newt_cycle": self.herblore_newt_cycle,
            "combat_core_cycle": self.combat_core_cycle,
            "ordered_first_exhausted": self.ordered_first_exhausted,
        }))
    }
}

fn witness<'a>(
    case: CoreCase,
    baseline: &Observation,
    observations: impl IntoIterator<Item = &'a Observation>,
) -> CoreWitness {
    let mut witness = CoreWitness::new(case, baseline.clone());
    for observation in observations {
        witness.observe(observation);
    }
    witness
}

#[derive(Clone)]
struct PendingStart {
    js: String,
    shape: script::LoadShape,
    settings: Option<Map<String, Value>>,
    siblings: Vec<(String, String)>,
}

struct LiveState {
    runner: ScenarioRunner,
    case: CoreCase,
    account: String,
    names: Arc<ObjNames>,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    pending: Option<PendingStart>,
    baseline: Option<Observation>,
    witness: Option<CoreWitness>,
    start_error: Option<String>,
    start_count: u32,
}

impl LiveState {
    fn publish(&mut self, client: &client::client::Client) -> Observation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        Observation::from_snapshot(&self.snapshot, &self.names)
    }

    fn capture_baseline(&mut self, observation: &Observation) -> Result<(), String> {
        if !observation.ingame || observation.scene_state != 2 {
            return Err(format!(
                "Start baseline is not attached ingame scene2: {observation:?}"
            ));
        }
        let player = observation
            .player
            .as_deref()
            .ok_or_else(|| "Start baseline has no local player".to_string())?;
        let expected_player = client::util::jstring::JString::to_screen_name(&self.account);
        if !player.eq_ignore_ascii_case(&expected_player) {
            return Err(format!(
                "Start baseline player {player:?} is not fresh account {:?}",
                self.account
            ));
        }
        validate_case_baseline(self.case, observation)?;
        self.baseline = Some(observation.clone());
        self.witness = Some(CoreWitness::new(self.case, observation.clone()));
        println!(
            "{}",
            json!({
                "phase": "baseline-after-preparation",
                "scenario": self.case,
                "account": self.account,
                "observation": observation,
            })
        );
        Ok(())
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish(client);
        let at_start = self.runner.on_start_script();
        if at_start && self.baseline.is_none() {
            if let Err(error) = self.capture_baseline(&observation) {
                self.start_error = Some(error);
                return;
            }
        } else if self.start_count == 1 {
            if let Some(witness) = self.witness.as_mut() {
                witness.observe(&observation);
            }
        }

        if at_start && self.pending.is_some() {
            let Some(handle) = self.start_handle.as_ref() else {
                self.start_error =
                    Some("StartScript reached before ScriptStartHandle install".into());
                return;
            };
            let pending = self.pending.as_ref().expect("pending checked");
            match handle.start_load(
                &self.account,
                pending.js.clone(),
                pending.shape,
                pending.settings.clone(),
                pending.siblings.clone(),
            ) {
                Ok(()) => {
                    self.pending = None;
                    self.start_count += 1;
                    println!(
                        "{}",
                        json!({"phase": "start", "account": self.account, "count": self.start_count})
                    );
                }
                Err(error) => {
                    self.start_error = Some(format!("catalog Start refused: {error}"));
                    return;
                }
            }
        }

        if !matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            self.runner.tick_with_hold(client, hold);
        }
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "274bot-catalog-boundary-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn selected_profile(
    revision: u16,
    nav_pack: PathBuf,
    catalog_root: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision.to_string()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(if revision == 289 { 44594 } else { 43594 }),
        http_port: Some(if revision == 289 { 1080 } else { 80 }),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("CATALOG_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("CATALOG_ENGINE_DIR").map(PathBuf::from),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.target() != client::BotTarget::Local || profile.client().game_host() != "127.0.0.1" {
        return Err("catalog proof requires a loopback-only local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn explicit_settings(schema: &[script::SettingDef]) -> Result<Map<String, Value>, String> {
    let Some(raw) = std::env::var_os("CATALOG_SETTINGS_JSON") else {
        return Ok(Map::new());
    };
    let raw = raw
        .into_string()
        .map_err(|_| "CATALOG_SETTINGS_JSON is not UTF-8".to_string())?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("malformed CATALOG_SETTINGS_JSON: {error}"))?;
    let map = value
        .as_object()
        .cloned()
        .ok_or_else(|| "CATALOG_SETTINGS_JSON must be a JSON object".to_string())?;
    for key in map.keys() {
        if !schema.iter().any(|field| field.id == *key) {
            return Err(format!("CATALOG_SETTINGS_JSON has unknown setting {key:?}"));
        }
    }
    Ok(map)
}

fn printable_settings(settings: &Map<String, Value>) -> Map<String, Value> {
    settings
        .iter()
        .map(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            let value = if ["password", "passphrase", "token", "secret", "private_key"]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                Value::String("[REDACTED]".into())
            } else {
                value.clone()
            };
            (key.clone(), value)
        })
        .collect()
}

fn prepare_catalog_card(
    root: &Path,
    temp: &Path,
    row: &CardLedger,
    case: CoreCase,
    scenario_inject: Option<&'static [scenario::ScriptSettingInject]>,
) -> Result<(PendingStart, Value), String> {
    let source_path = verify_source_identity(root, row)?;
    let mut library =
        script::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    let registered = library.register_rs2b0t(root, &temp.join("rs2b0t-path"))?;
    if registered == 0 {
        return Err("frozen catalog registered no script cards".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, case.card_name())?;
    let card = library
        .get(script::ScriptSource::Catalog, case.card_name())
        .cloned()
        .ok_or_else(|| format!("catalog registry has no {} card", case.card_name()))?;
    let canonical_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", source_path.display()))?;
    let canonical_card = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", card.path.display()))?;
    if canonical_card != canonical_source {
        return Err(format!(
            "registry path mismatch for {}: ledger {}, loader {}",
            case.card_name(),
            source_path.display(),
            card.path.display()
        ));
    }
    if card.sha256 != row.source_sha256 {
        return Err(format!(
            "loader source hash mismatch for {}: ledger {}, loader {}",
            case.card_name(),
            row.source_sha256,
            card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{} is unloadable: {import}", case.card_name()));
    }

    let scenario_settings = scenario::settings_inject_map(scenario_inject).unwrap_or_default();
    let explicit = explicit_settings(&card.settings_schema)?;
    // Required order for this controlled proof: schema defaults, scenario
    // injections, then explicit one-cell settings. Nothing is persisted.
    let settings = script::merge_bag(&card.settings_schema, &scenario_settings, Some(&explicit));
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
        },
    )?;
    let sibling_hashes = siblings
        .iter()
        .map(|(url, js)| {
            json!({
                "module": url,
                "compiled_sha256": script::JsCache::origin_sha(js.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    let identity = json!({
        "card": case.card_name(),
        "source_path": card.path,
        "source_sha256": card.sha256,
        "compiled_sha256": script::JsCache::origin_sha(card.js.as_bytes()),
        "siblings": sibling_hashes,
        "settings": printable_settings(&settings),
    });
    Ok((
        PendingStart {
            js: card.js,
            shape: card.shape,
            settings: (!settings.is_empty()).then_some(settings),
            siblings,
        },
        identity,
    ))
}

/// Accumulated CoreWitness plus latest observation for catalog FAIL/timeout
/// receipts. Does not change success, deadline, or lifecycle policy.
fn accumulated_core(witness: Option<&CoreWitness>) -> Value {
    witness
        .map(|witness| {
            witness
                .qualify()
                .unwrap_or_else(|error| json!({"error": error, "witness": witness}))
        })
        .unwrap_or_else(|| json!({"error": "no Start baseline"}))
}

fn run_cell() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let revision = required("CATALOG_REVISION")?
        .parse::<u16>()
        .map_err(|_| "CATALOG_REVISION must be 274 or 289".to_string())?;
    if !matches!(revision, 274 | 289) {
        return Err("CATALOG_REVISION must be 274 or 289".into());
    }
    let root = PathBuf::from(required("CATALOG_ROOT")?);
    if !root.is_dir() {
        return Err(format!(
            "CATALOG_ROOT is not a directory: {}",
            root.display()
        ));
    }
    let commit = required("CATALOG_COMMIT")?;
    let case = CoreCase::parse(&required("CATALOG_SCENARIO")?)?;
    validate_case_catalog(case, &commit)?;
    let nav_pack = PathBuf::from(required("CATALOG_NAV_PACK")?);
    let matrix = support_matrix()?;
    let catalog = catalog_ledger(&matrix, &commit)?;
    let row = ledger_card(&matrix, &commit, revision, case)?;
    let registry_path = verify_registry_identity(&root, catalog)?;

    let mut scenario = scenario::get(case.scenario_name())
        .ok_or_else(|| format!("scenario registry has no {}", case.scenario_name()))?;
    if scenario.settings.start_script != Some(case.card_name()) {
        return Err(format!(
            "scenario {} resolves card {:?}, expected {}",
            case.scenario_name(),
            scenario.settings.start_script,
            case.card_name()
        ));
    }
    let scenario_inject = scenario.settings.script_settings_inject;
    let deadline = scenario.settings.deadline;
    let mainland = scenario.seed.mainland;
    scenario.settings.nav.engine_speed_ms = None;

    let temp = TempRoot::new()?;
    let (profile, template) = selected_profile(revision, nav_pack, root.clone(), temp.path())?;
    let (pending, card_identity) =
        prepare_catalog_card(&root, temp.path(), &row, case, scenario_inject)?;
    let names = host_play::mint_live_names(1);
    let account = names
        .first()
        .cloned()
        .ok_or_else(|| "failed to mint live account".to_string())?;
    let credentials = host_play::mint_live_entries_for_target(&names, profile.target());
    let password = credentials
        .first()
        .map(|(_, password)| password.clone())
        .ok_or_else(|| "failed to mint local credential".to_string())?;

    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));

    let state = Arc::new(Mutex::new(LiveState {
        runner,
        case,
        account: account.clone(),
        names: Arc::new(ObjNames::default()),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        pending: Some(pending),
        baseline: None,
        witness: None,
        start_error: None,
        start_count: 0,
    }));
    let frame_state = Arc::clone(&state);
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        mainland,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            let mut state = frame_state.lock().unwrap();
            if username == state.account {
                state.frame(client, hold);
            }
        },
    )?;
    let obj_names = play.obj_names();
    {
        let mut state = state.lock().unwrap();
        state.names = Arc::clone(&obj_names);
        state.runner.set_obj_names(obj_names);
        state.start_handle = Some(play.script_start_handle());
    }
    let nav_sha256 = hash_file(profile.nav_pack())?;

    println!(
        "{}",
        json!({
            "phase": "identity",
            "revision": profile.revision().as_i32(),
            "profile": profile.label(),
            "cache_id": profile.cache_id(),
            "nav_pack": profile.nav_pack(),
            "nav_sha256": nav_sha256,
            "catalog_commit": commit,
            "catalog_identity": catalog.identity,
            "catalog_read_only_path": catalog.read_only_path,
            "catalog_root": root,
            "registry_path": registry_path,
            "registry_sha256": catalog.registry_sha256,
            "card": card_identity,
            "account": account,
            "engine_speed_ms": null,
        })
    );

    let account_profile = Profile {
        username: account.clone(),
        password,
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    play.try_spawn_slot(account_profile, None, None, None)?;
    play.focus(&account);

    let outer_deadline = Instant::now() + deadline + Duration::from_secs(5);
    let outcome = loop {
        if let Some(error) = play.script_last_error(&account) {
            break Err(format!("script error: {error}"));
        }
        let mut state = state.lock().unwrap();
        if let Some(error) = state.start_error.take() {
            break Err(error);
        }
        match state.runner.status() {
            RunnerStatus::Failed(error) => {
                let core = accumulated_core(state.witness.as_ref());
                break Err(format!(
                    "scenario failed: {error}; evidence={:?}; core={core}",
                    state.runner.evidence()
                ));
            }
            RunnerStatus::Passed => {
                if state.start_count != 1 {
                    break Err(format!(
                        "scenario passed with {} catalog Starts; expected exactly one",
                        state.start_count
                    ));
                }
                let Some(witness) = state.witness.as_ref() else {
                    break Err("scenario passed without a Start baseline".into());
                };
                if let Ok(core) = witness.qualify() {
                    let evidence = format!("{:?}", state.runner.evidence());
                    break Ok(json!({
                        "scenario": case,
                        "runner": evidence,
                        "core": core,
                    }));
                }
            }
            RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
        }
        if Instant::now() >= outer_deadline {
            let core = accumulated_core(state.witness.as_ref());
            break Err(format!(
                "bounded timeout; runner={:?}; core={core}",
                state.runner.status()
            ));
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(20));
    };

    play.stop_slot(&account);
    let outcome = outcome?;
    println!("PASS: catalog_boundary_live: {outcome}");
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, CATALOG_REVISION/ROOT/COMMIT/SCENARIO/NAV_PACK, and local engine"]
fn catalog_boundary_live() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_cell));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("FAIL: catalog_boundary_live: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("FAIL: catalog_boundary_live: {error:?}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn observation(items: &[(&str, i32)], xp: &[(&str, i32)], chat: &[(i32, &str)]) -> Observation {
        Observation {
            ingame: true,
            scene_state: 2,
            player: Some("catalogtest".into()),
            tile: Some((3220, 3212, 0)),
            tick: 10,
            bank: BTreeMap::new(),
            bank_ids: BTreeMap::new(),
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            items: items
                .iter()
                .map(|(name, count)| ((*name).to_string(), *count))
                .collect::<BTreeMap<_, _>>(),
            item_ids: BTreeMap::new(),
            levels: [
                ("thieving".to_string(), 99),
                ("hitpoints".to_string(), 99),
                ("magic".to_string(), 99),
                ("fletching".to_string(), 99),
            ]
            .into_iter()
            .collect(),
            xp: xp
                .iter()
                .map(|(name, value)| ((*name).to_string(), *value))
                .collect::<BTreeMap<_, _>>(),
            chat: chat
                .iter()
                .map(|(sequence, text)| (*sequence, (*text).to_string()))
                .collect(),
            loc_facts: Vec::new(),
            npc_facts: Vec::new(),
            ground_loot: Vec::new(),
            local_in_combat: false,
            local_target_npc: None,
            local_health: 0,
            local_animation: 0,
            equipment_ids: BTreeMap::new(),
            main_modal: -1,
            widget_ids: BTreeSet::new(),
        }
    }

    fn observation_ids(item_ids: &[(i32, i32)], bank_ids: &[(i32, i32)], xp: i32) -> Observation {
        let mut observation = observation(&[], &[("fletching", xp)], &[]);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn source_identity_rejects_bytes_that_do_not_match_the_frozen_ledger() {
        let matrix = support_matrix().unwrap();
        let row = ledger_card(
            &matrix,
            "100adccc037d9f6898080e1cad58fcfc43364775",
            274,
            CoreCase::BoneBurier,
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "catalog-boundary-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join(&row.source_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not the frozen card").unwrap();

        let error = verify_source_identity(&root, &row).unwrap_err();

        assert!(error.contains("source SHA-256 mismatch"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checked_in_frozen_core_sources_match_both_catalog_ledgers() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let matrix = support_matrix().unwrap();
        for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
            let root = repo.join(format!(".superpowers/inputs/rs2b0t-{commit}"));
            let catalog = catalog_ledger(&matrix, commit).unwrap();
            verify_registry_identity(&root, catalog).unwrap();
            for case in [
                CoreCase::BoneBurier,
                CoreCase::ChickenKiller,
                CoreCase::Thiever,
                CoreCase::Alcher,
                CoreCase::BankFletcher,
                CoreCase::DartFletcher,
                CoreCase::HerbCleaner,
                CoreCase::GemCutter,
                CoreCase::DoorOpener,
                CoreCase::GnomeCourse,
                CoreCase::FlaxPicker,
                CoreCase::FlaxAio,
                CoreCase::HerbloreSecondaries,
                CoreCase::ChaosDruid,
                CoreCase::MossGiant,
                CoreCase::HillGiant,
                CoreCase::AutoFighter,
                CoreCase::RockCrab,
                CoreCase::GreenDragon,
                CoreCase::FireGiant,
                CoreCase::ArdyFighter,
            ] {
                let row = ledger_card(&matrix, commit, 274, case).unwrap();
                verify_source_identity(&root, &row).unwrap();
            }
        }
    }

    #[test]
    fn start_baseline_rejects_a_seeded_outcome_without_case_preparation() {
        let baseline = observation(&[("Bones", 0)], &[("prayer", 100)], &[]);

        let error = validate_case_baseline(CoreCase::BoneBurier, &baseline).unwrap_err();

        assert!(error.contains("five Bones"), "{error}");
    }

    #[test]
    fn bone_burier_rejects_first_burial_and_requires_observed_bank_cycle() {
        let baseline = observation(&[("Bones", 5)], &[("prayer", 100)], &[]);
        let first = observation(
            &[("Bones", 4)],
            &[("prayer", 104)],
            &[(2, "You bury the bones.")],
        );
        assert!(witness(CoreCase::BoneBurier, &baseline, [&first])
            .qualify()
            .is_err());
        let empty = observation(&[], &[("prayer", 122)], &[(6, "You bury the bones.")]);
        assert!(witness(CoreCase::BoneBurier, &baseline, [&first, &empty])
            .qualify()
            .is_err());
        let mut opened = empty.clone();
        opened.tile = Some((3092, 3245, 0));
        opened.bank_open = true;
        opened.bank_loaded = true;
        opened.bank_generation = 1;
        opened.bank.insert("Bones".into(), 28);
        let mut withdrawn = opened.clone();
        withdrawn.items.insert("Bones".into(), 28);
        withdrawn.bank.clear();
        let mut buried = withdrawn.clone();
        buried.bank_open = false;
        buried.bank_loaded = false;
        buried.items.insert("Bones".into(), 27);
        buried.xp.insert("prayer".into(), 127);
        buried.chat.push((7, "You bury the bones.".into()));
        let observations = [&first, &empty, &opened, &withdrawn, &buried];
        assert!(witness(CoreCase::BoneBurier, &baseline, observations)
            .qualify()
            .is_ok());
        // An inventory rise without the observed bank transfer is insufficient.
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &withdrawn, &buried]
        )
        .qualify()
        .is_err());
        let mut stale = opened.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &stale, &withdrawn, &buried]
        )
        .qualify()
        .is_err());
        let mut no_consumption = buried.clone();
        no_consumption.items.insert("Bones".into(), 28);
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &opened, &withdrawn, &no_consumption]
        )
        .qualify()
        .is_err());
        let mut no_xp = buried.clone();
        no_xp.xp.insert("prayer".into(), 122);
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &opened, &withdrawn, &no_xp]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn chicken_killer_requires_combat_xp_and_a_post_start_loot_bury_cycle() {
        let baseline = observation(&[("Bones", 0)], &[("strength", 100), ("prayer", 100)], &[]);
        let looted = observation(&[("Bones", 1)], &[("strength", 104), ("prayer", 100)], &[]);
        let buried = observation(
            &[("Bones", 0)],
            &[("strength", 104), ("prayer", 104)],
            &[(3, "You bury the bones.")],
        );
        let changed = witness(CoreCase::ChickenKiller, &baseline, [&looted, &buried]);
        assert!(changed.qualify().is_ok());

        let preseeded_only = witness(CoreCase::ChickenKiller, &buried, [&buried]);
        assert!(preseeded_only.qualify().is_err());
    }

    fn chicken_bank_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        strength_xp: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("strength", strength_xp)], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn chicken_killer_bank_requires_combat_loot_fresh_deposit_return_and_further_work() {
        let mut baseline = chicken_bank_obs(FALADOR_CHICKENS, &[], &[], 100);
        assert!(validate_case_baseline(CoreCase::ChickenKillerBank, &baseline).is_err());
        baseline.levels.insert("attack".into(), 30);
        baseline.levels.insert("strength".into(), 30);
        validate_case_baseline(CoreCase::ChickenKillerBank, &baseline).unwrap();

        let looted = chicken_bank_obs(FALADOR_CHICKENS, &[(FEATHER_ID, 5)], &[], 104);
        let mut deposited = chicken_bank_obs((3012, 3355, 0), &[], &[(FEATHER_ID, 5)], 104);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = chicken_bank_obs(FALADOR_CHICKENS, &[], &[(FEATHER_ID, 5)], 104);
        returned.bank_generation = 2;
        let mut further = chicken_bank_obs(FALADOR_CHICKENS, &[(FEATHER_ID, 3)], &[], 108);
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::ChickenKillerBank, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::ChickenKillerBank, &baseline, [&looted])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &returned]
        )
        .qualify()
        .is_err());

        // A fabricated closed snapshot without a new modal generation is stale.
        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let xp_only = chicken_bank_obs(FALADOR_CHICKENS, &[], &[], 104);
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut name_only = looted.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Feather".into(), 5);
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &stale, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &closed, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut far = chicken_bank_obs((3185, 3440, 0), &[], &[(FEATHER_ID, 5)], 104);
        far.bank_generation = 2;
        let mut further_far = further.clone();
        further_far.tile = Some((3185, 3440, 0));
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &far, &further_far]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(FEATHER_ID, 1);
        assert!(validate_case_baseline(CoreCase::ChickenKillerBank, &seeded).is_err());

        let lumbridge = chicken_bank_obs((3235, 3295, 0), &[], &[], 100);
        assert!(validate_case_baseline(CoreCase::ChickenKillerBank, &lumbridge).is_err());
        validate_case_baseline(CoreCase::ChickenKiller, &lumbridge).unwrap();
    }

    #[test]
    fn thiever_requires_post_start_xp_and_coin_progress() {
        let baseline = observation(&[("Coins", 0)], &[("thieving", 100)], &[]);
        let after = observation(&[("Coins", 25)], &[("thieving", 108)], &[]);
        let changed = witness(CoreCase::Thiever, &baseline, [&after]);
        assert!(changed.qualify().is_ok());
    }

    #[test]
    fn alcher_requires_acquired_fuel_then_consumption_xp_and_coins() {
        let baseline = observation(
            &[("Rune chainbody", 0), ("Nature rune", 0), ("Coins", 0)],
            &[("magic", 10_000)],
            &[],
        );
        let stocked = observation(
            &[("Rune chainbody", 30), ("Nature rune", 200), ("Coins", 0)],
            &[("magic", 10_000)],
            &[],
        );
        let after = observation(
            &[
                ("Rune chainbody", 29),
                ("Nature rune", 199),
                ("Coins", 30_000),
            ],
            &[("magic", 10_065)],
            &[],
        );
        let changed = witness(CoreCase::Alcher, &baseline, [&stocked, &after]);
        assert!(changed.qualify().is_ok());
    }

    #[test]
    fn alcher_option_witnesses_reject_seeded_only_and_require_order_and_large_stack() {
        let baseline = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 0),
                ("Nature rune", 0),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let seeded = observation(
            &[
                ("Rune platebody", 1),
                ("Rune chainbody", 1),
                ("Nature rune", 2),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        assert!(witness(CoreCase::AlcherOrdered, &seeded, [&seeded])
            .qualify()
            .is_err());

        let plate_only = observation(
            &[
                ("Rune platebody", 1),
                ("Rune chainbody", 0),
                ("Nature rune", 1),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let plate_only_after = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 0),
                ("Nature rune", 0),
                ("Coins", 1_000),
            ],
            &[("magic", 10_033)],
            &[],
        );
        assert!(witness(
            CoreCase::AlcherOrdered,
            &baseline,
            [&plate_only, &plate_only_after]
        )
        .qualify()
        .is_err());

        let first = observation(
            &[
                ("Rune platebody", 1),
                ("Rune chainbody", 0),
                ("Nature rune", 2),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let second = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 1),
                ("Nature rune", 1),
                ("Coins", 1_000),
            ],
            &[("magic", 10_033)],
            &[],
        );
        let ordered_after = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 0),
                ("Nature rune", 0),
                ("Coins", 2_000),
            ],
            &[("magic", 10_066)],
            &[],
        );
        assert!(witness(
            CoreCase::AlcherOrdered,
            &baseline,
            [&first, &second, &ordered_after]
        )
        .qualify()
        .is_ok());
        assert!(witness(
            CoreCase::AlcherLargeBatch,
            &baseline,
            [&second, &ordered_after]
        )
        .qualify()
        .is_err());

        let large = observation(
            &[
                ("Rune chainbody", 1000),
                ("Nature rune", 1000),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let large_after = observation(
            &[
                ("Rune chainbody", 999),
                ("Nature rune", 999),
                ("Coins", 1_000),
            ],
            &[("magic", 10_033)],
            &[],
        );
        assert!(witness(
            CoreCase::AlcherLargeBatch,
            &baseline,
            [&large, &large_after]
        )
        .qualify()
        .is_ok());
    }

    #[test]
    fn bank_fletcher_requires_a_fresh_bank_cycle_and_further_crafting() {
        let baseline = observation(
            &[("Knife", 1), ("Willow logs", 27)],
            &[("fletching", 22_000)],
            &[],
        );
        let after = observation(
            &[("Willow logs", 26), ("Willow shortbow", 1)],
            &[("fletching", 22_033)],
            &[],
        );
        assert!(witness(CoreCase::BankFletcher, &baseline, [&after])
            .qualify()
            .is_err());
        let first_pack = observation(
            &[("Knife", 1), ("Willow shortbow", 27)],
            &[("fletching", 22_899)],
            &[],
        );
        let mut deposited = observation(&[("Knife", 1)], &[("fletching", 22_899)], &[]);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        deposited.bank =
            BTreeMap::from([("Willow shortbow".into(), 27), ("Willow logs".into(), 54)]);
        let mut withdrawn = deposited.clone();
        withdrawn.items.insert("Willow logs".into(), 27);
        withdrawn.bank.insert("Willow logs".into(), 27);
        let second_product = observation(
            &[("Knife", 1), ("Willow logs", 26), ("Willow shortbow", 1)],
            &[("fletching", 22_932)],
            &[],
        );
        let sequence = [&first_pack, &deposited, &withdrawn, &second_product];
        assert!(witness(CoreCase::BankFletcher, &baseline, sequence)
            .qualify()
            .is_ok());
        assert!(witness(
            CoreCase::BankFletcher,
            &baseline,
            sequence[..3].iter().copied()
        )
        .qualify()
        .is_err());
        // Stale bank rows and a changed bank session cannot prove transfer.
        deposited.bank_loaded = false;
        assert!(witness(
            CoreCase::BankFletcher,
            &baseline,
            [&first_pack, &deposited, &withdrawn, &second_product]
        )
        .qualify()
        .is_err());
        deposited.bank_loaded = true;
        withdrawn.bank_generation = 2;
        assert!(witness(
            CoreCase::BankFletcher,
            &baseline,
            [&first_pack, &deposited, &withdrawn, &second_product]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn bank_fletcher_string_requires_exact_ids_and_a_same_generation_bank_cycle() {
        let baseline = observation_ids(&[(60, 2), (1777, 2)], &[], 10_000);
        let first_pair = observation_ids(&[(849, 2)], &[], 10_066);
        let mut deposited = observation_ids(&[], &[(60, 28), (849, 2), (1777, 28)], 10_066);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut withdrawn = observation_ids(
            &[(60, 14), (1777, 14)],
            &[(60, 14), (849, 2), (1777, 14)],
            10_066,
        );
        withdrawn.bank_open = true;
        withdrawn.bank_loaded = true;
        withdrawn.bank_generation = 1;
        let crafted = observation_ids(&[(60, 13), (849, 1), (1777, 13)], &[], 10_099);

        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &deposited, &withdrawn, &crafted]
        )
        .qualify()
        .is_ok());
        assert!(
            witness(CoreCase::BankFletcherString, &baseline, [&baseline])
                .qualify()
                .is_err()
        );
        assert!(
            witness(CoreCase::BankFletcherString, &baseline, [&first_pair])
                .qualify()
                .is_err()
        );

        let mut same_name_only = first_pair.clone();
        same_name_only.item_ids.clear();
        same_name_only.items.insert("Willow shortbow".into(), 2);
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&same_name_only, &deposited, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());

        let mut stale_deposit = deposited.clone();
        stale_deposit.bank_loaded = false;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &stale_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
        let mut closed_deposit = deposited.clone();
        closed_deposit.bank_open = false;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &closed_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
        let mut loading_scene_deposit = deposited.clone();
        loading_scene_deposit.scene_state = 1;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &loading_scene_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
        let mut detached_deposit = deposited.clone();
        detached_deposit.ingame = false;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &detached_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());

        withdrawn.bank_generation = 2;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &deposited, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn bank_fletcher_cut_string_requires_cut_then_bank_then_string_by_id() {
        let mut baseline = observation_ids(&[(1519, 2)], &[], 20_000);
        baseline.items.insert("Knife".into(), 1);
        let cut = observation_ids(&[(60, 2)], &[], 20_066);
        let mut deposited = observation_ids(&[], &[(60, 2), (1777, 28)], 20_066);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut withdrawn = observation_ids(&[(60, 2), (1777, 14)], &[(1777, 14)], 20_066);
        withdrawn.bank_open = true;
        withdrawn.bank_loaded = true;
        withdrawn.bank_generation = 1;
        withdrawn.bank.insert("Knife".into(), 1);
        let finished = observation_ids(&[(849, 2), (1777, 12)], &[], 20_132);

        assert!(witness(
            CoreCase::BankFletcherCutString,
            &baseline,
            [&cut, &deposited, &withdrawn, &finished]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::BankFletcherCutString, &baseline, [&cut])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::BankFletcherCutString,
            &baseline,
            [&cut, &withdrawn, &finished]
        )
        .qualify()
        .is_err());
        let mut wrong_id_finished = finished.clone();
        wrong_id_finished.item_ids.remove(&849);
        wrong_id_finished.items.insert("Willow shortbow".into(), 2);
        assert!(witness(
            CoreCase::BankFletcherCutString,
            &baseline,
            [&cut, &deposited, &withdrawn, &wrong_id_finished]
        )
        .qualify()
        .is_err());

        let mut seeded_outcome = baseline.clone();
        seeded_outcome.item_ids.insert(60, 2);
        assert!(validate_case_baseline(CoreCase::BankFletcherCutString, &seeded_outcome).is_err());
    }

    fn alcher_generated_obs(item_ids: &[(i32, i32)], magic_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("magic", magic_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("magic".into(), 55);
        observation.item_ids = item_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn alcher_generated_custom_requires_noted_id_then_exact_consumption() {
        for case in [CoreCase::AlcherCustomAlias, CoreCase::AlcherCustomName] {
            let mut baseline = alcher_generated_obs(&[], 10_000);
            baseline.bank_generation = 1;
            validate_case_baseline(case, &baseline).unwrap();

            let mut withdrawn = alcher_generated_obs(
                &[(CERT_ADAMANT_SCIMITAR_ID, 1), (NATURE_RUNE_ID, 1)],
                10_000,
            );
            withdrawn.bank_generation = 2;
            let mut consumed =
                alcher_generated_obs(&[(COINS_ID, ADAMANT_SCIMITAR_ALCH_COINS)], 10_065);
            consumed.bank_generation = 2;

            assert!(witness(case, &baseline, [&withdrawn, &consumed])
                .qualify()
                .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&withdrawn]).qualify().is_err());

            let mut name_only = withdrawn.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Adamant scimitar".into(), 1);
            name_only.items.insert("Nature rune".into(), 1);
            assert!(witness(case, &baseline, [&name_only, &consumed])
                .qualify()
                .is_err());

            let mut unnoted = withdrawn.clone();
            unnoted.item_ids.remove(&CERT_ADAMANT_SCIMITAR_ID);
            unnoted.item_ids.insert(ADAMANT_SCIMITAR_ID, 1);
            assert!(witness(case, &baseline, [&unnoted, &consumed])
                .qualify()
                .is_err());

            let mut chainbody = alcher_generated_obs(&[(NATURE_RUNE_ID, 1)], 10_000);
            chainbody.items.insert("Rune chainbody".into(), 1);
            chainbody.items.insert("Nature rune".into(), 1);
            chainbody.bank_generation = 2;
            let mut chainbody_after = alcher_generated_obs(&[(COINS_ID, 30_000)], 10_065);
            chainbody_after.items.insert("Coins".into(), 30_000);
            chainbody_after.bank_generation = 2;
            assert!(witness(case, &baseline, [&chainbody, &chainbody_after])
                .qualify()
                .is_err());

            let mut wrong_coins = consumed.clone();
            wrong_coins.item_ids.insert(COINS_ID, 30_000);
            assert!(witness(case, &baseline, [&withdrawn, &wrong_coins])
                .qualify()
                .is_err());

            let mut no_bank_session = withdrawn.clone();
            no_bank_session.bank_generation = 1;
            assert!(witness(case, &baseline, [&no_bank_session, &consumed])
                .qualify()
                .is_err());

            let mut seeded_notes = baseline.clone();
            seeded_notes.item_ids.insert(CERT_ADAMANT_SCIMITAR_ID, 1);
            assert!(validate_case_baseline(case, &seeded_notes).is_err());
            let mut seeded_coins = baseline.clone();
            seeded_coins
                .item_ids
                .insert(COINS_ID, ADAMANT_SCIMITAR_ALCH_COINS);
            assert!(validate_case_baseline(case, &seeded_coins).is_err());
        }
    }

    fn dart_obs(item_ids: &[(i32, i32)], fletching_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("fletching", fletching_xp)], &[]);
        observation.item_ids = item_ids.iter().copied().collect();
        observation
    }

    fn herb_obs(item_ids: &[(i32, i32)], bank_ids: &[(i32, i32)], herblore_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("herblore", herblore_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("herblore".into(), 5);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    fn gem_obs(item_ids: &[(i32, i32)], bank_ids: &[(i32, i32)], crafting_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("crafting", crafting_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("crafting".into(), 20);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn dart_fletcher_requires_two_observed_actions_and_both_inputs() {
        for (case, tips, product, wrong) in [
            (
                CoreCase::DartFletcher,
                BRONZE_DART_TIP_ID,
                BRONZE_DART_ID,
                IRON_DART_ID,
            ),
            (
                CoreCase::DartFletcherIron,
                IRON_DART_TIP_ID,
                IRON_DART_ID,
                BRONZE_DART_ID,
            ),
        ] {
            let baseline = dart_obs(&[(tips, 100), (FEATHER_ID, 100)], 10_000);
            validate_case_baseline(case, &baseline).unwrap();

            let first = dart_obs(&[(tips, 90), (FEATHER_ID, 90), (product, 10)], 10_018);
            let further = dart_obs(&[(tips, 80), (FEATHER_ID, 80), (product, 20)], 10_036);
            assert!(witness(case, &baseline, [&first, &further])
                .qualify()
                .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());

            let xp_only = dart_obs(&[(tips, 100), (FEATHER_ID, 100)], 10_018);
            assert!(witness(case, &baseline, [&xp_only, &xp_only])
                .qualify()
                .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Bronze dart".into(), 10);
            name_only.items.insert("Iron dart".into(), 10);
            let mut name_only_further = further.clone();
            name_only_further.item_ids.clear();
            name_only_further.items.insert("Bronze dart".into(), 20);
            assert!(witness(case, &baseline, [&name_only, &name_only_further])
                .qualify()
                .is_err());

            let wrong_tier = dart_obs(&[(tips, 90), (FEATHER_ID, 90), (wrong, 10)], 10_018);
            let wrong_further = dart_obs(&[(tips, 80), (FEATHER_ID, 80), (wrong, 20)], 10_036);
            assert!(witness(case, &baseline, [&wrong_tier, &wrong_further])
                .qualify()
                .is_err());

            let no_tips = dart_obs(&[(tips, 100), (FEATHER_ID, 90), (product, 10)], 10_018);
            let no_tips_further = dart_obs(&[(tips, 100), (FEATHER_ID, 80), (product, 20)], 10_036);
            assert!(witness(case, &baseline, [&no_tips, &no_tips_further])
                .qualify()
                .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(product, 10);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    #[test]
    fn herb_cleaner_requires_deposit_restock_and_exact_unid_ids() {
        for (case, named) in [
            (CoreCase::HerbCleaner, false),
            (CoreCase::HerbCleanerNamed, true),
        ] {
            let mut baseline = herb_obs(&[], &[(UNIDENTIFIED_GUAM_ID, 30)], 1_000);
            if named {
                baseline.bank_ids.insert(UNIDENTIFIED_MARENTILL_ID, 4);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let first = herb_obs(&[(GUAM_LEAF_ID, 28)], &[], 1_070);
            let mut deposited = herb_obs(
                &[],
                &[
                    (GUAM_LEAF_ID, 28),
                    (UNIDENTIFIED_GUAM_ID, 2),
                    (UNIDENTIFIED_MARENTILL_ID, if named { 4 } else { 0 }),
                ],
                1_070,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = herb_obs(
                &[(UNIDENTIFIED_GUAM_ID, 2)],
                &[
                    (GUAM_LEAF_ID, 28),
                    (UNIDENTIFIED_MARENTILL_ID, if named { 4 } else { 0 }),
                ],
                1_070,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let cleaned = herb_obs(&[(GUAM_LEAF_ID, 2)], &[], 1_075);

            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cleaned])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Guam leaf".into(), 28);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &withdrawn, &cleaned]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(
                witness(case, &baseline, [&first, &stale, &withdrawn, &cleaned])
                    .qualify()
                    .is_err()
            );
            let mut closed_deposit = deposited.clone();
            closed_deposit.bank_open = false;
            assert!(witness(
                case,
                &baseline,
                [&first, &closed_deposit, &withdrawn, &cleaned]
            )
            .qualify()
            .is_err());
            withdrawn.bank_generation = 2;
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cleaned])
                    .qualify()
                    .is_err()
            );
            withdrawn.bank_generation = 1;

            if named {
                let mut took_filter = withdrawn.clone();
                took_filter.item_ids.insert(UNIDENTIFIED_MARENTILL_ID, 4);
                took_filter.bank_ids.insert(UNIDENTIFIED_MARENTILL_ID, 0);
                assert!(witness(
                    case,
                    &baseline,
                    [&first, &deposited, &took_filter, &cleaned]
                )
                .qualify()
                .is_err());
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(GUAM_LEAF_ID, 28);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    #[test]
    fn gem_cutter_requires_chisel_preservation_and_no_crush() {
        for (case, named) in [
            (CoreCase::GemCutter, false),
            (CoreCase::GemCutterNamed, true),
        ] {
            let mut baseline = gem_obs(&[], &[(CHISEL_ID, 1), (UNCUT_SAPPHIRE_ID, 28)], 20_000);
            if named {
                baseline.bank_ids.insert(UNCUT_OPAL_ID, 4);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let first = gem_obs(&[(CHISEL_ID, 1), (SAPPHIRE_ID, 27)], &[], 33_500);
            let mut deposited = gem_obs(
                &[(CHISEL_ID, 1)],
                &[
                    (SAPPHIRE_ID, 27),
                    (UNCUT_SAPPHIRE_ID, 1),
                    (UNCUT_OPAL_ID, if named { 4 } else { 0 }),
                ],
                33_500,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = gem_obs(
                &[(CHISEL_ID, 1), (UNCUT_SAPPHIRE_ID, 1)],
                &[
                    (SAPPHIRE_ID, 27),
                    (UNCUT_OPAL_ID, if named { 4 } else { 0 }),
                ],
                33_500,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let cut = gem_obs(&[(CHISEL_ID, 1), (SAPPHIRE_ID, 1)], &[], 34_000);

            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Sapphire".into(), 27);
            name_only.items.insert("Chisel".into(), 1);
            assert!(
                witness(case, &baseline, [&name_only, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );

            let crushed = gem_obs(
                &[(CHISEL_ID, 1), (SAPPHIRE_ID, 27), (CRUSHED_GEMSTONE_ID, 1)],
                &[],
                33_500,
            );
            assert!(
                witness(case, &baseline, [&crushed, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );

            let mut no_chisel = first.clone();
            no_chisel.item_ids.remove(&CHISEL_ID);
            assert!(
                witness(case, &baseline, [&no_chisel, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(case, &baseline, [&first, &stale, &withdrawn, &cut])
                .qualify()
                .is_err());
            withdrawn.bank_generation = 2;
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );
            withdrawn.bank_generation = 1;

            if named {
                let mut took_opal = withdrawn.clone();
                took_opal.item_ids.insert(UNCUT_OPAL_ID, 4);
                took_opal.bank_ids.insert(UNCUT_OPAL_ID, 0);
                assert!(
                    witness(case, &baseline, [&first, &deposited, &took_opal, &cut])
                        .qualify()
                        .is_err()
                );
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(SAPPHIRE_ID, 27);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn bounded_loc(id: i32, tile: (i32, i32, i32), name: &str, open: bool) -> BoundedLoc {
        BoundedLoc {
            id,
            x: tile.0,
            z: tile.1,
            level: tile.2,
            name: Some(name.into()),
            open,
        }
    }

    fn door_obs(tile: (i32, i32, i32), locs: Vec<BoundedLoc>) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.loc_facts = locs;
        observation
    }

    fn gnome_obs(tile: (i32, i32, i32), agility_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("agility", agility_xp)], &[]);
        observation.tile = Some(tile);
        observation
    }

    fn flax_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
    ) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn door_opener_requires_selected_world_change_not_queued_open() {
        for (case, stand, packed, closed, open, name) in [
            (
                CoreCase::DoorOpener,
                LUMBRIDGE_DOOR_STAND,
                LUMBRIDGE_DOOR,
                WOODEN_DOOR_CLOSED_ID,
                WOODEN_DOOR_OPEN_ID,
                "Door",
            ),
            (
                CoreCase::DoorOpenerGate,
                LUMBRIDGE_GATE_STAND,
                LUMBRIDGE_GATE,
                WOODEN_GATE_CLOSED_ID,
                WOODEN_GATE_OPEN_ID,
                "Gate",
            ),
        ] {
            let shut = bounded_loc(closed, packed, name, true);
            let baseline = door_obs(stand, vec![shut.clone()]);
            validate_case_baseline(case, &baseline).unwrap();

            let opened = door_obs(stand, vec![bounded_loc(open, packed, name, false)]);
            assert!(witness(case, &baseline, [&opened]).qualify().is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());

            let queued = door_obs(stand, vec![shut.clone()]);
            assert!(witness(case, &baseline, [&queued]).qualify().is_err());

            let unrelated = door_obs(
                stand,
                vec![
                    shut.clone(),
                    bounded_loc(open, (3100, 3100, 0), name, false),
                ],
            );
            assert!(witness(case, &baseline, [&unrelated]).qualify().is_err());

            let mut already_open = baseline.clone();
            already_open.loc_facts = vec![bounded_loc(open, packed, name, false)];
            assert!(validate_case_baseline(case, &already_open).is_err());
        }
    }

    #[test]
    fn gnome_course_requires_complete_lap_and_second_lap_progress() {
        for case in [CoreCase::GnomeCourse, CoreCase::GnomeCourseRadius] {
            let baseline = gnome_obs(GNOME_START, 0);
            validate_case_baseline(case, &baseline).unwrap();

            let log = gnome_obs(GNOME_AFTER_LOG, 7);
            let ground = gnome_obs(GNOME_GROUND_RETURN, 27);
            let pipe = gnome_obs(GNOME_PIPE, 32);
            let first_lap_return = gnome_obs((2482, 3437, 0), 86);
            let queued_next_log = gnome_obs(GNOME_AFTER_LOG, 86);
            let second = gnome_obs(GNOME_AFTER_LOG, 94);
            assert!(witness(case, &baseline, [&log, &ground, &pipe, &second])
                .qualify()
                .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&log]).qualify().is_err());
            assert!(witness(case, &baseline, [&log, &log]).qualify().is_err());
            assert!(witness(case, &baseline, [&log, &ground, &pipe])
                .qualify()
                .is_err());
            for incomplete in [&first_lap_return, &queued_next_log] {
                assert!(witness(case, &baseline, [&log, &ground, &pipe, incomplete])
                    .qualify()
                    .is_err());
            }
        }
    }

    #[test]
    fn flax_picker_requires_full_pack_deposit_return_and_further_pick() {
        let baseline = flax_obs(FLAX_FIELD, &[], &[]);
        validate_case_baseline(CoreCase::FlaxPicker, &baseline).unwrap();

        let first = flax_obs(FLAX_FIELD, &[(FLAX_ID, 28)], &[]);
        let mut deposited = flax_obs(FLAX_FIELD, &[], &[(FLAX_ID, 28)]);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = flax_obs(FLAX_FIELD, &[], &[]);
        returned.bank_generation = 2;
        let mut further = flax_obs(FLAX_FIELD, &[(FLAX_ID, 1)], &[]);
        further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxPicker,
            &baseline,
            [&first, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::FlaxPicker, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::FlaxPicker, &baseline, [&first])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::FlaxPicker, &baseline, [&first, &deposited])
                .qualify()
                .is_err()
        );

        // A fabricated closed snapshot without a new modal generation is stale.
        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::FlaxPicker,
            &baseline,
            [&first, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::FlaxPicker,
            &baseline,
            [&first, &stale, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(FLAX_ID, 28);
        assert!(validate_case_baseline(CoreCase::FlaxPicker, &seeded).is_err());
    }

    fn superheater_obs(
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        equipment_ids: &[(i32, i32)],
        magic_xp: i32,
        smithing_xp: i32,
        smithing_level: i32,
    ) -> Observation {
        let mut observation =
            observation(&[], &[("magic", magic_xp), ("smithing", smithing_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("magic".into(), 43);
        observation.levels.insert("smithing".into(), smithing_level);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.equipment_ids = equipment_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn superheater_requires_exact_ids_staff_and_bank_cycle() {
        for (case, bar, primary, secondary, staff, steel, smithing) in [
            (
                CoreCase::Superheater,
                BRONZE_BAR_ID,
                COPPER_ORE_ID,
                TIN_ORE_ID,
                STAFF_OF_FIRE_ID,
                false,
                1,
            ),
            (
                CoreCase::SuperheaterSteel,
                STEEL_BAR_ID,
                IRON_ORE_ID,
                COAL_ID,
                STAFF_OF_FIRE_ID,
                true,
                30,
            ),
            (
                CoreCase::SuperheaterFireBattlestaff,
                BRONZE_BAR_ID,
                COPPER_ORE_ID,
                TIN_ORE_ID,
                FIRE_BATTLESTAFF_ID,
                false,
                1,
            ),
        ] {
            let mut baseline = superheater_obs(&[], &[], &[], 10_000, 1_000, smithing);
            if case == CoreCase::SuperheaterFireBattlestaff {
                assert!(validate_case_baseline(case, &baseline).is_err());
                baseline.levels.insert("attack".into(), 30);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let primary_n = 9;
            let secondary_n = if steel { 18 } else { 9 };
            let first = superheater_obs(
                &[(bar, 9), (NATURE_RUNE_ID, 41)],
                &[],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            let mut deposited = superheater_obs(
                &[(NATURE_RUNE_ID, 41)],
                &[
                    (bar, 9),
                    (primary, 91),
                    (secondary, if steel { 182 } else { 91 }),
                ],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = superheater_obs(
                &[
                    (primary, primary_n),
                    (secondary, secondary_n),
                    (NATURE_RUNE_ID, 41),
                ],
                &[
                    (bar, 9),
                    (primary, 91 - primary_n),
                    (secondary, if steel { 182 - 18 } else { 91 - 9 }),
                ],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let further = superheater_obs(
                &[
                    (bar, 1),
                    (primary, primary_n - 1),
                    (secondary, secondary_n - if steel { 2 } else { 1 }),
                    (NATURE_RUNE_ID, 40),
                ],
                &[],
                &[(staff, 1)],
                10_106,
                1_062,
                smithing,
            );

            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &further])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());

            let xp_only = superheater_obs(
                &[(NATURE_RUNE_ID, 50)],
                &[],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            assert!(witness(
                case,
                &baseline,
                [&xp_only, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Bronze bar".into(), 9);
            name_only.items.insert("Nature rune".into(), 41);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(
                witness(case, &baseline, [&first, &stale, &withdrawn, &further])
                    .qualify()
                    .is_err()
            );
            let mut closed_deposit = deposited.clone();
            closed_deposit.bank_open = false;
            assert!(witness(
                case,
                &baseline,
                [&first, &closed_deposit, &withdrawn, &further]
            )
            .qualify()
            .is_err());
            withdrawn.bank_generation = 2;
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &further])
                    .qualify()
                    .is_err()
            );
            withdrawn.bank_generation = 1;

            let mut no_staff = first.clone();
            no_staff.equipment_ids.clear();
            assert!(witness(
                case,
                &baseline,
                [&no_staff, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            if steel {
                let mut one_coal = withdrawn.clone();
                one_coal.item_ids.insert(COAL_ID, 9);
                assert!(
                    witness(case, &baseline, [&first, &deposited, &one_coal, &further])
                        .qualify()
                        .is_err()
                );
                let mut iron_bar = first.clone();
                iron_bar.item_ids.insert(IRON_BAR_ID, 1);
                assert!(witness(
                    case,
                    &baseline,
                    [&iron_bar, &deposited, &withdrawn, &further]
                )
                .qualify()
                .is_err());
            } else {
                let mut steel_bar = first.clone();
                steel_bar.item_ids.insert(STEEL_BAR_ID, 1);
                assert!(witness(
                    case,
                    &baseline,
                    [&steel_bar, &deposited, &withdrawn, &further]
                )
                .qualify()
                .is_err());
            }

            if case == CoreCase::SuperheaterFireBattlestaff {
                let mut default_staff = first.clone();
                default_staff.equipment_ids.clear();
                default_staff.equipment_ids.insert(STAFF_OF_FIRE_ID, 1);
                assert!(witness(
                    case,
                    &baseline,
                    [&default_staff, &deposited, &withdrawn, &further]
                )
                .qualify()
                .is_err());
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(bar, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn vial_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
    ) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn vial_filler_requires_fountain_fill_deposit_restock_and_further_fill() {
        for (case, bank) in [
            (CoreCase::VialFiller, FALADOR_WEST_BANK),
            (CoreCase::VialFillerEast, FALADOR_EAST_BANK),
        ] {
            let baseline = vial_obs(bank, &[], &[]);
            validate_case_baseline(case, &baseline).unwrap();

            let filled = vial_obs(FALADOR_FOUNTAIN, &[(VIAL_OF_WATER_ID, 28)], &[]);
            let mut deposited = vial_obs(bank, &[], &[(VIAL_OF_WATER_ID, 28), (EMPTY_VIAL_ID, 28)]);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = vial_obs(bank, &[(EMPTY_VIAL_ID, 28)], &[(VIAL_OF_WATER_ID, 28)]);
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let mut returned = vial_obs(FALADOR_FOUNTAIN, &[(EMPTY_VIAL_ID, 28)], &[]);
            returned.bank_generation = 2;
            let mut further = vial_obs(FALADOR_FOUNTAIN, &[(VIAL_OF_WATER_ID, 1)], &[]);
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&filled, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&filled]).qualify().is_err());
            assert!(witness(
                case,
                &baseline,
                [&filled, &deposited, &withdrawn, &returned]
            )
            .qualify()
            .is_err());

            // A fabricated closed snapshot without a new modal generation is stale.
            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &filled,
                    &deposited,
                    &withdrawn,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = filled.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Vial of water".into(), 28);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&filled, &stale, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut not_fountain = filled.clone();
            not_fountain.tile = Some(bank);
            assert!(witness(
                case,
                &baseline,
                [&not_fountain, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(VIAL_OF_WATER_ID, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn potion_obs(
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        herblore_xp: i32,
        herblore_level: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("herblore", herblore_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("herblore".into(), herblore_level);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn potion_maker_requires_staged_unf_finished_deposit_and_further_product() {
        for (case, herb, unf, secondary, finished, wrong_unf, wrong_finished, named, level) in [
            (
                CoreCase::PotionMaker,
                GUAM_LEAF_ID,
                GUAM_UNF_ID,
                EYE_OF_NEWT_ID,
                ATTACK_POTION_3_ID,
                RANARR_UNF_ID,
                PRAYER_POTION_3_ID,
                false,
                3,
            ),
            (
                CoreCase::PotionMakerNamed,
                RANARR_WEED_ID,
                RANARR_UNF_ID,
                SNAPE_GRASS_ID,
                PRAYER_POTION_3_ID,
                GUAM_UNF_ID,
                ATTACK_POTION_3_ID,
                true,
                38,
            ),
        ] {
            let mut baseline = potion_obs(&[], &[], 1_000, level);
            if named {
                baseline.bank_ids.insert(GUAM_LEAF_ID, 14);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let unfinished = potion_obs(&[(unf, 14)], &[], 1_000, level);
            let made = potion_obs(&[(finished, 14)], &[], 1_025, level);
            let mut deposited_bank = vec![
                (finished, 14),
                (herb, 28),
                (VIAL_OF_WATER_ID, 28),
                (secondary, 28),
            ];
            if named {
                deposited_bank.push((GUAM_LEAF_ID, 14));
            }
            let mut deposited = potion_obs(&[], &deposited_bank, 1_025, level);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn_bank = vec![
                (finished, 14),
                (herb, 14),
                (VIAL_OF_WATER_ID, 14),
                (secondary, 28),
            ];
            if named {
                withdrawn_bank.push((GUAM_LEAF_ID, 14));
            }
            let mut withdrawn = potion_obs(
                &[(herb, 14), (VIAL_OF_WATER_ID, 14)],
                &withdrawn_bank,
                1_025,
                level,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let further = potion_obs(&[(unf, 14), (VIAL_OF_WATER_ID, 0)], &[], 1_025, level);

            assert!(witness(
                case,
                &baseline,
                [&unfinished, &made, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&unfinished]).qualify().is_err());
            assert!(witness(case, &baseline, [&unfinished, &made])
                .qualify()
                .is_err());
            assert!(witness(
                case,
                &baseline,
                [&unfinished, &made, &deposited, &withdrawn]
            )
            .qualify()
            .is_err());

            let mut name_only = unfinished.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Unfinished potion".into(), 14);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &made, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&unfinished, &made, &stale, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut wrong = made.clone();
            wrong.item_ids.insert(wrong_finished, 1);
            assert!(witness(
                case,
                &baseline,
                [&unfinished, &wrong, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());
            let mut wrong_unfinished = unfinished.clone();
            wrong_unfinished.item_ids.insert(wrong_unf, 1);
            assert!(witness(
                case,
                &baseline,
                [&wrong_unfinished, &made, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            if named {
                let mut took_guam = withdrawn.clone();
                took_guam.item_ids.insert(GUAM_LEAF_ID, 14);
                took_guam.bank_ids.insert(GUAM_LEAF_ID, 0);
                assert!(witness(
                    case,
                    &baseline,
                    [&unfinished, &made, &deposited, &took_guam, &further]
                )
                .qualify()
                .is_err());
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(finished, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
            let mut seeded_unf = baseline.clone();
            seeded_unf.item_ids.insert(unf, 1);
            assert!(validate_case_baseline(case, &seeded_unf).is_err());
        }
    }

    fn tanner_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        main_modal: i32,
        widgets: &[i32],
    ) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.main_modal = main_modal;
        observation.widget_ids = widgets.iter().copied().collect();
        observation
    }

    #[test]
    fn tanner_bot_requires_widget_conversion_deposit_restock_and_further_tan() {
        for (case, product, wrong, tan_all) in [
            (
                CoreCase::TannerBot,
                SOFT_LEATHER_ID,
                HARD_LEATHER_ID,
                SOFT_TAN_ALL_COM,
            ),
            (
                CoreCase::TannerBotHard,
                HARD_LEATHER_ID,
                SOFT_LEATHER_ID,
                HARD_TAN_ALL_COM,
            ),
        ] {
            let baseline = tanner_obs(AL_KHARID_BANK, &[], &[], -1, &[]);
            validate_case_baseline(case, &baseline).unwrap();

            let widget = tanner_obs(
                TANNER_STAND,
                &[(COW_HIDE_ID, 27), (COINS_ID, 2000)],
                &[],
                TANNER_IF,
                &[tan_all],
            );
            let tanned = tanner_obs(
                TANNER_STAND,
                &[(product, 27), (COINS_ID, 1973)],
                &[],
                TANNER_IF,
                &[tan_all],
            );
            let mut deposited = tanner_obs(
                AL_KHARID_BANK,
                &[(COINS_ID, 1973)],
                &[(product, 27), (COW_HIDE_ID, 1), (COINS_ID, 3000)],
                -1,
                &[],
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = tanner_obs(
                AL_KHARID_BANK,
                &[(COW_HIDE_ID, 1), (COINS_ID, 1973)],
                &[(product, 27), (COINS_ID, 3000)],
                -1,
                &[],
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let mut returned = tanner_obs(
                TANNER_STAND,
                &[(COW_HIDE_ID, 1), (COINS_ID, 1973)],
                &[],
                -1,
                &[],
            );
            returned.bank_generation = 2;
            let mut further = tanner_obs(
                TANNER_STAND,
                &[(product, 1), (COINS_ID, 1972)],
                &[],
                -1,
                &[],
            );
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&widget, &tanned, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&widget]).qualify().is_err());
            assert!(witness(case, &baseline, [&widget, &tanned])
                .qualify()
                .is_err());
            assert!(witness(
                case,
                &baseline,
                [&widget, &tanned, &deposited, &withdrawn, &returned]
            )
            .qualify()
            .is_err());

            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &widget,
                    &tanned,
                    &deposited,
                    &withdrawn,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = tanned.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Leather".into(), 27);
            assert!(witness(
                case,
                &baseline,
                [&widget, &name_only, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&widget, &tanned, &stale, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut coins_only = tanned.clone();
            coins_only.item_ids.remove(&product);
            assert!(witness(
                case,
                &baseline,
                [
                    &widget,
                    &coins_only,
                    &deposited,
                    &withdrawn,
                    &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let queued = widget.clone();
            assert!(witness(
                case,
                &baseline,
                [&queued, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut shop = widget.clone();
            shop.tile = Some(DOMMIK_STAND);
            shop.main_modal = SHOPMAIN;
            shop.widget_ids.clear();
            let mut shop_tanned = tanned.clone();
            shop_tanned.tile = Some(DOMMIK_STAND);
            shop_tanned.main_modal = SHOPMAIN;
            assert!(witness(
                case,
                &baseline,
                [
                    &shop,
                    &shop_tanned,
                    &deposited,
                    &withdrawn,
                    &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut wrong_obs = tanned.clone();
            wrong_obs.item_ids.insert(wrong, 1);
            assert!(witness(
                case,
                &baseline,
                [&widget, &wrong_obs, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(product, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn runecraft_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        xp: i32,
        rc_level: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("runecraft", xp)], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.levels.insert("runecraft".into(), rc_level);
        observation
    }

    #[test]
    fn rune_crafter_requires_temple_conversion_portal_deposit_restock_and_further_craft() {
        for (case, bank, ruins, altar, rune, wrong, talisman, rc_level) in [
            (
                CoreCase::RuneCrafter,
                FALADOR_EAST_BANK,
                RUNECRAFTER_AIR_RUINS,
                AIR_ALTAR,
                AIR_RUNE_ID,
                EARTH_RUNE_ID,
                AIR_TALISMAN_ID,
                1,
            ),
            (
                CoreCase::RuneCrafterEarth,
                VARROCK_EAST_BANK,
                RUNECRAFTER_EARTH_RUINS,
                EARTH_ALTAR,
                EARTH_RUNE_ID,
                AIR_RUNE_ID,
                EARTH_TALISMAN_ID,
                9,
            ),
            (
                CoreCase::MuleCrafter,
                FALADOR_EAST_BANK,
                MULECRAFTER_AIR_RUINS,
                AIR_ALTAR,
                AIR_RUNE_ID,
                EARTH_RUNE_ID,
                AIR_TALISMAN_ID,
                1,
            ),
        ] {
            let baseline = runecraft_obs(bank, &[], &[], 0, rc_level);
            validate_case_baseline(case, &baseline).unwrap();

            let withdrawn = runecraft_obs(
                bank,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[],
                0,
                rc_level,
            );
            let entered = runecraft_obs(
                altar,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[],
                0,
                rc_level,
            );
            let crafted = runecraft_obs(altar, &[(rune, 27), (talisman, 1)], &[], 5, rc_level);
            let exited = runecraft_obs(ruins, &[(rune, 27), (talisman, 1)], &[], 5, rc_level);
            let mut deposited = runecraft_obs(
                bank,
                &[(talisman, 1)],
                &[(rune, 27), (RUNE_ESSENCE_ID, 173)],
                5,
                rc_level,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut restocked = runecraft_obs(
                bank,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[(rune, 27), (RUNE_ESSENCE_ID, 146)],
                5,
                rc_level,
            );
            restocked.bank_open = true;
            restocked.bank_loaded = true;
            restocked.bank_generation = 1;
            let mut returned = runecraft_obs(
                ruins,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[],
                5,
                rc_level,
            );
            returned.bank_generation = 2;
            let mut further = runecraft_obs(altar, &[(rune, 27), (talisman, 1)], &[], 10, rc_level);
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &crafted, &exited, &deposited, &restocked, &returned,
                    &further,
                ]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&withdrawn]).qualify().is_err());
            assert!(witness(case, &baseline, [&withdrawn, &entered])
                .qualify()
                .is_err());
            assert!(witness(case, &baseline, [&withdrawn, &entered, &crafted])
                .qualify()
                .is_err());
            assert!(
                witness(case, &baseline, [&withdrawn, &entered, &crafted, &exited])
                    .qualify()
                    .is_err()
            );
            assert!(witness(
                case,
                &baseline,
                [&withdrawn, &entered, &crafted, &exited, &deposited, &restocked, &returned]
            )
            .qualify()
            .is_err());

            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn,
                    &entered,
                    &crafted,
                    &exited,
                    &deposited,
                    &restocked,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = crafted.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Air rune".into(), 27);
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &name_only, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &crafted, &exited, &stale, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut xp_only = crafted.clone();
            xp_only.item_ids.remove(&rune);
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &xp_only, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let queued = entered.clone();
            assert!(witness(
                case,
                &baseline,
                [&withdrawn, &queued, &exited, &deposited, &restocked, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut wrong_obs = crafted.clone();
            wrong_obs.item_ids.insert(wrong, 1);
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &wrong_obs, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut noted = withdrawn.clone();
            noted.item_ids.insert(NOTED_ESSENCE_ID, 27);
            noted.item_ids.remove(&RUNE_ESSENCE_ID);
            assert!(witness(
                case,
                &baseline,
                [
                    &noted, &entered, &crafted, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(rune, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    #[test]
    fn catalog_failure_emits_accumulated_core_witness_and_latest_observation() {
        assert_eq!(
            accumulated_core(None),
            json!({"error": "no Start baseline"})
        );

        let baseline = runecraft_obs(VARROCK_EAST_BANK, &[], &[], 0, 9);
        let withdrawn = runecraft_obs(
            VARROCK_EAST_BANK,
            &[(RUNE_ESSENCE_ID, 27), (EARTH_TALISMAN_ID, 1)],
            &[],
            0,
            9,
        );
        let entered = runecraft_obs(
            EARTH_ALTAR,
            &[(RUNE_ESSENCE_ID, 27), (EARTH_TALISMAN_ID, 1)],
            &[],
            0,
            9,
        );
        let crafted = runecraft_obs(
            EARTH_ALTAR,
            &[(EARTH_RUNE_ID, 27), (EARTH_TALISMAN_ID, 1)],
            &[],
            5,
            9,
        );
        let witness = witness(
            CoreCase::RuneCrafterEarth,
            &baseline,
            [&withdrawn, &entered, &crafted],
        );
        assert!(witness.qualify().is_err());
        let core = accumulated_core(Some(&witness));
        assert_eq!(
            core["error"],
            json!("rune_crafter_earth core post-Start delta incomplete")
        );
        assert!(core["witness"]["rune_crafter_cycle"]["crafted"].is_object());
        assert!(core["witness"]["rune_crafter_cycle"]["deposited"].is_null());
        assert_eq!(
            core["witness"]["rune_crafter_cycle"]["further"],
            json!(false)
        );
        assert_eq!(core["witness"]["latest"]["item_ids"]["557"], json!(27));
        assert_eq!(core["witness"]["latest"]["xp"]["runecraft"], json!(5));
        assert_eq!(core["witness"]["post_start_observations"], json!(3));
    }

    fn ardy_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        xp: i32,
        thieving: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("thieving", xp)], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.levels.insert("thieving".into(), thieving);
        observation.levels.insert("hitpoints".into(), 40);
        observation
    }

    #[test]
    fn ardy_cakes_requires_stall_food_xp_fresh_deposit_return_and_further_steal() {
        let baseline = ardy_obs(ARDY_CAKES_STAND, &[], &[], 0, 5);
        validate_case_baseline(CoreCase::ArdyCakes, &baseline).unwrap();

        let stolen = ardy_obs(ARDY_CAKES_STAND, &[(CAKE_ID, 4), (BREAD_ID, 2)], &[], 64, 5);
        let mut deposited = ardy_obs(ARDY_BANK, &[], &[(CAKE_ID, 4), (BREAD_ID, 2)], 64, 5);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = ardy_obs(ARDY_CAKES_STAND, &[], &[], 64, 5);
        returned.bank_generation = 2;
        let mut further = ardy_obs(ARDY_CAKES_STAND, &[(CHOCOLATE_SLICE_ID, 1)], &[], 80, 5);
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::ArdyCakes, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::ArdyCakes, &baseline, [&stolen])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &deposited, &returned]
        )
        .qualify()
        .is_err());

        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let mut name_only = stolen.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Cake".into(), 4);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &stale, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &closed, &returned, &further]
        )
        .qualify()
        .is_err());

        let xp_only = ardy_obs(ARDY_CAKES_STAND, &[], &[], 64, 5);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut wrong = stolen.clone();
        wrong.item_ids.insert(CHOCOLATE_CAKE_ID, 1);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&wrong, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut noted = stolen.clone();
        noted.item_ids.insert(NOTED_CAKE_ID, 1);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&noted, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(CAKE_ID, 1);
        assert!(validate_case_baseline(CoreCase::ArdyCakes, &seeded).is_err());
        let market = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 5);
        assert!(validate_case_baseline(CoreCase::ArdyCakes, &market).is_err());
    }

    #[test]
    fn ardy_thiever_requires_coins_xp_fresh_deposit_return_and_further_pickpocket() {
        for (case, thieving) in [
            (CoreCase::ArdyThiever, 40),
            (CoreCase::ArdyThieverKnight, 55),
        ] {
            let baseline = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, thieving);
            validate_case_baseline(case, &baseline).unwrap();

            let cakes_only = ardy_obs(ARDY_THIEVER_STAND, &[(CAKE_ID, 1)], &[], 16, thieving);
            let pickpocketed = ardy_obs(
                ARDY_THIEVER_STAND,
                &[(COINS_ID, 30), (CAKE_ID, 1)],
                &[],
                484,
                thieving,
            );
            let mut deposited =
                ardy_obs(ARDY_BANK, &[(CAKE_ID, 1)], &[(COINS_ID, 30)], 484, thieving);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut returned = ardy_obs(ARDY_THIEVER_STAND, &[(CAKE_ID, 1)], &[], 484, thieving);
            returned.bank_generation = 2;
            let mut further = ardy_obs(
                ARDY_THIEVER_STAND,
                &[(COINS_ID, 30), (CAKE_ID, 1)],
                &[],
                952,
                thieving,
            );
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &deposited, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&cakes_only]).qualify().is_err());
            assert!(witness(case, &baseline, [&pickpocketed]).qualify().is_err());
            assert!(
                witness(case, &baseline, [&pickpocketed, &deposited, &returned])
                    .qualify()
                    .is_err()
            );

            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &pickpocketed,
                    &deposited,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = pickpocketed.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Coins".into(), 30);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &stale, &returned, &further]
            )
            .qualify()
            .is_err());
            let mut closed = deposited.clone();
            closed.bank_open = false;
            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &closed, &returned, &further]
            )
            .qualify()
            .is_err());

            let xp_only = ardy_obs(ARDY_THIEVER_STAND, &[(CAKE_ID, 1)], &[], 484, thieving);
            assert!(
                witness(case, &baseline, [&xp_only, &deposited, &returned, &further])
                    .qualify()
                    .is_err()
            );

            let mut far = ardy_obs((3185, 3440, 0), &[(CAKE_ID, 1)], &[], 484, thieving);
            far.bank_generation = 2;
            let mut further_far = further.clone();
            further_far.tile = Some((3185, 3440, 0));
            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &deposited, &far, &further_far]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(COINS_ID, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }

        let guard_low = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 39);
        assert!(validate_case_baseline(CoreCase::ArdyThiever, &guard_low).is_err());
        let knight_low = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 54);
        assert!(validate_case_baseline(CoreCase::ArdyThieverKnight, &knight_low).is_err());
        validate_case_baseline(
            CoreCase::ArdyThiever,
            &ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 40),
        )
        .unwrap();
        assert!(validate_case_baseline(
            CoreCase::ArdyThieverKnight,
            &ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 40)
        )
        .is_err());
    }

    fn resource_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        equipment_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        levels: &[(&str, i32)],
    ) -> Observation {
        let mut observation = observation(&[], xp, &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.equipment_ids = equipment_ids.iter().copied().collect();
        for (name, level) in levels {
            observation.levels.insert((*name).into(), *level);
        }
        observation
    }

    #[test]
    fn gnome_chop_requires_log_xp_upstairs_deposit_ground_return_and_further_chop() {
        let levels = [("woodcutting", 75), ("fletching", 1)];
        let baseline = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 0)],
            &levels,
        );
        validate_case_baseline(CoreCase::GnomeChop, &baseline).unwrap();

        let chopped = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1), (MAGIC_LOGS_ID, 4)],
            &[],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        let mut deposited = resource_obs(
            GNOME_BANK_STAND,
            &[(STEEL_AXE_ID, 1)],
            &[(MAGIC_LOGS_ID, 4)],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = resource_obs(
            GNOME_BANK_STAIR_SOUTH,
            &[(STEEL_AXE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        returned.bank_generation = 2;
        let mut further = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1), (MAGIC_LOGS_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 500)],
            &levels,
        );
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::GnomeChop, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::GnomeChop, &baseline, [&chopped])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &deposited, &returned]
        )
        .qualify()
        .is_err());

        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let mut name_only = chopped.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Magic logs".into(), 4);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &stale, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &closed, &returned, &further]
        )
        .qualify()
        .is_err());

        let xp_only = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut wrong = chopped.clone();
        wrong.item_ids.insert(MAGIC_SHORTBOW_ID, 1);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&wrong, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut noted = chopped.clone();
        noted.item_ids.insert(NOTED_MAGIC_LOGS_ID, 1);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&noted, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(MAGIC_LOGS_ID, 1);
        assert!(validate_case_baseline(CoreCase::GnomeChop, &seeded).is_err());
        let low = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 0)],
            &[("woodcutting", 74), ("fletching", 1)],
        );
        assert!(validate_case_baseline(CoreCase::GnomeChop, &low).is_err());
        let no_axe = resource_obs(
            GNOME_WEST_MAGICS,
            &[],
            &[],
            &[],
            &[("woodcutting", 0)],
            &levels,
        );
        assert!(validate_case_baseline(CoreCase::GnomeChop, &no_axe).is_err());
        let wielded = resource_obs(
            GNOME_WEST_MAGICS,
            &[],
            &[],
            &[(STEEL_AXE_ID, 1)],
            &[("woodcutting", 0)],
            &levels,
        );
        validate_case_baseline(CoreCase::GnomeChop, &wielded).unwrap();
        let flax = resource_obs(
            FLAX_FIELD,
            &[(STEEL_AXE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 0)],
            &levels,
        );
        assert!(validate_case_baseline(CoreCase::GnomeChop, &flax).is_err());
    }

    #[test]
    fn gnome_fletch_requires_unstrung_product_xp_deposit_return_and_further_chop() {
        for (case, fletching, product, other) in [
            (
                CoreCase::GnomeFletchShort,
                80,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                UNSTRUNG_MAGIC_LONGBOW_ID,
            ),
            (
                CoreCase::GnomeFletchLong,
                85,
                UNSTRUNG_MAGIC_LONGBOW_ID,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
            ),
        ] {
            let levels = [("woodcutting", 75), ("fletching", fletching)];
            let baseline = resource_obs(
                GNOME_WEST_MAGICS,
                &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1)],
                &[],
                &[],
                &[("woodcutting", 0), ("fletching", 0)],
                &levels,
            );
            validate_case_baseline(case, &baseline).unwrap();

            let chopped = resource_obs(
                GNOME_WEST_MAGICS,
                &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1), (MAGIC_LOGS_ID, 4)],
                &[],
                &[],
                &[("woodcutting", 250), ("fletching", 0)],
                &levels,
            );
            let fletched = resource_obs(
                GNOME_WEST_MAGICS,
                &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1), (product, 4)],
                &[],
                &[],
                &[("woodcutting", 250), ("fletching", 168)],
                &levels,
            );
            let mut deposited = resource_obs(
                GNOME_BANK_STAND,
                &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1)],
                &[(product, 4)],
                &[],
                &[("woodcutting", 250), ("fletching", 168)],
                &levels,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut returned = resource_obs(
                GNOME_BANK_STAIR_SOUTH,
                &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1)],
                &[],
                &[],
                &[("woodcutting", 250), ("fletching", 168)],
                &levels,
            );
            returned.bank_generation = 2;
            let mut further = resource_obs(
                GNOME_WEST_MAGICS,
                &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1), (MAGIC_LOGS_ID, 1)],
                &[],
                &[],
                &[("woodcutting", 500), ("fletching", 168)],
                &levels,
            );
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&chopped, &fletched, &deposited, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&chopped]).qualify().is_err());
            assert!(witness(case, &baseline, [&chopped, &fletched])
                .qualify()
                .is_err());
            assert!(witness(
                case,
                &baseline,
                [&chopped, &fletched, &deposited, &returned]
            )
            .qualify()
            .is_err());

            let mut strung = fletched.clone();
            strung.item_ids.insert(MAGIC_SHORTBOW_ID, 1);
            assert!(witness(
                case,
                &baseline,
                [&chopped, &strung, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut cross = fletched.clone();
            cross.item_ids.insert(other, 1);
            assert!(witness(
                case,
                &baseline,
                [&chopped, &cross, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut no_knife = baseline.clone();
            no_knife.item_ids.remove(&KNIFE_ID);
            assert!(validate_case_baseline(case, &no_knife).is_err());
        }

        let short_high = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 0), ("fletching", 0)],
            &[("woodcutting", 75), ("fletching", 85)],
        );
        assert!(validate_case_baseline(CoreCase::GnomeFletchShort, &short_high).is_err());
        validate_case_baseline(CoreCase::GnomeFletchLong, &short_high).unwrap();
        let long_low = resource_obs(
            GNOME_WEST_MAGICS,
            &[(STEEL_AXE_ID, 1), (KNIFE_ID, 1)],
            &[],
            &[],
            &[("woodcutting", 0), ("fletching", 0)],
            &[("woodcutting", 75), ("fletching", 84)],
        );
        assert!(validate_case_baseline(CoreCase::GnomeFletchLong, &long_low).is_err());
        validate_case_baseline(CoreCase::GnomeFletchShort, &long_low).unwrap();
    }

    #[test]
    fn coal_trucks_requires_mining_xp_truck_deposit_not_bank_and_further_mine() {
        let levels = [("mining", 30)];
        let baseline = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 0)],
            &levels,
        );
        validate_case_baseline(CoreCase::CoalTrucks, &baseline).unwrap();

        let mined = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1), (COAL_ID, 27)],
            &[],
            &[],
            &[("mining", 1350)],
            &levels,
        );
        let trucked = resource_obs(
            COAL_MINE_TRUCK_STAND,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 1350)],
            &levels,
        );
        let further = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1), (COAL_ID, 1)],
            &[],
            &[],
            &[("mining", 1400)],
            &levels,
        );

        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&mined, &trucked, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::CoalTrucks, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::CoalTrucks, &baseline, [&mined])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::CoalTrucks, &baseline, [&mined, &trucked])
            .qualify()
            .is_err());

        let mut name_only = mined.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Coal".into(), 27);
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&name_only, &trucked, &further]
        )
        .qualify()
        .is_err());

        let xp_only = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 1350)],
            &levels,
        );
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&xp_only, &trucked, &further]
        )
        .qualify()
        .is_err());

        let mut banked = trucked.clone();
        banked.bank_open = true;
        banked.bank_loaded = true;
        banked.bank_generation = 1;
        banked.bank_ids.insert(COAL_ID, 27);
        banked.tile = Some(SEERS_BANK);
        assert!(
            witness(CoreCase::CoalTrucks, &baseline, [&mined, &banked, &further])
                .qualify()
                .is_err()
        );

        let mut noted = mined.clone();
        noted.item_ids.insert(NOTED_COAL_ID, 1);
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&noted, &trucked, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(COAL_ID, 1);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &seeded).is_err());
        let low = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 0)],
            &[("mining", 29)],
        );
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &low).is_err());
        let no_pick = resource_obs(COAL_MINE, &[], &[], &[], &[("mining", 0)], &levels);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &no_pick).is_err());
        let wrong_stand = resource_obs(
            SEERS_BANK,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 0)],
            &levels,
        );
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &wrong_stand).is_err());
    }

    #[test]
    fn station_production_requires_xp_product_deposit_restock_return_and_further() {
        let cook_levels = [("cooking", COOKING_FIXTURE_LEVEL)];
        let baseline = resource_obs(
            CATHERBY_BANK,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &cook_levels,
        );
        validate_case_baseline(CoreCase::CookBot, &baseline).unwrap();

        let withdrawn = resource_obs(
            CATHERBY_BANK,
            &[(RAW_SALMON_ID, 28)],
            &[(RAW_SALMON_ID, 28)],
            &[],
            &[("cooking", 0)],
            &cook_levels,
        );
        let produced = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(SALMON_ID, 28)],
            &[(RAW_SALMON_ID, 28)],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        let mut deposited = resource_obs(
            CATHERBY_BANK,
            &[],
            &[(SALMON_ID, 28), (RAW_SALMON_ID, 28)],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut restocked = resource_obs(
            CATHERBY_BANK,
            &[(RAW_SALMON_ID, 28)],
            &[(SALMON_ID, 28)],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        restocked.bank_open = true;
        restocked.bank_loaded = true;
        restocked.bank_generation = 1;
        let mut returned = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(RAW_SALMON_ID, 28)],
            &[],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        returned.bank_generation = 2;
        let mut further = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(SALMON_ID, 1)],
            &[],
            &[],
            &[("cooking", 500)],
            &cook_levels,
        );
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::CookBot, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::CookBot, &baseline, [&withdrawn])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::CookBot, &baseline, [&withdrawn, &produced])
                .qualify()
                .is_err()
        );
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &deposited, &restocked, &returned]
        )
        .qualify()
        .is_err());

        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [
                &withdrawn,
                &produced,
                &deposited,
                &restocked,
                &unclosed_return,
                &unclosed_further
            ]
        )
        .qualify()
        .is_err());

        let mut name_only = produced.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Salmon".into(), 28);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &name_only, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &stale, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &closed, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let xp_only = resource_obs(
            CATHERBY_RANGE_STAND,
            &[],
            &[],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &xp_only, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let no_consume = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(RAW_SALMON_ID, 28), (SALMON_ID, 1)],
            &[],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [
                &withdrawn,
                &no_consume,
                &deposited,
                &restocked,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_err());

        let mut wrong = produced.clone();
        wrong.item_ids.insert(LOBSTER_ID, 1);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &wrong, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut noted = produced.clone();
        noted.item_ids.insert(NOTED_SALMON_ID, 1);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &noted, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut burnt = produced.clone();
        burnt.item_ids.insert(BURNT_FISH_2_ID, 1);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &burnt, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(SALMON_ID, 1);
        assert!(validate_case_baseline(CoreCase::CookBot, &seeded).is_err());
        let low = resource_obs(
            CATHERBY_BANK,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &[("cooking", 79)],
        );
        assert!(validate_case_baseline(CoreCase::CookBot, &low).is_err());
        let wrong_stand = resource_obs(
            CATHERBY_RANGE_STAND,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &cook_levels,
        );
        assert!(validate_case_baseline(CoreCase::CookBot, &wrong_stand).is_err());

        let lobster_levels = [("cooking", COOKING_FIXTURE_LEVEL)];
        let lobster_baseline = resource_obs(
            CATHERBY_BANK,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &lobster_levels,
        );
        validate_case_baseline(CoreCase::CookBotLobster, &lobster_baseline).unwrap();
        let lobster_withdrawn = resource_obs(
            CATHERBY_BANK,
            &[(RAW_LOBSTER_ID, 28)],
            &[],
            &[],
            &[("cooking", 0)],
            &lobster_levels,
        );
        let lobster_produced = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(LOBSTER_ID, 28)],
            &[],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        let mut lobster_deposited = resource_obs(
            CATHERBY_BANK,
            &[],
            &[(LOBSTER_ID, 28), (RAW_LOBSTER_ID, 28)],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        lobster_deposited.bank_open = true;
        lobster_deposited.bank_loaded = true;
        lobster_deposited.bank_generation = 1;
        let mut lobster_restocked = resource_obs(
            CATHERBY_BANK,
            &[(RAW_LOBSTER_ID, 28)],
            &[(LOBSTER_ID, 28)],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        lobster_restocked.bank_open = true;
        lobster_restocked.bank_loaded = true;
        lobster_restocked.bank_generation = 1;
        let mut lobster_returned = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(RAW_LOBSTER_ID, 28)],
            &[],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        lobster_returned.bank_generation = 2;
        let mut lobster_further = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(LOBSTER_ID, 1)],
            &[],
            &[],
            &[("cooking", 500)],
            &lobster_levels,
        );
        lobster_further.bank_generation = 2;
        assert!(witness(
            CoreCase::CookBotLobster,
            &lobster_baseline,
            [
                &lobster_withdrawn,
                &lobster_produced,
                &lobster_deposited,
                &lobster_restocked,
                &lobster_returned,
                &lobster_further
            ]
        )
        .qualify()
        .is_ok());
        let mut lobster_cross = lobster_produced.clone();
        lobster_cross.item_ids.insert(SALMON_ID, 1);
        assert!(witness(
            CoreCase::CookBotLobster,
            &lobster_baseline,
            [
                &lobster_withdrawn,
                &lobster_cross,
                &lobster_deposited,
                &lobster_restocked,
                &lobster_returned,
                &lobster_further
            ]
        )
        .qualify()
        .is_err());

        let bronze_levels = [("smithing", 1)];
        let bronze_baseline = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[],
            &[],
            &[("smithing", 0)],
            &bronze_levels,
        );
        validate_case_baseline(CoreCase::SmelterBot, &bronze_baseline).unwrap();
        let bronze_withdrawn = resource_obs(
            AL_KHARID_BANK,
            &[(COPPER_ORE_ID, 14), (TIN_ORE_ID, 14)],
            &[],
            &[],
            &[("smithing", 0)],
            &bronze_levels,
        );
        let bronze_produced = resource_obs(
            AL_KHARID_FURNACE,
            &[(BRONZE_BAR_ID, 14)],
            &[],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        let mut bronze_deposited = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[(BRONZE_BAR_ID, 14), (COPPER_ORE_ID, 42), (TIN_ORE_ID, 42)],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        bronze_deposited.bank_open = true;
        bronze_deposited.bank_loaded = true;
        bronze_deposited.bank_generation = 1;
        let mut bronze_restocked = resource_obs(
            AL_KHARID_BANK,
            &[(COPPER_ORE_ID, 14), (TIN_ORE_ID, 14)],
            &[(BRONZE_BAR_ID, 14), (COPPER_ORE_ID, 28), (TIN_ORE_ID, 28)],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        bronze_restocked.bank_open = true;
        bronze_restocked.bank_loaded = true;
        bronze_restocked.bank_generation = 1;
        let mut bronze_returned = resource_obs(
            AL_KHARID_FURNACE,
            &[(COPPER_ORE_ID, 14), (TIN_ORE_ID, 14)],
            &[],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        bronze_returned.bank_generation = 2;
        let mut bronze_further = resource_obs(
            AL_KHARID_FURNACE,
            &[(BRONZE_BAR_ID, 1)],
            &[],
            &[],
            &[("smithing", 93)],
            &bronze_levels,
        );
        bronze_further.bank_generation = 2;
        assert!(witness(
            CoreCase::SmelterBot,
            &bronze_baseline,
            [
                &bronze_withdrawn,
                &bronze_produced,
                &bronze_deposited,
                &bronze_restocked,
                &bronze_returned,
                &bronze_further
            ]
        )
        .qualify()
        .is_ok());
        let mut bronze_wrong = bronze_produced.clone();
        bronze_wrong.item_ids.insert(STEEL_BAR_ID, 1);
        assert!(witness(
            CoreCase::SmelterBot,
            &bronze_baseline,
            [
                &bronze_withdrawn,
                &bronze_wrong,
                &bronze_deposited,
                &bronze_restocked,
                &bronze_returned,
                &bronze_further
            ]
        )
        .qualify()
        .is_err());
        let low_smith = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[],
            &[],
            &[("smithing", 0)],
            &[("smithing", 0)],
        );
        assert!(validate_case_baseline(CoreCase::SmelterBot, &low_smith).is_err());
        assert!(validate_case_baseline(CoreCase::SmelterBotSteel, &low_smith).is_err());
        validate_case_baseline(CoreCase::SmelterBot, &bronze_baseline).unwrap();
        assert!(validate_case_baseline(CoreCase::SmelterBotSteel, &bronze_baseline).is_err());

        let steel_levels = [("smithing", 30)];
        let steel_baseline = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[],
            &[],
            &[("smithing", 0)],
            &steel_levels,
        );
        validate_case_baseline(CoreCase::SmelterBotSteel, &steel_baseline).unwrap();
        let steel_withdrawn = resource_obs(
            AL_KHARID_BANK,
            &[(IRON_ORE_ID, 9), (COAL_ID, 18)],
            &[],
            &[],
            &[("smithing", 0)],
            &steel_levels,
        );
        let steel_produced = resource_obs(
            AL_KHARID_FURNACE,
            &[(STEEL_BAR_ID, 9)],
            &[],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        let mut steel_deposited = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[(STEEL_BAR_ID, 9), (IRON_ORE_ID, 47), (COAL_ID, 94)],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        steel_deposited.bank_open = true;
        steel_deposited.bank_loaded = true;
        steel_deposited.bank_generation = 1;
        let mut steel_restocked = resource_obs(
            AL_KHARID_BANK,
            &[(IRON_ORE_ID, 9), (COAL_ID, 18)],
            &[(STEEL_BAR_ID, 9), (IRON_ORE_ID, 38), (COAL_ID, 76)],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        steel_restocked.bank_open = true;
        steel_restocked.bank_loaded = true;
        steel_restocked.bank_generation = 1;
        let mut steel_returned = resource_obs(
            AL_KHARID_FURNACE,
            &[(IRON_ORE_ID, 9), (COAL_ID, 18)],
            &[],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        steel_returned.bank_generation = 2;
        let mut steel_further = resource_obs(
            AL_KHARID_FURNACE,
            &[(STEEL_BAR_ID, 1)],
            &[],
            &[],
            &[("smithing", 175)],
            &steel_levels,
        );
        steel_further.bank_generation = 2;
        assert!(witness(
            CoreCase::SmelterBotSteel,
            &steel_baseline,
            [
                &steel_withdrawn,
                &steel_produced,
                &steel_deposited,
                &steel_restocked,
                &steel_returned,
                &steel_further
            ]
        )
        .qualify()
        .is_ok());

        let spin_levels = [("crafting", 1)];
        let spin_baseline = resource_obs(
            FLAX_SPINNER_BANK,
            &[],
            &[],
            &[],
            &[("crafting", 0)],
            &spin_levels,
        );
        validate_case_baseline(CoreCase::FlaxSpinner, &spin_baseline).unwrap();
        let spin_withdrawn = resource_obs(
            FLAX_SPINNER_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 0)],
            &spin_levels,
        );
        let spin_produced = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        let mut spin_deposited = resource_obs(
            FLAX_SPINNER_BANK,
            &[],
            &[(BOW_STRING_ID, 28), (FLAX_ID, 28)],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        spin_deposited.bank_open = true;
        spin_deposited.bank_loaded = true;
        spin_deposited.bank_generation = 1;
        let mut spin_restocked = resource_obs(
            FLAX_SPINNER_BANK,
            &[(FLAX_ID, 28)],
            &[(BOW_STRING_ID, 28)],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        spin_restocked.bank_open = true;
        spin_restocked.bank_loaded = true;
        spin_restocked.bank_generation = 1;
        let mut spin_returned = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        spin_returned.bank_generation = 2;
        let mut spin_further = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &spin_levels,
        );
        spin_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxSpinner,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &spin_returned,
                &spin_further
            ]
        )
        .qualify()
        .is_ok());
        let mut wool = spin_produced.clone();
        wool.item_ids.insert(BALL_OF_WOOL_ID, 1);
        assert!(witness(
            CoreCase::FlaxSpinner,
            &spin_baseline,
            [
                &spin_withdrawn,
                &wool,
                &spin_deposited,
                &spin_restocked,
                &spin_returned,
                &spin_further
            ]
        )
        .qualify()
        .is_err());
        let mut ground_return = resource_obs(
            FLAX_SPINNER_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        ground_return.bank_generation = 2;
        let mut ground_further = resource_obs(
            FLAX_SPINNER_BANK,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &spin_levels,
        );
        ground_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxSpinner,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &ground_return,
                &ground_further
            ]
        )
        .qualify()
        .is_err());
        let mut seeded_string = spin_baseline.clone();
        seeded_string.item_ids.insert(BOW_STRING_ID, 1);
        assert!(validate_case_baseline(CoreCase::FlaxSpinner, &seeded_string).is_err());
    }

    #[test]
    fn flax_aio_and_secondary_cycles_require_exact_ids_deposit_return_and_further() {
        let craft = [("crafting", 1)];
        let aio_baseline = resource_obs(FLAX_FIELD, &[], &[], &[], &[("crafting", 0)], &craft);
        validate_case_baseline(CoreCase::FlaxAio, &aio_baseline).unwrap();
        let picked = resource_obs(
            FLAX_FIELD,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 0)],
            &craft,
        );
        let produced = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        let mut deposited = resource_obs(
            FLAX_AIO_BANK,
            &[],
            &[(BOW_STRING_ID, 28)],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = resource_obs(FLAX_FIELD, &[], &[], &[], &[("crafting", 420)], &craft);
        returned.bank_generation = 2;
        let mut further = resource_obs(
            FLAX_FIELD,
            &[(FLAX_ID, 1)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &produced, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::FlaxAio, &aio_baseline, [&aio_baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::FlaxAio, &aio_baseline, [&picked])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::FlaxAio, &aio_baseline, [&picked, &produced])
                .qualify()
                .is_err()
        );
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &produced, &deposited, &returned]
        )
        .qualify()
        .is_err());
        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [
                &picked,
                &produced,
                &deposited,
                &unclosed_return,
                &unclosed_further
            ]
        )
        .qualify()
        .is_err());
        let no_consume = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(FLAX_ID, 28), (BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &no_consume, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let xp_only = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut wool = produced.clone();
        wool.item_ids.insert(BALL_OF_WOOL_ID, 1);
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &wool, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut noted = produced.clone();
        noted.item_ids.insert(NOTED_BOW_STRING_ID, 1);
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &noted, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut name_only = produced.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Bow string".into(), 28);
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut seeded = aio_baseline.clone();
        seeded.item_ids.insert(BOW_STRING_ID, 1);
        assert!(validate_case_baseline(CoreCase::FlaxAio, &seeded).is_err());

        let pick_baseline = flax_obs(FLAX_FIELD, &[], &[]);
        validate_case_baseline(CoreCase::FlaxAioPick, &pick_baseline).unwrap();
        let first = flax_obs(FLAX_FIELD, &[(FLAX_ID, 28)], &[]);
        let mut pick_deposited = flax_obs(FLAX_AIO_BANK, &[], &[(FLAX_ID, 28)]);
        pick_deposited.bank_open = true;
        pick_deposited.bank_loaded = true;
        pick_deposited.bank_generation = 1;
        let mut pick_returned = flax_obs(FLAX_FIELD, &[], &[]);
        pick_returned.bank_generation = 2;
        let mut pick_further = flax_obs(FLAX_FIELD, &[(FLAX_ID, 1)], &[]);
        pick_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAioPick,
            &pick_baseline,
            [&first, &pick_deposited, &pick_returned, &pick_further]
        )
        .qualify()
        .is_ok());
        let mut spun = first.clone();
        spun.item_ids.insert(BOW_STRING_ID, 1);
        assert!(witness(
            CoreCase::FlaxAioPick,
            &pick_baseline,
            [&spun, &pick_deposited, &pick_returned, &pick_further]
        )
        .qualify()
        .is_err());
        let mut seeded_pick = pick_baseline.clone();
        seeded_pick.item_ids.insert(FLAX_ID, 28);
        assert!(validate_case_baseline(CoreCase::FlaxAioPick, &seeded_pick).is_err());

        let spin_baseline = resource_obs(FLAX_AIO_BANK, &[], &[], &[], &[("crafting", 0)], &craft);
        validate_case_baseline(CoreCase::FlaxAioSpin, &spin_baseline).unwrap();
        let spin_withdrawn = resource_obs(
            FLAX_AIO_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 0)],
            &craft,
        );
        let spin_produced = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        let mut spin_deposited = resource_obs(
            FLAX_AIO_BANK,
            &[],
            &[(BOW_STRING_ID, 28), (FLAX_ID, 28)],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        spin_deposited.bank_open = true;
        spin_deposited.bank_loaded = true;
        spin_deposited.bank_generation = 1;
        let mut spin_restocked = resource_obs(
            FLAX_AIO_BANK,
            &[(FLAX_ID, 28)],
            &[(BOW_STRING_ID, 28)],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        spin_restocked.bank_open = true;
        spin_restocked.bank_loaded = true;
        spin_restocked.bank_generation = 1;
        let mut spin_returned = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        spin_returned.bank_generation = 2;
        let mut spin_further = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &craft,
        );
        spin_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAioSpin,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &spin_returned,
                &spin_further
            ]
        )
        .qualify()
        .is_ok());
        let mut ground_return = resource_obs(
            FLAX_AIO_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        ground_return.bank_generation = 2;
        let mut ground_further = resource_obs(
            FLAX_AIO_BANK,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &craft,
        );
        ground_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAioSpin,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &ground_return,
                &ground_further
            ]
        )
        .qualify()
        .is_err());

        let egg_baseline = flax_obs(EGG_FIELD, &[], &[]);
        validate_case_baseline(CoreCase::HerbloreSecondaries, &egg_baseline).unwrap();
        let taken = flax_obs(EGG_FIELD, &[(RED_SPIDERS_EGGS_ID, 18)], &[]);
        let mut egg_deposited = flax_obs(EGG_FIELD, &[], &[(RED_SPIDERS_EGGS_ID, 18)]);
        egg_deposited.bank_open = true;
        egg_deposited.bank_loaded = true;
        egg_deposited.bank_generation = 1;
        let mut egg_returned = flax_obs(EGG_FIELD, &[], &[]);
        egg_returned.bank_generation = 2;
        let mut egg_further = flax_obs(EGG_FIELD, &[(RED_SPIDERS_EGGS_ID, 1)], &[]);
        egg_further.bank_generation = 2;
        assert!(witness(
            CoreCase::HerbloreSecondaries,
            &egg_baseline,
            [&taken, &egg_deposited, &egg_returned, &egg_further]
        )
        .qualify()
        .is_ok());
        assert!(
            witness(CoreCase::HerbloreSecondaries, &egg_baseline, [&taken])
                .qualify()
                .is_err()
        );
        let mut mixed = taken.clone();
        mixed.item_ids.insert(EYE_OF_NEWT_ID, 1);
        assert!(witness(
            CoreCase::HerbloreSecondaries,
            &egg_baseline,
            [&mixed, &egg_deposited, &egg_returned, &egg_further]
        )
        .qualify()
        .is_err());
        let mut noted_egg = taken.clone();
        noted_egg.item_ids.insert(NOTED_RED_SPIDERS_EGGS_ID, 1);
        assert!(witness(
            CoreCase::HerbloreSecondaries,
            &egg_baseline,
            [&noted_egg, &egg_deposited, &egg_returned, &egg_further]
        )
        .qualify()
        .is_err());
        let mut seeded_egg = egg_baseline.clone();
        seeded_egg.item_ids.insert(RED_SPIDERS_EGGS_ID, 1);
        assert!(validate_case_baseline(CoreCase::HerbloreSecondaries, &seeded_egg).is_err());

        let newt_baseline = flax_obs(BETTY_SHOP, &[], &[]);
        validate_case_baseline(CoreCase::HerbloreSecondariesNewt, &newt_baseline).unwrap();
        let coins_held = flax_obs(BETTY_SHOP, &[(COINS_ID, 5000)], &[]);
        let bought = flax_obs(BETTY_SHOP, &[(COINS_ID, 4900), (EYE_OF_NEWT_ID, 10)], &[]);
        let mut newt_deposited = flax_obs(BETTY_SHOP, &[(COINS_ID, 4900)], &[(EYE_OF_NEWT_ID, 10)]);
        newt_deposited.bank_open = true;
        newt_deposited.bank_loaded = true;
        newt_deposited.bank_generation = 1;
        let mut newt_returned = flax_obs(BETTY_SHOP, &[(COINS_ID, 4900)], &[]);
        newt_returned.bank_generation = 2;
        let mut newt_further = flax_obs(BETTY_SHOP, &[(COINS_ID, 4800), (EYE_OF_NEWT_ID, 1)], &[]);
        newt_further.bank_generation = 2;
        assert!(witness(
            CoreCase::HerbloreSecondariesNewt,
            &newt_baseline,
            [
                &coins_held,
                &bought,
                &newt_deposited,
                &newt_returned,
                &newt_further
            ]
        )
        .qualify()
        .is_ok());
        let no_spend = flax_obs(BETTY_SHOP, &[(COINS_ID, 5000), (EYE_OF_NEWT_ID, 10)], &[]);
        let mut no_spend_deposited =
            flax_obs(BETTY_SHOP, &[(COINS_ID, 5000)], &[(EYE_OF_NEWT_ID, 10)]);
        no_spend_deposited.bank_open = true;
        no_spend_deposited.bank_loaded = true;
        no_spend_deposited.bank_generation = 1;
        let mut no_spend_returned = flax_obs(BETTY_SHOP, &[(COINS_ID, 5000)], &[]);
        no_spend_returned.bank_generation = 2;
        let mut no_spend_further =
            flax_obs(BETTY_SHOP, &[(COINS_ID, 5000), (EYE_OF_NEWT_ID, 1)], &[]);
        no_spend_further.bank_generation = 2;
        assert!(witness(
            CoreCase::HerbloreSecondariesNewt,
            &newt_baseline,
            [
                &coins_held,
                &no_spend,
                &no_spend_deposited,
                &no_spend_returned,
                &no_spend_further
            ]
        )
        .qualify()
        .is_err());
        let mut mixed_buy = bought.clone();
        mixed_buy.item_ids.insert(RED_SPIDERS_EGGS_ID, 1);
        assert!(witness(
            CoreCase::HerbloreSecondariesNewt,
            &newt_baseline,
            [
                &coins_held,
                &mixed_buy,
                &newt_deposited,
                &newt_returned,
                &newt_further
            ]
        )
        .qualify()
        .is_err());
        let mut seeded_newt = newt_baseline.clone();
        seeded_newt.item_ids.insert(EYE_OF_NEWT_ID, 1);
        assert!(validate_case_baseline(CoreCase::HerbloreSecondariesNewt, &seeded_newt).is_err());
    }

    fn combat_npc(
        index: usize,
        name: &str,
        health: i32,
        in_combat: bool,
        tile: (i32, i32, i32),
    ) -> BoundedNpc {
        BoundedNpc {
            index,
            name: Some(name.into()),
            health,
            total_health: 60,
            animation: if in_combat { 422 } else { 0 },
            in_combat,
            targeting_local: in_combat,
            tile,
            distance: 1,
        }
    }

    fn combat_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        levels: &[(&str, i32)],
        npcs: &[BoundedNpc],
        local_in_combat: bool,
        local_target_npc: Option<usize>,
    ) -> Observation {
        let mut observation =
            resource_obs(tile, item_ids, &[], &[(ADAMANT_SCIMITAR_ID, 1)], xp, levels);
        observation.npc_facts = npcs.to_vec();
        observation.local_in_combat = local_in_combat;
        observation.local_target_npc = local_target_npc;
        observation.local_health = 40;
        observation
    }

    #[test]
    fn combat_cores_require_two_engagements_verified_defeat_style_xp_and_exact_loot() {
        let levels = [
            ("attack", COMBAT_ATTACK_LEVEL),
            ("strength", COMBAT_ATTACK_LEVEL),
            ("hitpoints", COMBAT_ATTACK_LEVEL),
        ];
        let moss_base = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100), ("attack", 100)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::MossGiant, &moss_base).unwrap();

        let first = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100), ("attack", 100)],
            &levels,
            &[combat_npc(4, "Moss giant", 50, true, MOSS_GIANT_SAFESPOT)],
            true,
            Some(4),
        );
        let second = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 104), ("attack", 100)],
            &levels,
            &[
                combat_npc(4, "Moss giant", 0, false, MOSS_GIANT_SAFESPOT),
                combat_npc(7, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT),
            ],
            true,
            Some(7),
        );
        let mut looted = second.clone();
        looted.item_ids.insert(BIG_BONES_ID, 1);
        looted.npc_facts = vec![combat_npc(7, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT)];
        assert!(
            witness(CoreCase::MossGiant, &moss_base, [&first, &second, &looted])
                .qualify()
                .is_ok()
        );

        assert!(witness(CoreCase::MossGiant, &moss_base, [&moss_base])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::MossGiant, &moss_base, [&first])
            .qualify()
            .is_err());
        let xp_only = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 108), ("attack", 100)],
            &levels,
            &[],
            false,
            None,
        );
        assert!(witness(CoreCase::MossGiant, &moss_base, [&xp_only])
            .qualify()
            .is_err());
        let attack_only = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 100), ("attack", 108)],
            &levels,
            &[
                combat_npc(4, "Moss giant", 0, true, MOSS_GIANT_SAFESPOT),
                combat_npc(7, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT),
            ],
            true,
            Some(4),
        );
        assert!(
            witness(CoreCase::MossGiant, &moss_base, [&first, &attack_only])
                .qualify()
                .is_err()
        );
        let despawn = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 104), ("attack", 100)],
            &levels,
            &[],
            false,
            None,
        );
        assert!(witness(CoreCase::MossGiant, &moss_base, [&first, &despawn])
            .qualify()
            .is_err());
        let mut noted = looted.clone();
        noted.item_ids.insert(NOTED_BIG_BONES_ID, 1);
        assert!(
            witness(CoreCase::MossGiant, &moss_base, [&first, &second, &noted])
                .qualify()
                .is_err()
        );
        let mut seeded = moss_base.clone();
        seeded.item_ids.insert(BIG_BONES_ID, 1);
        assert!(validate_case_baseline(CoreCase::MossGiant, &seeded).is_err());

        let hill_base = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD)],
            &[("strength", 100)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::HillGiant, &hill_base).unwrap();
        let hill_first = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD)],
            &[("strength", 100)],
            &levels,
            &[combat_npc(2, "Giant", 35, true, HILL_GIANT_PIT)],
            true,
            Some(2),
        );
        let hill_second = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 110)],
            &levels,
            &[
                combat_npc(2, "Giant", 0, false, HILL_GIANT_PIT),
                combat_npc(3, "Giant", 20, true, HILL_GIANT_PIT),
            ],
            true,
            Some(3),
        );
        assert!(
            witness(CoreCase::HillGiant, &hill_base, [&hill_first, &hill_second])
                .qualify()
                .is_ok()
        );
        let alias = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 110)],
            &levels,
            &[
                combat_npc(2, "Hill giant", 0, true, HILL_GIANT_PIT),
                combat_npc(3, "Hill giant", 20, true, HILL_GIANT_PIT),
            ],
            true,
            Some(2),
        );
        assert!(witness(CoreCase::HillGiant, &hill_base, [&alias])
            .qualify()
            .is_err());

        let chaos_base = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 50)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::ChaosDruid, &chaos_base).unwrap();
        let chaos_first = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 50)],
            &levels,
            &[combat_npc(1, "Chaos druid", 20, true, CHAOS_DRUID_FIELD)],
            true,
            Some(1),
        );
        let chaos_second = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (UNIDENTIFIED_GUAM_ID, 1)],
            &[("strength", 54)],
            &levels,
            &[
                combat_npc(1, "Chaos druid", 0, false, CHAOS_DRUID_FIELD),
                combat_npc(8, "Chaos druid", 18, true, CHAOS_DRUID_FIELD),
            ],
            true,
            Some(8),
        );
        assert!(witness(
            CoreCase::ChaosDruid,
            &chaos_base,
            [&chaos_first, &chaos_second]
        )
        .qualify()
        .is_ok());
        let mut noted_herb = chaos_second.clone();
        noted_herb.item_ids.insert(NOTED_HERB_ID, 1);
        assert!(witness(
            CoreCase::ChaosDruid,
            &chaos_base,
            [&chaos_first, &noted_herb]
        )
        .qualify()
        .is_err());

        let auto_base = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::AutoFighter, &auto_base).unwrap();
        let auto_first = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &levels,
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );
        let auto_second = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 88)],
            &levels,
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
        );
        assert!(witness(
            CoreCase::AutoFighter,
            &auto_base,
            [&auto_first, &auto_second]
        )
        .qualify()
        .is_ok());

        let rock_base = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 40)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::RockCrab, &rock_base).unwrap();
        let rocks = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 40)],
            &levels,
            &[combat_npc(3, "Rocks", 50, false, ROCK_CRAB_SPOT)],
            false,
            None,
        );
        let woke = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 40)],
            &levels,
            &[combat_npc(3, "Rock Crab", 50, true, ROCK_CRAB_SPOT)],
            true,
            Some(3),
        );
        let rock_second = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 48)],
            &levels,
            &[
                combat_npc(3, "Rock Crab", 0, false, ROCK_CRAB_SPOT),
                combat_npc(6, "Rock Crab", 40, true, ROCK_CRAB_SPOT),
            ],
            true,
            Some(6),
        );
        assert!(witness(
            CoreCase::RockCrab,
            &rock_base,
            [&rocks, &woke, &rock_second]
        )
        .qualify()
        .is_ok());
        assert!(
            witness(CoreCase::RockCrab, &rock_base, [&woke, &rock_second])
                .qualify()
                .is_err()
        );
        let crab_alias = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 48)],
            &levels,
            &[
                combat_npc(3, "Rock crab", 0, true, ROCK_CRAB_SPOT),
                combat_npc(6, "Rock crab", 40, true, ROCK_CRAB_SPOT),
            ],
            true,
            Some(3),
        );
        assert!(
            witness(CoreCase::RockCrab, &rock_base, [&rocks, &crab_alias])
                .qualify()
                .is_err()
        );

        let mut dragon_base = combat_obs(
            GREEN_DRAGON_FIELD,
            &[(LOBSTER_ID, GREEN_DRAGON_FOOD)],
            &[("strength", 90)],
            &levels,
            &[],
            false,
            None,
        );
        dragon_base.equipment_ids.clear();
        dragon_base.equipment_ids.insert(RUNE_SCIMITAR_ID, 1);
        dragon_base.equipment_ids.insert(DRAGONFIRE_SHIELD_ID, 1);
        validate_case_baseline(CoreCase::GreenDragon, &dragon_base).unwrap();
        let mut pack_only = dragon_base.clone();
        pack_only.equipment_ids.remove(&DRAGONFIRE_SHIELD_ID);
        pack_only.item_ids.insert(DRAGONFIRE_SHIELD_ID, 1);
        assert!(validate_case_baseline(CoreCase::GreenDragon, &pack_only).is_err());
        let dragon_first = {
            let mut observation = combat_obs(
                GREEN_DRAGON_FIELD,
                &[(LOBSTER_ID, GREEN_DRAGON_FOOD)],
                &[("strength", 90)],
                &levels,
                &[combat_npc(2, "Green dragon", 80, true, GREEN_DRAGON_FIELD)],
                true,
                Some(2),
            );
            observation.equipment_ids = dragon_base.equipment_ids.clone();
            observation
        };
        let dragon_second = {
            let mut observation = combat_obs(
                GREEN_DRAGON_FIELD,
                &[(LOBSTER_ID, GREEN_DRAGON_FOOD), (DRAGON_BONES_ID, 1)],
                &[("strength", 110)],
                &levels,
                &[
                    combat_npc(2, "Green dragon", 0, false, GREEN_DRAGON_FIELD),
                    combat_npc(4, "Green dragon", 70, true, GREEN_DRAGON_FIELD),
                ],
                true,
                Some(4),
            );
            observation.equipment_ids = dragon_base.equipment_ids.clone();
            observation
        };
        assert!(witness(
            CoreCase::GreenDragon,
            &dragon_base,
            [&dragon_first, &dragon_second]
        )
        .qualify()
        .is_ok());
        let mut hide_ok = dragon_second.clone();
        hide_ok.item_ids.remove(&DRAGON_BONES_ID);
        hide_ok.item_ids.insert(GREEN_DRAGONHIDE_ID, 1);
        assert!(witness(
            CoreCase::GreenDragon,
            &dragon_base,
            [&dragon_first, &hide_ok]
        )
        .qualify()
        .is_ok());
        let mut wrong_hide = dragon_second.clone();
        wrong_hide.item_ids.insert(BLACK_DRAGONHIDE_ID, 1);
        assert!(witness(
            CoreCase::GreenDragon,
            &dragon_base,
            [&dragon_first, &wrong_hide]
        )
        .qualify()
        .is_err());
        let dragon_alias = {
            let mut observation = combat_obs(
                GREEN_DRAGON_FIELD,
                &[(LOBSTER_ID, GREEN_DRAGON_FOOD), (DRAGON_BONES_ID, 1)],
                &[("strength", 110)],
                &levels,
                &[
                    combat_npc(2, "Green Dragon", 0, true, GREEN_DRAGON_FIELD),
                    combat_npc(4, "Green Dragon", 70, true, GREEN_DRAGON_FIELD),
                ],
                true,
                Some(2),
            );
            observation.equipment_ids = dragon_base.equipment_ids.clone();
            observation
        };
        assert!(
            witness(CoreCase::GreenDragon, &dragon_base, [&dragon_alias])
                .qualify()
                .is_err()
        );
        let mut unworn = dragon_second.clone();
        unworn.equipment_ids.remove(&DRAGONFIRE_SHIELD_ID);
        unworn.item_ids.insert(DRAGONFIRE_SHIELD_ID, 1);
        let mut unworn_first = dragon_first.clone();
        unworn_first.equipment_ids.remove(&DRAGONFIRE_SHIELD_ID);
        assert!(witness(
            CoreCase::GreenDragon,
            &dragon_base,
            [&unworn_first, &unworn]
        )
        .qualify()
        .is_err());

        let fire_base = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
            ],
            &[("strength", 70)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::FireGiant, &fire_base).unwrap();
        let mut no_amulet = fire_base.clone();
        no_amulet.item_ids.remove(&GLARIALS_AMULET_ID);
        assert!(validate_case_baseline(CoreCase::FireGiant, &no_amulet).is_err());
        let fire_first = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
            ],
            &[("strength", 70)],
            &levels,
            &[combat_npc(8, "Fire giant", 90, true, FIRE_GIANT_ROOM)],
            true,
            Some(8),
        );
        let fire_second = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
                (BIG_BONES_ID, 1),
            ],
            &[("strength", 86)],
            &levels,
            &[
                combat_npc(8, "Fire giant", 0, false, FIRE_GIANT_ROOM),
                combat_npc(11, "Fire giant", 80, true, FIRE_GIANT_ROOM),
            ],
            true,
            Some(11),
        );
        assert!(
            witness(CoreCase::FireGiant, &fire_base, [&fire_first, &fire_second])
                .qualify()
                .is_ok()
        );
        let fire_alias = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
                (BIG_BONES_ID, 1),
            ],
            &[("strength", 86)],
            &levels,
            &[
                combat_npc(8, "Fire Giant", 0, true, FIRE_GIANT_ROOM),
                combat_npc(11, "Fire Giant", 80, true, FIRE_GIANT_ROOM),
            ],
            true,
            Some(8),
        );
        assert!(witness(CoreCase::FireGiant, &fire_base, [&fire_alias])
            .qualify()
            .is_err());

        let mut ardy_base = combat_obs(
            ARDY_THIEVER_STAND,
            &[],
            &[("strength", 60)],
            &levels,
            &[],
            false,
            None,
        );
        ardy_base.levels.insert("thieving".into(), 5);
        validate_case_baseline(CoreCase::ArdyFighter, &ardy_base).unwrap();
        let mut seeded_cake = ardy_base.clone();
        seeded_cake.item_ids.insert(CAKE_ID, 1);
        assert!(validate_case_baseline(CoreCase::ArdyFighter, &seeded_cake).is_err());
        let stolen = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[(CAKE_ID, 1)],
                &[("strength", 60)],
                &levels,
                &[],
                false,
                None,
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        let ardy_first = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[(CAKE_ID, 1)],
                &[("strength", 60)],
                &levels,
                &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
                true,
                Some(5),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        let ardy_second = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[(CAKE_ID, 1)],
                &[("strength", 72)],
                &levels,
                &[
                    combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                    combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
                ],
                true,
                Some(9),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        assert!(witness(
            CoreCase::ArdyFighter,
            &ardy_base,
            [&stolen, &ardy_first, &ardy_second]
        )
        .qualify()
        .is_ok());
        let ardy_first_empty = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[],
                &[("strength", 60)],
                &levels,
                &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
                true,
                Some(5),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        let ardy_second_empty = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[],
                &[("strength", 72)],
                &levels,
                &[
                    combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                    combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
                ],
                true,
                Some(9),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        assert!(witness(
            CoreCase::ArdyFighter,
            &ardy_base,
            [&ardy_first_empty, &ardy_second_empty]
        )
        .qualify()
        .is_err());
        let chocolate = {
            let mut observation = ardy_second.clone();
            observation.item_ids.insert(CHOCOLATE_CAKE_ID, 1);
            observation
        };
        assert!(witness(
            CoreCase::ArdyFighter,
            &ardy_base,
            [&stolen, &ardy_first, &chocolate]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn old_catalog_explicitly_refuses_cut_string_mode() {
        let error =
            validate_case_catalog(CoreCase::BankFletcherCutString, CATALOG_COMMIT_A).unwrap_err();
        assert!(error.contains("has no mode setting"), "{error}");
        validate_case_catalog(CoreCase::BankFletcherCutString, CATALOG_COMMIT_B).unwrap();
        validate_case_catalog(CoreCase::BankFletcherString, CATALOG_COMMIT_A).unwrap();
        let staff_error =
            validate_case_catalog(CoreCase::SuperheaterFireBattlestaff, CATALOG_COMMIT_A)
                .unwrap_err();
        assert!(
            staff_error.contains("requires Staff of fire"),
            "{staff_error}"
        );
        validate_case_catalog(CoreCase::SuperheaterFireBattlestaff, CATALOG_COMMIT_B).unwrap();
        validate_case_catalog(CoreCase::Superheater, CATALOG_COMMIT_A).unwrap();
        validate_case_catalog(CoreCase::SuperheaterSteel, CATALOG_COMMIT_A).unwrap();
        for case in [
            CoreCase::AlcherCustomAlias,
            CoreCase::AlcherCustomName,
            CoreCase::DartFletcher,
            CoreCase::DartFletcherIron,
            CoreCase::HerbCleaner,
            CoreCase::HerbCleanerNamed,
            CoreCase::GemCutter,
            CoreCase::GemCutterNamed,
            CoreCase::DoorOpener,
            CoreCase::DoorOpenerGate,
            CoreCase::GnomeCourse,
            CoreCase::GnomeCourseRadius,
            CoreCase::FlaxPicker,
            CoreCase::Superheater,
            CoreCase::SuperheaterSteel,
            CoreCase::ChickenKillerBank,
            CoreCase::VialFiller,
            CoreCase::VialFillerEast,
            CoreCase::PotionMaker,
            CoreCase::PotionMakerNamed,
            CoreCase::TannerBot,
            CoreCase::TannerBotHard,
            CoreCase::RuneCrafter,
            CoreCase::RuneCrafterEarth,
            CoreCase::MuleCrafter,
            CoreCase::ArdyCakes,
            CoreCase::ArdyThiever,
            CoreCase::ArdyThieverKnight,
            CoreCase::GnomeChop,
            CoreCase::GnomeFletchShort,
            CoreCase::GnomeFletchLong,
            CoreCase::CoalTrucks,
            CoreCase::CookBot,
            CoreCase::CookBotLobster,
            CoreCase::SmelterBot,
            CoreCase::SmelterBotSteel,
            CoreCase::FlaxSpinner,
            CoreCase::FlaxAio,
            CoreCase::FlaxAioPick,
            CoreCase::FlaxAioSpin,
            CoreCase::HerbloreSecondaries,
            CoreCase::HerbloreSecondariesNewt,
            CoreCase::ChaosDruid,
            CoreCase::MossGiant,
            CoreCase::HillGiant,
            CoreCase::AutoFighter,
            CoreCase::RockCrab,
            CoreCase::GreenDragon,
            CoreCase::FireGiant,
            CoreCase::ArdyFighter,
        ] {
            validate_case_catalog(case, CATALOG_COMMIT_A).unwrap();
            validate_case_catalog(case, CATALOG_COMMIT_B).unwrap();
        }
    }
}

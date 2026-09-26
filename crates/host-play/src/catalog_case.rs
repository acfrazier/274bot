use super::*;
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

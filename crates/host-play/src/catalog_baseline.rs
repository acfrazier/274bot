use super::*;
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
use super::*;
/// GreenDragon melee in the wilderness field. Shield 1540 is worn with the
/// native fixture operation on the safe tile before hostile-field teleport;
/// this does not qualify the frozen GearEquip branch. Catalog Start baseline
/// and cycle both require worn 1540 plus real dragon combat and exact bones 536
/// or green hide 1753. Escape/bank is not this cell.
pub(crate) fn green_dragon_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "green_dragon",
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_BASE_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// Prepared GreenDragon melee core. The strict core needs one kill, its loot
/// and further work on a second dragon; the measured first 75-HP kill alone
/// took 265 ticks after the first Attack at 40/40/40 with the Rune scimitar,
/// which leaves the 180s gold wall no room for the rest.
pub(crate) fn green_dragon_prepared_scenario() -> Scenario {
    let mut scenario = prepared_combat_core_scenario(
        CombatCorePlan {
            name: "green_dragon_prepared",
            card: "GreenDragon",
            tele: GREEN_DRAGON_FIELD,
            radius: 22,
            food_alias: "lobster",
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BASE_FOOD,
            weapon_alias: "rune_scimitar",
            weapon_id: RUNE_SCIMITAR_ID,
            extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
            wear: Some((
                "wear and acknowledge Dragonfire shield before hostile-field teleport",
                DRAGONFIRE_SHIELD_ID,
            )),
            loot_empty: GREEN_DRAGON_LOOT_EMPTY,
            inject: GREEN_DRAGON_INJECT,
            complete_quest: None,
            thieving: 0,
            agility: 0,
        },
        PreparedCombatPlan {
            level: COMBAT_ATTACK_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        },
    );
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}

/// Prepared GreenDragon Fire Strike option. Exact AutoFighter mage rune
/// constants stay intact. No rune armour or scimitar; shield stays worn.
pub(crate) fn green_dragon_mage_prepared_scenario() -> Scenario {
    let magic_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare 70-stat Fire Strike kit and shield on the safe tile",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &format!("setstat magic {REMAINING_COMBAT_PREPARED_LEVEL}"),
                );
                cheat(
                    c,
                    &format!("setstat defence {REMAINING_COMBAT_PREPARED_LEVEL}"),
                );
                cheat(
                    c,
                    &format!("setstat hitpoints {REMAINING_COMBAT_PREPARED_LEVEL}"),
                );
                cheat(c, "~clearinv");
                cheat(c, "give staff_of_fire 1");
                cheat(c, "give antidragonbreathshield 1");
                cheat(c, &format!("give lobster {GREEN_DRAGON_FOOD}"));
                cheat(c, &format!("give mindrune {AUTO_FIGHTER_MAGE_CASTS}"));
                cheat(c, &format!("give airrune {AUTO_FIGHTER_MAGE_AIR_RUNES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: MAGIC_STAT,
                min: REMAINING_COMBAT_PREPARED_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (name, arm) in [
        (
            "acknowledge prepared Defence 70 before Start",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: REMAINING_COMBAT_PREPARED_LEVEL,
            },
        ),
        (
            "acknowledge prepared Hitpoints 70 before Start",
            Proof::Stat {
                id: 3,
                min: REMAINING_COMBAT_PREPARED_LEVEL,
            },
        ),
        (
            "acknowledge exact lobster 12 before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: GREEN_DRAGON_FOOD,
            },
        ),
        (
            "acknowledge 150 Mind runes before Start",
            Proof::ItemId {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS,
            },
        ),
        (
            "acknowledge 300 Air runes before Start",
            Proof::ItemId {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES,
            },
        ),
        (
            "acknowledge Staff of fire before wielding",
            Proof::ItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge Dragonfire shield before wearing",
            Proof::ItemId {
                id: DRAGONFIRE_SHIELD_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    for (name, id) in [
        (
            "wield and acknowledge Staff of fire before hostile-field teleport",
            STAFF_OF_FIRE_ID,
        ),
        (
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        ),
    ] {
        steps.push(wear_combat_item_step(name, id));
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(
                        GREEN_DRAGON_FIELD.level,
                        GREEN_DRAGON_FIELD.x,
                        GREEN_DRAGON_FIELD.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: GREEN_DRAGON_FIELD.x,
                z: GREEN_DRAGON_FIELD.z,
                level: GREEN_DRAGON_FIELD.level,
                radius: 22,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch native Fire Strike autocast become armed",
            Proof::Varp {
                id: AUTOCAST_MAGIC_VARP,
                min: AUTOCAST_ARMED_VALUE,
            },
        ),
        ("watch Magic XP from real Fire Strike combat", magic_xp),
        (
            "watch a Mind rune consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS - 1,
            },
        ),
        (
            "watch two Air runes consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES - 2,
            },
        ),
        (
            "watch the Dragonfire shield stay worn during mage combat",
            Proof::EquipmentId {
                id: DRAGONFIRE_SHIELD_ID,
            },
        ),
        (
            "watch a Green dragon in the wilderness field",
            Proof::NpcNameNear {
                name: "Green dragon",
                x: GREEN_DRAGON_FIELD.x,
                z: GREEN_DRAGON_FIELD.z,
                level: GREEN_DRAGON_FIELD.level,
                radius: 22,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    let mut scenario = Scenario {
        name: "green_dragon_mage_prepared",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: magic_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: COMBAT_QUALIFICATION_DEADLINE,
            start_script: Some("GreenDragon"),
            script_settings_inject: Some(GREEN_DRAGON_MAGE_PREPARED_INJECT),
            fixture_loadouts: combat_fixture_loadouts("GreenDragon"),
            terminal_shot: Some("green_dragon_mage_prepared"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    };
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}

fn green_dragon_special_scenario_with_preparation(
    name: &'static str,
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let prepared_level = preparation.map(|prepared| prepared.level);
    let plan = CombatCorePlan {
        name,
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "dragon_dagger",
        weapon_id: DRAGON_DAGGER_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_SPECIAL_INJECT,
        complete_quest: Some(LOST_CITY_PREREQ),
        thieving: 0,
        agility: 0,
    };
    let mut scenario = if let Some(preparation) = preparation {
        prepared_combat_core_scenario(plan, preparation)
    } else {
        combat_core_scenario(plan)
    };
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            Step {
                name: "prepare and acknowledge the Dragon-dagger Attack profile before Start",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        let attack = prepared_level.unwrap_or(DRAGON_DAGGER_ATTACK_LEVEL);
                        cheat(c, &format!("setstat attack {attack}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat {
                        id: 0,
                        min: prepared_level.unwrap_or(DRAGON_DAGGER_ATTACK_LEVEL),
                    },
                    budget_ticks: 200,
                },
            },
            wear_combat_item_step(
                "wield and acknowledge Dragon dagger before hostile-field teleport",
                DRAGON_DAGGER_ID,
            ),
            // The card only arms when `Special.energy() >= cost`, so the pool
            // has to already cover the dagger's 250 before Start; a low pool
            // must fail this acknowledgement instead of passing unarmed.
            bank_fletcher_watch(
                "acknowledge the worn dagger's special cost is covered before Start",
                Proof::Varp {
                    id: SPECIAL_ENERGY_VARP,
                    min: DRAGON_DAGGER_SPECIAL_COST,
                },
            ),
        ],
    );
    if preparation.is_some() {
        apply_combat_qualification_budget(
            &mut scenario,
            COMBAT_QUALIFICATION_DEADLINE,
            COMBAT_QUALIFICATION_WATCH_TICKS,
        );
    }
    scenario
}

pub(crate) fn green_dragon_special_scenario() -> Scenario {
    green_dragon_special_scenario_with_preparation("green_dragon_special", None)
}

pub(crate) fn green_dragon_special_prepared_scenario() -> Scenario {
    green_dragon_special_scenario_with_preparation(
        "green_dragon_special_prepared",
        Some(PreparedCombatPlan {
            level: REMAINING_COMBAT_PREPARED_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        }),
    )
}

fn green_dragon_potions_scenario_with_preparation(
    name: &'static str,
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let plan = CombatCorePlan {
        name,
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("3dose2attack", SUPER_ATTACK_3_ID, 1),
            ("3dose2strength", SUPER_STRENGTH_3_ID, 1),
        ],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_POTIONS_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    };
    let attack_boost_min = preparation
        .as_ref()
        .map(|prep| prep.level + 1)
        .unwrap_or(COMBAT_ATTACK_LEVEL + 1);
    let mut scenario = if let Some(preparation) = preparation {
        prepared_combat_core_scenario(plan, preparation)
    } else {
        combat_core_scenario(plan)
    };
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            bank_fletcher_watch(
                "confirm no seeded Super attack(2) flask before Start",
                Proof::ItemIdAtMost {
                    id: SUPER_ATTACK_2_ID,
                    count: 0,
                },
            ),
            bank_fletcher_watch(
                "confirm no seeded Super strength(2) flask before Start",
                Proof::ItemIdAtMost {
                    id: SUPER_STRENGTH_2_ID,
                    count: 0,
                },
            ),
        ],
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.splice(
        start + 1..start + 1,
        [
            bank_fletcher_watch(
                "watch the Super attack(3) dose leave the pack after Start",
                Proof::ItemIdAtMost {
                    id: SUPER_ATTACK_3_ID,
                    count: 0,
                },
            ),
            bank_fletcher_watch(
                "watch the Super attack(2) dose enter the pack after Start",
                Proof::ItemId {
                    id: SUPER_ATTACK_2_ID,
                    count: 1,
                },
            ),
            bank_fletcher_watch(
                "watch the native Super attack boost before further combat",
                Proof::Stat {
                    id: 0,
                    min: attack_boost_min,
                },
            ),
        ],
    );
    if preparation.is_some() {
        apply_combat_qualification_budget(
            &mut scenario,
            COMBAT_QUALIFICATION_DEADLINE,
            COMBAT_QUALIFICATION_WATCH_TICKS,
        );
    }
    scenario
}

pub(crate) fn green_dragon_potions_scenario() -> Scenario {
    green_dragon_potions_scenario_with_preparation("green_dragon_potions", None)
}

pub(crate) fn green_dragon_potions_prepared_scenario() -> Scenario {
    green_dragon_potions_scenario_with_preparation(
        "green_dragon_potions_prepared",
        Some(PreparedCombatPlan {
            level: REMAINING_COMBAT_PREPARED_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        }),
    )
}
fn wear_shield_before_hostile_teleport(scenario: &mut Scenario) {
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.insert(
        hostile_teleport,
        wear_combat_item_step(
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        ),
    );
}

fn prepare_safe_empty_food_escape_before_hostile_teleport(scenario: &mut Scenario) {
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            bank_fletcher_watch(
                "confirm the prepared escape profile carries no food before Start",
                Proof::ItemIdAtMost {
                    id: LOBSTER_ID,
                    count: 0,
                },
            ),
            Step {
                name: "prepare and acknowledge 70 of 99 Hitpoints for the configured panic trigger",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(
                            c,
                            &format!(
                                "~hit {}",
                                GREEN_DRAGON_TELE_PREPARED_LEVEL - GREEN_DRAGON_TELE_PREPARED_HP
                            ),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::StatAtMost {
                        id: 3,
                        max: GREEN_DRAGON_TELE_PREPARED_HP,
                    },
                    budget_ticks: 200,
                },
            },
            bank_fletcher_watch(
                "confirm the prepared teleport retains 70 Hitpoints before Start",
                Proof::Stat {
                    id: 3,
                    min: GREEN_DRAGON_TELE_PREPARED_HP,
                },
            ),
        ],
    );
}

/// GreenDragon BankRun to Edgeville: food-gone / pack-full trip deposits
/// bones or hide, restocks lobster to `foodWithdraw` 20, and walks back
/// past the wilderness ditch.
pub(crate) fn green_dragon_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "green_dragon_bank",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_BANK_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch dragon bones or hide enter a fresh Edgeville bank",
                Proof::BankItemIdAny {
                    ids: &GREEN_DRAGON_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            ("watch GreenDragon close its bank", Proof::BankClosed),
            (
                "watch return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    scenario
}

/// Prepared earned-kill full-bank witness. The stock names `Dragon bones` and
/// `Dragonhide` are selected explicitly because the compatibility drop catalog
/// omits the hide. Twenty-six Lobsters leave two slots for that guaranteed pair;
/// exact upstream-preparation stats keep the pressure cycle inside its bound.
pub(crate) fn green_dragon_bank_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "green_dragon_bank_prepared",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_BANK_PREPARED_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_BANK_PREPARED_INJECT,
        "lobster",
        40,
        BANK_PRESSURE_PREPARED_LEVEL,
        GREEN_DRAGON_BANK_QUALIFICATION_DEADLINE,
        BANK_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch earned dragon bones or hide enter a fresh Edgeville bank",
                Proof::BankItemIdAny {
                    ids: &GREEN_DRAGON_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the inventory-pressure trip draw Lobster to 27",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_PREPARED_RESTOCK,
                },
            ),
            (
                "watch prepared GreenDragon close its bank",
                Proof::BankClosed,
            ),
            (
                "watch prepared return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
                },
            ),
            (
                "watch fresh Strength XP after the prepared bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

/// Default-loot Green full cycle. Same prepared bank loadout as
/// `green_dragon_bank_prepared` (99/Rune/26+40 Lobster, no potions/clues/
/// burial/special/teleport) but the `loot` inject is omitted so the generated
/// source default must earn both guaranteed drops. No seeded cargo counts.
pub(crate) fn green_dragon_bank_default_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "green_dragon_bank_default_prepared",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_BANK_PREPARED_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_BANK_DEFAULT_PREPARED_INJECT,
        "lobster",
        40,
        BANK_PRESSURE_PREPARED_LEVEL,
        GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_DEADLINE,
        GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch earned Dragon bones in pack after the selected defeat",
                Proof::ItemId {
                    id: DRAGON_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch earned Dragonhide in pack after the selected defeat",
                Proof::ItemId {
                    id: GREEN_DRAGONHIDE_ID,
                    count: 1,
                },
            ),
            (
                "watch earned Dragonhide enter a fresh Edgeville bank",
                Proof::BankItemId {
                    id: GREEN_DRAGONHIDE_ID,
                    count: 1,
                },
            ),
            (
                "watch the inventory-pressure trip draw Lobster to 27",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_PREPARED_RESTOCK,
                },
            ),
            (
                "watch default-loot GreenDragon close its bank",
                Proof::BankClosed,
            ),
            (
                "watch prepared return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
                },
            ),
            (
                "watch fresh Strength XP after the prepared bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

/// `escape=Teleport to Varrock`: Magic XP and a Varrock land, then the
/// Edgeville booth restock and a return past the ditch. A south-walk flee
/// without the teleport fails this cell.
pub(crate) fn green_dragon_tele_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "green_dragon_tele",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("lawrune", LAW_RUNE_ID, VARROCK_TELE_LAW),
            ("airrune", AIR_RUNE_ID, VARROCK_TELE_AIR),
            ("firerune", FIRE_RUNE_ID, VARROCK_TELE_FIRE),
        ],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_TELE_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch Magic XP from the Varrock teleport after Start",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
            ),
            (
                "watch the Varrock teleport land, not a queued button",
                Proof::ArrivedNear {
                    x: VARROCK_TELE_LAND.x,
                    z: VARROCK_TELE_LAND.z,
                    level: VARROCK_TELE_LAND.level,
                    radius: 8,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            (
                "watch GreenDragon close its bank after the teleport",
                Proof::BankClosed,
            ),
            (
                "watch return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [Step {
            name: "prepare and acknowledge Magic 25 for Varrock teleport before Start",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, &format!("setstat magic {VARROCK_TELE_MAGIC}"));
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: VARROCK_TELE_MAGIC,
                },
                budget_ticks: 200,
            },
        }],
    );
    scenario
}

/// The pinned source requires both no food and HP below `panicHp` to select
/// `Escape`; no food by itself selects the direct `BankRun`. This prepared
/// fixture uses the upstream max-stat profile, configures `panicHp=98`, and
/// enters the field at a safe 70/99 HP instead of reproducing the upstream
/// regression's death-prone 1-HP seed. The supported escape must walk south,
/// cast Varrock, restock/heal at Edgeville, return to the field and produce
/// later Strength XP.
pub(crate) fn green_dragon_tele_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "green_dragon_tele_prepared",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        0,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("lawrune", LAW_RUNE_ID, VARROCK_TELE_LAW),
            ("airrune", AIR_RUNE_ID, VARROCK_TELE_AIR),
            ("firerune", FIRE_RUNE_ID, VARROCK_TELE_FIRE),
        ],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_TELE_PREPARED_INJECT,
        "lobster",
        40,
        GREEN_DRAGON_TELE_PREPARED_LEVEL,
        GREEN_DRAGON_TELE_QUALIFICATION_DEADLINE,
        GREEN_DRAGON_TELE_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch Magic XP from the prepared Varrock teleport after Start",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
            ),
            (
                "watch the prepared Varrock teleport land",
                Proof::ArrivedNear {
                    x: VARROCK_TELE_LAND.x,
                    z: VARROCK_TELE_LAND.z,
                    level: VARROCK_TELE_LAND.level,
                    radius: 8,
                },
            ),
            (
                "watch the prepared escape restock Lobster to 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            (
                "watch prepared GreenDragon close its bank after teleporting",
                Proof::BankClosed,
            ),
            (
                "watch prepared return to the dragon field after teleporting",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
                },
            ),
            (
                "watch fresh Strength XP after the prepared escape return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.insert(
        hostile_teleport,
        Step {
            name: "prepare and acknowledge Magic 25 for the prepared Varrock teleport",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, &format!("setstat magic {VARROCK_TELE_MAGIC}"));
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: VARROCK_TELE_MAGIC,
                },
                budget_ticks: 200,
            },
        },
    );
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    prepare_safe_empty_food_escape_before_hostile_teleport(&mut scenario);
    scenario
}
const GREEN_DRAGON_BANK_PREPARED_FOOD: i32 = 26;
const GREEN_DRAGON_BANK_PREPARED_RESTOCK: i32 = 27;
const GREEN_DRAGON_BANK_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(600);
/// Default-table loot fills more pack slots than the explicit inject sibling;
/// measured pressure trip + return needs a longer wall while keeping 2.5 dirty/s.
const GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_DEADLINE: Duration =
    Duration::from_secs(720);
const GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_WATCH_TICKS: u32 = 1800;
/// The measured GreenDragon teleport/Edgeville/return path predicts 475.2s
/// including fresh post-return XP and route margin.
const GREEN_DRAGON_TELE_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(480);
const GREEN_DRAGON_TELE_QUALIFICATION_WATCH_TICKS: u32 = 1200;
/// The pinned GreenDragon source only enters `Escape` when it is both foodless
/// and below `panicHp`. At 70/99 HP and a configured 98% threshold the branch
/// is deterministic while retaining enough absolute health for the south run.
const GREEN_DRAGON_TELE_PREPARED_LEVEL: i32 = 99;
const GREEN_DRAGON_TELE_PREPARED_HP: i32 = 70;
const GREEN_DRAGON_TELE_PANIC_PERCENT: i32 = 98;
pub(crate) const DRAGONFIRE_SHIELD_ID: i32 = 1540;
const DRAGON_DAGGER_ID: i32 = 1215;
const DRAGON_DAGGER_ATTACK_LEVEL: i32 = 60;
const SPECIAL_ENERGY_VARP: i32 = 300;
const DRAGON_DAGGER_SPECIAL_COST: i32 = 250;
const SUPER_ATTACK_3_ID: i32 = 145;
const SUPER_ATTACK_2_ID: i32 = 147;
const SUPER_STRENGTH_3_ID: i32 = 157;
const SUPER_STRENGTH_2_ID: i32 = 159;
pub(crate) const DRAGON_BONES_ID: i32 = 536;
const NOTED_DRAGON_BONES_ID: i32 = 537;
pub(crate) const GREEN_DRAGONHIDE_ID: i32 = 1753;
const NOTED_GREEN_DRAGONHIDE_ID: i32 = 1754;
const GREEN_DRAGON_FIELD: WorldTile = WorldTile {
    x: 3096,
    z: 3814,
    level: 0,
};
pub(crate) const VARROCK_TELE_LAND: WorldTile = WorldTile {
    x: 3213,
    z: 3424,
    level: 0,
};
pub(crate) const FIRE_RUNE_ID: i32 = 554;
pub(crate) const VARROCK_TELE_MAGIC: i32 = 25;
const VARROCK_TELE_LAW: i32 = 3;
const VARROCK_TELE_AIR: i32 = 9;
const VARROCK_TELE_FIRE: i32 = 3;
const GREEN_DRAGON_BANK_RESTOCK: i32 = 20;
const GREEN_DRAGON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_MAGE_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("mage"),
    },
    ScriptSettingInject {
        id: "spell",
        value: ScriptInjectValue::Str("Fire Strike"),
    },
    ScriptSettingInject {
        id: "staff",
        value: ScriptInjectValue::Str("Staff of fire"),
    },
    ScriptSettingInject {
        id: "runesWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_MAGE_CASTS as f64),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_SPECIAL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Dragon dagger"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_POTIONS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_TELE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Teleport to Varrock"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_TELE_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Teleport to Varrock"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "panicHp",
        value: ScriptInjectValue::Num(GREEN_DRAGON_TELE_PANIC_PERCENT as f64),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_LOOT_EMPTY: &[i32] = &[
    DRAGON_BONES_ID,
    NOTED_DRAGON_BONES_ID,
    GREEN_DRAGONHIDE_ID,
    NOTED_GREEN_DRAGONHIDE_ID,
];
/// Lost City Of Zanaris (`quest_names_enum` 34, journal row `[zanaris]`):
/// `levelrequire_zanaris_quest_attack(60, last_slot)` gates
/// `opheld2 dragon_dagger` (`content/scripts/levelrequire/scripts/tier60.rs2`,
/// `content/scripts/levelrequire/scripts/levelrequire.rs2`).
pub(crate) const LOST_CITY_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Lost City",
    journal: "Lost City",
};
const GREEN_DRAGON_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_BANK_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "foodReserve",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_FOOD as f64),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Dragon bones", "Dragonhide"]),
    },
];
/// Same prepared inventory-pressure loadout as `green_dragon_bank_prepared`,
/// but the `loot` setting is omitted so the generated Green default
/// (`DROP_DB["Green dragon"]` minus `Bass`) is the catalog that must earn
/// both guaranteed drops. Existing accepted inject stays untouched.
const GREEN_DRAGON_BANK_DEFAULT_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "foodReserve",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_FOOD as f64),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
pub(crate) const GREEN_DRAGON_BANK_DEPOSIT: [i32; 2] = [DRAGON_BONES_ID, GREEN_DRAGONHIDE_ID];

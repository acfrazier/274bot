use super::*;
pub(crate) const RED_SPIDERS_EGGS_ID: i32 = 223;
const NOTED_RED_SPIDERS_EGGS_ID: i32 = 224;
const NOTED_EYE_OF_NEWT_ID: i32 = 222;
pub(crate) const HERBLORE_EGG_FOOD_SEED: i32 = 50;
/// Canonical `FOOD_DEFAULT_COUNT` / `foodWithdraw` default. Carried at Start
/// so loot `needsRestock` is false (takeFood && foodCount < 1).
pub(crate) const HERBLORE_EGG_FOOD_CARRY: i32 = 10;
pub(crate) const HERBLORE_NEWT_COIN_SEED: i32 = 5000;
const EGG_FIELD: WorldTile = WorldTile {
    x: 3120,
    z: 9952,
    level: 0,
};
/// Safe native approach stand for the selected Edgeville booth. Headed shots
/// for both selected revision packs show this south-adjacent tile as
/// walkable, immediately next to booth 2213, without changing bank APIs.
pub(crate) const EDGEVILLE_BANK_APPROACH: WorldTile = WorldTile {
    x: 3096,
    z: 3494,
    level: 0,
};
/// Selected 274/289 packs both contain this Edgeville booth as id 2213 with
/// native booth op2 (`Use-quickly`). Keep the walk stand separate from it.
pub(crate) const EDGEVILLE_BANK_BOOTH: WorldTile = WorldTile {
    x: 3096,
    z: 3493,
    level: 0,
};
pub(crate) const EDGEVILLE_BANK_BOOTH_ID: i32 = 2213;
pub(super) const BETTY_SHOP: WorldTile = WorldTile {
    x: 3012,
    z: 3259,
    level: 0,
};
const HERBLORE_EGGS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Red spiders' eggs"),
    },
    // scriptFood reads the selected loadout, not a food setting. Blank
    // loadout uses the operator's first saved carry (headed LIVE: Swordfish).
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Herblore food"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(HERBLORE_EGG_FOOD_CARRY as f64),
    },
];
const HERBLORE_EGGS_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Herblore food",
    carry: &[("Lobster", HERBLORE_EGG_FOOD_CARRY as u32)],
}];
const HERBLORE_NEWT_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "secondary",
    value: ScriptInjectValue::Str("Eye of newt"),
}];
fn herblore_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    herblore_open_seed_bank_at(name, arm, EDGEVILLE_BANK_BOOTH, EDGEVILLE_BANK_BOOTH_ID)
}

fn herblore_seed_bank_readiness() -> Step {
    bank_fletcher_watch(
        "acknowledge exact Edgeville booth identity and Use-quickly action before bank send",
        Proof::LocActionNear {
            id: EDGEVILLE_BANK_BOOTH_ID,
            x: EDGEVILLE_BANK_BOOTH.x,
            z: EDGEVILLE_BANK_BOOTH.z,
            level: EDGEVILLE_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        },
    )
}

fn herblore_newt_seed_bank_readiness() -> Step {
    bank_fletcher_watch(
        "acknowledge exact Draynor booth identity and Use-quickly action before bank send",
        Proof::LocActionNear {
            id: DRAYNOR_BANK_BOOTH_ID,
            x: DRAYNOR_BANK_BOOTH.x,
            z: DRAYNOR_BANK_BOOTH.z,
            level: DRAYNOR_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        },
    )
}
pub(crate) fn herblore_secondaries_scenario() -> Scenario {
    let field = EGG_FIELD;
    let bank_approach = EDGEVILLE_BANK_APPROACH;
    let eggs = Proof::ItemId {
        id: RED_SPIDERS_EGGS_ID,
        count: 1,
    };
    // stock289: 379=lobster (nonstackable), 380=cert_lobster (stackable note).
    let lobster = NativeSeed {
        unnoted_id: LOBSTER_ID,
        debug_alias: "lobster",
        note_alias: Some("cert_lobster"),
        quantity: HERBLORE_EGG_FOOD_SEED,
        note_id: Some(NOTED_LOBSTER_ID),
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed banked lobster and tele to the safe Edgeville booth approach before Start",
        bank_approach,
        vec![lobster],
        "hitpoints",
        10,
    ));
    for (step_name, arm) in [
        (
            "confirm the exact noted lobster seed in pack before deposit",
            Proof::ItemId {
                id: NOTED_LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "bound the noted lobster seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "confirm no seeded eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(herblore_seed_bank_readiness());
    steps.push(herblore_open_seed_bank(
        "open and acknowledge the lobster seed bank",
        Proof::BankItemIdAtMost {
            id: LOBSTER_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the noted lobster seed through the bank window",
        vec![lobster],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact lobster food seed bank",
            Proof::BankItemId {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "bound the lobster food seed bank count",
            Proof::BankItemIdAtMost {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "confirm the noted lobster seed was removed from pack",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm no noted lobster remains in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded red spiders' eggs in bank",
            Proof::BankItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded noted eggs in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded eye of newt in bank",
            Proof::BankItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    // Pack is empty of seed after deposit; clearinv only after bank accepts seed.
    // Carry canonical food so takeFood loot does not BankTrip before first Take.
    steps.push(Step {
        name: "tele to the Edgeville dungeon egg field with lobster food before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give lobster {HERBLORE_EGG_FOOD_CARRY}"));
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before Start",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted lobster in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm lobster food carry in pack before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_CARRY,
            },
        ),
        (
            "bound the lobster food carry in pack before Start",
            Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_CARRY,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch exact red spiders' eggs 223 from the ground after Start",
            eggs,
        ),
        (
            "watch script-taken eggs enter a fresh Edgeville bank",
            Proof::BankItemId {
                id: RED_SPIDERS_EGGS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of eggs after deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        ("watch the script close its egg bank", Proof::BankClosed),
        (
            "watch return to the egg field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 14,
            },
        ),
        ("watch further exact eggs after return", eggs),
    ] {
        // Deposit arms on first ground egg while food/respawn still run.
        // Return is the reverse dungeon hop. See HERBLORE_EGG_* constant
        // docs (dirty increments, not 150×600ms). Other arms keep 150.
        let budget_ticks = if matches!(arm, Proof::BankItemId { .. }) {
            HERBLORE_EGG_DEPOSIT_WATCH_TICKS
        } else if matches!(arm, Proof::ArrivedNear { .. }) {
            HERBLORE_EGG_RETURN_WATCH_TICKS
        } else {
            SCRIPT_GOLD_WATCH_TICKS
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    Scenario {
        name: "herblore_secondaries",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: eggs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: HERBLORE_EGG_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_EGGS_INJECT),
            fixture_loadouts: Some(HERBLORE_EGGS_FIXTURE_LOADOUTS),
            terminal_shot: Some("herblore_secondaries"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// HerbloreSecondaries Eye of newt shop branch. Banked coins via native
/// stackable seed + deposit (stock289 has no `givebank`), Betty stand.
/// Distinct from ground eggs. LIVE still waits on Shop.buy publication.
pub(crate) fn herblore_secondaries_newt_scenario() -> Scenario {
    let shop = BETTY_SHOP;
    let newt = Proof::ItemId {
        id: EYE_OF_NEWT_ID,
        count: 1,
    };
    // stock289: 995=coins (stackable base; no certificate).
    let coins = NativeSeed {
        unnoted_id: COINS_ID,
        debug_alias: "coins",
        note_alias: None,
        quantity: HERBLORE_NEWT_COIN_SEED,
        note_id: None,
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed banked coins at the Draynor booth approach before Start",
        DRAYNOR_BANK_APPROACH,
        vec![coins],
        "hitpoints",
        10,
    ));
    for (step_name, arm) in [
        (
            "confirm the exact coin seed in pack before deposit",
            Proof::ItemId {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "bound the coin seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(herblore_newt_seed_bank_readiness());
    steps.push(herblore_open_seed_bank_at(
        "open and acknowledge the coin seed bank",
        Proof::BankItemIdAtMost {
            id: COINS_ID,
            count: 0,
        },
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
    ));
    steps.extend(native_bank_deposit(
        "deposit the coin seed through the bank window",
        vec![coins],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact coin seed bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "bound the coin seed bank count",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "confirm the coin seed was removed from pack",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded eye of newt in bank",
            Proof::BankItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded noted newt in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded red spiders' eggs in bank",
            Proof::BankItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    // Pack is empty of seed after deposit; clearinv only after bank accepts seed.
    steps.push(Step {
        name: "tele to Betty's shop stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(shop.level, shop.x, shop.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: shop.x,
                z: shop.z,
                level: shop.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded eye of newt in pack before Start",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted newt in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch exact eye of newt 221 after Start", newt),
        (
            "watch script-bought newt enter a fresh Draynor bank",
            Proof::BankItemId {
                id: EYE_OF_NEWT_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of newt after deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        ("watch the script close its newt bank", Proof::BankClosed),
        (
            "watch return to Betty after banking",
            Proof::ArrivedNear {
                x: shop.x,
                z: shop.z,
                level: shop.level,
                radius: 6,
            },
        ),
        ("watch further exact newt after return", newt),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "herblore_secondaries_newt",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: newt,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_NEWT_INJECT),
            terminal_shot: Some("herblore_secondaries_newt"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

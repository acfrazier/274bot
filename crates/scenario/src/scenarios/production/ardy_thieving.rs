use super::*;
pub(crate) const THIEVING_STAT: i32 = 17;
const ARDY_CAKES_BALLAST_KNIVES: i32 = 22;
pub(crate) const CAKE_ID: i32 = 1891;
pub(crate) const BREAD_ID: i32 = 2309;
pub(crate) const CHOCOLATE_SLICE_ID: i32 = 1901;
pub(crate) const CHOCOLATE_CAKE_ID: i32 = 1897;
const ARDY_CAKES_STAND: WorldTile = WorldTile {
    x: 2668,
    z: 3312,
    level: 0,
};
pub(crate) const ARDY_BANK: WorldTile = WorldTile {
    x: 2655,
    z: 3286,
    level: 0,
};
const ARDY_CAKES_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
];

const ARDY_CAKES_FIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Fight"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
];

const ARDY_THIEVER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const ARDY_THIEVER_FIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Fight"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    // A full opening restock: one frozen stealCakes session (<= 90 s) at the
    // stall, where the Guard catches. See ardy_thiever_fight_scenario.
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(27.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const ARDY_THIEVER_KNIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Knight of Ardougne"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];
pub(crate) fn ardy_cakes_scenario() -> Scenario {
    let stand = ARDY_CAKES_STAND;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, hitpoints, 22-Knife ballast and Baker's stall stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 5");
                cheat(c, "setstat hitpoints 40");
                cheat(c, &format!("give knife {ARDY_CAKES_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Thieving 5 before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 5,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 40,
            },
        ),
        (
            "confirm 22 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 22 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bread in pack before Start",
            Proof::ItemIdAtMost {
                id: BREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate slice in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_SLICE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP from Baker's stall after Start", first_xp),
        ("watch exact Cake 1891 stolen after Start", cake),
        (
            "watch arrival at the Ardougne bank after the stall fill",
            Proof::ArrivedNear {
                x: ARDY_BANK.x,
                z: ARDY_BANK.z,
                level: ARDY_BANK.level,
                radius: 6,
            },
        ),
        (
            "watch script-stolen cake enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: CAKE_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of cake after deposit",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "watch return to the Baker's stall stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        ("watch the cake bank close after deposit", Proof::BankClosed),
        ("watch another exact Cake 1891 after return", cake),
    ] {
        let bank_arrival = matches!(
            arm,
            Proof::ArrivedNear { x, z, .. } if x == ARDY_BANK.x && z == ARDY_BANK.z
        );
        let mut step = bank_fletcher_watch(step_name, arm);
        // The fill plus the bank walk: see ARDY_CAKES_BANK_WATCH_TICKS.
        if bank_arrival {
            step.wait.budget_ticks = ARDY_CAKES_BANK_WATCH_TICKS;
        }
        steps.push(step);
    }
    Scenario {
        name: "ardy_cakes",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: cake,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: ARDY_CAKES_DEADLINE,
            start_script: Some("ArdyCakes"),
            script_settings_inject: Some(ARDY_CAKES_INJECT),
            terminal_shot: Some("ardy_cakes"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// `guardResponse=Fight`: stall steal then FightBack kill of the catching Guard.
/// Combat stats and a scimitar are prepared so the FightBack branch can land;
/// the Flee kite is not this cell. Catalog owns the Guard defeat witness.
pub(crate) fn ardy_cakes_fight_scenario() -> Scenario {
    let stand = ARDY_CAKES_STAND;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let style_xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, combat stats, scimitar, 22-Knife ballast and Baker's stall stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 5");
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "give adamant_scimitar 1");
                cheat(c, &format!("give knife {ARDY_CAKES_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Thieving 5 before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 5,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Strength 40 before Start",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm 22 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 22 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm prepared scimitar before wielding",
            Proof::ItemId {
                id: COMBAT_SCIMITAR_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bread in pack before Start",
            Proof::ItemIdAtMost {
                id: BREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate slice in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_SLICE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Adamant scimitar before Start",
        COMBAT_SCIMITAR_ID,
    ));
    steps.push(select_strength_combat_style_step());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP from Baker's stall after Start", first_xp),
        ("watch exact Cake 1891 stolen after Start", cake),
        (
            "watch Strength XP from FightBack on the catching Guard after Start",
            style_xp,
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_cakes_fight",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: style_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyCakes"),
            script_settings_inject: Some(ARDY_CAKES_FIGHT_INJECT),
            terminal_shot: Some("ardy_cakes_fight"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

struct ArdyThieverSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    thieving: i32,
}

pub(crate) fn ardy_thiever_scenario() -> Scenario {
    ardy_thiever_variant(ArdyThieverSpec {
        name: "ardy_thiever",
        inject: ARDY_THIEVER_INJECT,
        thieving: 40,
    })
}

pub(crate) fn ardy_thiever_knight_scenario() -> Scenario {
    ardy_thiever_variant(ArdyThieverSpec {
        name: "ardy_thiever_knight",
        inject: ARDY_THIEVER_KNIGHT_INJECT,
        thieving: 55,
    })
}

/// `guardResponse=Fight` on ArdyThiever: FightBack kills the Guard that catches
/// a stall steal, then pickpocket coins and the loot-count bank/return/further
/// cycle of the Flee cell. Combat kit is prepared before Start; catalog owns
/// the Guard defeat + no-Flee gate.
///
/// **Why the kill comes first (289 content).** A failed pickpocket never starts
/// combat: `fail_pick_pocket` (thieving.rs2) only stuns and deals 2 damage.
/// The only Guard aggression is `stealing_check_for_guard` on a stall steal
/// (stealing.rs2). ArdyThiever steals from the stall only while its food is at
/// `restockAtFood`, and after the first coins it gets there only by eating,
/// which needs HP lost to failed pickpockets (Guard success at Thieving 40 is
/// 126/256). Each success also costs a ~60-tick bank trip at loot-count 1.
/// Modelled, a catch after the coins within the 150-dirty watch is 1-9% with
/// the old inject and about 42% with the best food seed (at a 0.2 catch
/// chance per steal). So this cell watches
/// Strength XP first, and injects `foodTarget` 27 with `restockAtFood` 0: the
/// opening RestockCakes call is one full frozen stealCakes session at the
/// stall. A Monte Carlo of that session (8-tick respawn after a success, ~5
/// ticks per silent owner refusal with a swap after three) puts a Guard catch
/// within 150 ticks of Start at 0.97 on the pooled 289 rates (11 success /
/// 5 refused / 3 caught of 19 steals), 0.84 on the ArdyCakes run alone, and
/// 1.0 at the passing `ardy_cakes_fight` rates. The watch is 150 runner
/// dirties, about 143 engine ticks at the measured 1.05 dirties per tick,
/// which gives 0.967 / 0.837 / 1.0. The coins, deposit, return and
/// further-coins gates follow unchanged; the whole-run cap is
/// `ARDY_THIEVER_FIGHT_DEADLINE`.
pub(crate) fn ardy_thiever_fight_scenario() -> Scenario {
    let stand = ARDOUGNE_GUARD;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let style_xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, combat stats, scimitar, empty pack and market stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 40");
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "give adamant_scimitar 1");
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 40,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Strength 40 before Start",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm prepared scimitar before wielding",
            Proof::ItemId {
                id: COMBAT_SCIMITAR_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Adamant scimitar before Start",
        COMBAT_SCIMITAR_ID,
    ));
    steps.push(select_strength_combat_style_step());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Strength XP from FightBack on the catching Guard after Start",
            style_xp,
        ),
        ("watch Thieving XP after Start", first_xp),
        ("watch exact Coins 995 pickpocketed after Start", coins),
        (
            "watch script-pickpocketed coins enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of coins after deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch return to the market stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        (
            "watch the pickpocket bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Coins 995 after return", coins),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_thiever_fight",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coins,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: ARDY_THIEVER_FIGHT_DEADLINE,
            start_script: Some("ArdyThiever"),
            script_settings_inject: Some(ARDY_THIEVER_FIGHT_INJECT),
            terminal_shot: Some("ardy_thiever_fight"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack at the Guard/Knight stand. Flee, clues off, loot-count bank
/// at 1 slot. Script restocks one stall food, pickpockets coins with
/// Thieving XP, deposits those coins, returns to the stand and pickpockets
/// again. PeriodicBank Off is not the bank proof. Fight stays pending.
///
/// **Windows under the Baker owner model** (see `ARDY_CAKES_BANK_WATCH_TICKS`:
/// 0.63 refusals per attempt, guard catch 0.12). Only one stall success is
/// needed (`foodTarget` 1), so the ordinary 150-dirty (~143 engine ticks)
/// watches hold. A 20k-run simulation gives P(first Thieving XP ≤143) = 0.962
/// for the Guard and 0.964 for the Knight (p95 132 / 128). It gives P(coins
/// ≤143 after that XP) = 0.986 / 0.989 (p95 68 / 60). That counts
/// pickpockets at 126/256 and 155/256, stun damage 2 / 3, and a stall
/// revisit when a one-bite food is eaten.
fn ardy_thiever_variant(spec: ArdyThieverSpec) -> Scenario {
    let ArdyThieverSpec {
        name,
        inject,
        thieving,
    } = spec;
    let stand = ARDOUGNE_GUARD;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, hitpoints, empty pack and market stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat thieving {thieving}"));
                cheat(c, "setstat hitpoints 40");
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 40,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP after Start", first_xp),
        ("watch exact Coins 995 pickpocketed after Start", coins),
        (
            "watch script-pickpocketed coins enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of coins after deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch return to the market stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        (
            "watch the pickpocket bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Coins 995 after return", coins),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coins,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyThiever"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

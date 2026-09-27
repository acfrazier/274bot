use super::*;
const THIEVER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::Str(""),
    },
    // Catalog food name comes from the selected loadout. The matching
    // fixture loadout is posted only at harness catalog Start.
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Memory food"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(22.0),
    },
    ScriptSettingInject {
        id: "bankAtFood",
        value: ScriptInjectValue::Num(3.0),
    },
];

/// Posted at panel harness catalog Start; not a Play/operator name reservation.
const THIEVER_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Memory food",
    carry: &[("Lobster", 1)],
}];
/// The `thiever` scenario: live Thiever gold — Guard pickpocket at the
/// Ardougne tile, food via `give`, loot off. Proof is thieving XP delta.
pub(crate) fn thiever_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 17, min: 1 };
    let tele = ARDOUGNE_GUARD;
    Scenario {
        name: "thiever",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed stats, food, and tele to the guard stand",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "advancestat thieving 50");
                        cheat(c, "advancestat hitpoints 50");
                        cheat(c, "give lobster 10");
                        cheat(c, &tele_args(tele.level, tele.x, tele.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: tele.x,
                        z: tele.z,
                        level: tele.level,
                        radius: 10,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(drain_advancestat());
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script pickpocket guards",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Thiever"),
            script_settings_inject: Some(THIEVER_INJECT),
            fixture_prereqs: Some(THIEVER_FIXTURE_PREREQS),
            fixture_loadouts: Some(THIEVER_FIXTURE_LOADOUTS),
            nav: gold_script_nav(),
            terminal_shot: Some("thiever paint"),
            ..Default::default()
        },
    }
}

/// Diagnostic prerequisite workload: bank stock is seeded once, then the
/// unchanged catalog script must eat, restock, and return using host APIs.
pub fn thiever_sustained_scenario() -> Scenario {
    let mut scenario = thiever_scenario();
    let step = scenario
        .steps
        .iter_mut()
        .find(|s| s.name == "seed stats, food, and tele to the guard stand")
        .expect("Thiever seed");
    step.kind = StepKind::Perform {
        send: Box::new(|c, _| {
            cheat(c, "advancestat thieving 50");
            cheat(c, "advancestat hitpoints 50");
            cheat(c, "give lobster 4");
            cheat(c, "givebank lobster 2000");
            cheat(
                c,
                &tele_args(ARDOUGNE_GUARD.level, ARDOUGNE_GUARD.x, ARDOUGNE_GUARD.z),
            );
            true
        }),
    };
    scenario
}

const TRAP_HOUSE: WorldTile = WorldTile {
    x: 2671,
    z: 3316,
    level: 0,
};
const SWORDFISH_ID: i32 = 373;
const KNIGHT_THIEVING: i32 = 55;

const THIEVER_TRAP_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Knight of Ardougne"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::Str(""),
    },
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Swordfish food"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(22.0),
    },
    ScriptSettingInject {
        id: "bankAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const THIEVER_TRAP_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Swordfish food",
    carry: &[("Swordfish", 1)],
}];

const THIEVER_TRAP_PREREQS: &[Proof] = &[
    Proof::ArrivedNear {
        x: 2671,
        z: 3316,
        level: 0,
        radius: 2,
    },
    Proof::Stat {
        id: 17,
        min: KNIGHT_THIEVING,
    },
    Proof::Stat { id: 3, min: 50 },
    Proof::ItemIdAtMost {
        id: SWORDFISH_ID,
        count: 0,
    },
];

/// Sealed behind the shape-9 door at (2669,3316). Empty pack so FoodBank
/// walks to the Ardougne booth; unstick must open that door or the baked
/// walk logs `unreachable … best 30` and the script stops.
pub(crate) fn thiever_diagonal_door_trap_scenario() -> Scenario {
    let restocked = Proof::ItemId {
        id: SWORDFISH_ID,
        count: 22,
    };
    Scenario {
        name: "thiever_diagonal_door_trap",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed Knight stats, empty pack, banked swordfish, sealed house",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        cheat(c, "setstat thieving 55");
                        cheat(c, "setstat hitpoints 50");
                        cheat(c, "givebank swordfish 28");
                        cheat(c, &tele_args(TRAP_HOUSE.level, TRAP_HOUSE.x, TRAP_HOUSE.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: TRAP_HOUSE.x,
                        z: TRAP_HOUSE.z,
                        level: TRAP_HOUSE.level,
                        radius: 2,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch food restock complete: carrying 22 swordfish",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: restocked,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: restocked,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Thiever"),
            script_settings_inject: Some(THIEVER_TRAP_INJECT),
            fixture_prereqs: Some(THIEVER_TRAP_PREREQS),
            fixture_loadouts: Some(THIEVER_TRAP_LOADOUTS),
            nav: gold_script_nav(),
            terminal_shot: Some("thiever_diagonal_door_trap"),
            ..Default::default()
        },
    }
}

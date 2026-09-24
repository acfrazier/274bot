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

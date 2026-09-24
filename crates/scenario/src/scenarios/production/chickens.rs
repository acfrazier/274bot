use super::*;
/// Lumbridge chicken pen (east of the castle). ChickenKiller anchors at
/// Start — host tele lands here after mainland/tutskip/relog, then Start.
const LUMBRIDGE_CHICKENS: WorldTile = WorldTile {
    x: 3235,
    z: 3295,
    level: 0,
};

/// The `chicken_killer` scenario: live ChickenKiller gold — script anchors
/// at Start (no camp nav). Seed = mainland + tutskip + relog + tele to
/// the pen `(3235,3295)`, then Start. Melee defaults, DeathRecovery idle.
/// Proof is strength XP ≥ 1. Remaining miss is combat, not walk.
pub(crate) fn chicken_killer_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 2, min: 1 };
    let tele = LUMBRIDGE_CHICKENS;
    Scenario {
        name: "chicken_killer",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "tele to Lumbridge chickens",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(tele.level, tele.x, tele.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: tele.x,
                        z: tele.z,
                        level: tele.level,
                        radius: 8,
                    },
                    budget_ticks: 120,
                },
            });
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script fight chickens",
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
            start_script: Some("ChickenKiller"),
            terminal_shot: Some("chicken_killer terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
/// Falador south chicken pen, immediately south of the host cow-field pin
/// `(3029,3305,0)`. Default Lumbridge pen `(3235,3295,0)` cannot reach a
/// same-plane Use-quickly booth inside PeriodicBank's 60s walk: castle
/// booths are upstairs, and Draynor/Al Kharid/Varrock West are ≥128
/// Chebyshev. This interior is Chebyshev 61 from Falador East vs 64 from
/// Draynor, so nearest packed booth stays Falador East. ChickenKiller
/// `destination()` is null; return radius is the reviewed service 6.
const FALADOR_CHICKENS: WorldTile = WorldTile {
    x: 3029,
    z: 3294,
    level: 0,
};
const CHICKEN_KILLER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "lootMatch",
        value: ScriptInjectValue::Str("feather"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
];
/// ChickenKiller loot-count trip: melee, `bankEveryItems=1`, Feather 314.
/// Bones stay the bury keep-list default; melee `afterDeposit` is a no-op.
/// Prepare only before Start. Proof is combat, exact loot, fresh deposit,
/// return to the original anchor, then new exact feathers after the pack
/// was emptied. Same-id `StatXpGain { min: 1 }` cannot witness further
/// work: the runner keeps the first XP baseline per skill id.
pub(crate) fn chicken_killer_bank_scenario() -> Scenario {
    let tele = FALADOR_CHICKENS;
    let first_strength = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let further_feathers = Proof::ItemId {
        id: FEATHER_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare melee stats, empty pack and Falador chicken anchor before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "setstat attack 30");
                cheat(c, "setstat strength 30");
                cheat(c, "~clearinv");
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (name, id) in [
        ("acknowledge prepared Attack 30", 0),
        ("acknowledge prepared Strength 30", STRENGTH_STAT),
    ] {
        steps.push(bank_fletcher_watch(name, Proof::Stat { id, min: 30 }));
    }
    steps.push(bank_fletcher_watch(
        "confirm no seeded feathers in pack before Start",
        Proof::ItemIdAtMost {
            id: FEATHER_ID,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Strength XP from melee chicken combat",
            first_strength,
        ),
        (
            "watch exact Feather 314 looted after Start",
            Proof::ItemId {
                id: FEATHER_ID,
                count: 1,
            },
        ),
        (
            "watch script-looted feathers enter a fresh bank",
            Proof::BankItemId {
                id: FEATHER_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of feathers after deposit",
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: 0,
            },
        ),
        (
            "watch return to the Falador chicken anchor within radius 6",
            Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius: 6,
            },
        ),
        (
            "watch the periodic bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch new exact Feather 314 after return", further_feathers),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "chicken_killer_bank",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_feathers,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ChickenKiller"),
            script_settings_inject: Some(CHICKEN_KILLER_BANK_INJECT),
            terminal_shot: Some("chicken_killer_bank"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

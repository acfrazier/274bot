//! Quester qualification fixtures. Cheats are allowed here only.
use super::script_basics::script_live_seed_steps;
use crate::*;

const COOK_DEADLINE: Duration = Duration::from_secs(900);
const COOK_WATCH: u32 = 3600;
const COOK_CARD: &str = "Quester";
const COOK_KITCHEN: WorldTile = WorldTile {
    x: 3209,
    z: 3215,
    level: 0,
};

const SHEEP_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quest",
    value: ScriptInjectValue::Str("sheep"),
}];
const RUNE_MYSTERIES_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quest",
    value: ScriptInjectValue::Str("runemysteries"),
}];
const ROMEO_AND_JULIET_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quest",
    value: ScriptInjectValue::Str("romeojuliet"),
}];
/// Generic builder: jump a quest to a stage key with optional items, then
/// Start Quester. `varp`/`value` seed via `setvar` (fixture only).
pub fn quester_stage(
    name: &'static str,
    quest_display: &'static str,
    varp: &'static str,
    value: i32,
    items: &'static [(&'static str, i32)],
    stand: WorldTile,
) -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "reset quest stage",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setvar {varp} {value}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 40,
        },
    });
    if !items.is_empty() {
        steps.push(Step {
            name: "seed items",
            kind: StepKind::Perform {
                send: Box::new(move |c, _| {
                    cheat(c, "~clearinv");
                    for (alias, qty) in items {
                        cheat(c, &format!("give {alias} {qty}"));
                    }
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 0 },
                budget_ticks: 80,
            },
        });
    }
    steps.push(Step {
        name: "relog so the quest tab colour matches the seeded varp",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 600,
        },
    });
    steps.push(Step {
        name: "stand at the quest start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: stand.x,
                z: stand.z,
                level: stand.level,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "watch the quest tab turn complete",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::QuestDone {
                name: quest_display,
            },
            budget_ticks: COOK_WATCH,
        },
    });
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::QuestDone {
            name: quest_display,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: COOK_DEADLINE,
            start_script: Some(COOK_CARD),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Fresh account Cook's Assistant, colour-only, no journal.
pub(crate) fn quester_cook_scenario() -> Scenario {
    quester_stage(
        "quester_cook",
        "Cook's Assistant",
        "cookquest",
        0,
        &[],
        COOK_KITCHEN,
    )
}

pub(crate) fn quester_sheep_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_sheep",
        "Sheep Shearer",
        "sheep",
        0,
        &[],
        WorldTile {
            x: 3189,
            z: 3273,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(SHEEP_SETTINGS);
    scenario
}

pub(crate) fn quester_rune_mysteries_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_rune_mysteries",
        "Rune Mysteries Quest",
        "runemysteries",
        0,
        &[],
        WorldTile {
            x: 3208,
            z: 3222,
            level: 1,
        },
    );
    scenario.settings.script_settings_inject = Some(RUNE_MYSTERIES_SETTINGS);
    scenario
}

pub(crate) fn quester_romeo_and_juliet_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_romeo_and_juliet",
        "Romeo & Juliet",
        "rjquest",
        0,
        &[],
        WorldTile {
            x: 3211,
            z: 3425,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(ROMEO_AND_JULIET_SETTINGS);
    scenario
}

/// Resume Cook from in-progress with the three products already held.
pub(crate) fn quester_cook_resume_scenario() -> Scenario {
    quester_stage(
        "quester_cook_resume",
        "Cook's Assistant",
        "cookquest",
        1,
        &[("egg", 1), ("bucket_milk", 1), ("pot_flour", 1)],
        COOK_KITCHEN,
    )
}

/// Start during relog, before the new session has posted its quest-tab colours.
pub(crate) fn quester_cook_login_scenario() -> Scenario {
    let mut scenario = quester_cook_resume_scenario();
    scenario.name = "quester_cook_login";
    scenario.settings.terminal_shot = Some("quester_cook_login");
    let relog = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::Relog))
        .unwrap();
    scenario.steps[relog].wait.arm = Proof::LoggedOut;
    scenario.steps.remove(relog + 1);
    scenario
}

/// Stop after an observed egg, then restart from live colour with that item held.
pub(crate) fn quester_cook_restart_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_cook_restart",
        "Cook's Assistant",
        "cookquest",
        1,
        &[],
        COOK_KITCHEN,
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    scenario.steps.splice(
        start + 1..start + 1,
        [
            Step {
                name: "watch Quester acquire an egg before Stop",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::Item {
                        name: "Egg",
                        count: 1,
                    },
                    budget_ticks: COOK_WATCH,
                },
            },
            Step {
                name: "stop Quester mid-quest",
                kind: StepKind::StopScript,
                wait: Wait {
                    arm: Proof::ScriptIdle,
                    budget_ticks: 10,
                },
            },
            start_compiled_step(),
        ],
    );
    scenario
}

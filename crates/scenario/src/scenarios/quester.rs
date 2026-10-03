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

const COOK_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quests",
    value: ScriptInjectValue::StrList(&["cook"]),
}];
const SHEEP_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quests",
    value: ScriptInjectValue::StrList(&["sheep"]),
}];
const RUNE_MYSTERIES_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quests",
    value: ScriptInjectValue::StrList(&["runemysteries"]),
}];
const ROMEO_AND_JULIET_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quests",
    value: ScriptInjectValue::StrList(&["romeojuliet"]),
}];
const QUEUE_QUESTS: &[&str] = &["cook", "sheep", "romeojuliet", "imp"];
const QUEUE_SETTINGS: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "quests",
        value: ScriptInjectValue::StrList(QUEUE_QUESTS),
    },
    ScriptSettingInject {
        id: "order_override",
        value: ScriptInjectValue::StrList(QUEUE_QUESTS),
    },
    ScriptSettingInject {
        id: "skip",
        value: ScriptInjectValue::StrList(&[]),
    },
];
const QUEUE_DEADLINE: Duration = Duration::from_secs(7200);
const QUEUE_WATCH_TICKS: u32 = 24_000;

fn queue_start_stage_step(name: &'static str, varp: &'static str, witness: &'static str) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setvar {varp} 0"));
                cheat(c, &format!("getvar {varp}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat { needle: witness },
            budget_ticks: 200,
        },
    }
}
// Q2 route threshold: the Sheep and Rune legs cross bearded dark wizards
// (vislevel 20, huntmode `ranged`) and young dark wizards (vislevel 7,
// huntmode `ranged`). Engine `Npc.huntPlayers` skips a player whose combat
// exceeds twice the hunter's vislevel (`Npc.ts`, `check_nottoostrong =
// outside_wilderness` in `all.hunt [ranged]`), so the bearded wizard stays
// passive only at combat 41+, the young at 15+, and Aubury's Mugger
// (vislevel 6, `cowardly`) at 13+. Sheep and Rune therefore stage the
// tested combat-41 minimum from the minme fresh base (all stats 1,
// Hitpoints 10 — `ClientCheatHandler.ts` `minme`) and hold Start until the
// posted stats prove it. "Minimum" is the combat threshold, not
// individually minimal skills: Attack/Strength/Defence 40 follows the
// existing combat-prep precedent (`combat.rs` `COMBAT_ATTACK_LEVEL`), and
// Hitpoints 20 keeps the total at exactly 41 while adding a survivability
// margin. This covers only the passive-threshold route hazard: Draynor
// jail guards (vislevel 26, huntmode `aggressive_melee`,
// `check_nottoostrong = off`) hunt at any combat level and remain a route
// hazard until nav zones land. Romeo & Juliet stays at the fresh base —
// no hunter reaches its stands — and is the unchanged baseline.
const S2_ATTACK_STAT_ID: i32 = 0;
const S2_DEFENCE_STAT_ID: i32 = 1;
const S2_STRENGTH_STAT_ID: i32 = 2;
const S2_HITPOINTS_STAT_ID: i32 = 3;
/// Staged Attack/Strength/Defence: the existing combat-prep precedent
/// (`combat.rs` `COMBAT_ATTACK_LEVEL`). No maxme: nothing above 40.
const S2_SAFE_COMBAT_SKILL: i32 = 40;
/// Staged Hitpoints. Engine `Player.getCombatLevel` posts exactly combat
/// 41 for the tuple: base 0.25*(40+20) = 15, melee 0.325*(40+40) = 26,
/// floor(41) = 41 — past twice the bearded wizard's 20. Dropping any one
/// of the four staged stats by a level posts 40, still hunted.
const S2_SAFE_HITPOINTS: i32 = 20;
/// Fresh-base reset every S2 quest fixture emits before any seed.
fn s2_reset_step() -> Step {
    Step {
        name: "reset fixture stats to the fresh base",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "minme");
                true
            }),
        },
        wait: Wait {
            // Posted Defence back at base 1 before any seed lands. Fails
            // closed while a reused account still carries a staged profile.
            arm: Proof::StatAtMost {
                id: S2_DEFENCE_STAT_ID,
                max: 1,
            },
            budget_ticks: 80,
        },
    }
}
/// Shared combat-41 profile for the Sheep and Rune fixtures: one raise per
/// tuple stat plus an observed guard per stat, so Start waits until the
/// posted levels prove exactly combat 41 (see `S2_SAFE_HITPOINTS`). Observe
/// before teleport/relog/Start; relog clears the stat-level-up UI. The
/// guards say nothing about the jail guards, which hunt at any combat.
fn s2_combat_profile_steps() -> Vec<Step> {
    const PROFILE: [(i32, &str, i32); 4] = [
        (S2_ATTACK_STAT_ID, "attack", S2_SAFE_COMBAT_SKILL),
        (S2_DEFENCE_STAT_ID, "defence", S2_SAFE_COMBAT_SKILL),
        (S2_STRENGTH_STAT_ID, "strength", S2_SAFE_COMBAT_SKILL),
        (S2_HITPOINTS_STAT_ID, "hitpoints", S2_SAFE_HITPOINTS),
    ];
    PROFILE
        .iter()
        .map(|(id, skill, level)| {
            let id = *id;
            let skill = *skill;
            let level = *level;
            Step {
                name: "stage combat-41 profile and observe it before Start",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &format!("setstat {skill} {level}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat { id, min: level },
                    budget_ticks: 80,
                },
            }
        })
        .collect()
}
/// Splice S2 safe-stat staging into a quest fixture ahead of its `setvar`
/// stage seed. `profile` holds the quest's raises (empty for Romeo &
/// Juliet, which stays at the fresh base). Cook fixtures are out of scope
/// and keep the shared `quester_stage` path untouched.
fn stage_s2_safe_stats(scenario: &mut Scenario, profile: Vec<Step>) {
    let seed = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .expect("quester fixture has a quest-stage seed step");
    let mut prep = vec![s2_reset_step()];
    prep.extend(profile);
    scenario.steps.splice(seed..seed, prep);
}
/// Generic builder: jump a quest to a stage key with optional items, then
/// Start Quester. `varp`/`value` seed via `setvar` (fixture only). The pack
/// is always cleared first, even when `items` is empty, so a reused fixture
/// account cannot carry quest items into the run.
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
    steps.push(Step {
        name: "clear and seed inventory",
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
    steps.push(Step {
        name: "stand at the quest start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z)).is_sent()
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
    steps.push(Step {
        name: "relog so the quest tab colour matches the seeded varp",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 600,
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
    let mut scenario = quester_stage(
        "quester_cook",
        "Cook's Assistant",
        "cookquest",
        0,
        &[],
        COOK_KITCHEN,
    );
    scenario.settings.script_settings_inject = Some(COOK_SETTINGS);
    scenario
}

pub(crate) fn quester_sheep_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_sheep",
        "Sheep Shearer",
        "sheep",
        0,
        &[("coins", 100)],
        WorldTile {
            x: 3197,
            z: 3266,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(SHEEP_SETTINGS);
    // Bearded-dark-wizard leg: reset, then the combat-41 profile whose
    // observed guards hold Start until the posted stats prove combat 41.
    // No wool or weapon seed: the Path shears, spins and hands in from
    // zero, buying shears with the seeded coins when none are banked.
    stage_s2_safe_stats(&mut scenario, s2_combat_profile_steps());
    scenario
}

/// Fresh Duke Horacio start: the fixture stands on the castle's upper floor
/// by the Duke and plays the whole quest; nothing is seeded.
pub(crate) fn quester_rune_mysteries_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_rune_mysteries",
        "Rune Mysteries Quest",
        "runemysteries",
        0,
        &[],
        WorldTile {
            x: 3212,
            z: 3220,
            level: 1,
        },
    );
    scenario.settings.script_settings_inject = Some(RUNE_MYSTERIES_SETTINGS);
    // Same bearded-dark-wizard leg as Sheep: reset, then the combat-41
    // profile whose observed guards hold Start until the posted stats prove
    // combat 41. The stand is the Path's own `start-duke` anchor
    // (paths/289/runemysteries.json, upstairs level 1).
    stage_s2_safe_stats(&mut scenario, s2_combat_profile_steps());
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
    // Unchanged baseline: no hunter reaches its stands, so the fresh base
    // (Tier A, passive 3) is the safe profile — reset, no raise.
    stage_s2_safe_stats(&mut scenario, Vec::new());
    scenario
}

/// Fresh account through Cook, Sheep Shearer, Romeo & Juliet, then Imp Catcher.
/// Only stats, coins, and a melee weapon are staged; every quest varp starts at 0.
pub(crate) fn quester_queue_scenario() -> Scenario {
    const RUNE_SCIMITAR_ID: i32 = 1333;
    let mut steps = script_live_seed_steps();
    steps.push(s2_reset_step());
    steps.extend(s2_combat_profile_steps());
    steps.push(Step {
        name: "stage Imp Catcher hitpoints for the queued melee leg",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat hitpoints 40");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: S2_HITPOINTS_STAT_ID,
                min: 40,
            },
            budget_ticks: 80,
        },
    });
    steps.extend([
        queue_start_stage_step("reset Cook stage to 0", "cookquest", "get cookquest: 0"),
        queue_start_stage_step("reset Sheep stage to 0", "sheep", "get sheep: 0"),
        queue_start_stage_step(
            "reset Romeo and Juliet stage to 0",
            "rjquest",
            "get rjquest: 0",
        ),
        queue_start_stage_step("reset Imp Catcher stage to 0", "imp", "get imp: 0"),
    ]);
    steps.push(Step {
        name: "seed only coins and the Imp melee weapon",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give coins 100");
                cheat(c, "give rune_scimitar 1");
                cheat(c, "givebank coins 500");
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: RUNE_SCIMITAR_ID,
                count: 1,
            },
            budget_ticks: 80,
        },
    });
    steps.push(Step {
        name: "stand at Cook's Assistant start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(COOK_KITCHEN.level, COOK_KITCHEN.x, COOK_KITCHEN.z),
                )
                .is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: COOK_KITCHEN.x,
                z: COOK_KITCHEN.z,
                level: COOK_KITCHEN.level,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "relog so all four quest colours reflect their zero stages",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 600,
        },
    });
    steps.push(super::combat::wear_combat_item_step(
        "wield Rune scimitar before the queued Imp fight",
        RUNE_SCIMITAR_ID,
    ));
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "watch the queued Imp Catcher completion",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::QuestDone {
                name: "Imp Catcher",
            },
            budget_ticks: QUEUE_WATCH_TICKS,
        },
    });
    Scenario {
        name: "quester_queue",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::QuestDone {
            name: "Imp Catcher",
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: QUEUE_DEADLINE,
            start_script: Some(COOK_CARD),
            script_settings_inject: Some(QUEUE_SETTINGS),
            terminal_shot: Some("quester_queue"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Resume Cook from in-progress with the three products already held.
pub(crate) fn quester_cook_resume_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_cook_resume",
        "Cook's Assistant",
        "cookquest",
        1,
        &[("egg", 1), ("bucket_milk", 1), ("pot_flour", 1)],
        COOK_KITCHEN,
    );
    scenario.settings.script_settings_inject = Some(COOK_SETTINGS);
    scenario
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
    scenario.settings.script_settings_inject = Some(COOK_SETTINGS);
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

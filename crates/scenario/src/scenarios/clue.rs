use super::production::bank_fletcher_watch;
use super::script_basics::script_live_seed_steps;
use crate::*;

/// Talk / search / unguarded dig / coord walks are a few tiles from the
/// seed tele; 10 minutes covers login + dialog + one step.
const SHERLOCK_DEADLINE: Duration = Duration::from_secs(600);
const SHERLOCK_WATCH_TICKS: u32 = 600;
const SHERLOCK_CARD: &str = "Sherlock";

const TALK_CLUE_ID: i32 = 2681;
const TALK_CLUE_ALIAS: &str = "trail_clue_easy_simple005";
const TALK_STAND: WorldTile = WorldTile {
    x: 3207,
    z: 3233,
    level: 0,
};

const SEARCH_CLUE_ID: i32 = 2679;
const SEARCH_CLUE_ALIAS: &str = "trail_clue_easy_simple003";
const SEARCH_STAND: WorldTile = WorldTile {
    x: 3248,
    z: 3246,
    level: 0,
};

const DIG_CLUE_ID: i32 = 2827;
const DIG_CLUE_ALIAS: &str = "trail_clue_medium_map001";
const DIG_STAND: WorldTile = WorldTile {
    x: 3093,
    z: 3227,
    level: 0,
};

const COORD_CLUE_ID: i32 = 2823;
const COORD_CLUE_ALIAS: &str = "trail_clue_medium_sextant012";
const COORD_STAND: WorldTile = WorldTile {
    x: 3217,
    z: 3177,
    level: 0,
};

const SPADE_ALIAS: &str = "spade";
const SEXTANT_ALIAS: &str = "trail_sextant";
const WATCH_ALIAS: &str = "trail_watch";
const CHART_ALIAS: &str = "trail_chart";

fn watch_replaced(seeded: i32) -> Step {
    Step {
        name: "watch Sherlock replace the seeded clue",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::ClueReplaced { seeded },
            budget_ticks: SHERLOCK_WATCH_TICKS,
        },
    }
}

fn seed_pack(
    name: &'static str,
    clue_alias: &'static str,
    clue_id: i32,
    tools: &'static [(&'static str, i32)],
    stand: WorldTile,
) -> Vec<Step> {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give {clue_alias} 1"));
                for (alias, qty) in tools {
                    cheat(c, &format!("give {alias} {qty}"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: clue_id,
                count: 1,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "teleport next to the first clue step",
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
    steps
}

fn sherlock_scenario(
    name: &'static str,
    seed_name: &'static str,
    clue_alias: &'static str,
    clue_id: i32,
    tools: &'static [(&'static str, i32)],
    stand: WorldTile,
) -> Scenario {
    let mut steps = seed_pack(seed_name, clue_alias, clue_id, tools, stand);
    steps.push(start_compiled_step());
    steps.push(watch_replaced(clue_id));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ClueReplaced { seeded: clue_id },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SHERLOCK_DEADLINE,
            start_script: Some(SHERLOCK_CARD),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Easy talk: Hans in the Lumbridge courtyard. Unique spawn, no challenge.
pub(crate) fn sherlock_talk_scenario() -> Scenario {
    sherlock_scenario(
        "sherlock_talk",
        "seed the Sherlock talk pack",
        TALK_CLUE_ALIAS,
        TALK_CLUE_ID,
        &[],
        TALK_STAND,
    )
}

/// Easy search: boxes in the goblin house east of Lumbridge courtyard.
pub(crate) fn sherlock_search_scenario() -> Scenario {
    sherlock_scenario(
        "sherlock_search",
        "seed the Sherlock search pack",
        SEARCH_CLUE_ALIAS,
        SEARCH_CLUE_ID,
        &[],
        SEARCH_STAND,
    )
}

/// Medium unguarded map dig just west of Lumbridge / Draynor, spade only.
pub(crate) fn sherlock_dig_scenario() -> Scenario {
    sherlock_scenario(
        "sherlock_dig",
        "seed the Sherlock dig pack",
        DIG_CLUE_ALIAS,
        DIG_CLUE_ID,
        &[(SPADE_ALIAS, 1)],
        DIG_STAND,
    )
}

/// Medium unguarded sextant coordinate in Lumbridge swamp. Trio + spade seeded
/// so the machine skips acquire and Digs.
pub(crate) fn sherlock_coord_scenario() -> Scenario {
    sherlock_scenario(
        "sherlock_coord",
        "seed the Sherlock coord pack",
        COORD_CLUE_ALIAS,
        COORD_CLUE_ID,
        &[
            (SPADE_ALIAS, 1),
            (SEXTANT_ALIAS, 1),
            (WATCH_ALIAS, 1),
            (CHART_ALIAS, 1),
        ],
        COORD_STAND,
    )
}

const GUARDIAN_CLUE_ID: i32 = 2735;
const GUARDIAN_CLUE_ALIAS: &str = "trail_clue_hard_sextant007";
const GUARDIAN_STAND: WorldTile = WorldTile {
    x: 2946,
    z: 3819,
    level: 0,
};
const LOBSTER_ID: i32 = 379;
const RUNE_SCIMITAR_ID: i32 = 1333;
const PRAYER_POT_ID: i32 = 2434;
const SPADE_ID: i32 = 952;
const SEXTANT_ID: i32 = 2574;
const WATCH_ID: i32 = 2575;
const CHART_ID: i32 = 2576;
const GUARDIAN_DEADLINE: Duration = Duration::from_secs(900);
const GUARDIAN_WATCH_TICKS: u32 = 900;

/// Hard Zamorak guardian: Combat eats, prays Protect from Magic, and kills.
/// Operator amendment (design-combat.md): NPCs without Attack are never engaged.
pub(crate) fn sherlock_guardian_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "set melee stats to 40",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat attack 40");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 0, min: 40 },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge Attack 40",
        Proof::Stat { id: 0, min: 40 },
    ));
    steps.push(Step {
        name: "set strength 40",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat strength 40");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 2, min: 40 },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge Strength 40",
        Proof::Stat { id: 2, min: 40 },
    ));
    steps.push(Step {
        name: "set defence 40",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat defence 40");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 1, min: 40 },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge Defence 40",
        Proof::Stat { id: 1, min: 40 },
    ));
    steps.push(Step {
        name: "set hitpoints 40",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat hitpoints 40");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 3, min: 40 },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge Hitpoints 40",
        Proof::Stat { id: 3, min: 40 },
    ));
    steps.push(Step {
        name: "set prayer 43",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat prayer 43");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 5, min: 43 },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge Prayer 43",
        Proof::Stat { id: 5, min: 43 },
    ));
    steps.push(Step {
        name: "clear pack and give guardian kit",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give {GUARDIAN_CLUE_ALIAS} 1"));
                cheat(c, "give spade 1");
                cheat(c, "give trail_sextant 1");
                cheat(c, "give trail_watch 1");
                cheat(c, "give trail_chart 1");
                cheat(c, "give rune_scimitar 1");
                cheat(c, "give lobster 8");
                cheat(c, "give 4doseprayerrestore 1");
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: GUARDIAN_CLUE_ID,
                count: 1,
            },
            budget_ticks: 200,
        },
    });
    for (name, id, count) in [
        ("spade", SPADE_ID, 1),
        ("sextant", SEXTANT_ID, 1),
        ("watch", WATCH_ID, 1),
        ("chart", CHART_ID, 1),
        ("rune scimitar", RUNE_SCIMITAR_ID, 1),
        ("lobsters", LOBSTER_ID, 8),
        ("prayer potion", PRAYER_POT_ID, 1),
    ] {
        steps.push(bank_fletcher_watch(name, Proof::ItemId { id, count }));
    }
    steps.push(Step {
        name: "teleport to the Zamorak clue tile",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(
                    c,
                    &tele_args(GUARDIAN_STAND.level, GUARDIAN_STAND.x, GUARDIAN_STAND.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: GUARDIAN_STAND.x,
                z: GUARDIAN_STAND.z,
                level: GUARDIAN_STAND.level,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "watch protect acknowledgement, bounded in-flight hit, prayer off, and casket",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::ClueProtectWindow {
                seeded: GUARDIAN_CLUE_ID,
            },
            budget_ticks: GUARDIAN_WATCH_TICKS,
        },
    });
    Scenario {
        name: "sherlock_guardian",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ClueProtectWindow {
            seeded: GUARDIAN_CLUE_ID,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: GUARDIAN_DEADLINE,
            start_script: Some(SHERLOCK_CARD),
            terminal_shot: Some("sherlock_guardian"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

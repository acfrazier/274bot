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
    steps.push(start_catalog_step());
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

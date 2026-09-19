use super::production::*;
use crate::*;

/// Run the real catalog BoneBurier through inventory depletion, off-scene
/// banking, withdrawal and another burial. All stock is prepared before Start;
/// the harness only observes after that boundary.
pub(crate) fn bone_burier_scenario() -> Scenario {
    let watch = |name, arm| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            // The off-scene trip may legitimately outlast the ordinary
            // 150-tick action observation. Observe the script's existing
            // 120-second travel timeout instead of ending the proof first.
            budget_ticks: if matches!(arm, Proof::BankItem { .. }) {
                240
            } else {
                SCRIPT_GOLD_WATCH_TICKS
            },
            arm,
        },
    };
    Scenario {
        name: "bone_burier",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "prepare five carried bones and twenty-eight banked bones",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give bones 5");
                        cheat(c, "givebank bones 28");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get tutorial: 1000",
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "relog so the inv tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            start_catalog_step(),
            watch(
                "watch the first five burials",
                Proof::StatXpGain { id: 5, min: 22 },
            ),
            watch(
                "watch the carried bones run out",
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch(
                "watch the script reach and open its stocked bank",
                Proof::BankItem {
                    name: "Bones",
                    count: 28,
                },
            ),
            watch(
                "watch the script withdraw a full pack",
                Proof::Item {
                    name: "Bones",
                    count: 28,
                },
            ),
            watch(
                "watch the bank stock decrease",
                Proof::BankItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch("watch the script close its bank", Proof::BankClosed),
            // XP baselines are retained for the scenario: six normal bones
            // yield 27 integer XP. Five seed bones alone can yield only 22.
            watch(
                "watch a burial from the withdrawn pack",
                Proof::StatXpGain { id: 5, min: 27 },
            ),
        ],
        proof: Proof::StatXpGain { id: 5, min: 27 },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BoneBurier"),
            terminal_shot: Some("bone_burier terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const GENIE_LAMP_ID: i32 = 2528;
const LAMP_STRENGTH_STAT: i32 = 2;
pub(crate) const PRAYER_STAT: i32 = 5;

/// Real selected-289 lamp redemption while the existing BoneBurier catalog
/// script remains running. The fixture injects only the authentic lamp item;
/// the shared guardian owns Rub, skill selection, Confirm and dialogue drain.
pub(crate) fn lamp_redemption_scenario() -> Scenario {
    let watch = |name, arm| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    };
    let fresh_prayer = Proof::FreshStatXpGain {
        id: PRAYER_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare twenty-seven ordinary bones before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| cheat(c, "give bones 27")),
        },
        wait: Wait {
            arm: Proof::Item {
                name: "Bones",
                count: 27,
            },
            budget_ticks: 60,
        },
    });
    steps.push(start_catalog_step());
    steps.push(watch(
        "watch BoneBurier do real work before the lamp",
        fresh_prayer,
    ));
    steps.push(watch(
        "confirm no pre-injection continuation can seed the lamp episode",
        Proof::NoActiveContinue,
    ));
    steps.push(Step {
        name: "inject one authentic selected-289 genie lamp after Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| cheat(c, "give macro_genilamp 1")),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: GENIE_LAMP_ID,
                count: 1,
            },
            budget_ticks: 60,
        },
    });
    steps.push(Step {
        name: "observe native lamp reward dialogue drain and hold release",
        kind: StepKind::ObserveLampRedemption {
            lamp_id: GENIE_LAMP_ID,
            reward_stat: LAMP_STRENGTH_STAT,
        },
        wait: Wait {
            arm: Proof::NoActiveContinue,
            budget_ticks: 60,
        },
    });
    steps.push(watch(
        "watch fresh BoneBurier Prayer XP after lamp release",
        fresh_prayer,
    ));

    Scenario {
        name: "lamp_redemption",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: fresh_prayer,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BoneBurier"),
            terminal_shot: Some("lamp_redemption"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Verified packed Varrock East bank tile used by the v2 writer preset and login gate.
pub(crate) const BONE_BURIER_V2_BANK: WorldTile = WorldTile {
    x: 3253,
    z: 3421,
    level: 0,
};

/// Isolate self-stop reason from `crates/script/examples/bone_burier_v2.ts`.
const BONE_BURIER_V2_STOP_REASON: &str = "confirmed loaded current-generation bank exhaustion";

pub(crate) const BONE_BURIER_V2_PREREQS: &[Proof] = &[
    Proof::Item {
        name: "Bones",
        count: 5,
    },
    Proof::SideTabAvailable { index: 3 },
    Proof::ArrivedNear {
        x: BONE_BURIER_V2_BANK.x,
        z: BONE_BURIER_V2_BANK.z,
        level: BONE_BURIER_V2_BANK.level,
        radius: 8,
    },
];

/// Bury 5 + restock 28 + bury 28 + reopen bank + clean self-stop.
pub(crate) const BONE_BURIER_V2_DEADLINE: Duration = Duration::from_secs(360);
const BONE_BURIER_V2_WATCH_TICKS: u32 = 240;

/// Headed File-card BoneBurier v2: one exact in-tree example per scenario id.
/// v1 [`bone_burier_scenario`] stays the catalog compatibility witness.
pub(crate) fn bone_burier_v2_scenario(name: &'static str, file_name: &'static str) -> Scenario {
    let watch = |step_name, arm| Step {
        name: step_name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: BONE_BURIER_V2_WATCH_TICKS,
            arm,
        },
    };
    let later_bank = Proof::BankItemAtMost {
        name: "Bones",
        count: 0,
    };
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "prepare five carried bones and twenty-eight banked bones",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give bones 5");
                        cheat(c, "givebank bones 28");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get tutorial: 1000",
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "tele to the verified Varrock East bank tile",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(
                            c,
                            &tele_args(
                                BONE_BURIER_V2_BANK.level,
                                BONE_BURIER_V2_BANK.x,
                                BONE_BURIER_V2_BANK.z,
                            ),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: BONE_BURIER_V2_BANK.x,
                        z: BONE_BURIER_V2_BANK.z,
                        level: BONE_BURIER_V2_BANK.level,
                        radius: 8,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "relog so the inv tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            start_catalog_step(),
            watch(
                "watch the first five burials",
                Proof::StatXpGain { id: 5, min: 22 },
            ),
            watch(
                "watch the carried bones run out",
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch(
                "watch the script reach and open its stocked bank",
                Proof::BankItem {
                    name: "Bones",
                    count: 28,
                },
            ),
            watch(
                "watch the script withdraw a full pack",
                Proof::Item {
                    name: "Bones",
                    count: 28,
                },
            ),
            watch(
                "watch the bank stock decrease",
                Proof::BankItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch("watch the script close its bank", Proof::BankClosed),
            watch(
                "watch the replenished pack yield further Prayer XP",
                Proof::FreshStatXpGain { id: 5, min: 1 },
            ),
            watch(
                "watch the replenished pack run out",
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch(
                "watch a later loaded current bank still at zero",
                later_bank,
            ),
        ],
        proof: later_bank,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: BONE_BURIER_V2_DEADLINE,
            start_script: None,
            start_file: Some(file_name),
            wait_script_stop: Some(BONE_BURIER_V2_STOP_REASON),
            terminal_shot: Some(name),
            fixture_prereqs: Some(BONE_BURIER_V2_PREREQS),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Shared live-script seed: stick `tutorial=1000`, relog so side tab 3 binds.
pub(crate) fn script_live_seed_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, "setvar tutorial 1000");
                    cheat(c, "getvar tutorial");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Chat {
                    needle: "get tutorial: 1000",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "relog so the inv tab binds",
            kind: StepKind::Relog,
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
    ]
}

/// One real server-owned Strange Plant lifecycle. The upstream debugproc
/// spawns `macro_triffidseed` for the logged-in player; the shared Rust
/// guardian must probe the otherwise featureless adjacent seed, authenticate
/// the canonical growing response, retry without click spam, then pick the
/// ripe fruit. A send or growing message alone is not terminal evidence.
pub(crate) fn strange_plant_owned_scenario() -> Scenario {
    let fruit = Proof::Item {
        name: "Strange fruit",
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    // Use the existing inert TradeBot file fixture so run-prepared can retain
    // the real post-Start macro injection while stripping every setup cheat.
    // An empty partner makes the fixture passive; the Rust guardian owns the
    // random lifecycle under test.
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "spawn the upstream owned Strange Plant",
        kind: StepKind::Perform {
            send: Box::new(|c, _| cheat(c, "~macro_event 5")),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "The fruit isn't ready to be picked yet",
            },
            budget_ticks: 40,
        },
    });
    steps.push(Step {
        name: "watch the authenticated plant ripen and yield its fruit",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: fruit,
            budget_ticks: 180,
        },
    });
    Scenario {
        name: "strange_plant_owned",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: fruit,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(150),
            start_script: Some("TradeBot"),
            terminal_shot: Some("strange_plant_owned"),
            fixture_prereqs: Some(THIEVER_FIXTURE_PREREQS),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const MAZE_RETURN_ANCHOR: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};
pub(crate) const MAZE_SPAWNS: &[WorldTile] = &[
    WorldTile {
        x: 2891,
        z: 4597,
        level: 0,
    },
    WorldTile {
        x: 2933,
        z: 4597,
        level: 0,
    },
    WorldTile {
        x: 2933,
        z: 4555,
        level: 0,
    },
    WorldTile {
        x: 2891,
        z: 4555,
        level: 0,
    },
];
pub(crate) const MAZE_SHRINE: WorldTile = WorldTile {
    x: 2911,
    z: 4575,
    level: 0,
};

/// One server-owned Maze lifecycle. The single selected-289 macro trigger
/// spawns the Mysterious Old Man; the shared guardian must engage him, enter
/// whichever canonical spawn the server chooses, solve doors, Touch the
/// shrine, and release its hold after the server returns and rewards us.
pub(crate) fn maze_owned_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "spawn one authentic Maze event",
        kind: StepKind::Perform {
            send: Box::new(|c, _| cheat(c, "~macro_event 8")),
        },
        wait: Wait {
            arm: Proof::NpcNameNear {
                name: "Mysterious old man",
                x: MAZE_RETURN_ANCHOR.x,
                z: MAZE_RETURN_ANCHOR.z,
                level: MAZE_RETURN_ANCHOR.level,
                radius: 24,
            },
            budget_ticks: 80,
        },
    });
    steps.push(Step {
        name: "observe held Maze entry, door progress, shrine return, reward, and release",
        kind: StepKind::ObserveMazeCompletion {
            spawns: MAZE_SPAWNS,
            shrine: MAZE_SHRINE,
            shrine_radius: 4,
            min_progress: 8,
            entry_shot: "maze_owned entered",
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 900,
        },
    });

    Scenario {
        name: "maze_owned",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::IngameScene2,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(300),
            start_script: Some("TradeBot"),
            terminal_shot: Some("maze_owned final"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

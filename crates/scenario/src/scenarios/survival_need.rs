use super::script_basics::script_live_seed_steps;
use crate::*;

const YEW: WorldTile = WorldTile {
    x: 2345,
    z: 4551,
    level: 0,
};
const BRONZE_AXE_ID: i32 = 1351;
// Engine L debugproc on the account's own state: `queue(poison_player, 0,
// severity)` (`_test/scripts/cheats/cheat_other.rs2:1-3`), so the engine's own
// poison script (`skill_combat/scripts/poison.rs2:6-18`) applies the poison.
const POISON_DEBUG_COMMAND: &str = "~poison 27";
const YEW_LOC_ID: i32 = 1309;
const WOODCUTTING_STAT_ID: i32 = 8;
const HITPOINTS_STAT_ID: i32 = 3;
const SURVIVAL_GATHERER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "skill",
        value: ScriptInjectValue::Str("Woodcutting"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Start"),
    },
    ScriptSettingInject {
        id: "woodcuttingResources",
        value: ScriptInjectValue::StrList(&["yew"]),
    },
    ScriptSettingInject {
        id: "radius",
        value: ScriptInjectValue::Num(32.0),
    },
    ScriptSettingInject {
        id: "disposition",
        value: ScriptInjectValue::Str("Power"),
    },
];

/// Start a real Yew Gatherer, poison it at the Yew with the engine's poison
/// debugproc without a held cure or food, and retain the live status row
/// through the first fire.
pub(crate) fn survival_need_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    // `setstat` and `give` mutate the player in ClientCheatHandler.ts:340-354
    // and :472-485; two observed ticks allow dispatch and the next PLAYER_INFO.
    // Use the real bronze axe, which has no Attack requirement.
    for (name, command, arm) in [
        (
            "seed Woodcutting 99 before Start",
            "setstat woodcutting 99",
            Proof::Stat {
                id: WOODCUTTING_STAT_ID,
                min: 99,
            },
        ),
        (
            "seed Hitpoints 30 before Start",
            "setstat hitpoints 30",
            Proof::Stat {
                id: HITPOINTS_STAT_ID,
                min: 30,
            },
        ),
        (
            "give the account-owned bronze axe before Start",
            "give bronze_axe 1",
            Proof::ItemId {
                id: BRONZE_AXE_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(Step {
            name,
            kind: StepKind::Perform {
                send: Box::new(move |client, _| cheat(client, command).is_sent()),
            },
            wait: Wait {
                arm,
                budget_ticks: 2,
            },
        });
    }
    steps.push(Step {
        name: "equip the account-owned bronze axe before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, snapshot| {
                matches!(
                    Interactions::new(snapshot, client).wear(BRONZE_AXE_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id: BRONZE_AXE_ID },
            budget_ticks: 40,
        },
    });
    steps.push(Step {
        name: "stand at the live Yew work site before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, &tele_args(YEW.level, YEW.x, YEW.z)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: YEW.x,
                z: YEW.z,
                level: YEW.level,
            },
            budget_ticks: 10,
        },
    });
    steps.push(Step {
        name: "confirm the real Yew object before Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::LocIdNear {
                id: YEW_LOC_ID,
                x: YEW.x,
                z: YEW.z,
                level: YEW.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "observe active Yew work before the real poison source",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::FreshStatXpGain {
                id: WOODCUTTING_STAT_ID,
                min: 1,
            },
            // A Yew attempt takes three ticks and the depleted tree respawns
            // in 200 (woodcut.rs2:59-60; trees.dbrow:130-143): 360 covers one
            // respawn plus 53 attempts.
            budget_ticks: 360,
        },
    });
    steps.push(Step {
        name: "seed account-owned poison at the Yew work site",
        kind: StepKind::Perform {
            send: Box::new(|client, _| cheat(client, POISON_DEBUG_COMMAND).is_sent()),
        },
        wait: Wait {
            arm: Proof::FreshChat {
                needle: "You have been poisoned!",
            },
            // The debugproc queues poison with delay 0, and poison.rs2:12-14
            // emits the chat line on the first eligible queue pass.
            budget_ticks: 2,
        },
    });
    steps.push(Step {
        name: "observe the first real poison fire while the need row remains",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::StatAtMost {
                id: HITPOINTS_STAT_ID,
                max: 24,
            },
            // Severity 27 deals (27 + 4) / 5 = 6 HP, and poison.rs2:72-75
            // schedules the first fire after exactly 30 game ticks.
            budget_ticks: 30,
        },
    });

    Scenario {
        name: "survival_need",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::StatAtMost {
            id: HITPOINTS_STAT_ID,
            max: 24,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(300),
            start_script: Some("Gatherer"),
            terminal_shot: Some("survival need after real poison fire"),
            script_settings_inject: Some(SURVIVAL_GATHERER_INJECT),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

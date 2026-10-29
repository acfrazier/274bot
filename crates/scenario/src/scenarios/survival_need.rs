use super::script_basics::script_live_seed_steps;
use crate::*;

const YEW: WorldTile = WorldTile {
    x: 2345,
    z: 4551,
    level: 0,
};
const CHEST_REGION: WorldTile = WorldTile {
    x: 40 * 64 + 32,
    z: 148 * 64 + 32,
    level: 0,
};
const CHEST_LOC_ID: i32 = 377;
const SINISTER_KEY_ID: i32 = 993;
const RUNE_AXE_ID: i32 = 1359;
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

/// Start a real Yew Gatherer, poison it from the Yanille twisted chest without
/// a held cure or food, and retain the live status row through the first fire.
pub(crate) fn survival_need_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Woodcutting 99, Hitpoints 30, rune axe and sinister key",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, "setstat woodcutting 99").is_sent()
                    && cheat(client, "setstat hitpoints 30").is_sent()
                    && cheat(client, "give rune_axe 1").is_sent()
                    && cheat(client, "give sinister_key 1").is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: WOODCUTTING_STAT_ID,
                min: 99,
            },
            budget_ticks: 40,
        },
    });
    steps.push(Step {
        name: "confirm the account-owned axe and key before Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: SINISTER_KEY_ID,
                count: 1,
            },
            budget_ticks: 40,
        },
    });
    steps.push(Step {
        name: "equip the account-owned rune axe before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, snapshot| {
                matches!(
                    Interactions::new(snapshot, client).wear(RUNE_AXE_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id: RUNE_AXE_ID },
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
    let chest_teleport = tele_args(CHEST_REGION.level, CHEST_REGION.x, CHEST_REGION.z);
    steps.push(Step {
        name: "teleport to the Yanille dungeon region",
        kind: StepKind::Perform {
            send: Box::new(move |client, _| cheat(client, &chest_teleport).is_sent()),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: CHEST_REGION.x,
                z: CHEST_REGION.z,
                level: CHEST_REGION.level,
            },
            budget_ticks: 10,
        },
    });
    steps.push(Step {
        name: "observe the real Yanille twisted chest",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::LocIdNear {
                id: CHEST_LOC_ID,
                x: CHEST_REGION.x,
                z: CHEST_REGION.z,
                level: CHEST_REGION.level,
                radius: 32,
            },
            budget_ticks: 10,
        },
    });
    steps.push(Step {
        name: "use the account-owned sinister key on the real chest",
        kind: StepKind::Perform {
            send: Box::new(|client, snapshot| {
                let Some(loc) = snapshot.locs().iter().find(|loc| loc.id == CHEST_LOC_ID) else {
                    return false;
                };
                let Some(key) = snapshot
                    .inventory()
                    .iter()
                    .find(|item| item.def.id == SINISTER_KEY_ID && item.count > 0)
                else {
                    return false;
                };
                matches!(
                    Interactions::new(snapshot, client).use_item_on(key, OpTarget::Loc(loc)),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::FreshChat {
                needle: "You have been poisoned!",
            },
            // agility_dungeon.rs2:101-112 queues poison after p_delay(3);
            // poison.rs2:12-18 emits the fresh chat line.
            budget_ticks: 10,
        },
    });
    let yew_teleport = tele_args(YEW.level, YEW.x, YEW.z);
    steps.push(Step {
        name: "return to the Yew work site after the poison event",
        kind: StepKind::Perform {
            send: Box::new(move |client, _| cheat(client, &yew_teleport).is_sent()),
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

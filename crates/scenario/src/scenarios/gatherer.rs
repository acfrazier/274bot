use super::script_basics::script_live_seed_steps;
use crate::*;

const GATHERER_START: WorldTile = WorldTile {
    x: 3217,
    z: 3231,
    level: 0,
};
const BRONZE_AXE_ID: i32 = 1351;
const NORMAL_TREE_ID: i32 = 1276;
const WOODCUTTING_STAT_ID: i32 = 8;
const GATHERER_START_INJECT: &[ScriptSettingInject] = &[
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
        value: ScriptInjectValue::StrList(&["normal"]),
    },
    ScriptSettingInject {
        id: "radius",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "disposition",
        value: ScriptInjectValue::Str("Power"),
    },
];
const GATHERER_WATCH_TICKS: u32 = 240;

/// Starts the compiled Gatherer with a real axe at a live normal tree and
/// requires fresh Woodcutting XP after Start.
pub(crate) fn gatherer_scenario() -> Scenario {
    let fresh_xp = Proof::FreshStatXpGain {
        id: WOODCUTTING_STAT_ID,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Woodcutting 1 and a real bronze axe before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, "setstat woodcutting 1").is_sent()
                    && cheat(client, "give bronze_axe 1").is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: BRONZE_AXE_ID,
                count: 1,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "equip the observed bronze axe before Start",
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
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "stand at the live Draynor normal tree before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, &tele_args(0, GATHERER_START.x, GATHERER_START.z)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: GATHERER_START.x,
                z: GATHERER_START.z,
                level: GATHERER_START.level,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "confirm the selected-289 normal tree is live before Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::LocIdNear {
                id: NORMAL_TREE_ID,
                x: GATHERER_START.x,
                z: GATHERER_START.z,
                level: GATHERER_START.level,
                radius: 6,
            },
            budget_ticks: 120,
        },
    });
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "watch the compiled Gatherer produce fresh Woodcutting XP",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: fresh_xp,
            budget_ticks: GATHERER_WATCH_TICKS,
        },
    });

    Scenario {
        name: "gatherer",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::StatXpGain {
            id: WOODCUTTING_STAT_ID,
            min: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(240),
            start_script: Some("Gatherer"),
            script_settings_inject: Some(GATHERER_START_INJECT),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

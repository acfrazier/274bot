use super::*;
pub(crate) const GATE_CLOSED_ID: i32 = 1551;
pub(crate) const GATE_OPEN_ID: i32 = 1552;
/// Packed closed wooden door 1530 on selected 274/289 m50_50, adjacent to
/// DoorOpener's default Lumbridge stand.
const LUMBRIDGE_DOOR: WorldTile = WorldTile {
    x: 3208,
    z: 3211,
    level: 0,
};
const LUMBRIDGE_DOOR_STAND: WorldTile = WorldTile {
    x: 3208,
    z: 3212,
    level: 0,
};

/// Packed closed wooden gate 1551 on selected 274/289 m50_50.
const LUMBRIDGE_GATE: WorldTile = WorldTile {
    x: 3213,
    z: 3261,
    level: 0,
};
const LUMBRIDGE_GATE_STAND: WorldTile = WorldTile {
    x: 3213,
    z: 3260,
    level: 0,
};
const DOOR_OPENER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "stand",
    value: ScriptInjectValue::Str("3208,3212,0"),
}];

const DOOR_OPENER_GATE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "stand",
        value: ScriptInjectValue::Str("3213,3260,0"),
    },
    ScriptSettingInject {
        id: "obstacle",
        value: ScriptInjectValue::Str("gate"),
    },
];
pub(crate) fn door_opener_scenario() -> Scenario {
    door_opener_variant(
        "door_opener",
        DOOR_OPENER_INJECT,
        LUMBRIDGE_DOOR,
        LUMBRIDGE_DOOR_STAND,
        CLOSED_ID,
        OPEN_ID,
    )
}

pub(crate) fn door_opener_gate_scenario() -> Scenario {
    door_opener_variant(
        "door_opener_gate",
        DOOR_OPENER_GATE_INJECT,
        LUMBRIDGE_GATE,
        LUMBRIDGE_GATE_STAND,
        GATE_CLOSED_ID,
        GATE_OPEN_ID,
    )
}

/// Walk to an adjacent stand, Close any open leaf before Start, then require
/// the selected shut loc to become the open id through a same-session world
/// change. Queued Open or script counters are not this proof.
fn door_opener_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    packed: WorldTile,
    stand: WorldTile,
    closed_id: i32,
    open_id: i32,
) -> Scenario {
    let shut = Proof::LocActionNear {
        id: closed_id,
        x: packed.x,
        z: packed.z,
        level: packed.level,
        radius: 1,
        action: "Open",
        present: true,
    };
    let opened = Proof::LocIdNear {
        id: open_id,
        x: packed.x,
        z: packed.z,
        level: packed.level,
        radius: 3,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele adjacent to the selected shut loc",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "close the selected loc if it is still open",
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                if let Some(loc) = snapshot.locs().iter().find(|loc| {
                    loc.id == open_id
                        && loc.tile.level == packed.level
                        && (loc.tile.x - packed.x)
                            .abs()
                            .max((loc.tile.z - packed.z).abs())
                            <= 3
                }) {
                    op_loc(c, loc.tile.x, loc.tile.z, loc.id);
                }
                true
            }),
        },
        wait: Wait {
            arm: shut,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch the selected loc become open after Start",
        opened,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: opened,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("DoorOpener"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

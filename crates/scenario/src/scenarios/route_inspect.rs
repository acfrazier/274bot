use super::script_basics::script_live_seed_steps;
use crate::*;

/// Frozen `config.ts` @ rs2b0t `beecd912`.
pub(crate) const BRIMHAVEN_FIELD: WorldTile = WorldTile {
    x: 2698,
    z: 3206,
    level: 0,
};
pub(crate) const BRIMHAVEN_BANK: WorldTile = WorldTile {
    x: 2655,
    z: 3283,
    level: 0,
};
pub(crate) const BRIMHAVEN_PIER: WorldTile = WorldTile {
    x: 2683,
    z: 3272,
    level: 0,
};

/// Bank-cycle return can exceed ordinary script gold deadlines.
const BRIMHAVEN_INSPECT_V1_DEADLINE: Duration = Duration::from_secs(900);
const BRIMHAVEN_INSPECT_V1_FIELD_WATCH: u32 = 3600;

/// Matches `route_inspect_brimhaven_v2.ts` `STOP_OK`.
pub(crate) const ROUTE_INSPECT_BRIMHAVEN_V2_STOP: &str =
    "route inspect brimhaven qualification complete";

const ROUTE_INSPECT_V2_DEADLINE: Duration = Duration::from_secs(360);
const ROUTE_INSPECT_V2_WATCH: u32 = 240;

/// Headed catalog witness for frozen `BrimhavenMossGiants` `sailToField` /
/// `Navigator.findPath` after a legitimate bank return.
///
/// Harness proof is [`Proof::ArrivedNear`] on the field tile after the script's
/// own bank cycle. The acceptance line `planned route uses Captain Barnaby` is
/// emitted via `bot.log` (not game chat); root must capture script log or paint
/// for that predicate.
pub(crate) fn brimhaven_moss_inspect_v1_scenario() -> Scenario {
    let field = Proof::ArrivedNear {
        x: BRIMHAVEN_FIELD.x,
        z: BRIMHAVEN_FIELD.z,
        level: BRIMHAVEN_FIELD.level,
        radius: 14,
    };
    let watch = |name, arm, budget| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait { arm, budget_ticks: budget },
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare island bank-cycle trip stock before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar tutorial 1000");
                cheat(c, "getvar tutorial");
                cheat(c, "setstat agility 30");
                cheat(c, "setstat attack 40");
                cheat(c, "setstat strength 40");
                cheat(c, "setstat hitpoints 40");
                cheat(c, "setstat defence 40");
                cheat(c, "~clearinv");
                cheat(c, "give bronze_scimitar 1");
                cheat(c, "give bones 28");
                cheat(c, "givebank lobster 28");
                cheat(c, "givebank coins 500");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Item {
                name: "Bones",
                count: 28,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "tele to the Brimhaven moss field before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(
                        BRIMHAVEN_FIELD.level,
                        BRIMHAVEN_FIELD.x,
                        BRIMHAVEN_FIELD.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRIMHAVEN_FIELD.x,
                z: BRIMHAVEN_FIELD.z,
                level: BRIMHAVEN_FIELD.level,
                radius: 14,
            },
            budget_ticks: 120,
        },
    });
    steps.push(start_catalog_step());
    steps.push(watch(
        "watch return to the field after bank-cycle route inspect and sail",
        field,
        BRIMHAVEN_INSPECT_V1_FIELD_WATCH,
    ));

    Scenario {
        name: "brimhaven_moss_inspect_v1",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: field,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: BRIMHAVEN_INSPECT_V1_DEADLINE,
            start_script: Some("BrimhavenMossGiants"),
            terminal_shot: Some("brimhaven_moss_inspect_v1"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Headed File-card v2 witness on the same pier→field inspect geometry.
///
/// Inspect `ok` + Barnaby `locName` and snapshot `request_id:0` consumption
/// are validated inside the example (paint + self-stop). The scenario only
/// binds pre-Start placement and post-inspect ordinary walk arrival at the
/// bank stand (distinct from the seeded pier tile).
pub(crate) fn route_inspect_brimhaven_v2_scenario() -> Scenario {
    let bank_arrival = Proof::ArrivedNear {
        x: BRIMHAVEN_BANK.x,
        z: BRIMHAVEN_BANK.z,
        level: BRIMHAVEN_BANK.level,
        radius: 6,
    };
    let watch = |name, arm| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: ROUTE_INSPECT_V2_WATCH,
            arm,
        },
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele to Captain Barnaby pier before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(
                        BRIMHAVEN_PIER.level,
                        BRIMHAVEN_PIER.x,
                        BRIMHAVEN_PIER.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRIMHAVEN_PIER.x,
                z: BRIMHAVEN_PIER.z,
                level: BRIMHAVEN_PIER.level,
                radius: 4,
            },
            budget_ticks: 120,
        },
    });
    steps.push(start_catalog_step());
    steps.push(watch(
        "watch arrival at the Ardougne pier bank stand after inspect walk",
        bank_arrival,
    ));

    Scenario {
        name: "route_inspect_brimhaven_v2_ts",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: bank_arrival,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: ROUTE_INSPECT_V2_DEADLINE,
            start_script: None,
            start_file: Some("route_inspect_brimhaven_v2.ts"),
            wait_script_stop: Some(ROUTE_INSPECT_BRIMHAVEN_V2_STOP),
            terminal_shot: Some("route_inspect_brimhaven_v2_ts"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `boat_fare_v1.ts` `STOP_OK`.
pub(crate) const BOAT_FARE_V1_STOP: &str = "boat fare v1 reached Port Sarim";

/// Musa Point dock: the `karamjashipplank` landing (`static_routes.rs`
/// Port Sarim → Musa disembark), a standable Karamja tile.
pub(crate) const MUSA_DOCK: WorldTile = WorldTile {
    x: 2956,
    z: 3146,
    level: 0,
};
/// Port Sarim dock: the `karamjashipplank_off` landing the example walks to.
pub(crate) const PORT_SARIM_DOCK: WorldTile = WorldTile {
    x: 3029,
    z: 3217,
    level: 0,
};

/// The whole plantation job (walks, ten picks, ten packs, two talks) and
/// the crossing: well past the ordinary script watch.
const BOAT_FARE_V1_DEADLINE: Duration = Duration::from_secs(900);
const BOAT_FARE_V1_WATCH: u32 = 3600;

/// Headed/headless File v1 witness for frozen `Traversal.walkTo`'s Karamja
/// boat-fare recovery.
///
/// PRESTART: an emptied pack holding only 5 coins (short of the 30-coin
/// fare), the plantation job reset (`%hunt_store_employed`, `%crate_bananas`
/// and `%crate_rum` cleared; `luthas.rs2` offers employment whenever the
/// employed bit is clear), standing on the Musa Point dock. After Start the
/// only way to Port Sarim is to earn the fare from Luthas and pay the
/// customs officer; the example stops with its named reason once
/// `Traversal.walkTo` returns true on the Port Sarim dock.
pub(crate) fn boat_fare_v1_scenario() -> Scenario {
    let mainland = Proof::ArrivedNear {
        x: PORT_SARIM_DOCK.x,
        z: PORT_SARIM_DOCK.z,
        level: PORT_SARIM_DOCK.level,
        radius: 6,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "empty the pack and reset the banana plantation job before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setvar hunt_store_employed 0");
                cheat(c, "setvar crate_bananas 0");
                cheat(c, "setvar crate_rum 0");
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemAtMost {
                name: "Coins",
                count: 0,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "carry 5 coins, short of the 30-coin fare, before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "give coins 5");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Item {
                name: "Coins",
                count: 5,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "tele to the Musa Point dock before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, &tele_args(MUSA_DOCK.level, MUSA_DOCK.x, MUSA_DOCK.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: MUSA_DOCK.x,
                z: MUSA_DOCK.z,
                level: MUSA_DOCK.level,
                radius: 4,
            },
            budget_ticks: 120,
        },
    });
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the fare earned at the plantation and the boat to Port Sarim",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: mainland,
            budget_ticks: BOAT_FARE_V1_WATCH,
        },
    });
    Scenario {
        name: "boat_fare_v1_ts",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: mainland,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: BOAT_FARE_V1_DEADLINE,
            start_script: None,
            start_file: Some("boat_fare_v1.ts"),
            wait_script_stop: Some(BOAT_FARE_V1_STOP),
            terminal_shot: Some("boat_fare_v1_ts"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

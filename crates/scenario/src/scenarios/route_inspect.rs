use super::script_basics::script_live_seed_steps;
use crate::*;

/// Frozen field tile from `config.ts` @ rs2b0t `beecd912`. Not a v1 seed or
/// harness PASS predicate (field sit / fallback arrival is Core FAIL).
#[allow(dead_code)]
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
const BRIMHAVEN_INSPECT_V1_PIER_WATCH: u32 = 3600;

/// Matches `route_inspect_brimhaven_v2.ts` `STOP_OK`.
pub(crate) const ROUTE_INSPECT_BRIMHAVEN_V2_STOP: &str =
    "route inspect brimhaven qualification complete";

const ROUTE_INSPECT_V2_DEADLINE: Duration = Duration::from_secs(360);
const ROUTE_INSPECT_V2_WATCH: u32 = 240;

/// Headed catalog witness for frozen `BrimhavenMossGiants` Travel→Bank→
/// `sailToField` / `Navigator.findPath`.
///
/// Seed is the reference-aligned empty pack at the Ardougne SE bank, with
/// lobster+coins in the bank only. No afterStart seed or tele. Scenario
/// position proof is pier progress after Start — the bank seed cannot
/// satisfy it. Accepted Barnaby inspect is the catalog Core witness, not
/// `Proof::Chat` / invented game chat. Frozen fallback without accept must
/// FAIL that Core predicate. Fatality is a run failure; this card does not
/// claim DeathRecovery (matrix still records it BLOCKED).
pub(crate) fn brimhaven_moss_inspect_v1_scenario() -> Scenario {
    let pier = Proof::ArrivedNear {
        x: BRIMHAVEN_PIER.x,
        z: BRIMHAVEN_PIER.z,
        level: BRIMHAVEN_PIER.level,
        radius: 8,
    };
    let watch = |name, arm, budget| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm,
            budget_ticks: budget,
        },
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare empty pack and Ardougne bank food+coins before Start",
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
                cheat(c, "givebank lobster 28");
                cheat(c, "givebank coins 500");
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemAtMost {
                name: "Lobster",
                count: 0,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "tele to the Ardougne SE bank before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(BRIMHAVEN_BANK.level, BRIMHAVEN_BANK.x, BRIMHAVEN_BANK.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRIMHAVEN_BANK.x,
                z: BRIMHAVEN_BANK.z,
                level: BRIMHAVEN_BANK.level,
                radius: 6,
            },
            budget_ticks: 120,
        },
    });
    steps.push(start_catalog_step());
    steps.push(watch(
        "watch ordinary walk progress toward Captain Barnaby pier after bank restock and inspect",
        pier,
        BRIMHAVEN_INSPECT_V1_PIER_WATCH,
    ));

    Scenario {
        name: "brimhaven_moss_inspect_v1",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: pier,
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
/// PRESTART carries coins ≥ 30 and `setstat agility 10` (verified coins
/// before Start). Inspect `ok` + Barnaby `locName` and snapshot
/// `request_id:0` consumption are validated inside the example (paint +
/// self-stop) together with an actual bank-tile check. Membership stays
/// the selected profile's `map_members` fact; this fixture does not flip
/// a global option.
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
        name: "carry coins 30 and setstat agility 10 before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat agility 10");
                cheat(c, "give coins 30");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Item {
                name: "Coins",
                count: 30,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "tele to Captain Barnaby pier before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(BRIMHAVEN_PIER.level, BRIMHAVEN_PIER.x, BRIMHAVEN_PIER.z),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_prestart_verifies_carried_coins_before_start() {
        let s = route_inspect_brimhaven_v2_scenario();
        let start = s
            .steps
            .iter()
            .position(|st| st.name == "start the catalog card")
            .expect("catalog Start");
        assert!(
            s.steps[..start].iter().any(|st| {
                st.name == "carry coins 30 and setstat agility 10 before Start"
                    && st.wait.arm
                        == Proof::Item {
                            name: "Coins",
                            count: 30,
                        }
            }),
            "v2 must prove carried coins before Start: {:?}",
            s.steps[..start]
                .iter()
                .map(|st| st.name)
                .collect::<Vec<_>>()
        );
        assert!(
            !s.steps[start..]
                .iter()
                .any(|st| { st.name.contains("give") || st.name.contains("setstat") }),
            "no post-Start seed"
        );
    }

    #[test]
    fn v1_bank_coins_and_agility_30_unchanged() {
        let s = brimhaven_moss_inspect_v1_scenario();
        assert!(s
            .steps
            .iter()
            .any(|st| st.name.contains("empty pack and Ardougne bank food+coins")));
        assert_eq!(
            s.settings.start_script.as_deref(),
            Some("BrimhavenMossGiants")
        );
    }
}

use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `prayer_v2.ts` `STOP_OK`.
pub(crate) const PRAYER_V2_STOP: &str = "prayer v2 qualification complete";
/// Matches `prayer_v1.ts` `STOP_OK`.
pub(crate) const PRAYER_V1_STOP: &str = "prayer v1 qualification complete";

const PRAYER_DEADLINE: Duration = Duration::from_secs(180);
const PRAYER_WATCH: u32 = 240;
/// Protect from Melee selected-source level. Not run-energy `Proof::Stat` 16.
const PRAYER_SKILL_ID: i32 = 5;
const PRAYER_VARP0: i32 = 83;

fn prayer_seed_steps() -> Vec<Step> {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "setstat prayer 43 and read positive points before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setstat prayer 43");
                true
            }),
        },
        wait: Wait {
            // Effective points, not base and not run-energy id 16.
            // Core separately requires levels.prayer >= 43.
            arm: Proof::Stat {
                id: PRAYER_SKILL_ID,
                min: 1,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "setvar prayer 83..97 off and prove first overlay before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                for varp in PRAYER_VARP0..=97 {
                    cheat(c, &format!("setvar {varp} 0"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::VarpExact {
                id: PRAYER_VARP0,
                value: 0,
            },
            budget_ticks: 120,
        },
    });
    steps
}

fn prayer_file_scenario(name: &'static str, file_name: &'static str, stop: &'static str) -> Scenario {
    let mut steps = prayer_seed_steps();
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card reach its named helper stop",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: PRAYER_WATCH,
            arm: Proof::Stat {
                id: PRAYER_SKILL_ID,
                min: 1,
            },
        },
    });
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::Stat {
            id: PRAYER_SKILL_ID,
            min: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: PRAYER_DEADLINE,
            start_script: None,
            start_file: Some(file_name),
            wait_script_stop: Some(stop),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Headed File v2 witness: actual `prayer_v2.ts` helpers, not a stub.
pub(crate) fn prayer_v2_scenario() -> Scenario {
    prayer_file_scenario("prayer_v2_ts", "prayer_v2.ts", PRAYER_V2_STOP)
}

/// Headed File v1 witness: thin frozen Prayer adapter, not a catalog whale.
pub(crate) fn prayer_v1_scenario() -> Scenario {
    prayer_file_scenario("prayer_v1_ts", "prayer_v1.ts", PRAYER_V1_STOP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_prestart_seed(s: &Scenario, file: &str, stop: &str) {
        let start = s
            .steps
            .iter()
            .position(|st| st.name == "start the catalog card")
            .expect("catalog Start");
        assert!(
            s.steps[..start].iter().any(|st| {
                st.name.contains("setstat prayer 43")
                    && st.wait.arm
                        == Proof::Stat {
                            id: PRAYER_SKILL_ID,
                            min: 1,
                        }
            }),
            "must prove prayer points (stat 5), not run-energy 16: {:?}",
            s.steps[..start].iter().map(|st| st.name).collect::<Vec<_>>()
        );
        assert!(
            s.steps[..start].iter().any(|st| {
                st.name.contains("83..97")
                    && st.wait.arm
                        == Proof::VarpExact {
                            id: PRAYER_VARP0,
                            value: 0,
                        }
            }),
            "must prove an overlay varp off before Start"
        );
        assert!(
            !s.steps[start..]
                .iter()
                .any(|st| st.name.contains("setstat") || st.name.contains("setvar")),
            "no post-Start prayer seed"
        );
        assert_eq!(s.settings.start_file.as_deref(), Some(file));
        assert_eq!(s.settings.start_script, None);
        assert_eq!(s.settings.wait_script_stop, Some(stop));
        assert_ne!(
            s.proof,
            Proof::Stat { id: 16, min: 0 },
            "terminal proof must not be the Start-gate run-energy dummy"
        );
    }

    #[test]
    fn v2_prestart_reads_points_and_overlay_before_start() {
        assert_prestart_seed(
            &prayer_v2_scenario(),
            "prayer_v2.ts",
            PRAYER_V2_STOP,
        );
    }

    #[test]
    fn v1_prestart_reads_points_and_overlay_before_start() {
        assert_prestart_seed(
            &prayer_v1_scenario(),
            "prayer_v1.ts",
            PRAYER_V1_STOP,
        );
    }
}
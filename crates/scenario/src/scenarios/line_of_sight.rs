use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `line_of_sight_v2.ts` `STOP_OK`.
pub(crate) const LOS_V2_STOP: &str = "line of sight qualification complete";

const LOS_DEADLINE: Duration = Duration::from_secs(180);
const LOS_WATCH: u32 = 240;

/// Headed File witness: real NativeTick v2 plus frozen Reachability.lineOfSight.
pub(crate) fn line_of_sight_v2_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card reach its named helper stop",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: LOS_WATCH,
            arm: Proof::Stat { id: 16, min: 0 },
        },
    });
    Scenario {
        name: "line_of_sight_v2_ts",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: LOS_DEADLINE,
            start_script: None,
            start_file: Some("line_of_sight_v2.ts"),
            wait_script_stop: Some(LOS_V2_STOP),
            terminal_shot: Some("line_of_sight_v2_ts"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_card_uses_mainland_seed_and_named_stop() {
        let s = line_of_sight_v2_scenario();
        let start = s
            .steps
            .iter()
            .position(|st| st.name == "start the catalog card")
            .expect("catalog Start");
        assert!(
            !s.steps[start..]
                .iter()
                .any(|st| st.name.contains("setstat")
                    || st.name.contains("setvar")
                    || st.name.contains("collision")
                    || st.name.contains("flag")),
            "no post-Start collision injection: {:?}",
            s.steps[start..].iter().map(|st| st.name).collect::<Vec<_>>()
        );
        assert_eq!(s.settings.start_file.as_deref(), Some("line_of_sight_v2.ts"));
        assert_eq!(s.settings.start_script, None);
        assert_eq!(s.settings.wait_script_stop, Some(LOS_V2_STOP));
        assert_eq!(s.settings.deadline, LOS_DEADLINE);
        assert!(s.seed.mainland);
        assert!(s.settings.require_mainland_base);
        assert_eq!(s.settings.terminal_shot.as_deref(), Some("line_of_sight_v2_ts"));
    }
}

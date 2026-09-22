use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `cell_v2.ts` `STOP_OK`.
pub(crate) const CELL_V2_STOP: &str = "cell qualification complete";

const CELL_DEADLINE: Duration = Duration::from_secs(180);
const CELL_WATCH: u32 = 240;

/// Headed File witness: key-call first effect. Not cell entry.
pub(crate) fn cell_v2_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card reach its named helper stop",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: CELL_WATCH,
            arm: Proof::Stat { id: 16, min: 0 },
        },
    });
    Scenario {
        name: "cell_v2_ts",
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
            deadline: CELL_DEADLINE,
            start_script: None,
            start_file: Some("cell_v2.ts"),
            wait_script_stop: Some(CELL_V2_STOP),
            terminal_shot: Some("cell_v2_ts"),
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
        let s = cell_v2_scenario();
        let start = s
            .steps
            .iter()
            .position(|st| st.name == "start the catalog card")
            .expect("catalog Start");
        assert!(
            !s.steps[start..].iter().any(|st| st.name.contains("setstat")
                || st.name.contains("setvar")
                || st.name.contains("collision")
                || st.name.contains("npc")
                || st.name.contains("flag")),
            "no post-Start world mutation: {:?}",
            s.steps[start..]
                .iter()
                .map(|st| st.name)
                .collect::<Vec<_>>()
        );
        assert_eq!(s.settings.start_file.as_deref(), Some("cell_v2.ts"));
        assert_eq!(s.settings.start_script, None);
        assert_eq!(s.settings.wait_script_stop, Some(CELL_V2_STOP));
        assert_eq!(s.settings.deadline, CELL_DEADLINE);
        assert!(s.seed.mainland);
        assert!(s.settings.require_mainland_base);
        assert_eq!(s.settings.terminal_shot.as_deref(), Some("cell_v2_ts"));
    }
}

use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `actor_observation_v2.ts` `STOP_OK`.
pub(crate) const ACTOR_OBSERVATION_V2_STOP: &str = "actor observation qualification complete";

const ACTOR_DEADLINE: Duration = Duration::from_secs(180);
const ACTOR_WATCH: u32 = 240;

/// Headed File witness: real posted NPC size>=1 plus existing LOS helpers.
pub(crate) fn actor_observation_v2_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card reach its named helper stop",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: ACTOR_WATCH,
            arm: Proof::Stat { id: 16, min: 0 },
        },
    });
    Scenario {
        name: "actor_observation_v2_ts",
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
            deadline: ACTOR_DEADLINE,
            start_script: None,
            start_file: Some("actor_observation_v2.ts"),
            wait_script_stop: Some(ACTOR_OBSERVATION_V2_STOP),
            terminal_shot: Some("actor_observation_v2_ts"),
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
        let s = actor_observation_v2_scenario();
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
        assert_eq!(s.settings.start_file, Some("actor_observation_v2.ts"));
        assert_eq!(s.settings.start_script, None);
        assert_eq!(s.settings.wait_script_stop, Some(ACTOR_OBSERVATION_V2_STOP));
        assert_eq!(s.settings.deadline, ACTOR_DEADLINE);
        assert!(s.seed.mainland);
        assert!(s.settings.require_mainland_base);
        assert_eq!(s.settings.terminal_shot, Some("actor_observation_v2_ts"));
    }
}

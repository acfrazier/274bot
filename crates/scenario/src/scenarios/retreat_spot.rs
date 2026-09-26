use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `retreat_spot_v2.ts` `STOP_OK`.
pub(crate) const RETREAT_SPOT_V2_STOP: &str = "retreat spot qualification complete";

const RETREAT_SPOT_DEADLINE: Duration = Duration::from_secs(180);
const RETREAT_SPOT_WATCH: u32 = 240;

/// `retreat_spot_v2.ts` hops to a tile Chebyshev 2–6 from its Start tile,
/// the mainland landing; the seed leaves the player on the landing itself.
pub(crate) const RETREAT_SPOT_DEST: Proof = Proof::ArrivedRing {
    x: MAINLAND_LANDING.x,
    z: MAINLAND_LANDING.z,
    level: MAINLAND_LANDING.level,
    min: 2,
    max: 6,
};

/// Headed File witness: one awaited retreat hop. The gate needs the host's
/// walk-to dest, the end tile on dest and the settled receipt; never Attack /
/// a world walk / npc.
pub(crate) fn retreat_spot_v2_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card retreat to a tile 2-6 from its Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: RETREAT_SPOT_WATCH,
            arm: RETREAT_SPOT_DEST,
        },
    });
    Scenario {
        name: "retreat_spot_v2_ts",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: RETREAT_SPOT_DEST,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: RETREAT_SPOT_DEADLINE,
            start_script: None,
            start_file: Some("retreat_spot_v2.ts"),
            wait_script_stop: Some(RETREAT_SPOT_V2_STOP),
            terminal_shot: Some("retreat_spot_v2_ts"),
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
        let s = retreat_spot_v2_scenario();
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
        assert_eq!(s.settings.start_file, Some("retreat_spot_v2.ts"));
        assert_eq!(s.settings.start_script, None);
        assert_eq!(s.settings.wait_script_stop, Some(RETREAT_SPOT_V2_STOP));
        assert_eq!(s.settings.deadline, RETREAT_SPOT_DEADLINE);
        assert!(s.seed.mainland);
        assert!(s.settings.require_mainland_base);
        assert_eq!(s.settings.terminal_shot, Some("retreat_spot_v2_ts"));
    }
}

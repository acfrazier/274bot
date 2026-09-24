use super::acquire_key::{taverley_dungeon_seed_steps, TAVERLEY_DUNGEON_LANDING};
use crate::*;

/// Matches `cell_v2.ts` `STOP_OK`.
pub(crate) const CELL_V2_STOP: &str = "cell qualification complete";

/// A walk of about 108 tiles to the jail door, the unlock, Velrak's talk and
/// the door back out.
const CELL_DEADLINE: Duration = Duration::from_secs(300);
const CELL_WATCH: u32 = 460;

/// Headed File witness: one trip through Velrak's cell from the dungeon
/// landing, jail key in hand (frozen `fetchFromVelrak`, `supply.ts:942-965`).
/// The gate needs the host's walk-near the jail door, the unlock, the talk,
/// the door opened from inside, the dusty key in the pack outside the cell
/// and the settled `true` receipt.
pub(crate) fn cell_v2_scenario() -> Scenario {
    let mut steps = taverley_dungeon_seed_steps(true);
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
    use super::super::acquire_key::tests::starts_at_the_landing_after_its_seed;
    use super::*;

    #[test]
    fn cell_card_starts_at_the_dungeon_landing_holding_the_jail_key() {
        let s = cell_v2_scenario();
        starts_at_the_landing_after_its_seed(&s, TAVERLEY_DUNGEON_LANDING);
        let start = s
            .steps
            .iter()
            .position(|st| st.name == "start the catalog card")
            .expect("catalog Start");
        assert!(
            s.steps[..start].iter().any(|st| matches!(
                st.wait.arm,
                Proof::Item {
                    name: "Jail key",
                    count: 1
                }
            )),
            "the jail key is seeded and seen before Start"
        );
        assert_eq!(s.settings.start_file.as_deref(), Some("cell_v2.ts"));
        assert_eq!(s.settings.wait_script_stop, Some(CELL_V2_STOP));
    }
}

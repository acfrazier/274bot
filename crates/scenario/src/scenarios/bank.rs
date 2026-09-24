use super::pair::tele_step;
use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `bank_v2.ts` `STOP_OK`.
pub(crate) const BANK_V2_STOP: &str = "bank qualification complete";

/// Falador park fountain, Chebyshev 12 from the site bank (2946,3369,0):
/// outside its approach radius of 3, so the run walks, but a short walk.
/// From the mainland Lumbridge base the bank is about 280 tiles away, more
/// than the watch can wait out.
pub(crate) const FALADOR_FOUNTAIN: WorldTile = WorldTile {
    x: 2949,
    z: 3381,
    level: 0,
};

const BANK_DEADLINE: Duration = Duration::from_secs(180);
const BANK_WATCH: u32 = 240;

/// Headed File witness: one bank trip from the Falador fountain. The gate
/// needs the host's walk-near the bank at radius 3, the end tile within 3 of
/// it and the settled `true` receipt.
pub(crate) fn bank_v2_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(tele_step(
        "teleport to the Falador fountain before Start",
        FALADOR_FOUNTAIN,
    ));
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card reach its named helper stop",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: BANK_WATCH,
            arm: Proof::Stat { id: 16, min: 0 },
        },
    });
    Scenario {
        name: "bank_v2_ts",
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
            deadline: BANK_DEADLINE,
            start_script: None,
            start_file: Some("bank_v2.ts"),
            wait_script_stop: Some(BANK_V2_STOP),
            terminal_shot: Some("bank_v2_ts"),
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
    fn bank_card_starts_at_the_falador_fountain() {
        let s = bank_v2_scenario();
        starts_at_the_landing_after_its_seed(&s, FALADOR_FOUNTAIN);
        assert_eq!(s.settings.start_file, Some("bank_v2.ts"));
        assert_eq!(s.settings.wait_script_stop, Some(BANK_V2_STOP));
    }
}

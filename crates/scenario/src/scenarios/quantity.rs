use crate::proof::Proof;
use crate::{Scenario, ScenarioSettings, Seed, Step, StepKind, Wait};

pub(crate) const NORMAL_LOG_ID: i32 = 1511;
pub(crate) const ARROW_SHAFT_ID: i32 = 52;
pub(crate) const FLETCHING_STAT_ID: i32 = 9;

const MAKE_X_PROOF: &[Proof] = &[
    Proof::ProductDelta {
        id: ARROW_SHAFT_ID,
        count: 30,
    },
    Proof::XpDelta {
        id: FLETCHING_STAT_ID,
        min: 100,
    },
];

/// Cut two normal logs into arrow shafts with the real 377 Make-X prompt.
/// The host fixture opens the Make panel with the account's own knife/logs.
pub(crate) fn fletching_make_x_scenario() -> Scenario {
    Scenario {
        name: "fletching_make_x",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![
            Step {
                name: "select arrow shafts Make X",
                kind: StepKind::MakeProduct {
                    product_id: ARROW_SHAFT_ID,
                },
                wait: Wait {
                    arm: Proof::AmountPrompt,
                    budget_ticks: 20,
                },
            },
            Step {
                name: "enter two logs",
                kind: StepKind::EnterAmount { value: 2 },
                wait: Wait {
                    arm: Proof::InputConsumed {
                        id: NORMAL_LOG_ID,
                        count: 2,
                    },
                    budget_ticks: 30,
                },
            },
        ],
        proof: Proof::All(MAKE_X_PROOF),
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

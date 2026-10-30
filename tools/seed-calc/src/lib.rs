//! Offline developer calculator for minimum quest-campaign combat seeds.
//! See the tool README for assumptions, source formulas and JSON examples.
pub mod crossing;
pub mod fight;
mod food;
pub mod self_damage;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Query {
    Fight(fight::FightQuery),
    Crossing(crossing::CrossQuery),
    SelfDamage(self_damage::SelfDamageQuery),
}
#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Answer {
    Fight(fight::FightResult),
    Crossing(crossing::CrossResult),
    SelfDamage(self_damage::SelfDamageResult),
}
#[derive(Debug, Error)]
pub enum Error {
    #[error("selected combat facts unavailable: {0}")]
    Facts(String),
    #[error(transparent)]
    Fight(#[from] fight::FightError),
    #[error(transparent)]
    Crossing(#[from] crossing::CrossError),
    #[error(transparent)]
    SelfDamage(#[from] self_damage::SelfDamageError),
}
pub fn calculate(query: &Query) -> Result<Answer, Error> {
    match query {
        Query::Fight(query) => {
            let tables = script::combat::guard::shared_tables()
                .map_err(|error| Error::Facts(format!("{error:?}")))?;
            Ok(Answer::Fight(fight::calculate(query, &tables)?))
        }
        Query::Crossing(query) => Ok(Answer::Crossing(crossing::calculate(query)?)),
        Query::SelfDamage(query) => {
            let tables = script::combat::guard::shared_tables()
                .map_err(|error| Error::Facts(format!("{error:?}")))?;
            Ok(Answer::SelfDamage(self_damage::calculate(query, &tables)?))
        }
    }
}
impl Answer {
    pub fn summary(&self) -> String {
        match self {
            Self::Fight(result) => match &result.minimum {
                Some(minimum) => format!("fight {} ({} consecutive kills): Attack/Strength/Defence {}, HP {}, win {:.9}, death {:.9}, mean {:.3} ticks, max states {}; {}", result.npc, result.sequential_kills, minimum.stats.attack, minimum.stats.hitpoints, minimum.outcome.win_probability, minimum.outcome.death_probability, minimum.outcome.expected_ticks, result.max_explored_states, result.policy_reason),
                None => format!("fight {}: {}{}; max states {}; {}", result.npc, result.status, if result.reasons.is_empty() { String::new() } else { format!(": {}", result.reasons.join("; ")) }, result.max_explored_states, result.policy_reason),
            },
            Self::Crossing(result) => result.summary(),
            Self::SelfDamage(result) => {
                let food = match (result.minimum_lobsters, result.accepted_death_bound) {
                    (Some(count), Some(bound)) => {
                        format!(
                            "{count} conditional lobsters at {:.0}% death risk ({})",
                            bound * 100.0,
                            result.food_result_condition
                        )
                    }
                    _ => "no conditional food count meets 5%".into(),
                };
                format!(
                    "self-damage {}: today's no-eating death risk {:.9}; {food}; conditional food death {:.9}, budget stops {:.9}",
                    result.label,
                    result.no_food_death_probability,
                    result.death_probability_with_policy_food,
                    result.budget_stop_probability_with_policy_food
                )
            }
        }
    }
}

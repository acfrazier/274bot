//! Offline developer calculator for minimum quest-campaign combat seeds.
//! See the tool README for assumptions, source formulas and JSON examples.
pub mod crossing;
pub mod fight;
mod food;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Query {
    Fight(fight::FightQuery),
    Crossing(crossing::CrossQuery),
}
#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Answer {
    Fight(fight::FightResult),
    Crossing(crossing::CrossResult),
}
#[derive(Debug, Error)]
pub enum Error {
    #[error("selected combat facts unavailable: {0}")]
    Facts(String),
    #[error(transparent)]
    Fight(#[from] fight::FightError),
    #[error(transparent)]
    Crossing(#[from] crossing::CrossError),
}
pub fn calculate(query: &Query) -> Result<Answer, Error> {
    match query {
        Query::Fight(query) => {
            let tables = script::combat::guard::shared_tables()
                .map_err(|error| Error::Facts(format!("{error:?}")))?;
            Ok(Answer::Fight(fight::calculate(query, &tables)?))
        }
        Query::Crossing(query) => Ok(Answer::Crossing(crossing::calculate(query)?)),
    }
}
impl Answer {
    pub fn summary(&self) -> String {
        match self {
            Self::Fight(result) => match &result.minimum {
                Some(minimum) => format!("fight {} ({} consecutive kills): Attack/Strength/Defence {}, HP {}, win {:.9}, death {:.9}, mean {:.3} ticks", result.npc, result.sequential_kills, minimum.stats.attack, minimum.stats.hitpoints, minimum.outcome.win_probability, minimum.outcome.death_probability, minimum.outcome.expected_ticks),
                None => format!("fight {}: {}{}", result.npc, result.status, if result.reasons.is_empty() { String::new() } else { format!(": {}", result.reasons.join("; ")) }),
            },
            Self::Crossing(result) => result.summary(),
        }
    }
}

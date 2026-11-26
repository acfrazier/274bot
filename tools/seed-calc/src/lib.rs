//! Offline developer calculator for minimum quest-campaign combat seeds.
//! See the tool README for assumptions, source formulas and JSON examples.
pub mod crossing;
pub mod fight;
mod food;
pub mod self_damage;

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use api::selected::ClientRevision;
use script::combat::tables::CombatTables;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Supported game revisions for offline seed calculations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Revision {
    #[default]
    R289,
    R377,
}

impl Revision {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "289" => Ok(Self::R289),
            "377" => Ok(Self::R377),
            other => Err(format!(
                "unsupported revision {other:?}; expected 289 or 377"
            )),
        }
    }

    pub const fn as_u16(self) -> u16 {
        match self {
            Self::R289 => 289,
            Self::R377 => 377,
        }
    }

    pub const fn client_revision(self) -> ClientRevision {
        match self {
            Self::R289 => ClientRevision::R289,
            Self::R377 => ClientRevision::R377,
        }
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_u16())
    }
}

impl FromStr for Revision {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

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

fn build_combat_tables(revision: Revision) -> Result<Arc<CombatTables>, String> {
    let selected = api::game_data::for_revision(revision.client_revision())
        .map_err(|error| format!("revision {revision}: {error:?}"))?;
    if selected.revision() != i32::from(revision.as_u16()) {
        return Err(format!(
            "selected game data describes revision {}, expected revision {revision}",
            selected.revision()
        ));
    }
    CombatTables::build(selected).map_err(|error| format!("revision {revision}: {error:?}"))
}

/// Load and cache selected combat tables for the requested revision.
pub fn combat_tables(revision: Revision) -> Result<Arc<CombatTables>, Error> {
    static TABLES_289: LazyLock<Result<Arc<CombatTables>, String>> =
        LazyLock::new(|| build_combat_tables(Revision::R289));
    static TABLES_377: LazyLock<Result<Arc<CombatTables>, String>> =
        LazyLock::new(|| build_combat_tables(Revision::R377));
    let cached = match revision {
        Revision::R289 => &*TABLES_289,
        Revision::R377 => &*TABLES_377,
    };
    match cached {
        Ok(tables) => Ok(Arc::clone(tables)),
        Err(message) => Err(Error::Facts(message.clone())),
    }
}

pub fn calculate(query: &Query) -> Result<Answer, Error> {
    calculate_for_revision(query, Revision::R289)
}

pub fn calculate_for_revision(query: &Query, revision: Revision) -> Result<Answer, Error> {
    match query {
        Query::Fight(query) => {
            let tables = combat_tables(revision)?;
            Ok(Answer::Fight(fight::calculate(query, &tables)?))
        }
        Query::Crossing(query) => Ok(Answer::Crossing(crossing::calculate_for_revision(
            query, revision,
        )?)),
        Query::SelfDamage(query) => {
            let tables = combat_tables(revision)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_parser_accepts_only_289_and_377() {
        assert_eq!(Revision::parse("289"), Ok(Revision::R289));
        assert_eq!(Revision::parse("377"), Ok(Revision::R377));
        assert_eq!(Revision::default(), Revision::R289);
        for bad in ["274", "0", "378", "", "r377", "abc"] {
            assert!(
                Revision::parse(bad).is_err(),
                "expected revision {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn combat_tables_select_distinct_289_and_377_datasets() {
        if !api::test_support::game_data_available() {
            return;
        }
        let tables_289 = combat_tables(Revision::R289).expect("revision 289 combat tables");
        assert_eq!(tables_289.selected().revision(), 289);
        assert!(
            tables_289
                .selected()
                .npc_by_config("slayer_rockslug")
                .is_none(),
            "revision 289 must not contain 377-only slayer NPCs"
        );

        let tables_377 = combat_tables(Revision::R377).expect("revision 377 combat tables");
        assert_eq!(tables_377.selected().revision(), 377);
        let rockslug = tables_377
            .selected()
            .npc_by_config("slayer_rockslug")
            .expect("revision 377 must contain slayer_rockslug");
        assert_eq!(rockslug.id, 1622);
    }
}

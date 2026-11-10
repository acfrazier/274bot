//! Wire format for the `prereq_steps.json` family.

use super::{PrereqCatalog, PrereqJournal, PrereqStep};
use crate::selected::{FactError, SourceSpan};
use serde::Deserialize;
use std::sync::Arc;

pub(super) const SCHEMA: u16 = 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    schema: u16,
    facts: Facts,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    rows: Vec<PrereqStep>,
    journals: Vec<PrereqJournal>,
}

fn invalid(reason: impl Into<String>) -> FactError {
    FactError::Invalid {
        source: SourceSpan {
            file: Arc::from("prereq_steps.json"),
            first: 0,
            last: 0,
        },
        reason: Arc::from(reason.into()),
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<PrereqCatalog, FactError> {
    let family: Family = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("prerequisite family: {error}")))?;
    if family.schema != SCHEMA {
        return Err(FactError::Schema {
            expected: SCHEMA,
            actual: family.schema,
        });
    }
    for row in &family.facts.rows {
        for entry in row.crossings.iter() {
            check_tile(&entry.crossing.at, "anchor")?;
            check_tile(&entry.crossing.takeoff, "takeoff")?;
            check_tile(&entry.crossing.to, "destination")?;
            check_tile(&entry.pass.landing, "landing")?;
        }
    }
    PrereqCatalog::from_entries(family.facts.rows, family.facts.journals)
        .map_err(|reason| invalid(reason.to_string()))
}

/// World coordinates are non-negative and the plane is a game level.
/// The required takeoff is validated wherever at/to are.
fn check_tile(tile: &crate::WorldTile, role: &str) -> Result<(), FactError> {
    if tile.x < 0 || tile.z < 0 || !(0..=3).contains(&tile.level) {
        return Err(invalid(format!("prerequisite {role} tile is out of range")));
    }
    Ok(())
}

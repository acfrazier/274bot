//! Wire format for the source-pinned `prereq_steps.json` family.

use super::{PrereqCatalog, PrereqJournal, PrereqStep};
use crate::selected::{FactError, SelectedPin, SourceSpan};
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::sync::Arc;

pub(super) const SCHEMA: u16 = 3;

#[derive(Deserialize)]
struct Header<'a> {
    schema: u16,
    revision: i32,
    engine_commit: &'a str,
    content_commit: &'a str,
    cache_id: &'a str,
    content_id: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    #[serde(rename = "schema")]
    _schema: IgnoredAny,
    #[serde(rename = "revision")]
    _revision: IgnoredAny,
    #[serde(rename = "engine_commit")]
    _engine_commit: IgnoredAny,
    #[serde(rename = "content_commit")]
    _content_commit: IgnoredAny,
    #[serde(rename = "cache_id")]
    _cache_id: IgnoredAny,
    #[serde(rename = "content_id")]
    _content_id: IgnoredAny,
    #[serde(rename = "inputs")]
    _inputs: IgnoredAny,
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

fn check_header(pin: &SelectedPin, header: &Header<'_>) -> Result<(), FactError> {
    if header.schema != SCHEMA {
        return Err(FactError::Schema {
            expected: SCHEMA,
            actual: header.schema,
        });
    }
    if header.revision != pin.revision.as_i32()
        || header.engine_commit != &*pin.engine_commit
        || header.content_commit != &*pin.content_commit
        || header.cache_id != &*pin.cache_id
        || header.content_id != &*pin.content_id
    {
        return Err(FactError::PinMismatch);
    }
    Ok(())
}

pub(super) fn decode(pin: &SelectedPin, bytes: &[u8]) -> Result<PrereqCatalog, FactError> {
    let header: Header<'_> = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("prerequisite family header: {error}")))?;
    check_header(pin, &header)?;
    let family: Family = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("prerequisite family: {error}")))?;
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

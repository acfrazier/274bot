//! Wire format for the identity-only `quests.json` family.

use crate::game_data::QuestIdentityFacts;
use crate::selected::{FactError, SelectedPin, SourceSpan};
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::sync::Arc;

/// The extractor schema that this decoder reads.
pub(super) const SCHEMA: u16 = 1;

/// Artifact identity is checked before the fact payload is decoded.
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
    facts: QuestIdentityFacts,
}

fn invalid(reason: impl Into<String>) -> FactError {
    FactError::Invalid {
        source: SourceSpan {
            file: Arc::from("quests.json"),
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

/// Decode facts only after checking the family header against the selected pin.
pub(super) fn decode(pin: &SelectedPin, bytes: &[u8]) -> Result<QuestIdentityFacts, FactError> {
    let header: Header<'_> = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("quests family header: {error}")))?;
    check_header(pin, &header)?;
    let family: Family = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("quests family: {error}")))?;
    Ok(family.facts)
}

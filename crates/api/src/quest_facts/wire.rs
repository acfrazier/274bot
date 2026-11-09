//! Wire format for the identity-only `quests.json` family.

use crate::game_data::QuestIdentityFacts;
use crate::selected::{FactError, SourceSpan};
use serde::Deserialize;
use std::sync::Arc;

/// The extractor schema that this decoder reads.
pub(super) const SCHEMA: u16 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    schema: u16,
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

/// Decode the schema-checked local family without artifact identity binding.
pub(super) fn decode(bytes: &[u8]) -> Result<QuestIdentityFacts, FactError> {
    let family: Family = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("quests family: {error}")))?;
    if family.schema != SCHEMA {
        return Err(FactError::Schema {
            expected: SCHEMA,
            actual: family.schema,
        });
    }
    Ok(family.facts)
}

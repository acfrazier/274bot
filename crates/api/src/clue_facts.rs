//! Clue row lookup over the landed trail membership family. Takes the family
//! directly so a missing family is not an empty list.
//! The V8 wrapper is the only production caller of `SelectedGameData::trails`.

use crate::game_data::{TrailFacts, TrailMembershipRow, TrailParam};
use serde_json::{json, Value};

pub const TRAILS_UNAVAILABLE: &str = "family-unavailable:trails";
pub const UNKNOWN_ID: &str = "unknown-id";

/// Exactly one pin. The wrapper rejects neither and both before this call.
/// The id is the packed `i32` itself, never a converted number or string.
pub enum CluePin<'a> {
    Id(i32),
    Alias(&'a str),
}

/// The landed membership row. Not `{ rows }`. Membership is not support, and
/// the challenge answers stay unread.
pub fn clue_row(facts: Option<&TrailFacts>, pin: CluePin<'_>) -> Result<Value, &'static str> {
    let Some(facts) = facts else {
        return Err(TRAILS_UNAVAILABLE);
    };
    let row = match pin {
        CluePin::Id(id) => facts.rows.iter().find(|row| row.id == id),
        CluePin::Alias(raw) => facts.rows.iter().find(|row| alias_eq(&row.alias, raw)),
    };
    row.map(row_value).ok_or(UNKNOWN_ID)
}

/// Landed alias only, after trim and ASCII case-fold. A blank alias matches no
/// row, which is a miss and not family absence.
fn alias_eq(key: &str, raw: &str) -> bool {
    let wanted = raw.trim();
    !wanted.is_empty() && key.eq_ignore_ascii_case(wanted)
}

/// Built by hand the way `quest_facts::identity_value` is. `access` is present
/// only on the one bounded inclusion, so a row that did not land it omits the
/// key: never `null` and never rewritten to `open`.
fn row_value(row: &TrailMembershipRow) -> Value {
    let mut value = json!({
        "alias": row.alias,
        "id": row.id,
        "role": row.role,
        "params": row.params.iter().map(param_value).collect::<Vec<_>>(),
    });
    if let Some(access) = row.access.as_deref() {
        value["access"] = json!(access);
    }
    value
}

/// One raw param line, in file order. `^true` and `yes` stay strings.
fn param_value(row: &TrailParam) -> Value {
    json!({ "key": row.key, "value": row.value })
}

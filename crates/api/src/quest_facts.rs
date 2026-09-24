//! Quest identity and prereqs query. Takes the family directly so a missing
//! family is not an empty list.
//! The V8 wrapper is the only production caller of `SelectedGameData::quest_identity`.

use crate::game_data::{
    QuestIdentityFacts, QuestIdentityRow, QuestItemAlias, QuestRequirements, QuestSkillGate,
};
use serde_json::{json, Value};

pub const IDENTITY_UNAVAILABLE: &str = "family-unavailable:quest_identity";
pub const PREREQS_UNAVAILABLE: &str = "family-unavailable:quest_prereqs";
pub const UNKNOWN_QUEST: &str = "unknown-quest";

/// Exactly one pin. The wrapper rejects neither and both before this call.
pub enum IdentityPin<'a> {
    Id(&'a str),
    Name(&'a str),
}

/// The landed row object. Not `{ rows }`. Coverage is not a row.
pub fn quest_identity(
    facts: Option<&QuestIdentityFacts>,
    pin: IdentityPin<'_>,
) -> Result<Value, &'static str> {
    let Some(facts) = facts else {
        return Err(IDENTITY_UNAVAILABLE);
    };
    let row = match pin {
        IdentityPin::Id(raw) => facts.rows.iter().find(|row| key_eq(&row.id, raw)),
        IdentityPin::Name(raw) => facts.rows.iter().find(|row| key_eq(&row.display, raw)),
    };
    row.map(identity_value).ok_or(UNKNOWN_QUEST)
}

/// The requirements object only. Seed id, not display. Coverage is not attached.
pub fn quest_prereqs(facts: Option<&QuestIdentityFacts>, id: &str) -> Result<Value, &'static str> {
    let Some(facts) = facts else {
        return Err(PREREQS_UNAVAILABLE);
    };
    facts
        .rows
        .iter()
        .find(|row| key_eq(&row.id, id))
        .map(|row| requirements_value(&row.requirements))
        .ok_or(UNKNOWN_QUEST)
}

fn key_eq(key: &str, raw: &str) -> bool {
    let wanted = raw.trim();
    !wanted.is_empty() && key.eq_ignore_ascii_case(wanted)
}

fn identity_value(row: &QuestIdentityRow) -> Value {
    json!({
        "id": row.id,
        "component": row.component,
        "display": row.display,
        "varp": row.varp,
        "varp_id": row.varp_id,
        "complete": row.complete,
        "quest_points": row.quest_points,
        "unknown_sides": row.unknown_sides,
        "requirements": requirements_value(&row.requirements),
    })
}

fn requirements_value(req: &QuestRequirements) -> Value {
    json!({
        "qualification": req.qualification,
        "skills": req.skills.iter().map(skill_value).collect::<Vec<_>>(),
        "items": req.items.iter().map(item_value).collect::<Vec<_>>(),
        "empty_must_have": req.empty_must_have,
        "unknown_as_satisfied": req.unknown_as_satisfied,
    })
}

fn skill_value(row: &QuestSkillGate) -> Value {
    json!({ "skill": row.skill, "level": row.level })
}

fn item_value(row: &QuestItemAlias) -> Value {
    json!({
        "alias": row.alias,
        "quantity": row.quantity,
        "kind": row.kind,
    })
}

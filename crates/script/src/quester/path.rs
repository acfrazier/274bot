//! Path authoring values, not a runtime JSON interpreter.
use super::pair::PartnerDeclaration;
use api::selected::{FactKey, SkillMinimum};
use api::WorldTile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathDocument {
    pub schema: u16,
    pub id: FactKey,
    pub display_name: String,
    pub required: Vec<FactKey>,
    pub tested_stats: Option<Vec<SkillMinimum>>,
    pub partner: Option<PartnerDeclaration>,
    /// Schema 2 quest header. Omitted on schema 1 documents.
    #[serde(default)]
    pub quest: Option<QuestHeaderDocument>,
    pub roles: Vec<PathRoleDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestHeaderDocument {
    pub members: bool,
    pub quest_points: u16,
    pub requirements: Vec<QuestRequirementDocument>,
    pub items: Vec<QuestItemDocument>,
    pub acquire: BTreeMap<String, Vec<StepDocument>>,
    pub bank: QuestBankDocument,
    pub coin_float: u32,
    pub loadouts: BTreeMap<String, QuestLoadoutDocument>,
    pub areas: BTreeMap<String, NamedAreaDocument>,
    pub tools: Vec<String>,
    pub owns_inventory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestRequirementDocument {
    pub id: FactKey,
    pub kind: serde_json::Value,
    pub at: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestItemDocument {
    pub obj: String,
    pub qty: u32,
    pub kind: String,
    #[serde(default)]
    pub acquire: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum QuestBankDocument {
    Nearest(String),
    Tile {
        tile: [i32; 3],
        source: String,
        #[serde(default)]
        required: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestLoadoutDocument {
    #[serde(default)]
    pub worn: BTreeMap<String, String>,
    #[serde(default)]
    pub carry: Vec<LoadoutCarryDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadoutCarryDocument {
    pub item: String,
    pub qty: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedAreaDocument {
    pub boxes: Vec<[i32; 5]>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathRoleDocument {
    pub role: Option<FactKey>,
    pub progress_binding: FactKey,
    #[serde(default)]
    pub progress: Option<ProgressDocument>,
    #[serde(default)]
    pub prelude: Vec<StepDocument>,
    pub sequences: Vec<SequenceDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressDocument {
    pub colour: ProgressColourDocument,
    pub rules: Vec<ProgressRuleDocument>,
    pub flags: Vec<ProgressFlagDocument>,
    pub monotonic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressColourDocument {
    pub not_started: FactKey,
    pub in_progress: FactKey,
    pub complete: FactKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressRuleDocument {
    pub stage: FactKey,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub not: Vec<String>,
    #[serde(default)]
    pub varp: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressFlagDocument {
    pub flag: FactKey,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub count: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceDocument {
    pub stage: FactKey,
    pub required: Vec<FactKey>,
    pub terminal: bool,
    pub recovery_entry: Option<FactKey>,
    pub steps: Vec<StepDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepDocument {
    pub id: FactKey,
    pub kind: String,
    pub version: u16,
    pub args: serde_json::Value,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub advances: bool,
    pub skip_if: PredicateDocument,
    pub settle: PredicateDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PredicateDocument {
    All(Vec<PredicateDocument>),
    Any(Vec<PredicateDocument>),
    Not(Box<PredicateDocument>),
    Fact {
        kind: String,
        version: u16,
        args: serde_json::Value,
    },
}

impl PredicateDocument {
    /// Compile-time polarity of `All`/`Any`/`Not` only. Facts are unknown.
    pub fn constant_truth(&self) -> Option<bool> {
        match self {
            Self::All(items) if items.is_empty() => Some(true),
            Self::Any(items) if items.is_empty() => Some(false),
            Self::All(items) => {
                let mut all_true = true;
                for item in items {
                    match item.constant_truth() {
                        Some(false) => return Some(false),
                        Some(true) => {}
                        None => all_true = false,
                    }
                }
                all_true.then_some(true)
            }
            Self::Any(items) => {
                let mut all_false = true;
                for item in items {
                    match item.constant_truth() {
                        Some(true) => return Some(true),
                        Some(false) => {}
                        None => all_false = false,
                    }
                }
                all_false.then_some(false)
            }
            Self::Not(inner) => inner.constant_truth().map(|value| !value),
            Self::Fact { .. } => None,
        }
    }
}

/// Inclusive world box `[x1, z1, x2, z2, level]`.
pub fn box_contains(box5: [i32; 5], tile: WorldTile) -> bool {
    let [x1, z1, x2, z2, level] = box5;
    if tile.level != level {
        return false;
    }
    let (min_x, max_x) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
    let (min_z, max_z) = if z1 <= z2 { (z1, z2) } else { (z2, z1) };
    tile.x >= min_x && tile.x <= max_x && tile.z >= min_z && tile.z <= max_z
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn omitted_gates_and_versions_are_not_empty_success() {
        let document =
            json!({"schema":1,"id":"cook","display_name":"Cook","required":[],"roles":[]});
        let decoded: PathDocument = serde_json::from_value(document.clone()).unwrap();
        assert_eq!(decoded.id.0.as_ref(), "cook");
        assert!(decoded.quest.is_none());
        for field in ["schema", "required", "roles"] {
            let mut missing = document.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<PathDocument>(missing).is_err(),
                "{field}"
            );
        }
        let step = json!({"id":"talk","kind":"interact","version":1,"args":{},"skip_if":{"Any":[]},"settle":{"All":[]}});
        for field in ["version", "skip_if", "settle", "args"] {
            let mut missing = step.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<StepDocument>(missing).is_err(),
                "{field}"
            );
        }
        let mut unknown = step;
        unknown["setlle"] = json!({"All":[]});
        assert!(serde_json::from_value::<StepDocument>(unknown).is_err());
    }

    #[test]
    fn schema2_header_denies_unknown_fields_and_keeps_polarity() {
        let header = json!({
            "members": false,
            "quest_points": 1,
            "requirements": [],
            "items": [],
            "acquire": {},
            "bank": {"tile": [3093, 3243, 0], "source": "A defs/cooksassistant.ts:145"},
            "coin_float": 0,
            "loadouts": {},
            "areas": {},
            "tools": [],
            "owns_inventory": false
        });
        let decoded: QuestHeaderDocument = serde_json::from_value(header.clone()).unwrap();
        assert!(!decoded.members);
        assert!(matches!(
            decoded.bank,
            QuestBankDocument::Tile {
                required: false,
                ..
            }
        ));
        let mut required = header.clone();
        required["bank"]["required"] = json!(true);
        assert!(matches!(
            serde_json::from_value::<QuestHeaderDocument>(required)
                .unwrap()
                .bank,
            QuestBankDocument::Tile { required: true, .. }
        ));
        let mut extra = header;
        extra["food"] = json!([]);
        assert!(serde_json::from_value::<QuestHeaderDocument>(extra).is_err());
        assert_eq!(PredicateDocument::All(vec![]).constant_truth(), Some(true));
        assert_eq!(PredicateDocument::Any(vec![]).constant_truth(), Some(false));
    }
}

//! Path authoring values, not a runtime JSON interpreter.
use super::pair::PartnerDeclaration;
use api::selected::{FactKey, SkillMinimum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathDocument {
    pub schema: u16,
    pub id: FactKey,
    pub display_name: String,
    pub required: Vec<FactKey>,
    pub tested_stats: Option<Vec<SkillMinimum>>,
    pub partner: Option<PartnerDeclaration>,
    pub roles: Vec<PathRoleDocument>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathRoleDocument {
    pub role: Option<FactKey>,
    pub progress_binding: FactKey,
    pub sequences: Vec<SequenceDocument>,
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
    pub comment: Option<String>,
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
}

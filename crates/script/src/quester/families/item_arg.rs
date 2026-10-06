//! One item selector for Path arguments: a selected alias or an exact selected id.

use super::super::compile::CompileError;
use api::game_data::{GameItem, SelectedGameData};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ItemArg {
    Alias(String),
    Exact(ExactItemArg),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExactItemArg {
    pub id: i32,
}

impl ItemArg {
    pub fn resolve<'a>(
        &self,
        selected: &'a SelectedGameData,
    ) -> Result<&'a GameItem, CompileError> {
        match self {
            Self::Alias(alias) => selected
                .item_by_alias(alias)
                .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias.as_str())),
            Self::Exact(item) => selected.item_by_id(item.id).ok_or_else(|| {
                CompileError::code("unresolved-obj").with_detail(item.id.to_string())
            }),
        }
    }

    pub fn id(&self, selected: &SelectedGameData) -> Result<i32, CompileError> {
        self.resolve(selected).map(|item| item.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::ClientRevision;
    use serde_json::json;

    #[test]
    fn exact_id_never_resolves_by_a_same_named_item() {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let duplicates: Vec<_> = selected
            .items()
            .iter()
            .filter(|item| item.name.as_deref() == Some("Scorpion cage"))
            .collect();
        assert!(duplicates.len() > 1);
        for item in duplicates {
            let arg: ItemArg = serde_json::from_value(json!({"id": item.id})).unwrap();
            assert_eq!(arg.id(&selected).unwrap(), item.id);
        }
        let invalid: ItemArg = serde_json::from_value(json!({"id": -1})).unwrap();
        assert!(invalid.id(&selected).is_err());
        assert!(
            serde_json::from_value::<ItemArg>(json!({"id": 1, "name": "Scorpion cage"})).is_err()
        );
    }
}

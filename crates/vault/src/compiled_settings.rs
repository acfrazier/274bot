//! Versioned compiled-card settings envelope. M-297 owns migration and delivery.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledSettingsRecord {
    pub schema_version: u16,
    pub values: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_version_or_values_cannot_reset_saved_settings() {
        assert!(serde_json::from_str::<CompiledSettingsRecord>(r#"{"values":{}}"#).is_err());
        assert!(serde_json::from_str::<CompiledSettingsRecord>(r#"{"schema_version":1}"#).is_err());
        let record: CompiledSettingsRecord =
            serde_json::from_str(r#"{"schema_version":2,"values":{"partner_account":"alice"}}"#)
                .unwrap();
        assert_eq!(record.schema_version, 2);
        assert_eq!(record.values["partner_account"], "alice");
    }
}

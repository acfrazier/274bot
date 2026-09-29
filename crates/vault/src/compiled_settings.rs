//! Versioned compiled-card settings in the existing per-account settings map.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledSettingsRecord {
    pub schema_version: u16,
    pub values: Map<String, Value>,
}

impl CompiledSettingsRecord {
    /// An empty pre-envelope native entry is the schema-1 legacy form.
    /// Malformed envelopes are never interpreted as an empty settings bag.
    pub fn view(entry: &Map<String, Value>) -> Result<(u16, &Map<String, Value>), &'static str> {
        if entry.is_empty() {
            return Ok((1, entry));
        }
        if entry.len() != 2 {
            return Err("invalid compiled settings envelope");
        }
        let version = entry
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|version| *version != 0)
            .ok_or("invalid compiled settings schema version")?;
        let values = entry
            .get("values")
            .and_then(Value::as_object)
            .ok_or("compiled settings values must be an object")?;
        Ok((version, values))
    }

    pub fn into_entry(self) -> Map<String, Value> {
        Map::from_iter([
            ("schema_version".into(), Value::from(self.schema_version)),
            ("values".into(), Value::Object(self.values)),
        ])
    }
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

    #[test]
    fn envelope_view_preserves_versions_and_rejects_lossy_fallbacks() {
        let legacy = Map::new();
        assert_eq!(CompiledSettingsRecord::view(&legacy), Ok((1, &legacy)));
        let future = serde_json::json!({"schema_version": 2, "values": {"partner": ""}});
        let (version, values) = CompiledSettingsRecord::view(future.as_object().unwrap()).unwrap();
        assert_eq!(version, 2);
        assert_eq!(values["partner"], "");
        for malformed in [
            serde_json::json!({"schema_version": 0, "values": {}}),
            serde_json::json!({"schema_version": 65536, "values": {}}),
            serde_json::json!({"schema_version": 1, "values": null}),
            serde_json::json!({"schema_version": 1, "values": {}, "extra": true}),
            serde_json::json!({"partner": "Alice"}),
        ] {
            assert!(
                CompiledSettingsRecord::view(malformed.as_object().unwrap()).is_err(),
                "{malformed}"
            );
        }
    }
}

//! Operator script settings: per-card overrides persisted at
//! `~/.274bot/script-settings.json` (0o600), merged with schema defaults
//! before posting into an isolate.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::rs2b0t_registry::{ScriptSource, SettingDef};
use vault::write_private_file;

/// Default operator settings path (`~/.274bot/script-settings.json`).
pub fn default_script_settings_path() -> PathBuf {
    crate::bot_file("script-settings.json")
}

/// Stable file key for a `(source, name)` card.
pub fn card_key(source: ScriptSource, name: &str) -> String {
    format!("{}:{name}", source_key(source))
}

fn source_key(source: ScriptSource) -> &'static str {
    match source {
        ScriptSource::Catalog => "catalog",
        ScriptSource::File => "file",
        ScriptSource::Builtin => "builtin",
    }
}

/// On-disk shape: `{ "catalog:ChickenKiller": { "buryBones": false, … } }`.
type StoreFile = BTreeMap<String, Map<String, Value>>;

/// Persisted operator overrides keyed by [`card_key`].
pub struct ScriptSettingsStore {
    path: PathBuf,
    cards: StoreFile,
    dirty: bool,
}

impl ScriptSettingsStore {
    pub fn at(path: PathBuf) -> Self {
        let cards = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        ScriptSettingsStore {
            path,
            cards,
            dirty: false,
        }
    }

    pub fn with_default_path() -> Self {
        Self::at(default_script_settings_path())
    }

    pub fn card_key(&self, source: ScriptSource, name: &str) -> String {
        card_key(source, name)
    }

    pub fn overrides(&self, source: ScriptSource, name: &str) -> Map<String, Value> {
        self.cards
            .get(&card_key(source, name))
            .cloned()
            .unwrap_or_default()
    }

    fn card_mut(&mut self, source: ScriptSource, name: &str) -> &mut Map<String, Value> {
        self.dirty = true;
        self.cards.entry(card_key(source, name)).or_default()
    }

    pub fn set_bool(&mut self, source: ScriptSource, name: &str, id: &str, value: bool) {
        self.card_mut(source, name)
            .insert(id.to_string(), Value::Bool(value));
    }

    pub fn set_num(&mut self, source: ScriptSource, name: &str, id: &str, value: f64) {
        let v = serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::Number(0.into()));
        self.card_mut(source, name).insert(id.to_string(), v);
    }

    pub fn set_str(&mut self, source: ScriptSource, name: &str, id: &str, value: &str) {
        self.card_mut(source, name)
            .insert(id.to_string(), Value::String(value.to_string()));
    }

    pub fn set_value(&mut self, source: ScriptSource, name: &str, id: &str, value: Value) {
        self.card_mut(source, name).insert(id.to_string(), value);
    }

    pub fn save(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        let raw = serde_json::to_string_pretty(&self.cards)
            .map_err(|e| format!("script-settings encode: {e}"))?;
        write_private_file(&self.path, raw.as_bytes()).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    /// Schema defaults, then persisted overrides, then optional scenario inject.
    pub fn merged_bag(
        &self,
        source: ScriptSource,
        name: &str,
        schema: &[SettingDef],
        inject: Option<&Map<String, Value>>,
    ) -> Map<String, Value> {
        merge_bag(schema, &self.overrides(source, name), inject)
    }
}

/// Build the operator bag: schema defaults, operator overrides, inject last.
pub fn merge_bag(
    schema: &[SettingDef],
    overrides: &Map<String, Value>,
    inject: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut bag = Map::new();
    for def in schema {
        if let Some(default) = def.default.as_deref() {
            bag.insert(def.id.clone(), default_for_type(&def.ty, default));
        }
    }
    for (k, v) in overrides {
        bag.insert(k.clone(), v.clone());
    }
    if let Some(inject) = inject {
        for (k, v) in inject {
            bag.insert(k.clone(), v.clone());
        }
    }
    for def in schema {
        if let Some(v) = bag.get_mut(&def.id) {
            *v = coerce_setting_value(&def.ty, v);
        }
    }
    bag
}

/// Normalize a stored setting value to the JSON shape the prelude expects.
pub fn coerce_setting_value(ty: &str, value: &Value) -> Value {
    match ty {
        "tile" => coerce_tile(value),
        "list" | "string[]" => coerce_list(value),
        _ => value.clone(),
    }
}

fn coerce_tile(value: &Value) -> Value {
    if let Value::Object(obj) = value {
        if obj.get("x").and_then(|v| v.as_i64()).is_some()
            && obj.get("z").and_then(|v| v.as_i64()).is_some()
        {
            let level = obj.get("level").and_then(|v| v.as_i64()).unwrap_or(0);
            return serde_json::json!({
                "x": obj.get("x").and_then(|v| v.as_i64()).unwrap(),
                "z": obj.get("z").and_then(|v| v.as_i64()).unwrap(),
                "level": level,
            });
        }
    }
    if let Value::String(s) = value {
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            return coerce_tile(&v);
        }
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        if parts.len() >= 2 {
            if let (Ok(x), Ok(z)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
                let level = parts
                    .get(2)
                    .and_then(|p| p.parse::<i64>().ok())
                    .unwrap_or(0);
                return serde_json::json!({ "x": x, "z": z, "level": level });
            }
        }
    }
    value.clone()
}

fn coerce_list(value: &Value) -> Value {
    if let Value::Array(items) = value {
        return Value::Array(
            items
                .iter()
                .map(|v| match v {
                    Value::String(s) => Value::String(s.clone()),
                    other => Value::String(other.to_string()),
                })
                .collect(),
        );
    }
    if let Value::String(s) = value {
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            return coerce_list(&v);
        }
        if s.is_empty() {
            return Value::Array(vec![]);
        }
        return Value::Array(
            s.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(|part| Value::String(part.to_string()))
                .collect(),
        );
    }
    value.clone()
}

fn default_for_type(ty: &str, default: &str) -> Value {
    match ty {
        "boolean" => Value::Bool(default == "true"),
        "number" => default
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::Number(0.into())),
        "string" => Value::String(default.to_string()),
        "tile" | "list" | "string[]" => {
            let raw = if default.starts_with('{') || default.starts_with('[') {
                serde_json::from_str(default).unwrap_or_else(|_| Value::String(default.to_string()))
            } else {
                Value::String(default.to_string())
            };
            coerce_setting_value(ty, &raw)
        }
        _ => Value::String(default.to_string()),
    }
}

/// Whether a setting row should paint given its `showIf` raw text and the
/// current bag values. Inline `{ key: 'x', anyOf: ['y'] }` objects are
/// evaluated; unresolved identifier refs fail open (shown).
pub fn setting_visible(show_if: Option<&str>, bag: &Map<String, Value>) -> bool {
    let Some(raw) = show_if else {
        return true;
    };
    let raw = raw.trim();
    if !raw.starts_with('{') {
        return true;
    }
    let Some(key) = extract_quoted(raw, "key") else {
        return true;
    };
    let Some(any_of) = extract_any_of(raw) else {
        return true;
    };
    let current = bag.get(&key).map(value_as_setting_str).unwrap_or_default();
    any_of.iter().any(|v| v == &current)
}

fn extract_quoted(raw: &str, field: &str) -> Option<String> {
    let needle = format!("{field}:");
    let after = raw.split(&needle).nth(1)?.trim_start();
    if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        return Some(rest[..end].to_string());
    }
    if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    None
}

fn extract_any_of(raw: &str) -> Option<Vec<String>> {
    let start = raw.find("anyOf:")? + "anyOf:".len();
    let rest = raw[start..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }
    let end = rest.find(']')?;
    let inner = &rest[1..end];
    let mut out = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        let quoted = (part.starts_with('\'') && part.ends_with('\''))
            || (part.starts_with('"') && part.ends_with('"'));
        if quoted && part.len() >= 2 {
            out.push(part[1..part.len() - 1].to_string());
        }
    }
    Some(out)
}

fn value_as_setting_str(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// Display rows for the parameters section: `(label, value)` in schema order.
pub fn parameter_rows(schema: &[SettingDef], bag: &Map<String, Value>) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for def in schema {
        if !setting_visible(def.show_if.as_deref(), bag) {
            continue;
        }
        let label = def.label.as_deref().unwrap_or(&def.id).to_string();
        let value = bag
            .get(&def.id)
            .map(format_setting_value)
            .unwrap_or_else(|| "—".into());
        rows.push((label, value));
    }
    rows
}

/// Format a bag value for display in typed editors.
pub fn format_setting_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|x| match x {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(obj) if obj.contains_key("x") && obj.contains_key("z") => format!(
            "{}, {}, {}",
            obj.get("x").map(|v| v.to_string()).unwrap_or_default(),
            obj.get("z").map(|v| v.to_string()).unwrap_or_default(),
            obj.get("level").map(|v| v.to_string()).unwrap_or_default()
        ),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_bag_applies_defaults_then_overrides_then_inject() {
        let schema = vec![SettingDef {
            id: "buryBones".into(),
            ty: "boolean".into(),
            default: Some("true".into()),
            label: None,
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
        }];
        let mut overrides = Map::new();
        overrides.insert("buryBones".into(), Value::Bool(false));
        let mut inject = Map::new();
        inject.insert("buryBones".into(), Value::Bool(true));
        let bag = merge_bag(&schema, &overrides, Some(&inject));
        assert_eq!(bag.get("buryBones"), Some(&Value::Bool(true)));
    }

    #[test]
    fn show_if_any_of_hides_when_value_mismatch() {
        let mut bag = Map::new();
        bag.insert("combatStyle".into(), Value::String("mage".into()));
        let show = "{ key: 'combatStyle', anyOf: ['melee'] }";
        assert!(!setting_visible(Some(show), &bag));
        bag.insert("combatStyle".into(), Value::String("melee".into()));
        assert!(setting_visible(Some(show), &bag));
    }
}

//! Operator script settings are held by the local-state service. This module
//! keeps card identities and the pure settings-bag projection.

use serde_json::{Map, Value};

use crate::card::{ScriptSource, SettingDef};

/// Stable key for a `(source, name)` card in the operator settings domain.
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

/// The profile-global setting (frozen `SettingsStore.globalBag()`): the
/// account's clue duel partner. It rides every settings bag a run receives
/// and the isolate installs it as `globalSettingsBag`, apart from the card's.
pub const CLUE_DUEL_PARTNER: &str = "clueDuelPartner";

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
        "number" => coerce_number(value),
        "tile" => coerce_tile(value),
        "list" | "string[]" => coerce_list(value),
        _ => value.clone(),
    }
}
/// A keyed failure while validating a saved setting value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid saved value for setting `{setting}`: {kind}")]
pub struct SavedSettingValueError {
    pub setting: String,
    pub kind: SavedSettingValueErrorKind,
}

/// The reason a saved setting value does not match its current schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SavedSettingValueErrorKind {
    #[error("the setting has no default")]
    MissingDefault,
    #[error("the value has the wrong type")]
    WrongType,
    #[error("the number is outside its declared range")]
    OutsideRange,
    #[error("the setting has an invalid numeric range")]
    InvalidRange,
    #[error("the tile must contain only integer x, z, and level fields, with level from 0 to 3")]
    InvalidTile,
    #[error("the value is not a current option")]
    InvalidOption,
    #[error("the list contains a value that is not a current option")]
    InvalidListOption,
    #[error(transparent)]
    Loadout(#[from] crate::loadouts_store::LoadoutResolutionError),
}

/// Return the value of a setting's literal default.
pub fn setting_default_value(def: &SettingDef) -> Option<Value> {
    def.default
        .as_deref()
        .map(|default| default_for_type(&def.ty, default))
}

/// Validate one raw saved value without coercing, clipping, or replacing it.
///
/// `options` must contain the current values supplied by an `optionsFrom`
/// source. Static choices come from `def.options`.
pub fn validate_saved_setting_value(
    def: &SettingDef,
    value: &Value,
    options: &[String],
    loadouts: &[api::loadout::Loadout],
) -> Result<(), SavedSettingValueError> {
    let error = |kind| SavedSettingValueError {
        setting: def.id.clone(),
        kind,
    };
    if def.id == "loadout" && def.ty != "string" {
        return Err(error(SavedSettingValueErrorKind::WrongType));
    }

    let choices = if def.options_from.is_some() {
        Some(options)
    } else if def.options.is_empty() {
        None
    } else {
        Some(def.options.as_slice())
    };

    match def.ty.as_str() {
        "boolean" if !value.is_boolean() => {
            return Err(error(SavedSettingValueErrorKind::WrongType));
        }
        "boolean" => {}
        "number" => {
            let Some(number) = value.as_f64().filter(|number| number.is_finite()) else {
                return Err(error(SavedSettingValueErrorKind::WrongType));
            };
            let parse_bound = |bound: Option<&str>| {
                bound
                    .map(|raw| {
                        raw.parse::<f64>()
                            .ok()
                            .filter(|number| number.is_finite())
                            .ok_or(SavedSettingValueErrorKind::InvalidRange)
                    })
                    .transpose()
            };
            let min = parse_bound(def.min.as_deref()).map_err(error)?;
            let max = parse_bound(def.max.as_deref()).map_err(error)?;
            if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
                return Err(error(SavedSettingValueErrorKind::OutsideRange));
            }
        }
        "string" => {
            let Some(text) = value.as_str() else {
                return Err(error(SavedSettingValueErrorKind::WrongType));
            };
            if def.options_from.as_deref() == Some("named-banks") && text.is_empty() {
                return Err(error(SavedSettingValueErrorKind::InvalidOption));
            }
            if let Some(choices) = choices {
                let is_loadout = def.id == "loadout";
                let blank_loadouts =
                    def.options_from.as_deref() == Some("loadouts") && text.is_empty();
                if !blank_loadouts && !is_loadout && !choices.iter().any(|choice| choice == text) {
                    return Err(error(SavedSettingValueErrorKind::InvalidOption));
                }
            }
            if def.id == "loadout" && !text.trim().is_empty() {
                crate::loadouts_store::resolve_script_loadout(loadouts, text)
                    .map_err(|kind| error(SavedSettingValueErrorKind::Loadout(kind)))?;
            }
        }
        "string[]" | "list" => {
            let Some(values) = value.as_array() else {
                return Err(error(SavedSettingValueErrorKind::WrongType));
            };
            for value in values {
                let Some(text) = value.as_str() else {
                    return Err(error(SavedSettingValueErrorKind::WrongType));
                };
                if choices.is_some_and(|choices| !choices.iter().any(|choice| choice == text)) {
                    return Err(error(SavedSettingValueErrorKind::InvalidListOption));
                }
            }
        }
        "tile" => {
            let Some(tile) = value.as_object() else {
                return Err(error(SavedSettingValueErrorKind::WrongType));
            };
            let valid_integer = |value: Option<&Value>| {
                value
                    .and_then(Value::as_i64)
                    .is_some_and(|number| i32::try_from(number).is_ok())
            };
            if tile.len() != 3
                || !valid_integer(tile.get("x"))
                || !valid_integer(tile.get("z"))
                || !tile
                    .get("level")
                    .and_then(Value::as_i64)
                    .is_some_and(|level| (0..=3).contains(&level))
            {
                return Err(error(SavedSettingValueErrorKind::InvalidTile));
            }
        }
        _ => return Err(error(SavedSettingValueErrorKind::WrongType)),
    }
    Ok(())
}

/// Serde rejects `12.0` for integer settings even though it is an exact
/// integer. Preserve fractional and out-of-range numbers for typed consumers
/// to reject rather than silently rounding them.
fn coerce_number(value: &Value) -> Value {
    let Some(number) = value.as_number() else {
        return value.clone();
    };
    if number.is_i64() || number.is_u64() {
        return value.clone();
    }
    let Some(number) = number.as_f64() else {
        return value.clone();
    };
    if !number.is_finite() || number.fract() != 0.0 {
        return value.clone();
    }
    if number >= 0.0 {
        if number < u64::MAX as f64 {
            return Value::from(number as u64);
        }
    } else if number >= i64::MIN as f64 {
        return Value::from(number as i64);
    }
    value.clone()
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
        "number" => {
            let value = default
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .unwrap_or_else(|| Value::Number(0.into()));
            coerce_setting_value(ty, &value)
        }
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
/// current bag values. Inline `{ key: 'x', anyOf: ['y'] }` objects (the
/// legacy schema text) and the strict manifest's JSON form
/// `{"key":"x","anyOf":["y"]}` are evaluated; unresolved identifier refs
/// fail open (shown). Array master values match when any element equals an
/// `anyOf` token using the same case-sensitive scalar comparison as
/// strings. Empty arrays match nothing.
pub fn setting_visible(show_if: Option<&str>, bag: &Map<String, Value>) -> bool {
    let Some(raw) = show_if else {
        return true;
    };
    let raw = raw.trim();
    if !raw.starts_with('{') {
        return true;
    }
    let Some((key, any_of)) = show_condition(raw) else {
        return true;
    };
    match bag.get(&key) {
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| any_of.iter().any(|v| v == &value_as_setting_str(item))),
        other => {
            let current = other.map(value_as_setting_str).unwrap_or_default();
            any_of.iter().any(|v| v == &current)
        }
    }
}

/// The `(key, anyOf)` of a `showIf` text: the strict manifest's JSON object,
/// else the legacy inline object literal.
fn show_condition(raw: &str) -> Option<(String, Vec<String>)> {
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(raw) {
        let key = object.get("key")?.as_str()?.to_owned();
        let any_of = object
            .get("anyOf")?
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        return Some((key, any_of));
    }
    Some((extract_quoted(raw, "key")?, extract_any_of(raw)?))
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
    fn numeric_bag_normalizes_integral_floats_without_rounding_fractions() {
        let numeric = |id: &str, default: &str| SettingDef {
            id: id.into(),
            ty: "number".into(),
            default: Some(default.into()),
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
            item_option_spec: None,
        };
        let schema = vec![
            numeric("defaultRadius", "12"),
            numeric("legacyRadius", "2"),
            numeric("fractionalRadius", "2"),
        ];
        let mut overrides = Map::new();
        overrides.insert("legacyRadius".into(), serde_json::from_str("12.0").unwrap());
        overrides.insert("fractionalRadius".into(), serde_json::json!(12.5));

        let bag = merge_bag(&schema, &overrides, None);

        assert_eq!(bag["defaultRadius"].as_u64(), Some(12));
        assert_eq!(bag["legacyRadius"].as_u64(), Some(12));
        assert_eq!(bag["fractionalRadius"].as_f64(), Some(12.5));
    }
    fn setting(id: &str, ty: &str, default: Option<&str>) -> SettingDef {
        SettingDef {
            id: id.into(),
            ty: ty.into(),
            default: default.map(str::to_owned),
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
            item_option_spec: None,
        }
    }

    #[test]
    fn saved_value_validation_checks_supported_types_and_ranges() {
        let boolean = setting("enabled", "boolean", Some("true"));
        assert_eq!(setting_default_value(&boolean), Some(Value::Bool(true)));
        validate_saved_setting_value(&boolean, &Value::Bool(false), &[], &[]).unwrap();

        let mut number = setting("radius", "number", Some("2"));
        number.min = Some("1".into());
        number.max = Some("4".into());
        validate_saved_setting_value(&number, &serde_json::json!(2.5), &[], &[]).unwrap();
        let error =
            validate_saved_setting_value(&number, &serde_json::json!(5), &[], &[]).unwrap_err();
        assert_eq!(error.setting, "radius");
        assert_eq!(error.kind, SavedSettingValueErrorKind::OutsideRange);

        let text = setting("style", "string", Some("melee"));
        validate_saved_setting_value(&text, &Value::String("ranged".into()), &[], &[]).unwrap();
        assert_eq!(
            validate_saved_setting_value(&boolean, &Value::String("false".into()), &[], &[])
                .unwrap_err()
                .setting,
            "enabled"
        );

        let mut strings = setting("items", "string[]", Some("[]"));
        strings.options_from = Some("named-banks".into());
        validate_saved_setting_value(&strings, &serde_json::json!([]), &[], &[]).unwrap();
        let error = validate_saved_setting_value(
            &strings,
            &serde_json::json!(["stale"]),
            &["current".into()],
            &[],
        )
        .unwrap_err();
        assert_eq!(error.setting, "items");
        assert_eq!(error.kind, SavedSettingValueErrorKind::InvalidListOption);

        let legacy_list = setting("legacy", "list", Some(""));
        validate_saved_setting_value(&legacy_list, &serde_json::json!(["old-value"]), &[], &[])
            .unwrap();

        let tile = setting("destination", "tile", Some(r#"{"x":1,"z":2,"level":0}"#));
        validate_saved_setting_value(
            &tile,
            &serde_json::json!({"x": i32::MIN, "z": i32::MAX, "level": 3}),
            &[],
            &[],
        )
        .unwrap();
        let error = validate_saved_setting_value(
            &tile,
            &serde_json::json!({"x": 1, "z": 2, "level": 4}),
            &[],
            &[],
        )
        .unwrap_err();
        assert_eq!(error.setting, "destination");
        assert_eq!(error.kind, SavedSettingValueErrorKind::InvalidTile);
        assert!(setting_default_value(&setting("missing", "string", None)).is_none());
    }

    #[test]
    fn saved_value_validation_uses_dynamic_sources_and_loadout_resolution() {
        let loadouts = [
            api::loadout::Loadout::new("First choice"),
            api::loadout::Loadout::new("Second choice"),
        ];
        let invalid_selector = setting("loadout", "string[]", Some("[]"));
        let error = validate_saved_setting_value(
            &invalid_selector,
            &serde_json::json!(["First choice"]),
            &[],
            &loadouts,
        )
        .unwrap_err();
        assert_eq!(error.setting, "loadout");
        assert_eq!(error.kind, SavedSettingValueErrorKind::WrongType);

        let mut selected = setting("loadout", "string", Some(""));
        selected.options_from = Some("loadouts".into());
        validate_saved_setting_value(&selected, &Value::String("".into()), &[], &[]).unwrap();
        validate_saved_setting_value(
            &selected,
            &Value::String(" second choice ".into()),
            &["First choice".into(), "Second choice".into()],
            &loadouts,
        )
        .unwrap();
        let error = validate_saved_setting_value(
            &selected,
            &Value::String("missing".into()),
            &["First choice".into(), "Second choice".into()],
            &loadouts,
        )
        .unwrap_err();
        assert_eq!(error.setting, "loadout");
        assert!(matches!(
            error.kind,
            SavedSettingValueErrorKind::Loadout(
                crate::loadouts_store::LoadoutResolutionError::Unknown { .. }
            )
        ));
        let error =
            validate_saved_setting_value(&selected, &Value::String("missing".into()), &[], &[])
                .unwrap_err();
        assert!(matches!(
            error.kind,
            SavedSettingValueErrorKind::Loadout(
                crate::loadouts_store::LoadoutResolutionError::NoLoadouts { .. }
            )
        ));

        let mut loadout_source = setting("target", "string", Some(""));
        loadout_source.options_from = Some("loadouts".into());
        validate_saved_setting_value(&loadout_source, &Value::String("".into()), &[], &[]).unwrap();

        let mut bank = setting("bank", "string", Some(""));
        bank.options_from = Some("named-banks".into());
        let error =
            validate_saved_setting_value(&bank, &Value::String("".into()), &[], &[]).unwrap_err();
        assert_eq!(error.setting, "bank");
        assert_eq!(error.kind, SavedSettingValueErrorKind::InvalidOption);
    }

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
            item_option_spec: None,
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

    #[test]
    fn show_if_array_membership_is_case_sensitive_scalar() {
        let show = "{ key: 'items', anyOf: ['custom'] }";
        let mut bag = Map::new();
        bag.insert(
            "items".into(),
            Value::Array(vec![Value::String("custom".into())]),
        );
        assert!(setting_visible(Some(show), &bag));
        bag.insert(
            "items".into(),
            Value::Array(vec![Value::String("yew_longbow".into())]),
        );
        assert!(!setting_visible(Some(show), &bag));
        bag.insert("items".into(), Value::Array(vec![]));
        assert!(!setting_visible(Some(show), &bag));
        bag.insert(
            "items".into(),
            Value::Array(vec![Value::String("Custom".into())]),
        );
        assert!(
            !setting_visible(Some(show), &bag),
            "array membership keeps scalar case sensitivity"
        );
        bag.insert("combatStyle".into(), Value::String("Melee".into()));
        assert!(
            !setting_visible(Some("{ key: 'combatStyle', anyOf: ['melee'] }"), &bag),
            "scalar equality stays case-sensitive"
        );
        bag.insert("combatStyle".into(), Value::String("melee".into()));
        assert!(setting_visible(
            Some("{ key: 'combatStyle', anyOf: ['melee'] }"),
            &bag
        ));
    }

    #[test]
    fn strict_manifest_json_show_if_is_evaluated_like_the_legacy_literal() {
        // The strict manifest stores its condition as JSON text; a value may
        // hold commas, quotes and brackets.
        let show = r#"{"key":"ore","anyOf":["rune","a, b","it's [x]"]}"#;
        let mut bag = Map::new();
        bag.insert("ore".into(), Value::String("copper".into()));
        assert!(!setting_visible(Some(show), &bag), "a mismatch hides");
        for shown in ["rune", "a, b", "it's [x]"] {
            bag.insert("ore".into(), Value::String(shown.into()));
            assert!(setting_visible(Some(show), &bag), "{shown}");
        }
        bag.insert("ore".into(), Value::String("Rune".into()));
        assert!(!setting_visible(Some(show), &bag), "case-sensitive");
        bag.insert(
            "ore".into(),
            Value::Array(vec![Value::String("rune".into())]),
        );
        assert!(setting_visible(Some(show), &bag), "list-any matches");
        bag.insert("ore".into(), Value::Array(Vec::new()));
        assert!(
            !setting_visible(Some(show), &bag),
            "an empty list matches nothing"
        );
        bag.remove("ore");
        assert!(
            !setting_visible(Some(show), &bag),
            "a missing key reads blank"
        );
        // Anything else fails open, as before.
        for malformed in [
            r#"{"key":"ore"}"#,
            r#"{"key":1,"anyOf":["x"]}"#,
            r#"{"anyOf":["x"]}"#,
        ] {
            assert!(setting_visible(Some(malformed), &bag), "{malformed}");
        }
    }
}

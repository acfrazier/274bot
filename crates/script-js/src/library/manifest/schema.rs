use super::errors::{DeclarationReason, ManifestReason, ScriptDeclarationError, SettingReason};
use super::literal::{LiteralValue, SpannedProperty, SpannedValue};
use script::card::SettingDef;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

const FIELDS: &[&str] = &[
    "type",
    "default",
    "label",
    "min",
    "max",
    "options",
    "optionLabels",
    "group",
    "showIf",
    "optionsFrom",
    "help",
];

pub(super) fn project_settings(
    value: &SpannedValue,
    source: &str,
    path: &Path,
    field_path: &str,
) -> Result<Vec<SettingDef>, ScriptDeclarationError> {
    let LiteralValue::Object(entries) = &value.value else {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            field_path,
            ManifestReason::WrongType("object"),
        ));
    };
    let mut settings = Vec::with_capacity(entries.len());
    let mut references = Vec::new();
    for setting in entries {
        let id = setting.key.as_str();
        validate_id(
            id,
            source,
            path,
            &format!("{field_path}.{id}"),
            &setting.key_span,
        )?;
        let setting_path = format!("{field_path}.{id}");
        let properties = setting_object(&setting.value, source, path, &setting_path, "object")?;
        for property in properties {
            if !FIELDS.contains(&property.key.as_str()) {
                return Err(setting_error(
                    path,
                    source,
                    property.key_span.clone(),
                    &format!("{setting_path}.{}", property.key),
                    SettingReason::UnknownField(property.key.clone()),
                ));
            }
        }

        let ty = get(properties, "type").ok_or_else(|| {
            setting_error(
                path,
                source,
                setting.value.span.clone(),
                &format!("{setting_path}.type"),
                SettingReason::MissingType,
            )
        })?;
        let ty = setting_string(ty, source, path, &format!("{setting_path}.type"))?;
        if !matches!(ty, "boolean" | "number" | "string" | "string[]" | "tile") {
            return Err(setting_error(
                path,
                source,
                property_span(properties, "type", &setting.value),
                &format!("{setting_path}.type"),
                SettingReason::UnknownType(ty.to_owned()),
            ));
        }
        let default = get(properties, "default").ok_or_else(|| {
            setting_error(
                path,
                source,
                setting.value.span.clone(),
                &format!("{setting_path}.default"),
                SettingReason::MissingDefault,
            )
        })?;
        let default_text = default_text(
            default,
            ty,
            source,
            path,
            &format!("{setting_path}.default"),
        )?;

        let label = optional_text(properties, "label", source, path, &setting_path)?;
        let help = optional_text(properties, "help", source, path, &setting_path)?;
        let group = optional_text(properties, "group", source, path, &setting_path)?;

        let min = optional_number(properties, "min", ty, source, path, &setting_path)?;
        let max = optional_number(properties, "max", ty, source, path, &setting_path)?;
        if let (Some(min_value), Some(max_value)) = (min.as_deref(), max.as_deref()) {
            if min_value > max_value {
                return Err(setting_error(
                    path,
                    source,
                    property_span(properties, "max", &setting.value),
                    &format!("{setting_path}.max"),
                    SettingReason::InvertedRange,
                ));
            }
        }
        if let LiteralValue::Number(number) = &default.value {
            if min
                .as_deref()
                .is_some_and(|min| *number < parse_number(min))
                || max
                    .as_deref()
                    .is_some_and(|max| *number > parse_number(max))
            {
                return Err(setting_error(
                    path,
                    source,
                    default.span.clone(),
                    &format!("{setting_path}.default"),
                    SettingReason::DefaultOutsideRange,
                ));
            }
        }

        if get(properties, "options").is_some() && get(properties, "optionsFrom").is_some() {
            return Err(setting_error(
                path,
                source,
                property_span(properties, "optionsFrom", &setting.value),
                &format!("{setting_path}.optionsFrom"),
                SettingReason::CompetingOptions,
            ));
        }
        let options = optional_options(properties, ty, source, path, &setting_path)?;
        let options_from = optional_options_from(properties, ty, source, path, &setting_path)?;
        validate_default_options(
            default,
            ty,
            &options,
            source,
            path,
            &format!("{setting_path}.default"),
        )?;
        let option_labels =
            optional_option_labels(properties, ty, &options, source, path, &setting_path)?;
        let (show_if, show_reference) =
            optional_show_if(properties, id, source, path, &setting_path)?;
        if let Some(reference) = show_reference {
            references.push(reference);
        }

        settings.push(SettingDef {
            id: id.to_owned(),
            ty: ty.to_owned(),
            default: Some(default_text),
            label,
            min,
            max,
            step: None,
            options,
            option_labels,
            group,
            show_if,
            options_from,
            csv_toggle: None,
            help,
            item_option_spec: None,
        });
    }

    let types: HashMap<&str, &str> = settings
        .iter()
        .map(|setting| (setting.id.as_str(), setting.ty.as_str()))
        .collect();
    for reference in references {
        if !types.contains_key(reference.key.as_str()) {
            return Err(setting_error(
                path,
                source,
                reference.span,
                &reference.path,
                SettingReason::UnknownReference(reference.key),
            ));
        }
        if reference.owner == reference.key
            || !matches!(
                types.get(reference.key.as_str()),
                Some(&"string" | &"string[]")
            )
        {
            return Err(setting_error(
                path,
                source,
                reference.span,
                &reference.path,
                SettingReason::InvalidReference,
            ));
        }
    }
    Ok(settings)
}

struct ShowReference {
    owner: String,
    key: String,
    path: String,
    span: Range<usize>,
}

fn optional_show_if(
    properties: &[SpannedProperty],
    owner: &str,
    source: &str,
    path: &Path,
    setting_path: &str,
) -> Result<(Option<String>, Option<ShowReference>), ScriptDeclarationError> {
    let Some(value) = get(properties, "showIf") else {
        return Ok((None, None));
    };
    let condition_path = format!("{setting_path}.showIf");
    let condition = setting_object(value, source, path, &condition_path, "object")?;
    for property in condition {
        if !matches!(property.key.as_str(), "key" | "anyOf") {
            return Err(setting_error(
                path,
                source,
                property.key_span.clone(),
                &format!("{condition_path}.{}", property.key),
                SettingReason::UnknownField(property.key.clone()),
            ));
        }
    }
    let key = get(condition, "key").ok_or_else(|| {
        setting_error(
            path,
            source,
            value.span.clone(),
            &format!("{condition_path}.key"),
            SettingReason::WrongType("string"),
        )
    })?;
    let key = setting_string(key, source, path, &format!("{condition_path}.key"))?.to_owned();
    let any_of = get(condition, "anyOf").ok_or_else(|| {
        setting_error(
            path,
            source,
            value.span.clone(),
            &format!("{condition_path}.anyOf"),
            SettingReason::WrongType("nonempty array of strings"),
        )
    })?;
    let LiteralValue::Array(values) = &any_of.value else {
        return Err(setting_error(
            path,
            source,
            any_of.span.clone(),
            &format!("{condition_path}.anyOf"),
            SettingReason::WrongType("nonempty array of strings"),
        ));
    };
    if values.is_empty() {
        return Err(setting_error(
            path,
            source,
            any_of.span.clone(),
            &format!("{condition_path}.anyOf"),
            SettingReason::EmptyCondition,
        ));
    }
    let mut seen = HashSet::with_capacity(values.len());
    let mut cooked = Vec::with_capacity(values.len());
    for item in values {
        let LiteralValue::String(text) = &item.value else {
            return Err(setting_error(
                path,
                source,
                item.span.clone(),
                &format!("{condition_path}.anyOf"),
                SettingReason::WrongType("string"),
            ));
        };
        if !seen.insert(text.clone()) {
            return Err(setting_error(
                path,
                source,
                item.span.clone(),
                &format!("{condition_path}.anyOf"),
                SettingReason::DuplicateCondition(text.clone()),
            ));
        }
        cooked.push(text.clone());
    }
    let serialized = serde_json::to_string(&serde_json::json!({
        "key": key,
        "anyOf": cooked,
    }))
    .expect("a JSON-compatible showIf condition serializes");
    Ok((
        Some(serialized),
        Some(ShowReference {
            owner: owner.to_owned(),
            key,
            path: format!("{condition_path}.key"),
            span: get(condition, "key").map_or_else(|| value.span.clone(), |key| key.span.clone()),
        }),
    ))
}

fn optional_option_labels(
    properties: &[SpannedProperty],
    ty: &str,
    options: &[String],
    source: &str,
    path: &Path,
    setting_path: &str,
) -> Result<Vec<String>, ScriptDeclarationError> {
    let Some(value) = get(properties, "optionLabels") else {
        return Ok(Vec::new());
    };
    if !matches!(ty, "string" | "string[]") {
        return Err(field_not_allowed(
            value,
            "optionLabels",
            ty,
            source,
            path,
            setting_path,
        ));
    }
    let label_path = format!("{setting_path}.optionLabels");
    let labels = setting_object(value, source, path, &label_path, "object")?;
    let mut by_option = HashMap::with_capacity(labels.len());
    for label in labels {
        let LiteralValue::String(text) = &label.value.value else {
            return Err(setting_error(
                path,
                source,
                label.value.span.clone(),
                &format!("{label_path}.{}", label.key),
                SettingReason::WrongType("string"),
            ));
        };
        validate_display_text(
            text,
            source,
            path,
            &format!("{label_path}.{}", label.key),
            &label.value.span,
        )?;
        if !options.iter().any(|option| option == &label.key) {
            return Err(setting_error(
                path,
                source,
                label.key_span.clone(),
                &format!("{label_path}.{}", label.key),
                SettingReason::UnknownOptionLabel(label.key.clone()),
            ));
        }
        by_option.insert(label.key.as_str(), text.clone());
    }
    Ok(options
        .iter()
        .map(|option| {
            by_option
                .remove(option.as_str())
                .unwrap_or_else(|| option.clone())
        })
        .collect())
}

fn optional_options(
    properties: &[SpannedProperty],
    ty: &str,
    source: &str,
    path: &Path,
    setting_path: &str,
) -> Result<Vec<String>, ScriptDeclarationError> {
    let Some(value) = get(properties, "options") else {
        return Ok(Vec::new());
    };
    if !matches!(ty, "string" | "string[]") {
        return Err(field_not_allowed(
            value,
            "options",
            ty,
            source,
            path,
            setting_path,
        ));
    }
    let options_path = format!("{setting_path}.options");
    let LiteralValue::Array(values) = &value.value else {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            &options_path,
            SettingReason::WrongType("array of strings"),
        ));
    };
    if values.is_empty() {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            &options_path,
            SettingReason::EmptyOptions,
        ));
    }
    let mut seen = HashSet::with_capacity(values.len());
    let mut options = Vec::with_capacity(values.len());
    for item in values {
        let LiteralValue::String(option) = &item.value else {
            return Err(setting_error(
                path,
                source,
                item.span.clone(),
                &options_path,
                SettingReason::WrongType("string"),
            ));
        };
        if !seen.insert(option.clone()) {
            return Err(setting_error(
                path,
                source,
                item.span.clone(),
                &options_path,
                SettingReason::DuplicateOption(option.clone()),
            ));
        }
        options.push(option.clone());
    }
    Ok(options)
}

fn optional_options_from(
    properties: &[SpannedProperty],
    ty: &str,
    source: &str,
    path: &Path,
    setting_path: &str,
) -> Result<Option<String>, ScriptDeclarationError> {
    let Some(value) = get(properties, "optionsFrom") else {
        return Ok(None);
    };
    if !matches!(ty, "string" | "string[]") {
        return Err(field_not_allowed(
            value,
            "optionsFrom",
            ty,
            source,
            path,
            setting_path,
        ));
    }
    let options_from = setting_string(value, source, path, &format!("{setting_path}.optionsFrom"))?;
    if !matches!(options_from, "loadouts" | "named-banks") {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            &format!("{setting_path}.optionsFrom"),
            SettingReason::UnknownOptionSource(options_from.to_owned()),
        ));
    }
    Ok(Some(options_from.to_owned()))
}

fn validate_default_options(
    default: &SpannedValue,
    ty: &str,
    options: &[String],
    source: &str,
    path: &Path,
    field_path: &str,
) -> Result<(), ScriptDeclarationError> {
    if options.is_empty() {
        return Ok(());
    }
    let values: Vec<&str> = match (&default.value, ty) {
        (LiteralValue::String(value), "string") => vec![value],
        (LiteralValue::Array(values), "string[]") => values
            .iter()
            .filter_map(|value| match &value.value {
                LiteralValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .collect(),
        _ => return Ok(()),
    };
    for value in values {
        if !options.iter().any(|option| option == value) {
            return Err(setting_error(
                path,
                source,
                default.span.clone(),
                field_path,
                SettingReason::DefaultNotOption(value.to_owned()),
            ));
        }
    }
    Ok(())
}

fn optional_number(
    properties: &[SpannedProperty],
    field: &str,
    ty: &str,
    source: &str,
    path: &Path,
    setting_path: &str,
) -> Result<Option<String>, ScriptDeclarationError> {
    let Some(value) = get(properties, field) else {
        return Ok(None);
    };
    if ty != "number" {
        return Err(field_not_allowed(
            value,
            field,
            ty,
            source,
            path,
            setting_path,
        ));
    }
    let LiteralValue::Number(number) = &value.value else {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            &format!("{setting_path}.{field}"),
            SettingReason::WrongType("number"),
        ));
    };
    if !number.is_finite() {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            &format!("{setting_path}.{field}"),
            SettingReason::NonFinite,
        ));
    }
    Ok(Some(number.to_string()))
}

fn default_text(
    value: &SpannedValue,
    ty: &str,
    source: &str,
    path: &Path,
    field_path: &str,
) -> Result<String, ScriptDeclarationError> {
    let mismatch = match (ty, &value.value) {
        ("boolean", LiteralValue::Boolean(value)) => return Ok(value.to_string()),
        ("number", LiteralValue::Number(value)) => return Ok(value.to_string()),
        ("string", LiteralValue::String(value)) => return Ok(value.clone()),
        ("string[]", LiteralValue::Array(values)) => {
            let mut strings = Vec::with_capacity(values.len());
            for item in values {
                let LiteralValue::String(value) = &item.value else {
                    return Err(setting_error(
                        path,
                        source,
                        item.span.clone(),
                        field_path,
                        SettingReason::WrongType("array of strings"),
                    ));
                };
                strings.push(value.clone());
            }
            return Ok(serde_json::to_string(&strings).expect("string arrays serialize"));
        }
        ("tile", LiteralValue::Object(_)) => {
            let LiteralValue::Object(properties) = &value.value else {
                unreachable!();
            };
            if let Some(tile) = parse_tile(properties) {
                return Ok(serde_json::to_string(&tile).expect("tile serializes"));
            }
            return Err(setting_error(
                path,
                source,
                value.span.clone(),
                field_path,
                SettingReason::InvalidTile,
            ));
        }
        ("boolean", _) => "boolean",
        ("number", _) => "number",
        ("string", _) => "string",
        ("string[]", _) => "array of strings",
        ("tile", _) => "tile object",
        _ => "supported setting type",
    };
    Err(setting_error(
        path,
        source,
        value.span.clone(),
        field_path,
        SettingReason::WrongType(mismatch),
    ))
}

fn parse_tile(properties: &[SpannedProperty]) -> Option<serde_json::Value> {
    if properties.len() != 3 {
        return None;
    }
    let mut x = None;
    let mut z = None;
    let mut level = None;
    for property in properties {
        let LiteralValue::Number(number) = &property.value.value else {
            return None;
        };
        if number.fract() != 0.0 || *number < i32::MIN as f64 || *number > i32::MAX as f64 {
            return None;
        }
        match property.key.as_str() {
            "x" => x = Some(*number as i32),
            "z" => z = Some(*number as i32),
            "level" if (0.0..=3.0).contains(number) => level = Some(*number as u8),
            _ => return None,
        }
    }
    Some(serde_json::json!({
        "x": x?,
        "z": z?,
        "level": level?,
    }))
}

fn optional_text(
    properties: &[SpannedProperty],
    field: &str,
    source: &str,
    path: &Path,
    setting_path: &str,
) -> Result<Option<String>, ScriptDeclarationError> {
    let Some(value) = get(properties, field) else {
        return Ok(None);
    };
    let value_path = format!("{setting_path}.{field}");
    let text = setting_string(value, source, path, &value_path)?;
    validate_display_text(text, source, path, &value_path, &value.span)?;
    Ok(Some(text.to_owned()))
}

fn validate_id(
    id: &str,
    source: &str,
    path: &Path,
    field_path: &str,
    span: &Range<usize>,
) -> Result<(), ScriptDeclarationError> {
    if id.is_empty() || id.trim() != id || id.chars().any(is_forbidden_display_character) {
        return Err(setting_error(
            path,
            source,
            span.clone(),
            field_path,
            SettingReason::InvalidId,
        ));
    }
    Ok(())
}

pub(super) fn validate_display_text(
    value: &str,
    source: &str,
    path: &Path,
    field_path: &str,
    span: &Range<usize>,
) -> Result<(), ScriptDeclarationError> {
    if value.chars().any(is_forbidden_display_character) {
        return Err(ScriptDeclarationError::new(
            path,
            source,
            span.clone(),
            field_path,
            DeclarationReason::Manifest(ManifestReason::ControlCharacter),
        ));
    }
    Ok(())
}

pub(super) fn is_forbidden_display_character(ch: char) -> bool {
    matches!(ch as u32,
        0x0000..=0x001f
            | 0x007f..=0x009f
            | 0x2028..=0x2029
            | 0x202a..=0x202e
            | 0x2066..=0x2069
    )
}

fn field_not_allowed(
    value: &SpannedValue,
    field: &str,
    ty: &str,
    source: &str,
    path: &Path,
    setting_path: &str,
) -> ScriptDeclarationError {
    setting_error(
        path,
        source,
        value.span.clone(),
        &format!("{setting_path}.{field}"),
        SettingReason::FieldNotAllowed(Box::new((field.to_owned(), ty.to_owned()))),
    )
}

fn setting_object<'a>(
    value: &'a SpannedValue,
    source: &str,
    path: &Path,
    field_path: &str,
    expected: &'static str,
) -> Result<&'a [SpannedProperty], ScriptDeclarationError> {
    let LiteralValue::Object(properties) = &value.value else {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            field_path,
            SettingReason::WrongType(expected),
        ));
    };
    Ok(properties)
}

fn setting_string<'a>(
    value: &'a SpannedValue,
    source: &str,
    path: &Path,
    field_path: &str,
) -> Result<&'a str, ScriptDeclarationError> {
    let LiteralValue::String(value) = &value.value else {
        return Err(setting_error(
            path,
            source,
            value.span.clone(),
            field_path,
            SettingReason::WrongType("string"),
        ));
    };
    Ok(value)
}

fn get<'a>(properties: &'a [SpannedProperty], key: &str) -> Option<&'a SpannedValue> {
    properties
        .iter()
        .find(|property| property.key == key)
        .map(|property| &property.value)
}

fn property_span(
    properties: &[SpannedProperty],
    key: &str,
    fallback: &SpannedValue,
) -> Range<usize> {
    properties
        .iter()
        .find(|property| property.key == key)
        .map_or_else(
            || fallback.span.clone(),
            |property| property.value.span.clone(),
        )
}

fn parse_number(value: &str) -> f64 {
    value.parse().unwrap_or(f64::NAN)
}

fn setting_error(
    path: &Path,
    source: &str,
    span: Range<usize>,
    field_path: &str,
    reason: SettingReason,
) -> ScriptDeclarationError {
    ScriptDeclarationError::new(
        path,
        source,
        span,
        field_path,
        DeclarationReason::Setting(reason),
    )
}

fn manifest_error(
    path: &Path,
    source: &str,
    span: Range<usize>,
    field_path: &str,
    reason: ManifestReason,
) -> ScriptDeclarationError {
    ScriptDeclarationError::new(
        path,
        source,
        span,
        field_path,
        DeclarationReason::Manifest(reason),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_display_character_set_is_exact() {
        for code in [
            0x0000, 0x001f, 0x007f, 0x009f, 0x2028, 0x2029, 0x202a, 0x202e, 0x2066, 0x2069,
        ] {
            assert!(is_forbidden_display_character(
                char::from_u32(code).unwrap()
            ));
        }
        for code in [0x0020, 0x007e, 0x00a0, 0x2027, 0x202f, 0x2065, 0x206a] {
            assert!(!is_forbidden_display_character(
                char::from_u32(code).unwrap()
            ));
        }
    }
}

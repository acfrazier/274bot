use super::Scripts;
use crate::operations::{OperationId, Outcome};
use crate::session::OperatorSession;
use serde_json::Value;

/// Identity of one editable field in one profile's selected card.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParameterEditKey {
    profile: Option<String>,
    card: String,
    field: String,
}

impl ParameterEditKey {
    pub fn new(profile: Option<&str>, selection: &script::ScriptSel, field: &str) -> Self {
        let card = match selection {
            script::ScriptSel::Compiled(id) => script::compiled_identity_key(*id),
            script::ScriptSel::Loaded(source, lookup) => {
                let source = match source {
                    script::ScriptSource::Catalog => "catalog",
                    script::ScriptSource::File => "file",
                    script::ScriptSource::Builtin => "builtin",
                };
                format!("{source}:{lookup}")
            }
        };
        Self {
            profile: profile.map(str::to_owned),
            card,
            field: field.to_owned(),
        }
    }

    fn matches(&self, profile: Option<&str>, selection: &script::ScriptSel, field: &str) -> bool {
        if self.profile.as_deref() != profile || self.field != field {
            return false;
        }
        let (source, identity) = match selection {
            script::ScriptSel::Compiled(id) => ("compiled", id.0),
            script::ScriptSel::Loaded(source, identity) => (
                match source {
                    script::ScriptSource::Catalog => "catalog",
                    script::ScriptSource::File => "file",
                    script::ScriptSource::Builtin => "builtin",
                },
                identity.as_str(),
            ),
        };
        self.card
            .strip_prefix(source)
            .and_then(|rest| rest.strip_prefix(':'))
            == Some(identity)
    }
}

fn cached_parameter_key<'a>(
    cache: &'a mut Vec<ParameterEditKey>,
    scope: &mut Option<(Option<String>, script::ScriptSel)>,
    profile: Option<&str>,
    selection: &script::ScriptSel,
    field: &str,
) -> &'a ParameterEditKey {
    if scope
        .as_ref()
        .is_none_or(|(cached_profile, cached_selection)| {
            cached_profile.as_deref() != profile || cached_selection != selection
        })
    {
        cache.clear();
        *scope = Some((profile.map(str::to_owned), selection.clone()));
    }
    if let Some(index) = cache
        .iter()
        .position(|key| key.matches(profile, selection, field))
    {
        return &cache[index];
    }
    cache.push(ParameterEditKey::new(profile, selection, field));
    cache.last().expect("cached parameter key")
}
fn edit_mut_or_insert<'a>(
    edits: &'a mut std::collections::HashMap<ParameterEditKey, ParameterEdit>,
    key: &ParameterEditKey,
    initial_text: &str,
    baseline_value: &Option<Value>,
) -> &'a mut ParameterEdit {
    if !edits.contains_key(key) {
        edits.insert(
            key.clone(),
            ParameterEdit::new(initial_text, baseline_value.clone()),
        );
    }
    edits.get_mut(key).expect("inserted parameter edit")
}

#[derive(Debug)]
pub(super) struct ParameterEdit {
    text: String,
    validated_text: String,
    baseline_text: String,
    baseline_value: Option<Value>,
    last_attempted: Option<Value>,
    pending: Option<(OperationId, Value, String)>,
    error: Option<String>,
    dirty: bool,
    discard_after_pending: bool,
}

impl ParameterEdit {
    fn new(text: &str, baseline_value: Option<Value>) -> Self {
        Self {
            text: text.to_owned(),
            validated_text: text.to_owned(),
            baseline_text: text.to_owned(),
            baseline_value,
            last_attempted: None,
            pending: None,
            error: None,
            dirty: false,
            discard_after_pending: false,
        }
    }
}

/// Result of a committed settings edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterCommit {
    /// The value is already current and there is no save in flight.
    Unchanged,
    /// The same value is already being written.
    Pending(OperationId),
    /// One profile write was queued.
    Submitted(OperationId),
    /// A global legacy setting was saved synchronously.
    Saved,
}

/// Parse and validate a field as its editor is changed. Option values are the
/// same resolved keys used by each front end's discrete controls.
pub fn parse_parameter_text(
    def: &script::SettingDef,
    text: &str,
    options: &[String],
) -> Result<Value, String> {
    let value = match def.ty.as_str() {
        "number" => {
            let number = text
                .trim()
                .parse::<f64>()
                .map_err(|_| "invalid number".to_string())?;
            if !number.is_finite() {
                return Err("invalid number".into());
            }
            if def
                .min
                .as_deref()
                .and_then(|min| min.parse::<f64>().ok())
                .is_some_and(|min| number < min)
                || def
                    .max
                    .as_deref()
                    .and_then(|max| max.parse::<f64>().ok())
                    .is_some_and(|max| number > max)
            {
                return Err(format!(
                    "value must be between {} and {}",
                    def.min.as_deref().unwrap_or("-∞"),
                    def.max.as_deref().unwrap_or("∞")
                ));
            }
            let number =
                serde_json::Number::from_f64(number).ok_or_else(|| "invalid number".to_string())?;
            script::coerce_setting_value("number", &Value::Number(number))
        }
        "string" => Value::String(text.to_owned()),
        "tile" => parse_tile(text)?,
        "list" | "string[]" => script::coerce_setting_value("list", &Value::String(text.into())),
        other => return Err(format!("unsupported type {other}")),
    };

    if !options.is_empty() {
        match &value {
            Value::String(value) if !options.iter().any(|option| option == value) => {
                return Err(format!("unknown option: {value}"));
            }
            Value::Array(values) => {
                for value in values {
                    let Some(value) = value.as_str() else {
                        return Err(format!("invalid {}", def.ty));
                    };
                    if !options.iter().any(|option| option == value) {
                        return Err(format!("unknown option: {value}"));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(value)
}

fn parse_tile(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        let value =
            serde_json::from_str::<Value>(trimmed).map_err(|_| "invalid tile".to_string())?;
        let coerced = script::coerce_setting_value("tile", &value);
        return tile_matches(&coerced)
            .then_some(coerced)
            .ok_or_else(|| "invalid tile".into());
    }
    let parts: Vec<&str> = trimmed.split(',').map(str::trim).collect();
    if !(2..=3).contains(&parts.len()) {
        return Err("invalid tile".into());
    }
    let coords = parts
        .iter()
        .map(|part| part.parse::<i64>().map_err(|_| "invalid tile".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({
        "x": coords[0],
        "z": coords[1],
        "level": coords.get(2).copied().unwrap_or(0),
    }))
}

fn tile_matches(value: &Value) -> bool {
    value.as_object().is_some_and(|obj| {
        obj.get("x").and_then(Value::as_i64).is_some()
            && obj.get("z").and_then(Value::as_i64).is_some()
    })
}

impl Scripts {
    /// Persistent ImGui input buffer; the caller supplies the saved value on
    /// first use and reads/writes the same buffer across frames.
    pub fn parameter_text_mut(
        &mut self,
        key: &ParameterEditKey,
        initial_text: &str,
        baseline_value: Option<Value>,
    ) -> &mut String {
        let edit = self
            .parameter_edits
            .entry(key.clone())
            .or_insert_with(|| ParameterEdit::new(initial_text, baseline_value.clone()));
        if edit.pending.is_none() && !edit.dirty && edit.baseline_text != initial_text {
            edit.text.clear();
            edit.text.push_str(initial_text);
            edit.baseline_text.clear();
            edit.baseline_text.push_str(initial_text);
            edit.baseline_value = baseline_value;
            edit.error = None;
        }
        &mut edit.text
    }

    /// Reuse the editor's field identity across frames, allocating it only
    /// when the selected profile, card, or field changes.
    pub fn parameter_text_mut_for(
        &mut self,
        profile: Option<&str>,
        selection: &script::ScriptSel,
        field: &str,
        initial_text: &str,
        baseline_value: Option<Value>,
    ) -> &mut String {
        let (cache, scope, edits) = (
            &mut self.parameter_key_cache,
            &mut self.parameter_key_scope,
            &mut self.parameter_edits,
        );
        let key = cached_parameter_key(cache, scope, profile, selection, field);
        let edit = edit_mut_or_insert(edits, key, initial_text, &baseline_value);
        if edit.pending.is_none() && !edit.dirty && edit.baseline_text != initial_text {
            edit.text.clear();
            edit.text.push_str(initial_text);
            edit.baseline_text.clear();
            edit.baseline_text.push_str(initial_text);
            edit.baseline_value = baseline_value;
            edit.error = None;
        }
        &mut edit.text
    }

    /// Validate a persistent UI buffer without copying its text back through
    /// the coordinator.
    pub fn validate_parameter_buffer(
        &mut self,
        key: &ParameterEditKey,
        baseline_text: &str,
        baseline_value: Option<Value>,
        def: &script::SettingDef,
        options: &[String],
    ) -> Result<Value, String> {
        let edit = self
            .parameter_edits
            .entry(key.clone())
            .or_insert_with(|| ParameterEdit::new(baseline_text, baseline_value));
        validate_edit(edit, def, options)
    }

    pub fn validate_parameter_buffer_for(
        &mut self,
        profile: Option<&str>,
        selection: &script::ScriptSel,
        field: &str,
        baseline: (&str, Option<Value>),
        def: &script::SettingDef,
        options: &[String],
    ) -> Result<Value, String> {
        let (baseline_text, baseline_value) = baseline;
        let (cache, scope, edits) = (
            &mut self.parameter_key_cache,
            &mut self.parameter_key_scope,
            &mut self.parameter_edits,
        );
        let key = cached_parameter_key(cache, scope, profile, selection, field);
        let edit = edit_mut_or_insert(edits, key, baseline_text, &baseline_value);
        validate_edit(edit, def, options)
    }

    pub fn parameter_text_error(&self, key: &ParameterEditKey) -> Option<&str> {
        self.parameter_edits
            .get(key)
            .and_then(|edit| edit.error.as_deref())
    }

    pub fn parameter_text_error_for(
        &self,
        profile: Option<&str>,
        selection: &script::ScriptSel,
        field: &str,
    ) -> Option<&str> {
        self.parameter_key_cache
            .iter()
            .find(|key| key.matches(profile, selection, field))
            .and_then(|key| self.parameter_edits.get(key))
            .and_then(|edit| edit.error.as_deref())
    }

    pub fn discard_parameter_text_for(
        &mut self,
        profile: Option<&str>,
        selection: &script::ScriptSel,
        field: &str,
    ) {
        let (cache, edits) = (&self.parameter_key_cache, &mut self.parameter_edits);
        let Some(key) = cache
            .iter()
            .find(|key| key.matches(profile, selection, field))
        else {
            return;
        };
        if edits.get(key).is_some_and(|edit| edit.pending.is_some()) {
            if let Some(edit) = edits.get_mut(key) {
                edit.discard_after_pending = true;
            }
        } else {
            edits.remove(key);
        }
    }

    /// Queue one profile save for a committed field. Both UI front ends call
    /// this after shared text validation; failed attempts remain deduplicated
    /// until a different value is committed.
    pub fn commit_parameter_value<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        key: &ParameterEditKey,
        selection: &script::ScriptSel,
        field: &str,
        value: Value,
        current: Option<Value>,
    ) -> Result<ParameterCommit, String> {
        let existing = self.parameter_edits.get(key);
        if existing.is_some_and(|edit| edit.last_attempted.as_ref() == Some(&value)) {
            let edit = self.parameter_edits.get(key).expect("checked edit");
            if let Some((op, _, _)) = edit.pending {
                return Ok(ParameterCommit::Pending(op));
            }
            if let Some(error) = edit.error.as_ref() {
                return Err(error.clone());
            }
            return Ok(ParameterCommit::Unchanged);
        }
        if current.as_ref() == Some(&value) && existing.is_none_or(|edit| edit.pending.is_none()) {
            self.parameter_edits.remove(key);
            return Ok(ParameterCommit::Unchanged);
        }
        let (raw, baseline_text) = existing
            .map(|edit| (edit.text.clone(), edit.baseline_text.clone()))
            .unwrap_or_else(|| {
                (
                    script::format_setting_value(&value),
                    current
                        .as_ref()
                        .map(script::format_setting_value)
                        .unwrap_or_default(),
                )
            });
        {
            let edit = self
                .parameter_edits
                .entry(key.clone())
                .or_insert_with(|| ParameterEdit::new(&baseline_text, current.clone()));
            if edit.text != raw {
                edit.text.clone_from(&raw);
                edit.validated_text.clone_from(&raw);
            }
            edit.last_attempted = Some(value.clone());
            edit.pending = None;
            edit.error = None;
            edit.dirty = true;
            edit.discard_after_pending = false;
        }

        let save = match (key.profile.as_deref(), selection) {
            (Some(profile), script::ScriptSel::Compiled(id)) => self
                .set_compiled_setting(core, profile, *id, field, value.clone())
                .map(Some),
            (Some(profile), script::ScriptSel::Loaded(source, lookup)) => {
                let card = self
                    .js
                    .get(*source, lookup)
                    .cloned()
                    .ok_or_else(|| "parameters unavailable".to_string());
                card.and_then(|card| {
                    self.set_profile_setting(
                        core,
                        profile,
                        *source,
                        &card.name,
                        &card.path,
                        field,
                        value.clone(),
                    )
                    .map(Some)
                })
            }
            (None, script::ScriptSel::Loaded(source, lookup)) => {
                let Some(card) = self.js.get(*source, lookup).cloned() else {
                    return self.parameter_save_failed(key, "parameters unavailable");
                };
                self.legacy
                    .set_value(*source, &card.name, field, value.clone());
                self.legacy.save().map(|()| None)
            }
            (None, script::ScriptSel::Compiled(_)) => {
                return self.parameter_save_failed(key, "parameters: select a profile first");
            }
        };
        match save {
            Ok(Some(op)) => {
                if let Some(edit) = self.parameter_edits.get_mut(key) {
                    edit.pending = Some((op, value, raw));
                }
                Ok(ParameterCommit::Submitted(op))
            }
            Ok(None) => {
                self.parameter_edits.remove(key);
                Ok(ParameterCommit::Saved)
            }
            Err(error) => self.parameter_save_failed(key, &error),
        }
    }

    fn parameter_save_failed<T>(
        &mut self,
        key: &ParameterEditKey,
        error: &str,
    ) -> Result<T, String> {
        if let Some(edit) = self.parameter_edits.get_mut(key) {
            edit.error = Some(error.to_owned());
            edit.dirty = true;
        }
        Err(error.to_owned())
    }

    /// A text buffer was cancelled. An already accepted write keeps Start
    /// blocked until it settles, then the deliberate discard is honored.
    pub fn discard_parameter_text(&mut self, key: &ParameterEditKey) {
        if let Some(edit) = self.parameter_edits.get_mut(key) {
            if edit.pending.is_some() {
                edit.discard_after_pending = true;
            } else {
                self.parameter_edits.remove(key);
            }
        }
    }

    pub(crate) fn parameter_edit_blocks_start(&self, profile: &str) -> bool {
        self.parameter_edits.iter().any(|(key, edit)| {
            key.profile.as_deref() == Some(profile) && (edit.dirty || edit.pending.is_some())
        })
    }

    pub(crate) fn settle_parameter_edits<Io>(&mut self, core: &OperatorSession<Io>) {
        let mut remove = Vec::new();
        for (key, edit) in &mut self.parameter_edits {
            let Some((op, value, submitted_text)) = edit.pending.as_ref() else {
                continue;
            };
            let Some(outcome) = core
                .operation(*op)
                .and_then(|report| report.outcome(key.profile.as_deref()?))
                .cloned()
            else {
                continue;
            };
            if matches!(outcome, Outcome::Pending) {
                continue;
            }
            let value = value.clone();
            let submitted_text = submitted_text.clone();
            edit.pending = None;
            match outcome {
                Outcome::Completed => {
                    edit.baseline_value = Some(value);
                    edit.baseline_text = submitted_text.clone();
                    edit.error = None;
                    if edit.discard_after_pending || edit.text == submitted_text {
                        remove.push(key.clone());
                    } else {
                        edit.dirty = edit.text != edit.baseline_text;
                    }
                }
                Outcome::Failed(error) | Outcome::Skipped(error) => {
                    edit.error = Some(error);
                    edit.dirty = edit.text != edit.baseline_text;
                    if edit.discard_after_pending {
                        remove.push(key.clone());
                    }
                }
                Outcome::Cancelled => {
                    edit.error = Some("settings save was cancelled".into());
                    edit.dirty = edit.text != edit.baseline_text;
                    if edit.discard_after_pending {
                        remove.push(key.clone());
                    }
                }
                Outcome::Pending => unreachable!(),
            }
            edit.discard_after_pending = false;
        }
        for key in remove {
            self.parameter_edits.remove(&key);
        }
    }
}

fn validate_edit(
    edit: &mut ParameterEdit,
    def: &script::SettingDef,
    options: &[String],
) -> Result<Value, String> {
    if edit.validated_text != edit.text {
        edit.validated_text.clone_from(&edit.text);
        edit.error = None;
    }
    let result = parse_parameter_text(def, &edit.text, options);
    match &result {
        Ok(value) => {
            edit.dirty = edit.pending.is_some() || edit.baseline_value.as_ref() != Some(value);
            if edit
                .last_attempted
                .as_ref()
                .is_some_and(|previous| previous != value)
            {
                edit.error = None;
            }
        }
        Err(error) => {
            edit.error = Some(error.clone());
            edit.dirty = edit.pending.is_some() || edit.text != edit.baseline_text;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::parse_parameter_text;
    use script::SettingDef;

    fn setting(ty: &str) -> SettingDef {
        SettingDef {
            id: "value".into(),
            ty: ty.into(),
            default: None,
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
    fn validates_number_tiles_and_dynamic_list_options() {
        let mut number = setting("number");
        number.min = Some("2".into());
        number.max = Some("64".into());
        assert_eq!(
            parse_parameter_text(&number, "12.5", &[]),
            Ok(serde_json::json!(12.5))
        );
        assert!(parse_parameter_text(&number, "65", &[]).is_err());
        assert!(parse_parameter_text(&number, "-", &[]).is_err());

        assert_eq!(
            parse_parameter_text(&setting("tile"), "3200,3201,2", &[]),
            Ok(serde_json::json!({"x":3200,"z":3201,"level":2}))
        );
        let resources = vec!["mithril".to_string(), "copper".to_string()];
        assert_eq!(
            parse_parameter_text(&setting("list"), "mithril", &resources),
            Ok(serde_json::json!(["mithril"]))
        );
        assert!(parse_parameter_text(&setting("list"), "coppe", &resources).is_err());
    }
}

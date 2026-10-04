use super::Scripts;
use crate::operations::{OperationId, Outcome};
use crate::session::OperatorSession;
use serde_json::Value;

/// Options shared by the panel and TUI, including read-time aliases and
/// preserved values that are no longer present in the selected facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParameterOptions {
    pub values: Vec<String>,
    pub labels: Vec<String>,
    /// Number of trailing display-only values that are not picker choices.
    pub preserved: usize,
    aliases: Vec<(String, String)>,
    case_insensitive: bool,
}

impl ParameterOptions {
    fn canonical_option<'a>(&'a self, value: &'a str) -> &'a str {
        if self.values.iter().any(|option| option == value) {
            return value;
        }
        if self.case_insensitive {
            if let Some(option) = self
                .values
                .iter()
                .find(|option| option.eq_ignore_ascii_case(value))
            {
                return option;
            }
        }
        self.aliases
            .iter()
            .find(|(alias, _)| alias.eq_ignore_ascii_case(value))
            .map(|(_, canonical)| canonical.as_str())
            .unwrap_or(value)
    }

    pub fn matches_option(&self, value: &str, option: &str) -> bool {
        self.canonical_option(value) == option
    }

    pub fn value_for(&self, value: &str) -> String {
        self.canonical_option(value).to_owned()
    }

    pub fn label_for<'a>(&'a self, value: &'a str) -> &'a str {
        let canonical = self.canonical_option(value);
        self.values
            .iter()
            .position(|option| option == canonical)
            .and_then(|index| self.labels.get(index))
            .map(String::as_str)
            .unwrap_or(value)
    }
    /// Values eligible for scalar picker controls. Preserved entries remain
    /// in `values` for display and array removal, but never appear here.
    pub fn selectable(&self) -> &[String] {
        &self.values[..self.values.len().saturating_sub(self.preserved)]
    }

    /// Match one option's label or value with an ASCII case-insensitive
    /// substring query. This is allocation-free for use while drawing lists.
    pub fn matches_query(&self, index: usize, query: &str) -> bool {
        let Some(value) = self.values.get(index) else {
            return false;
        };
        let label = self
            .labels
            .get(index)
            .map_or(value.as_str(), String::as_str);
        ascii_contains_case_insensitive(value, query)
            || ascii_contains_case_insensitive(label, query)
    }

    /// Return the next selectable value, wrapping and skipping refused rows.
    pub fn next_selectable(&self, value: &str) -> Option<&str> {
        let values = self.selectable();
        if values.is_empty() {
            return None;
        }
        let current = values
            .iter()
            .position(|option| option.as_str() == self.canonical_option(value));
        let index = current.map_or(0, |current| (current + 1) % values.len());
        Some(values[index].as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.selectable().is_empty()
    }
    pub fn normalize_value(&self, value: &Value) -> Value {
        match value {
            Value::String(value) => Value::String(self.value_for(value)),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| match value.as_str() {
                        Some(value) => Value::String(self.value_for(value)),
                        None => value.clone(),
                    })
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
}

fn ascii_contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn move_unselectable_to_preserved(
    options: &mut ParameterOptions,
    mut is_selectable: impl FnMut(&str) -> bool,
) {
    let mut end = options.selectable().len();
    let initial = end;
    let mut index = 0;
    while index < end {
        if is_selectable(&options.values[index]) {
            index += 1;
        } else {
            let value = options.values.remove(index);
            let label = options.labels.remove(index);
            options.values.push(value);
            options.labels.push(label);
            end -= 1;
        }
    }
    options.preserved += initial - end;
}

/// Resolve a schema row once for both front ends. The renderers only choose
/// among these rows; aliases normalize only after an explicit user pick.
pub fn resolve_parameter_options(
    def: &script::SettingDef,
    bag: &serde_json::Map<String, Value>,
    loadouts: &script::LoadoutsStore,
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ParameterOptions {
    let base = script::resolve_setting_options_with_labels(def, loadouts, game_data);
    let mut options = ParameterOptions {
        values: base.values,
        labels: base.labels,
        ..ParameterOptions::default()
    };
    if options.labels.len() != options.values.len() {
        options.labels.clone_from(&options.values);
    }

    let source = def.options_from.as_deref().unwrap_or_default();
    options.case_insensitive = source == "gatherer-food" || source.starts_with("gather:");
    if source == "gather:sites" {
        if let Some(data) = game_data {
            resolve_gather_site_options(def, bag, data, &mut options);
        } else {
            preserve_unknown_values(def, bag, &mut options);
        }
        return options;
    }

    if let (Some(skill), Some(data)) = (source.strip_prefix("gather:"), game_data) {
        move_unselectable_to_preserved(&mut options, |value| {
            data.gather_option(skill, value)
                .is_none_or(|row| row.selectable)
        });
        for row in data.gather_resources_for(skill) {
            for alias in &row.aliases {
                if !alias.eq_ignore_ascii_case(&row.key) {
                    options.aliases.push((alias.clone(), row.key.clone()));
                }
            }
        }
    }

    if source == "released-path-order" {
        let candidate_values = options.values.clone();
        let candidate_labels = options.labels.clone();
        let quests = bag
            .get("quests")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let ids = if quests.is_empty() {
            candidate_values.clone()
        } else {
            quests
                .into_iter()
                .filter(|id| {
                    candidate_values
                        .iter()
                        .any(|candidate| candidate.as_str() == *id)
                })
                .map(str::to_owned)
                .collect()
        };
        let priority = setting_values(def, bag);
        options.values.clear();
        options.labels.clear();
        options.preserved = 0;
        for id in &ids {
            if options.values.iter().any(|existing| existing == id) {
                continue;
            }
            let Some(index) = candidate_values
                .iter()
                .position(|candidate| candidate == id)
            else {
                continue;
            };
            let label = candidate_labels
                .get(index)
                .map_or(id.as_str(), String::as_str);
            let label = priority
                .iter()
                .position(|selected| selected == id)
                .map_or_else(
                    || label.to_owned(),
                    |number| format!("{}. {label}", number + 1),
                );
            options.values.push(id.clone());
            options.labels.push(label);
        }
        options.aliases.clear();
        preserve_unknown_values(def, bag, &mut options);
        return options;
    }

    if source.starts_with("gather:") || source == "gatherer-food" || source == "released-paths" {
        preserve_unknown_values(def, bag, &mut options);
    }
    options
}

fn resolve_gather_site_options(
    def: &script::SettingDef,
    bag: &serde_json::Map<String, Value>,
    data: &api::game_data::SelectedGameData,
    options: &mut ParameterOptions,
) {
    let schema = script::gatherer::settings::schema();
    let skill = bag
        .get("skill")
        .and_then(Value::as_str)
        .or_else(|| {
            schema
                .iter()
                .find(|field| field.id == "skill")
                .and_then(|field| field.default.as_deref())
        })
        .unwrap_or("Woodcutting");
    let (resource_skill, resource_field) = if skill.eq_ignore_ascii_case("woodcutting") {
        (Some("woodcutting"), Some("woodcuttingResources"))
    } else if skill.eq_ignore_ascii_case("mining") {
        (Some("mining"), Some("miningResources"))
    } else if skill.eq_ignore_ascii_case("fishing") {
        (Some("fishing"), Some("fishingMethod"))
    } else {
        (None, None)
    };

    let mut selected_keys = Vec::new();
    if let (Some(resource_skill), Some(resource_field)) = (resource_skill, resource_field) {
        if let Some(resource_def) = schema.iter().find(|field| field.id == resource_field) {
            for value in setting_values(resource_def, bag) {
                if let Some(option) = data.gather_option(resource_skill, &value) {
                    if option.selectable && !selected_keys.contains(&option.key.as_str()) {
                        selected_keys.push(option.key.as_str());
                    }
                }
            }
        }
        for site in data.gather_sites_for(resource_skill) {
            if site
                .keys
                .iter()
                .any(|key| selected_keys.contains(&key.key.as_str()))
            {
                options.values.push(site.id.clone());
                options.labels.push(site.label.clone());
            }
        }
    }

    for value in setting_values(def, bag) {
        if value.is_empty()
            || options
                .selectable()
                .iter()
                .any(|candidate| options.matches_option(&value, candidate))
        {
            continue;
        }
        let label = if let Some(site) =
            resource_skill.and_then(|resource_skill| data.gather_site(resource_skill, &value))
        {
            format!("{} — not for the selected resources", site.label)
        } else if let Some(site) = data
            .gather_sites()
            .iter()
            .find(|site| site.id.eq_ignore_ascii_case(&value))
        {
            format!("{} — not a {skill} site", site.label)
        } else {
            format!("Unknown: {value}")
        };
        options.values.push(value);
        options.labels.push(label);
        options.preserved += 1;
    }
}

fn preserve_unknown_values(
    def: &script::SettingDef,
    bag: &serde_json::Map<String, Value>,
    options: &mut ParameterOptions,
) {
    for value in setting_values(def, bag) {
        if value.is_empty() {
            continue;
        }
        let resolved = options.canonical_option(&value);
        if !options.values.iter().any(|option| option == resolved) {
            options.values.push(value.clone());
            options.labels.push(format!("Unknown: {value}"));
            options.preserved += 1;
        }
    }
}

fn setting_values(def: &script::SettingDef, bag: &serde_json::Map<String, Value>) -> Vec<String> {
    let value = bag.get(&def.id).cloned().or_else(|| {
        def.default.as_deref().map(|default| {
            serde_json::from_str(default).unwrap_or_else(|_| Value::String(default.to_owned()))
        })
    });
    let Some(value) = value else {
        return Vec::new();
    };
    match def.ty.as_str() {
        "string" => value
            .as_str()
            .map(|value| vec![value.to_owned()])
            .unwrap_or_default(),
        "string[]" => {
            let value = script::coerce_setting_value("string[]", &value);
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        }
        _ => Vec::new(),
    }
}

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
    use super::{parse_parameter_text, resolve_parameter_options};
    use script::{LoadoutsStore, SettingDef};

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

    fn source_setting(id: &str, ty: &str, source: &str) -> SettingDef {
        let mut def = setting(ty);
        def.id = id.into();
        def.options_from = Some(source.into());
        def
    }

    #[test]
    fn gathers_preserve_unknowns_and_resolve_aliases_food_and_quest_order() {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R274).unwrap();
        let loadouts = LoadoutsStore::at(
            std::env::temp_dir().join(format!("parameter-options-{}.json", std::process::id())),
        );

        let resources = source_setting("woodcuttingResources", "string[]", "gather:woodcutting");
        let mut bag = serde_json::Map::new();
        bag.insert(
            resources.id.clone(),
            serde_json::json!(["removed-resource", "Normal"]),
        );
        let options = resolve_parameter_options(&resources, &bag, &loadouts, Some(data.as_ref()));
        assert_eq!(
            options.preserved, 2,
            "Jungle is refused and the saved resource is unknown"
        );
        assert!(options.values.iter().any(|value| value == "jungle"));
        assert!(!options.selectable().iter().any(|value| value == "jungle"));
        assert_eq!(
            options.values.len(),
            options.selectable().len() + options.preserved
        );
        assert_eq!(options.values.len(), options.labels.len());
        let removed = options
            .values
            .iter()
            .position(|value| value == "removed-resource")
            .unwrap();
        assert!(!options
            .selectable()
            .iter()
            .any(|value| value == "removed-resource"));
        assert!(options.matches_query(removed, "REMOVED-RESOURCE"));
        assert!(options.values.contains(&"removed-resource".to_string()));
        assert_eq!(
            options.label_for("removed-resource"),
            "Unknown: removed-resource"
        );
        assert!(options.matches_option("Normal", "normal"));
        assert!(options.matches_option("removed-resource", "removed-resource"));
        assert_eq!(
            options.normalize_value(&bag[&resources.id]),
            serde_json::json!(["removed-resource", "normal"]),
            "an unknown checked value survives while known values resolve case-insensitively"
        );

        let fishing = source_setting("fishingMethod", "string", "gather:fishing");
        bag.insert(
            fishing.id.clone(),
            serde_json::json!("fishing.saltfish.op1"),
        );
        let fishing_options =
            resolve_parameter_options(&fishing, &bag, &loadouts, Some(data.as_ref()));
        let group = data
            .gather_option("fishing", "fishing.saltfish.op1")
            .expect("legacy fishing id resolves");
        assert_eq!(fishing_options.value_for("fishing.saltfish.op1"), group.key);
        assert_eq!(
            fishing_options.label_for("fishing.saltfish.op1"),
            group.label
        );
        assert_eq!(
            bag[&fishing.id], "fishing.saltfish.op1",
            "resolving an alias does not mutate the setting bag"
        );

        let food = source_setting("food", "string", "gatherer-food");
        bag.insert(food.id.clone(), serde_json::json!("lobster"));
        let food_options = resolve_parameter_options(&food, &bag, &loadouts, Some(data.as_ref()));
        assert_eq!(food_options.values.first().map(String::as_str), Some(""));
        assert_eq!(
            food_options.labels.first().map(String::as_str),
            Some("None")
        );
        assert_eq!(food_options.value_for("lobster"), "Lobster");
        assert_eq!(food_options.label_for("LOBSTER"), "Lobster");

        let mut order = source_setting("order_override", "string[]", "released-path-order");
        order.options = vec!["cook".into(), "sheep".into()];
        order.option_labels = vec!["Cook display".into(), "Sheep display".into()];
        bag.insert("quests".into(), serde_json::json!(["sheep", "cook"]));
        bag.insert(
            "order_override".into(),
            serde_json::json!(["cook", "sheep"]),
        );
        let order_options = resolve_parameter_options(&order, &bag, &loadouts, None);
        assert_eq!(order_options.values, ["sheep", "cook"]);
        assert_eq!(
            order_options.labels,
            ["2. Sheep display", "1. Cook display"]
        );

        let mut paths = source_setting("quests", "string[]", "released-paths");
        paths.options = vec!["cook".into(), "sheep".into()];
        paths.option_labels = vec!["Cook display".into(), "Sheep display".into()];
        bag.insert("quests".into(), serde_json::json!(["removed-path"]));
        let quest_options = resolve_parameter_options(&paths, &bag, &loadouts, None);
        assert!(quest_options.values.contains(&"removed-path".to_string()));
        assert_eq!(
            quest_options.label_for("removed-path"),
            "Unknown: removed-path"
        );
        assert_eq!(
            quest_options.normalize_value(&bag["quests"]),
            serde_json::json!(["removed-path"]),
            "unknown saved Path ids remain unchanged until explicitly removed"
        );

        let skip = source_setting("skip", "string[]", "released-paths");
        bag.insert("skip".into(), serde_json::json!(["removed-path"]));
        let skip_options = resolve_parameter_options(&skip, &bag, &loadouts, None);
        assert_eq!(
            skip_options.label_for("removed-path"),
            "Unknown: removed-path"
        );

        let mut order = source_setting("order_override", "string[]", "released-path-order");
        order.options = vec!["cook".into(), "sheep".into()];
        order.option_labels = vec!["Cook display".into(), "Sheep display".into()];
        bag.insert("quests".into(), serde_json::json!(["sheep", "cook"]));
        bag.insert("order_override".into(), serde_json::json!(["removed-path"]));
        let order_options = resolve_parameter_options(&order, &bag, &loadouts, None);
        assert_eq!(
            order_options.label_for("removed-path"),
            "Unknown: removed-path"
        );

        let unavailable =
            resolve_parameter_options(&resources, &serde_json::Map::new(), &loadouts, None);
        assert!(
            unavailable.is_empty(),
            "unresolved list sources do not become text fields"
        );
        let mut radius = setting("number");
        radius.id = "radius".into();
        radius.min = Some("2".into());
        radius.max = Some("64".into());
        assert_eq!(
            parse_parameter_text(&radius, "20", &[]),
            Ok(serde_json::json!(20))
        );
    }

    #[test]
    fn gather_sites_use_selected_resources_defaults_and_fishing_aliases() {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let loadouts = LoadoutsStore::at(
            std::env::temp_dir().join(format!("site-options-{}.json", std::process::id())),
        );
        let site = script::gatherer::settings::schema()
            .iter()
            .find(|field| field.id == "site")
            .expect("the Gatherer Site field uses shared options");
        let mut bag = serde_json::Map::new();
        let bagless = resolve_parameter_options(site, &bag, &loadouts, None);
        assert!(
            bagless.is_empty(),
            "script options remain empty without facts"
        );

        let woodcutting = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert!(
            woodcutting
                .selectable()
                .iter()
                .any(|id| id == "woodcutting.draynor"),
            "schema defaults select the normal-tree Draynor site"
        );

        bag.insert("skill".into(), serde_json::json!("Mining"));
        let mining = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert!(
            mining
                .selectable()
                .iter()
                .any(|id| id == "mining.varrock_east.se"),
            "the copper/tin schema defaults admit Varrock East"
        );
        assert!(mining
            .selectable()
            .iter()
            .all(|id| data.gather_site("mining", id).is_some()));

        bag.insert("skill".into(), serde_json::json!("Fishing"));
        bag.insert(
            "fishingMethod".into(),
            serde_json::json!("fishing.loc_2027.op1"),
        );
        let alias = data
            .gather_option("fishing", "fishing.loc_2027.op1")
            .expect("legacy dispatch id is a fishing alias");
        let expected = data
            .gather_sites_for("fishing")
            .filter(|site| site.keys.iter().any(|key| key.key == alias.key))
            .map(|site| site.id.clone())
            .collect::<Vec<_>>();
        let fishing = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert_eq!(fishing.selectable(), expected.as_slice());
        assert!(
            bag.get("fishingMethod")
                .is_some_and(|value| value == "fishing.loc_2027.op1"),
            "resolving the alias does not rewrite the saved method"
        );
    }

    #[test]
    fn gather_site_invalid_current_is_display_only_and_has_a_reason() {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let loadouts = LoadoutsStore::at(
            std::env::temp_dir().join(format!("site-invalid-{}.json", std::process::id())),
        );
        let site = script::gatherer::settings::schema()
            .iter()
            .find(|field| field.id == "site")
            .expect("the Gatherer Site field uses shared options");
        let mut bag = serde_json::Map::new();
        bag.insert("skill".into(), serde_json::json!("Mining"));
        bag.insert("miningResources".into(), serde_json::json!(["rune stones"]));
        bag.insert("site".into(), serde_json::json!("mining.varrock_east.se"));

        let options = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert!(options.is_empty(), "no surface site offers rune stones");
        assert_eq!(options.preserved, 1);
        assert_eq!(
            options.values.len(),
            options.selectable().len() + options.preserved
        );
        assert_eq!(options.values.len(), options.labels.len());
        assert!(options.selectable().is_empty());
        assert!(options
            .label_for("mining.varrock_east.se")
            .ends_with("not for the selected resources"));

        bag.insert("miningResources".into(), serde_json::json!(["copper"]));
        bag.insert(
            "site".into(),
            serde_json::json!("mining.barbarian_village.e"),
        );
        let choices = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert!(!choices.is_empty());
        assert!(choices
            .selectable()
            .iter()
            .all(|id| id != "mining.barbarian_village.e"));
        assert!(
            parse_parameter_text(site, "mining.barbarian_village.e", choices.selectable()).is_err()
        );

        bag.insert("miningResources".into(), serde_json::json!(["rune stones"]));
        bag.insert("site".into(), serde_json::json!("fishing.catherby"));
        let wrong_skill = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert!(wrong_skill
            .label_for("fishing.catherby")
            .ends_with("not a Mining site"));

        bag.insert("site".into(), serde_json::json!("removed-site"));
        let unknown = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert_eq!(unknown.label_for("removed-site"), "Unknown: removed-site");
        assert!(unknown.is_empty());
    }

    #[test]
    fn gather_sites_exclude_unselectable_resource_keys() {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let refused = data
            .gather_resources_for("fishing")
            .find(|resource| {
                !resource.selectable
                    && data.gather_sites_for("fishing").any(|site| {
                        site.keys
                            .iter()
                            .any(|site_key| site_key.key == resource.key)
                    })
            })
            .expect("a refused fishing key is present at a named site");
        let refused_site_ids = data
            .gather_sites_for("fishing")
            .filter(|site| site.keys.iter().any(|site_key| site_key.key == refused.key))
            .map(|site| site.id.clone())
            .collect::<Vec<_>>();
        assert!(!refused_site_ids.is_empty());

        let loadouts = LoadoutsStore::at(
            std::env::temp_dir().join(format!("site-unselectable-{}.json", std::process::id())),
        );
        let site = script::gatherer::settings::schema()
            .iter()
            .find(|field| field.id == "site")
            .expect("the Gatherer Site field uses shared options");
        let mut bag = serde_json::Map::new();
        bag.insert("skill".into(), serde_json::json!("Fishing"));
        bag.insert("fishingMethod".into(), serde_json::json!(refused.key));

        let options = resolve_parameter_options(site, &bag, &loadouts, Some(data.as_ref()));
        assert!(
            refused_site_ids
                .iter()
                .all(|id| !options.selectable().contains(id)),
            "sites that only offer a refused resource must not be selectable"
        );
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

    #[test]
    fn fishing_scalar_cycle_visits_every_selectable_group_and_wraps() {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let loadouts = LoadoutsStore::at(
            std::env::temp_dir().join(format!("fishing-cycle-{}.json", std::process::id())),
        );
        let fishing = source_setting("fishingMethod", "string", "gather:fishing");
        let options = resolve_parameter_options(
            &fishing,
            &serde_json::Map::new(),
            &loadouts,
            Some(data.as_ref()),
        );
        let selectable = data
            .gather_resources_for("fishing")
            .filter(|row| row.selectable)
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>();
        let refused = data
            .gather_resources_for("fishing")
            .filter(|row| !row.selectable)
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>();
        assert!(!selectable.is_empty());
        assert!(!refused.is_empty());

        let start = options
            .values
            .iter()
            .find(|value| value.as_str() == selectable[0])
            .map(String::as_str)
            .unwrap();
        let mut current = start;
        let mut cycle = Vec::with_capacity(selectable.len() + 1);
        cycle.push(start);
        for _ in 0..selectable.len() {
            current = options
                .next_selectable(current)
                .expect("at least one fishing group is selectable");
            cycle.push(current);
        }

        assert_eq!(current, start, "the cycle wraps to its starting group");
        let cycle = &cycle[1..];
        assert_eq!(
            cycle.iter().collect::<std::collections::HashSet<_>>().len(),
            selectable.len(),
            "every selectable group appears exactly once"
        );
        assert!(selectable.iter().all(|value| cycle.contains(value)));
        assert!(refused.iter().all(|value| !cycle.contains(value)));
    }
}

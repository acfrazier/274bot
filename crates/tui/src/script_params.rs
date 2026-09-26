//! Script parameter editors for the TUI script pane (schema-driven).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use script::{
    coerce_setting_value, format_setting_value, resolve_setting_options_with_labels,
    setting_visible, LoadoutsStore, ResolvedSettingOptions, SettingDef,
};

/// Mutable params-pane state: form open, cursor, and in-progress edit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamsState {
    pub open: bool,
    pub cursor: usize,
    pub scroll: usize,
    pub editing: bool,
    pub multi_select: bool,
    pub scratch: String,
    pub choice_cursor: usize,
    pub choice_selected: Vec<String>,
    pub error: Option<String>,
    /// Apply-to-all confirmation line (scope and the y/n keys) while one
    /// is prepared.
    pub sync_prompt: Option<String>,
    /// The last Apply-to-all result, shown until the next key.
    pub report: Option<String>,
}

/// Outcome of a params key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamsKey {
    Toggle,
    Saved,
    Edit,
    Cancel,
    Up,
    Down,
    Close,
    /// `a`: prepare Apply to all (copy these parameters to same-card
    /// members).
    SyncPrepare,
    /// `y`/Enter on the Apply-to-all confirmation.
    SyncApply,
    /// `n`/Esc on the Apply-to-all confirmation.
    SyncCancel,
    None,
}

/// Where a parameter edit goes: the focused profile's bag through the
/// shared script coordinator. `Err` keeps the old value and shows it.
pub type ParamsCommit<'a> = dyn FnMut(&str, serde_json::Value) -> Result<(), String> + 'a;

/// The script params popup over a card's settings schema. `bag` is the
/// view of the focused profile's merged bag; edits go through `commit`.
pub struct ParamsPane<'a> {
    pub schema: &'a [SettingDef],
    pub bag: &'a mut serde_json::Map<String, serde_json::Value>,
    pub commit: &'a mut ParamsCommit<'a>,
    pub loadouts: &'a LoadoutsStore,
    pub game_data: Option<&'a api::game_data::SelectedGameData>,
    pub state: &'a mut ParamsState,
}

impl<'a> ParamsPane<'a> {
    pub fn visible_rows(&self) -> Vec<&'a SettingDef> {
        self.schema
            .iter()
            .filter(|d| setting_visible(d.show_if.as_deref(), self.bag))
            .collect()
    }

    fn resolved_options(&self, def: &SettingDef) -> ResolvedSettingOptions {
        resolve_setting_options_with_labels(def, self.loadouts, self.game_data)
    }

    /// Centered overlay; leaves the surrounding map/status cells alone.
    pub fn popup_rect(area: Rect) -> Rect {
        let w = area.width.saturating_sub(2).clamp(24, 72).min(area.width);
        let h = area.height.saturating_sub(2).clamp(8, 22).min(area.height);
        Rect {
            x: area.x + area.width.saturating_sub(w) / 2,
            y: area.y + area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }

    pub fn on_key(&mut self, code: crossterm::event::KeyCode) -> ParamsKey {
        self.state.report = None;
        self.clamp_cursor();
        let rows = self.visible_rows();
        if rows.is_empty() {
            return ParamsKey::Close;
        }
        if self.state.editing {
            return self.on_edit_key(code, &rows);
        }
        if self.state.sync_prompt.is_some() {
            return match code {
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char('y') => {
                    ParamsKey::SyncApply
                }
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('n') => {
                    self.state.sync_prompt = None;
                    ParamsKey::SyncCancel
                }
                _ => ParamsKey::None,
            };
        }
        match code {
            crossterm::event::KeyCode::Esc => ParamsKey::Close,
            crossterm::event::KeyCode::Char('a') => ParamsKey::SyncPrepare,
            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                if self.state.cursor > 0 {
                    self.state.cursor -= 1;
                }
                ParamsKey::Up
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                if self.state.cursor + 1 < rows.len() {
                    self.state.cursor += 1;
                }
                ParamsKey::Down
            }
            crossterm::event::KeyCode::PageUp => {
                self.state.cursor = self.state.cursor.saturating_sub(8);
                ParamsKey::Up
            }
            crossterm::event::KeyCode::PageDown => {
                self.state.cursor = (self.state.cursor + 8).min(rows.len().saturating_sub(1));
                ParamsKey::Down
            }
            crossterm::event::KeyCode::Enter => self.activate(&rows, true),
            crossterm::event::KeyCode::Char(' ') => self.activate(&rows, false),
            _ => ParamsKey::None,
        }
    }

    fn clamp_cursor(&mut self) {
        let n = self
            .schema
            .iter()
            .filter(|d| setting_visible(d.show_if.as_deref(), self.bag))
            .count();
        if n == 0 {
            self.state.cursor = 0;
        } else if self.state.cursor >= n {
            self.state.cursor = n - 1;
        }
    }

    fn on_edit_key(&mut self, code: crossterm::event::KeyCode, rows: &[&SettingDef]) -> ParamsKey {
        if self.state.multi_select {
            return self.on_choice_key(code, rows);
        }
        match code {
            crossterm::event::KeyCode::Esc => {
                self.cancel_edit();
                ParamsKey::Cancel
            }
            crossterm::event::KeyCode::Enter => self.commit_text(rows),
            crossterm::event::KeyCode::Backspace => {
                self.state.scratch.pop();
                self.state.error = None;
                ParamsKey::None
            }
            crossterm::event::KeyCode::Char(c) => {
                self.state.scratch.push(c);
                self.state.error = None;
                ParamsKey::None
            }
            _ => ParamsKey::None,
        }
    }

    fn on_choice_key(
        &mut self,
        code: crossterm::event::KeyCode,
        rows: &[&SettingDef],
    ) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor) else {
            return ParamsKey::None;
        };
        let opts = self.resolved_options(def);
        if opts.is_empty() {
            self.cancel_edit();
            return ParamsKey::Cancel;
        }
        match code {
            crossterm::event::KeyCode::Esc => {
                self.cancel_edit();
                ParamsKey::Cancel
            }
            crossterm::event::KeyCode::Enter => self.commit_choices(def),
            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                if self.state.choice_cursor > 0 {
                    self.state.choice_cursor -= 1;
                }
                ParamsKey::Up
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                if self.state.choice_cursor + 1 < opts.values.len() {
                    self.state.choice_cursor += 1;
                }
                ParamsKey::Down
            }
            crossterm::event::KeyCode::Char(' ') => {
                let Some(opt) = opts.values.get(self.state.choice_cursor) else {
                    return ParamsKey::None;
                };
                if let Some(i) = self.state.choice_selected.iter().position(|s| s == opt) {
                    self.state.choice_selected.remove(i);
                } else {
                    self.state.choice_selected.push(opt.clone());
                }
                ParamsKey::Toggle
            }
            _ => ParamsKey::None,
        }
    }

    fn activate(&mut self, rows: &[&SettingDef], enter: bool) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor) else {
            return ParamsKey::None;
        };
        if def.ty == "boolean" {
            let cur = self
                .bag
                .get(&def.id)
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| def.default.as_deref() == Some("true"));
            return if self.persist(&def.id, serde_json::json!(!cur)) {
                ParamsKey::Toggle
            } else {
                ParamsKey::None
            };
        }
        let opts = self.resolved_options(def);
        if def.ty == "string" && !opts.is_empty() {
            let cur = self
                .bag
                .get(&def.id)
                .and_then(|v| v.as_str())
                .or(def.default.as_deref())
                .unwrap_or("")
                .to_string();
            let next = opts
                .values
                .iter()
                .position(|o| o == &cur)
                .map(|i| opts.values[(i + 1) % opts.values.len()].clone())
                .unwrap_or_else(|| opts.values[0].clone());
            return if self.persist(&def.id, serde_json::json!(next)) {
                ParamsKey::Toggle
            } else {
                ParamsKey::None
            };
        }
        if !enter && def.ty != "string[]" {
            return ParamsKey::None;
        }
        if def.ty == "string[]" && !opts.is_empty() {
            self.state.editing = true;
            self.state.multi_select = true;
            self.state.choice_cursor = 0;
            self.state.choice_selected = current_string_list(self.bag.get(&def.id));
            self.state.scratch.clear();
            self.state.error = None;
            return ParamsKey::Edit;
        }
        if matches!(
            def.ty.as_str(),
            "number" | "string" | "tile" | "list" | "string[]"
        ) {
            self.state.editing = true;
            self.state.multi_select = false;
            self.state.scratch = self
                .bag
                .get(&def.id)
                .map(format_setting_value)
                .unwrap_or_else(|| def.default.clone().unwrap_or_default());
            self.state.choice_selected.clear();
            self.state.error = None;
            return ParamsKey::Edit;
        }
        ParamsKey::None
    }

    fn commit_text(&mut self, rows: &[&SettingDef]) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor) else {
            return ParamsKey::None;
        };
        match value_from_scratch(def, &self.state.scratch) {
            Ok(value) => {
                if self.persist(&def.id, value) {
                    self.cancel_edit();
                    ParamsKey::Saved
                } else {
                    ParamsKey::None
                }
            }
            Err(err) => {
                self.state.error = Some(err);
                ParamsKey::None
            }
        }
    }

    fn commit_choices(&mut self, def: &SettingDef) -> ParamsKey {
        let raw = serde_json::json!(self.state.choice_selected.clone());
        let value = coerce_setting_value(&def.ty, &raw);
        if !coerced_matches_type(&def.ty, &value) {
            self.state.error = Some("invalid value".into());
            return ParamsKey::None;
        }
        if self.persist(&def.id, value) {
            self.cancel_edit();
            ParamsKey::Saved
        } else {
            ParamsKey::None
        }
    }

    fn cancel_edit(&mut self) {
        self.state.editing = false;
        self.state.multi_select = false;
        self.state.scratch.clear();
        self.state.choice_selected.clear();
        self.state.choice_cursor = 0;
        self.state.error = None;
    }

    fn persist(&mut self, id: &str, value: serde_json::Value) -> bool {
        match (self.commit)(id, value.clone()) {
            Ok(()) => {
                self.bag.insert(id.to_string(), value);
                self.state.error = None;
                true
            }
            Err(err) => {
                self.state.error = Some(format!("Save failed: {err}"));
                false
            }
        }
    }

    fn hint(&self) -> String {
        if let Some(err) = self.state.error.as_deref() {
            return err.to_string();
        }
        if let Some(prompt) = self.state.sync_prompt.as_deref() {
            return prompt.to_string();
        }
        if let Some(report) = self.state.report.as_deref() {
            return report.to_string();
        }
        if self.state.editing {
            if self.state.multi_select {
                "space toggle · enter save · esc cancel".into()
            } else {
                "enter save · esc cancel".into()
            }
        } else {
            "enter edit · space toggle · a apply to same-card · esc close".into()
        }
    }
}

fn current_string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn value_from_scratch(def: &SettingDef, scratch: &str) -> Result<serde_json::Value, String> {
    match def.ty.as_str() {
        "number" => parse_number(scratch),
        "string" => Ok(serde_json::Value::String(scratch.to_string())),
        "tile" => {
            if !tile_scratch_ok(scratch) {
                return Err("invalid tile".into());
            }
            let coerced = coerce_setting_value(&def.ty, &serde_json::json!(scratch));
            if coerced_matches_type(&def.ty, &coerced) {
                Ok(coerced)
            } else {
                Err("invalid tile".into())
            }
        }
        "list" | "string[]" => {
            let coerced = coerce_setting_value(&def.ty, &serde_json::json!(scratch));
            if coerced_matches_type(&def.ty, &coerced) {
                Ok(coerced)
            } else {
                Err(format!("invalid {}", def.ty))
            }
        }
        other => Err(format!("unsupported type {other}")),
    }
}

fn tile_scratch_ok(scratch: &str) -> bool {
    let text = scratch.trim();
    if text.starts_with('{') {
        return serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .is_some_and(|v| coerced_matches_type("tile", &v));
    }
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|part| part.parse::<i64>().is_ok())
}

fn parse_number(scratch: &str) -> Result<serde_json::Value, String> {
    let text = scratch.trim();
    let n: f64 = text.parse().map_err(|_| "invalid number".to_string())?;
    if !n.is_finite() {
        return Err("invalid number".into());
    }
    serde_json::Number::from_f64(n)
        .map(serde_json::Value::Number)
        .ok_or_else(|| "invalid number".into())
}

fn coerced_matches_type(ty: &str, value: &serde_json::Value) -> bool {
    match ty {
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "tile" => value.as_object().is_some_and(|obj| {
            obj.get("x").and_then(|v| v.as_i64()).is_some()
                && obj.get("z").and_then(|v| v.as_i64()).is_some()
        }),
        "list" | "string[]" => value.is_array(),
        _ => false,
    }
}

fn display_value(bag: &serde_json::Map<String, serde_json::Value>, def: &SettingDef) -> String {
    bag.get(&def.id)
        .map(format_setting_value)
        .unwrap_or_else(|| "—".into())
}

impl Widget for ParamsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.open {
            return;
        }
        let rows = self.visible_rows();
        if rows.is_empty() {
            return;
        }
        self.state.cursor = self.state.cursor.min(rows.len().saturating_sub(1));
        let popup = ParamsPane::popup_rect(area);
        Clear.render(popup, buf);
        let block = Block::default().borders(Borders::ALL).title("parameters");
        let inner = block.inner(popup);
        block.render(popup, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let mut lines = Vec::new();
        let mut cursor_line = 0usize;
        if self.state.editing && self.state.multi_select {
            if let Some(def) = rows.get(self.state.cursor) {
                let opts = self.resolved_options(def);
                let label = def.label.as_deref().unwrap_or(&def.id);
                lines.push(Line::from(format!("{label} choices")));
                cursor_line = 1 + self
                    .state
                    .choice_cursor
                    .min(opts.values.len().saturating_sub(1));
                for (i, opt) in opts.values.iter().enumerate() {
                    let mark = if i == self.state.choice_cursor {
                        "> "
                    } else {
                        "  "
                    };
                    let tick = if self.state.choice_selected.iter().any(|s| s == opt) {
                        "[x]"
                    } else {
                        "[ ]"
                    };
                    let shown = opts.labels.get(i).map(String::as_str).unwrap_or(opt);
                    lines.push(Line::from(format!("{mark}{tick} {shown}")));
                }
            }
        } else {
            let mut last_group: Option<&str> = None;
            for (i, def) in rows.iter().enumerate() {
                if def.group.as_deref() != last_group {
                    last_group = def.group.as_deref();
                    if let Some(g) = last_group {
                        lines.push(Line::from(format!("— {g} —")));
                    }
                }
                if i == self.state.cursor {
                    cursor_line = lines.len();
                }
                let label = def.label.as_deref().unwrap_or(&def.id);
                let mark = if i == self.state.cursor { "> " } else { "  " };
                let value = if self.state.editing && i == self.state.cursor {
                    format!("{}_", self.state.scratch)
                } else {
                    display_value(self.bag, def)
                };
                lines.push(Line::from(format!("{mark}{label}: {value}")));
            }
        }

        let hint = self.hint();
        // A report or prompt wider than the popup wraps onto a second row.
        let reserve = if hint.chars().count() > inner.width as usize {
            2u16
        } else {
            1u16
        };
        let content_h = inner.height.saturating_sub(reserve) as usize;
        if cursor_line < self.state.scroll {
            self.state.scroll = cursor_line;
        }
        if content_h > 0 && cursor_line >= self.state.scroll + content_h {
            self.state.scroll = cursor_line + 1 - content_h;
        }
        let max_off = lines.len().saturating_sub(content_h.max(1));
        self.state.scroll = self.state.scroll.min(max_off);
        let window: Vec<Line> = if content_h == 0 {
            Vec::new()
        } else {
            lines
                .into_iter()
                .skip(self.state.scroll)
                .take(content_h)
                .collect()
        };
        let list = Rect {
            height: inner.height.saturating_sub(reserve),
            ..inner
        };
        Paragraph::new(window).render(list, buf);
        let hint_area = Rect {
            y: list.y + list.height,
            height: inner.height - list.height,
            ..inner
        };
        Paragraph::new(hint)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .render(hint_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crossterm::event::KeyCode;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;
    use script::{ScriptSettingsStore, ScriptSource, SettingDef};

    static TEST_DIRS: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "274bot-tui-params-{}-{}-{}",
            label,
            std::process::id(),
            TEST_DIRS.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setting(
        id: &str,
        ty: &str,
        default: Option<&str>,
        label: Option<&str>,
        options: &[&str],
    ) -> SettingDef {
        SettingDef {
            id: id.into(),
            ty: ty.into(),
            default: default.map(str::to_string),
            label: label.map(str::to_string),
            min: None,
            max: None,
            step: None,
            options: options.iter().map(|s| (*s).to_string()).collect(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        }
    }

    fn bury_schema() -> Vec<SettingDef> {
        vec![setting(
            "buryBones",
            "boolean",
            Some("true"),
            Some("Bury bones"),
            &[],
        )]
    }

    fn alcher_schema() -> Vec<SettingDef> {
        vec![
            setting("items", "string[]", None, Some("Items to alch"), &[]),
            setting("alchs", "number", Some("27"), Some("Alchs per trip"), &[]),
        ]
    }

    /// Commit sink over a legacy store, standing in for the coordinator.
    fn store_commit<'s>(
        store: &'s mut ScriptSettingsStore,
        name: &'static str,
    ) -> impl FnMut(&str, serde_json::Value) -> Result<(), String> + 's {
        move |id, value| {
            store.set_value(ScriptSource::Catalog, name, id, value);
            store.save()
        }
    }

    fn type_replace(pane: &mut ParamsPane<'_>, text: &str) {
        while !pane.state.scratch.is_empty() {
            pane.on_key(KeyCode::Backspace);
        }
        for c in text.chars() {
            pane.on_key(KeyCode::Char(c));
        }
    }

    #[test]
    fn params_pane_space_toggles_a_bool_param() {
        let dir = temp_dir("bool");
        let path = dir.join("script-settings.json");
        let mut store = ScriptSettingsStore::at(path);
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = bury_schema();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "ChickenKiller", &schema, None);
        assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(true)));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "ChickenKiller"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(false)));
    }

    #[test]
    fn params_pane_space_cycles_loadout_combo_from_store() {
        let dir = temp_dir("loadout");
        let mut loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        loadouts.upsert(script::Loadout::new("fish"));
        loadouts.upsert(script::Loadout::new("mine"));
        loadouts.save().unwrap();
        let schema = vec![SettingDef {
            options_from: Some("loadouts".into()),
            ..setting("loadout", "string", Some("fish"), Some("Loadout"), &[])
        }];
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Thiever", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Thiever"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(bag.get("loadout"), Some(&serde_json::json!("mine")));
    }

    #[test]
    fn alcher_numeric_enter_saves_and_esc_cancels() {
        let dir = temp_dir("alcher-num");
        let path = dir.join("script-settings.json");
        let mut store = ScriptSettingsStore::at(path.clone());
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = alcher_schema();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 1,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Alcher"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        type_replace(&mut pane, "40");
        assert_eq!(pane.on_key(KeyCode::Esc), ParamsKey::Cancel);
        assert_eq!(pane.bag.get("alchs").and_then(|v| v.as_f64()), Some(27.0));
        assert!(!pane.state.editing);

        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        type_replace(&mut pane, "40");
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(pane.bag.get("alchs").and_then(|v| v.as_f64()), Some(40.0));
        let reloaded = ScriptSettingsStore::at(path);
        let start = reloaded.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        assert_eq!(
            start.get("alchs").and_then(|v| v.as_f64()),
            Some(40.0),
            "Start merge must see the saved number"
        );
    }

    #[test]
    fn alcher_string_array_csv_persists_as_array() {
        let dir = temp_dir("alcher-items");
        let path = dir.join("script-settings.json");
        let mut store = ScriptSettingsStore::at(path.clone());
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = alcher_schema();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        assert!(bag.get("items").is_none());
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Alcher"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        for c in "Maple longbow, Yew longbow".chars() {
            pane.on_key(KeyCode::Char(c));
        }
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(
            pane.bag.get("items"),
            Some(&serde_json::json!(["Maple longbow", "Yew longbow"]))
        );
        let reloaded = ScriptSettingsStore::at(path);
        let start = reloaded.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        assert_eq!(
            start.get("items"),
            Some(&serde_json::json!(["Maple longbow", "Yew longbow"]))
        );
    }

    #[test]
    fn optioned_string_array_space_toggles_choice_and_cancel_keeps_old() {
        let dir = temp_dir("choices");
        let path = dir.join("script-settings.json");
        let mut store = ScriptSettingsStore::at(path);
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = vec![setting(
            "items",
            "string[]",
            Some("Maple longbow"),
            Some("Items to alch"),
            &["Maple longbow", "Yew longbow", "Custom"],
        )];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Alcher"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        pane.on_key(KeyCode::Down);
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(pane.on_key(KeyCode::Esc), ParamsKey::Cancel);
        assert_eq!(
            pane.bag.get("items"),
            Some(&serde_json::json!(["Maple longbow"])),
            "cancel must not persist the extra choice"
        );
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        pane.on_key(KeyCode::Down);
        pane.on_key(KeyCode::Char(' '));
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(
            pane.bag.get("items"),
            Some(&serde_json::json!(["Maple longbow", "Yew longbow"]))
        );
    }

    #[test]
    fn tile_and_list_coercion_rejects_invalid_without_replacing() {
        let dir = temp_dir("coerce");
        let path = dir.join("script-settings.json");
        let mut store = ScriptSettingsStore::at(path);
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = vec![
            setting("spot", "tile", Some("3200,3200,0"), Some("Spot"), &[]),
            setting("drops", "list", Some("bones,shells"), Some("Drops"), &[]),
        ];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", &schema, None);
        let before_tile = bag.get("spot").cloned();
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Gatherer"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        type_replace(&mut pane, "nope");
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
        assert_eq!(pane.bag.get("spot"), before_tile.as_ref());
        assert_eq!(pane.state.error.as_deref(), Some("invalid tile"));
        assert!(pane.state.editing, "invalid save stays in the editor");
        pane.on_key(KeyCode::Esc);
        pane.on_key(KeyCode::Down);
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        pane.on_key(KeyCode::Char(','));
        pane.on_key(KeyCode::Char('a'));
        pane.on_key(KeyCode::Char('s'));
        pane.on_key(KeyCode::Char('h'));
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(
            pane.bag.get("drops"),
            Some(&serde_json::json!(["bones", "shells", "ash"]))
        );
        pane.on_key(KeyCode::Up);
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        for _ in 0..20 {
            pane.on_key(KeyCode::Backspace);
        }
        for c in "3210, 3211, 2".chars() {
            pane.on_key(KeyCode::Char(c));
        }
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(
            pane.bag.get("spot"),
            Some(&serde_json::json!({"x": 3210, "z": 3211, "level": 2}))
        );
    }

    /// `a` asks for Apply to all; the confirmation takes `y`/Enter or
    /// `n`/Esc and nothing else, and a refused commit keeps the old value.
    #[test]
    fn apply_to_all_keys_and_a_refused_commit() {
        let dir = temp_dir("sync-keys");
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = alcher_schema();
        let mut bag = serde_json::Map::new();
        bag.insert("alchs".into(), serde_json::json!(27));
        let mut state = ParamsState {
            open: true,
            cursor: 1,
            ..Default::default()
        };
        let mut refuse = |_: &str, _: serde_json::Value| Err("vault locked".to_string());
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut refuse,
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char('a')), ParamsKey::SyncPrepare);
        pane.state.sync_prompt = Some("Copy Alcher parameters".into());
        assert_eq!(pane.on_key(KeyCode::Down), ParamsKey::None);
        assert_eq!(pane.on_key(KeyCode::Char('y')), ParamsKey::SyncApply);
        assert_eq!(pane.on_key(KeyCode::Char('n')), ParamsKey::SyncCancel);
        assert_eq!(pane.state.sync_prompt, None);

        pane.on_key(KeyCode::Enter);
        type_replace(&mut pane, "5");
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
        assert_eq!(
            pane.state.error.as_deref(),
            Some("Save failed: vault locked")
        );
        assert_eq!(pane.bag.get("alchs"), Some(&serde_json::json!(27)));
    }

    #[test]
    fn overlay_clears_background_and_keeps_cursor_visible_on_long_schema() {
        let dir = temp_dir("draw");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema: Vec<SettingDef> = (0..20)
            .map(|i| {
                setting(
                    &format!("n{i}"),
                    "number",
                    Some("0"),
                    Some(&format!("Field {i:02}")),
                    &[],
                )
            })
            .collect();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "LongCard", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 18,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(48, 12)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("X".repeat(48 * 12)), frame.area());
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "LongCard"),
                    loadouts: &loadouts,
                    game_data: None,
                    state: &mut state,
                };
                frame.render_widget(pane, frame.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let popup = ParamsPane::popup_rect(Rect {
            x: 0,
            y: 0,
            width: 48,
            height: 12,
        });
        for y in popup.y + 1..popup.y + popup.height.saturating_sub(1) {
            for x in popup.x + 1..popup.x + popup.width.saturating_sub(1) {
                assert_ne!(
                    buf[(x, y)].symbol(),
                    "X",
                    "overlay must clear background at {x},{y}"
                );
            }
        }
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("Field 18"),
            "cursor row must stay visible: {text:?}"
        );
        assert!(
            !text.contains("Field 00"),
            "scrolled long schema must clip the first rows: {text:?}"
        );
        assert!(
            text.contains("enter edit"),
            "idle hint must paint: {text:?}"
        );
    }

    #[test]
    fn editing_consumes_letters_instead_of_moving_the_row() {
        let dir = temp_dir("letters");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = vec![setting(
            "customItem",
            "string",
            Some(""),
            Some("Custom item"),
            &[],
        )];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Alcher"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        assert_eq!(pane.on_key(KeyCode::Char('j')), ParamsKey::None);
        assert_eq!(pane.state.cursor, 0);
        assert_eq!(pane.state.scratch, "j");
        pane.on_key(KeyCode::Enter);
        assert_eq!(pane.bag.get("customItem"), Some(&serde_json::json!("j")));
    }

    #[test]
    fn item_choices_paint_identity_label_and_persist_key() {
        let dir = temp_dir("item-labels");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let schema = vec![SettingDef {
            id: "items".into(),
            ty: "string[]".into(),
            default: None,
            label: Some("Items".into()),
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
            item_option_spec: Some(script::ItemOptionSpec {
                prefix: vec!["custom".into()],
                candidates: vec![script::ItemOptionCandidate {
                    key: "dragonhide_body".into(),
                    label: Some("Green d'hide body".into()),
                }],
                sort_keys_by_label: false,
            }),
        }];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Alcher"),
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            pane.on_key(KeyCode::Down);
            assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
            assert_eq!(
                pane.bag.get("items"),
                Some(&serde_json::json!(["dragonhide_body"])),
                "choice must persist the key, not the identity label"
            );
        }

        let mut bag = store.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            editing: true,
            multi_select: true,
            choice_cursor: 1,
            choice_selected: vec!["dragonhide_body".into()],
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(48, 12)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Alcher"),
                    loadouts: &loadouts,
                    game_data: Some(data.as_ref()),
                    state: &mut state,
                };
                frame.render_widget(pane, frame.area());
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            text.contains("Green d'hide body"),
            "identity label must paint: {text:?}"
        );
        assert!(
            !text.contains("Dragonhide body"),
            "shared client name must not paint: {text:?}"
        );
        assert!(
            text.contains("custom"),
            "prefix chip must remain first: {text:?}"
        );
    }
}

//! Script parameter editors for the TUI script pane (schema-driven).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use script::{
    coerce_setting_value, format_setting_value, setting_visible, LoadoutsStore, SettingDef,
};

/// Mutable params-pane state: form open, cursor, and in-progress edit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamsState {
    pub open: bool,
    /// Latest durable permission view for inherited setting rows.
    pub walk_permissions: frontend_core::WalkGlobalsView,
    pub cursor: usize,
    pub scroll: usize,
    pub editing: bool,
    pub multi_select: bool,
    pub choice_single: bool,
    pub scratch: String,
    pub choice_cursor: usize,
    pub choice_selected: Vec<String>,
    pub choice_search: String,
    pub choice_searchable: bool,
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
pub type ParamsCommit<'a> =
    dyn FnMut(&str, serde_json::Value, Option<serde_json::Value>) -> Result<(), String> + 'a;

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

    fn resolved_options(&self, def: &SettingDef) -> frontend_core::scripts::ParameterOptions {
        frontend_core::scripts::resolve_parameter_options(
            def,
            self.bag,
            self.loadouts,
            self.game_data,
        )
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
        if self.state.multi_select || self.state.choice_single {
            return self.on_choice_key(code, rows);
        }
        match code {
            crossterm::event::KeyCode::Esc => {
                self.cancel_edit();
                ParamsKey::Cancel
            }
            crossterm::event::KeyCode::Enter
            | crossterm::event::KeyCode::Tab
            | crossterm::event::KeyCode::BackTab => self.commit_text(rows),
            crossterm::event::KeyCode::Backspace => {
                self.state.scratch.pop();
                self.refresh_text_error(rows);
                ParamsKey::None
            }
            crossterm::event::KeyCode::Char(c) => {
                self.state.scratch.push(c);
                self.refresh_text_error(rows);
                ParamsKey::None
            }
            _ => ParamsKey::None,
        }
    }

    fn refresh_text_error(&mut self, rows: &[&SettingDef]) {
        let Some(def) = rows.get(self.state.cursor) else {
            return;
        };
        let options = self.resolved_options(def);
        self.state.error = frontend_core::scripts::parse_parameter_text(
            def,
            &self.state.scratch,
            options.selectable(),
        )
        .err();
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
        let searchable = self.state.choice_searchable;
        match code {
            crossterm::event::KeyCode::Esc => {
                self.cancel_edit();
                ParamsKey::Cancel
            }
            crossterm::event::KeyCode::Enter if self.state.choice_single => {
                self.commit_single_choice(def, &opts)
            }
            crossterm::event::KeyCode::Enter => self.commit_choices(def),
            crossterm::event::KeyCode::Backspace if searchable => {
                self.state.choice_search.pop();
                self.state.choice_cursor = 0;
                ParamsKey::None
            }
            crossterm::event::KeyCode::Char(ch)
                if searchable && !ch.is_control() && (self.state.choice_single || ch != ' ') =>
            {
                self.state.choice_search.push(ch);
                self.state.choice_cursor = 0;
                ParamsKey::None
            }
            crossterm::event::KeyCode::Up => {
                if self.state.choice_cursor > 0 {
                    self.state.choice_cursor -= 1;
                }
                ParamsKey::Up
            }
            crossterm::event::KeyCode::Down => {
                if self.state.choice_cursor + 1 < self.choice_indices(&opts).len() {
                    self.state.choice_cursor += 1;
                }
                ParamsKey::Down
            }
            crossterm::event::KeyCode::Char('k') => {
                if self.state.choice_cursor > 0 {
                    self.state.choice_cursor -= 1;
                }
                ParamsKey::Up
            }
            crossterm::event::KeyCode::Char('j') => {
                if self.state.choice_cursor + 1 < self.choice_indices(&opts).len() {
                    self.state.choice_cursor += 1;
                }
                ParamsKey::Down
            }
            crossterm::event::KeyCode::Char(' ') => {
                let visible = self.choice_indices(&opts);
                let Some(index) = visible.get(self.state.choice_cursor).copied() else {
                    return ParamsKey::None;
                };
                if opts.toggle_array_choice(&mut self.state.choice_selected, index) {
                    ParamsKey::Toggle
                } else {
                    ParamsKey::None
                }
            }
            _ => ParamsKey::None,
        }
    }

    fn choice_indices(&self, options: &frontend_core::scripts::ParameterOptions) -> Vec<usize> {
        let values = if self.state.choice_single {
            options.selectable()
        } else {
            options.values.as_slice()
        };
        values
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                options
                    .matches_query(index, &self.state.choice_search)
                    .then_some(index)
            })
            .collect()
    }

    fn activate(&mut self, rows: &[&SettingDef], enter: bool) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor) else {
            return ParamsKey::None;
        };
        if def.ty == "boolean" {
            if self.state.walk_permissions.permission_enabled(&def.id) == Some(true) {
                return ParamsKey::None;
            }
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
        if def.options_from.is_some() && opts.is_empty() {
            return ParamsKey::None;
        }
        if def.ty == "string" && !opts.is_empty() {
            let stored = self
                .bag
                .get(&def.id)
                .and_then(|v| v.as_str())
                .or(def.default.as_deref())
                .unwrap_or("");
            if enter {
                self.state.editing = true;
                self.state.multi_select = false;
                self.state.choice_single = true;
                self.state.choice_cursor = opts
                    .selectable()
                    .iter()
                    .position(|option| opts.matches_option(stored, option))
                    .unwrap_or(0);
                self.state.choice_search.clear();
                self.state.choice_searchable = opts.selectable().len() > 16;
                self.state.choice_selected.clear();
                self.state.scratch.clear();
                self.state.error = None;
                return ParamsKey::Edit;
            }
            let Some(next) = opts.next_selectable(stored) else {
                return ParamsKey::None;
            };
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
            self.state.choice_single = false;
            self.state.choice_cursor = 0;
            self.state.choice_search.clear();
            self.state.choice_searchable = opts.values.len() > 16;
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
            self.state.choice_single = false;
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
        let options = self.resolved_options(def);
        match frontend_core::scripts::parse_parameter_text(
            def,
            &self.state.scratch,
            options.selectable(),
        ) {
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

    fn commit_single_choice(
        &mut self,
        def: &SettingDef,
        options: &frontend_core::scripts::ParameterOptions,
    ) -> ParamsKey {
        let visible = self.choice_indices(options);
        let Some(index) = visible.get(self.state.choice_cursor).copied() else {
            return ParamsKey::None;
        };
        let Some(value) = options.selectable().get(index) else {
            return ParamsKey::None;
        };
        let value = options.value_for(value);
        if self.persist(&def.id, serde_json::json!(value)) {
            self.cancel_edit();
            ParamsKey::Saved
        } else {
            ParamsKey::None
        }
    }

    fn commit_choices(&mut self, def: &SettingDef) -> ParamsKey {
        let raw = serde_json::json!(self.state.choice_selected.clone());
        let value = coerce_setting_value(&def.ty, &raw);
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
        self.state.choice_single = false;
        self.state.scratch.clear();
        self.state.choice_selected.clear();
        self.state.choice_cursor = 0;
        self.state.choice_search.clear();
        self.state.choice_searchable = false;
        self.state.error = None;
    }

    fn persist(&mut self, id: &str, value: serde_json::Value) -> bool {
        let current = self.bag.get(id).cloned();
        if current.as_ref() == Some(&value) {
            self.state.error = None;
            return true;
        }
        match (self.commit)(id, value.clone(), current) {
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
            if self.state.choice_single {
                if self.state.choice_searchable {
                    "type to search · enter pick · esc cancel".into()
                } else {
                    "enter pick · esc cancel".into()
                }
            } else if self.state.multi_select {
                if self.state.choice_searchable {
                    "type to search · backspace clear · space toggle · enter save · esc cancel"
                        .into()
                } else {
                    "space toggle · enter save · esc cancel".into()
                }
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

/// Wrapped detail for stored quest picks that are no longer choices (an
/// unavailable quest and its reason), so the narrow row never clips it.
fn stored_quest_reason(
    bag: &serde_json::Map<String, serde_json::Value>,
    def: &SettingDef,
    options: &frontend_core::scripts::ParameterOptions,
) -> Option<String> {
    if !matches!(
        def.options_from.as_deref(),
        Some("released-paths" | "released-path-order")
    ) || options.preserved == 0
    {
        return None;
    }
    options.preserved_stored_labels(bag.get(&def.id)?)
}

fn display_value(
    bag: &serde_json::Map<String, serde_json::Value>,
    def: &SettingDef,
    options: &frontend_core::scripts::ParameterOptions,
) -> String {
    let value = bag.get(&def.id).cloned().or_else(|| {
        def.default.as_deref().map(|default| {
            serde_json::from_str(default)
                .unwrap_or_else(|_| serde_json::Value::String(default.to_owned()))
        })
    });
    let Some(value) = value else {
        return "—".into();
    };
    let value = options.normalize_value(&value);
    match &value {
        serde_json::Value::String(value) => options.label_for(value).to_owned(),
        serde_json::Value::Array(values) => {
            let labels = values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|value| options.label_for(value))
                .collect::<Vec<_>>();
            if labels.is_empty() {
                "—".into()
            } else {
                labels.join(", ")
            }
        }
        _ => format_setting_value(&value),
    }
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
        let mut unavailable_current = None;
        let mut site_reason_detail = false;
        let mut quest_reason_detail = false;
        if self.state.editing && (self.state.multi_select || self.state.choice_single) {
            if let Some(def) = rows.get(self.state.cursor) {
                let opts = self.resolved_options(def);
                if def.options_from.as_deref() == Some("gather:sites") && opts.preserved > 0 {
                    unavailable_current = Some(display_value(self.bag, def, &opts));
                    site_reason_detail = true;
                } else if let Some(detail) = stored_quest_reason(self.bag, def, &opts) {
                    unavailable_current = Some(detail);
                    quest_reason_detail = true;
                }
                let label = def.label.as_deref().unwrap_or(&def.id);
                lines.push(Line::from(format!("{label} choices")));
                if self.state.choice_searchable {
                    let query = if self.state.choice_search.is_empty() {
                        "type to filter"
                    } else {
                        &self.state.choice_search
                    };
                    lines.push(Line::from(format!("search: {query}")));
                }
                let indices = self.choice_indices(&opts);
                let start = lines.len();
                if indices.is_empty() {
                    cursor_line = start;
                    lines.push(Line::from("no matching options"));
                } else {
                    cursor_line = start
                        + self
                            .state
                            .choice_cursor
                            .min(indices.len().saturating_sub(1));
                    let values = if self.state.choice_single {
                        opts.selectable()
                    } else {
                        opts.values.as_slice()
                    };
                    let stored = self
                        .bag
                        .get(&def.id)
                        .and_then(serde_json::Value::as_str)
                        .or(def.default.as_deref())
                        .unwrap_or("");
                    for (row, index) in indices.into_iter().enumerate() {
                        let Some(opt) = values.get(index) else {
                            continue;
                        };
                        let mark = if row == self.state.choice_cursor {
                            "> "
                        } else {
                            "  "
                        };
                        let status = if self.state.choice_single {
                            if opts.matches_option(stored, opt) {
                                "(•)"
                            } else {
                                "( )"
                            }
                        } else if self
                            .state
                            .choice_selected
                            .iter()
                            .any(|selected| opts.matches_option(selected, opt))
                        {
                            "[x]"
                        } else if opts.is_selectable_index(index) {
                            "[ ]"
                        } else {
                            "[-]"
                        };
                        lines.push(Line::from(format!(
                            "{mark}{status} {}",
                            opts.label_for(opt)
                        )));
                    }
                }
            }
        } else {
            let mut last_group: Option<&str> = None;
            for (i, def) in rows.iter().enumerate() {
                let group = if self
                    .state
                    .walk_permissions
                    .permission_enabled(&def.id)
                    .is_some()
                {
                    Some("Walk permissions")
                } else {
                    def.group.as_deref()
                };
                if group != last_group {
                    last_group = group;
                    if let Some(g) = group {
                        lines.push(Line::from(format!("— {g} —")));
                        if g == "Walk permissions" {
                            lines.push(Line::from(
                                "Allow for this script even when the global setting is off.",
                            ));
                        }
                    }
                }
                if i == self.state.cursor {
                    cursor_line = lines.len();
                }
                let label = def.label.as_deref().unwrap_or(&def.id);
                let mark = if i == self.state.cursor { "> " } else { "  " };
                let value = if self.state.editing && i == self.state.cursor {
                    format!("{}_", self.state.scratch)
                } else if let Some(global) = self.state.walk_permissions.permission_enabled(&def.id)
                {
                    if global {
                        "On (inherited globally; script cannot veto)".into()
                    } else {
                        let enabled = self
                            .bag
                            .get(&def.id)
                            .and_then(|value| value.as_bool())
                            .unwrap_or_else(|| def.default.as_deref() == Some("true"));
                        if enabled {
                            "On (script opt-in)".into()
                        } else {
                            "Off".into()
                        }
                    }
                } else {
                    let options = self.resolved_options(def);
                    if def.options_from.is_some() && options.is_empty() {
                        if def.options_from.as_deref() == Some("loadouts") {
                            "no loadouts available".to_string()
                        } else if options.preserved > 0 {
                            let current = display_value(self.bag, def, &options);
                            if i == self.state.cursor {
                                unavailable_current = Some(current);
                                if def.options_from.as_deref() == Some("gather:sites") {
                                    site_reason_detail = true;
                                }
                                "options unavailable".to_string()
                            } else {
                                if !self.state.editing
                                    && def.options_from.as_deref() == Some("gather:sites")
                                {
                                    unavailable_current = Some(current.clone());
                                    site_reason_detail = true;
                                }
                                format!("options unavailable · {current}")
                            }
                        } else {
                            "options unavailable".to_string()
                        }
                    } else {
                        let current = display_value(self.bag, def, &options);
                        if !self.state.editing
                            && def.options_from.as_deref() == Some("gather:sites")
                            && options.preserved > 0
                        {
                            unavailable_current = Some(current.clone());
                            site_reason_detail = true;
                        } else if !self.state.editing
                            && (i == self.state.cursor || unavailable_current.is_none())
                        {
                            if let Some(detail) = stored_quest_reason(self.bag, def, &options) {
                                unavailable_current = Some(detail);
                                quest_reason_detail = true;
                            }
                        }
                        current
                    }
                };
                lines.push(Line::from(format!("{mark}{label}: {value}")));
            }
        }

        let (hint, reserve) = if let Some(current) = unavailable_current {
            // Keep the full saved-site label and reason visible beneath the row or picker.
            let reserve = if site_reason_detail {
                inner.height.min(3)
            } else if quest_reason_detail {
                // Every wrapped row of the stored quests' reasons, one spare
                // for word wrapping, never more than half the pane.
                let width = usize::from(inner.width.max(1));
                let rows = current.chars().count().div_ceil(width) + 1;
                u16::try_from(rows)
                    .unwrap_or(u16::MAX)
                    .min(inner.height / 2)
                    .max(1)
            } else {
                inner.height.saturating_sub(5).max(1)
            };
            (current, reserve)
        } else {
            let hint = self.hint();
            let reserve = if hint.chars().count() > inner.width as usize {
                2u16
            } else {
                1u16
            };
            (hint, reserve)
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
    ) -> impl FnMut(&str, serde_json::Value, Option<serde_json::Value>) -> Result<(), String> + 's
    {
        move |id, value, _current| {
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
    fn global_walk_permission_is_inherited_and_script_additive_when_off() {
        let dir = temp_dir("walk-permissions");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = vec![SettingDef {
            group: Some("Quester".into()),
            ..setting(
                "allow_teleports",
                "boolean",
                Some("false"),
                Some("Allow teleports"),
                &[],
            )
        }];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Quester", &schema, None);
        assert_eq!(bag.get("allow_teleports"), Some(&serde_json::json!(false)));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            walk_permissions: frontend_core::WalkGlobalsView {
                globals: host_play::WalkGlobals {
                    allow_teleports: true,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    allow_danger_zones: false,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Quester"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::None);
            assert_eq!(
                pane.bag.get("allow_teleports"),
                Some(&serde_json::json!(false)),
                "a script cannot veto a global allow"
            );
        }

        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut read_only = |_: &str, _: serde_json::Value, _: Option<serde_json::Value>| Ok(());
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut read_only,
                    loadouts: &loadouts,
                    game_data: None,
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
        assert!(text.contains("Walk permissions"), "{text:?}");
        assert!(
            text.contains("Allow for this script even when the global setting is off."),
            "{text:?}"
        );
        assert!(
            text.contains("On (inherited globally; script cannot veto)"),
            "{text:?}"
        );

        state.walk_permissions = frontend_core::WalkGlobalsView {
            globals: host_play::WalkGlobals {
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                allow_danger_zones: false,
            },
            ..Default::default()
        };
        let outcome = {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Quester"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            pane.on_key(KeyCode::Char(' '))
        };
        assert_eq!(outcome, ParamsKey::Toggle);
        assert_eq!(bag.get("allow_teleports"), Some(&serde_json::json!(true)));
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
        assert_eq!(pane.bag.get("alchs").and_then(|v| v.as_u64()), Some(27));
        assert!(!pane.state.editing);

        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        type_replace(&mut pane, "40");
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(pane.bag.get("alchs").and_then(|v| v.as_u64()), Some(40));
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["catalog:Alcher"]["alchs"].as_u64(),
            Some(40),
            "the TUI writes integral settings as JSON integers"
        );
        let reloaded = ScriptSettingsStore::at(path);
        let start = reloaded.merged_bag(ScriptSource::Catalog, "Alcher", &schema, None);
        assert_eq!(
            start.get("alchs").and_then(|v| v.as_u64()),
            Some(40),
            "Start merge must load the saved integer"
        );
    }

    #[test]
    fn text_edits_submit_once_on_commit_and_skip_unchanged_or_invalid_values() {
        let loadouts = LoadoutsStore::at(temp_dir("commit").join("loadouts.json"));
        let schema = vec![setting("count", "number", Some("1"), Some("Count"), &[])];
        let mut bag = script::merge_bag(&schema, &serde_json::Map::new(), None);
        let attempts = AtomicU64::new(0);
        let mut commit = |_: &str, _: serde_json::Value, _: Option<serde_json::Value>| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        };
        let mut state = ParamsState {
            open: true,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut commit,
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };

        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        type_replace(&mut pane, "123");
        assert_eq!(attempts.load(Ordering::Relaxed), 0);
        assert_eq!(pane.bag.get("count"), Some(&serde_json::json!(1)));
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(pane.bag.get("count"), Some(&serde_json::json!(123)));

        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);

        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        type_replace(&mut pane, "-");
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(pane.bag.get("count"), Some(&serde_json::json!(123)));
        assert!(pane.state.error.is_some());
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
        let mut refuse = |_: &str, _: serde_json::Value, _: Option<serde_json::Value>| {
            Err("vault locked".to_string())
        };
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

    #[test]
    fn gatherer_dynamic_lists_keep_unknowns_and_allow_radius_edits() {
        let dir = temp_dir("gather-picker");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let mut resources = setting(
            "woodcuttingResources",
            "string[]",
            None,
            Some("Woodcutting resources"),
            &[],
        );
        resources.options_from = Some("gather:woodcutting".into());
        let mut radius = setting("radius", "number", Some("12"), Some("Radius"), &[]);
        radius.min = Some("2".into());
        radius.max = Some("64".into());
        let schema = vec![resources, radius];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", &schema, None);
        bag.insert(
            "woodcuttingResources".into(),
            serde_json::json!(["removed-resource"]),
        );
        let mut state = ParamsState {
            open: true,
            cursor: 1,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Gatherer"),
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            type_replace(&mut pane, "20");
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
            assert_eq!(pane.bag.get("radius"), Some(&serde_json::json!(20)));
            assert_eq!(
                pane.bag.get("woodcuttingResources"),
                Some(&serde_json::json!(["removed-resource"])),
                "editing another setting must not reject or rewrite an unknown resource"
            );
            assert_eq!(pane.on_key(KeyCode::Up), ParamsKey::Up);
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            assert!(pane.state.multi_select);
            assert_eq!(
                pane.state.choice_selected,
                ["removed-resource"],
                "unknown resources remain checked in the picker"
            );
        }

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Gatherer"),
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
            text.contains("Unknown: removed-resource"),
            "unknown resource must have an explicit removable label: {text:?}"
        );
    }

    #[test]
    fn multiselect_picker_searches_lists_larger_than_sixteen() {
        let dir = temp_dir("picker-search");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let mut quests = setting("quests", "string[]", None, Some("Quests"), &[]);
        quests.options = (0..18).map(|index| format!("quest-{index:02}")).collect();
        quests.option_labels = (0..18).map(|index| format!("Quest {index:02}")).collect();
        let schema = vec![quests];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Quester", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Quester"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            assert!(pane.state.choice_searchable);
            pane.on_key(KeyCode::Char('1'));
            pane.on_key(KeyCode::Char('7'));
            assert_eq!(pane.state.choice_search, "17");
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Quester"),
                    loadouts: &loadouts,
                    game_data: None,
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
            text.contains("search: 17"),
            "search query must paint: {text:?}"
        );
        assert!(
            text.contains("Quest 17"),
            "filtered quest must paint: {text:?}"
        );
        assert!(
            !text.contains("Quest 16"),
            "unmatched quests must be hidden: {text:?}"
        );
    }

    #[test]
    fn dynamic_picker_without_facts_is_disabled_not_a_text_box() {
        let dir = temp_dir("picker-unavailable");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let mut resources = setting(
            "woodcuttingResources",
            "string[]",
            None,
            Some("Woodcutting resources"),
            &[],
        );
        resources.options_from = Some("gather:woodcutting".into());
        let schema = vec![resources];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Gatherer"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
            assert!(!pane.state.editing);
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Gatherer"),
                    loadouts: &loadouts,
                    game_data: None,
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
        assert!(text.contains("options unavailable"), "{text:?}");
    }

    #[test]
    fn scalar_fishing_cycle_skips_refused_groups_and_wraps() {
        let dir = temp_dir("fishing-cycle");
        let store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let mut fishing = setting("fishingMethod", "string", None, Some("Fishing method"), &[]);
        fishing.options_from = Some("gather:fishing".into());
        let schema = vec![fishing];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", &schema, None);
        let selectable = data
            .gather_resources_for("fishing")
            .filter(|row| row.selectable)
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        let refused = data
            .gather_resources_for("fishing")
            .filter(|row| !row.selectable)
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        assert!(!selectable.is_empty());
        assert!(!refused.is_empty());
        let start = selectable[0].clone();
        bag.insert("fishingMethod".into(), serde_json::json!(start.clone()));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut commit = |_: &str, value: serde_json::Value, _: Option<serde_json::Value>| {
            let value = value.as_str().unwrap();
            if data
                .gather_resources_for("fishing")
                .any(|row| row.key.as_str() == value && row.selectable)
            {
                Ok(())
            } else {
                Err("method-incomplete".into())
            }
        };
        let mut cycle = Vec::new();
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut commit,
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            for _ in 0..selectable.len() {
                assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
                cycle.push(
                    pane.bag
                        .get("fishingMethod")
                        .and_then(serde_json::Value::as_str)
                        .unwrap()
                        .to_owned(),
                );
            }
            assert_eq!(cycle.last(), Some(&start), "the cycle returns to its start");
        }
        assert_eq!(
            cycle.iter().collect::<std::collections::HashSet<_>>().len(),
            selectable.len(),
            "every selectable fishing group appears once per cycle"
        );
        assert!(
            selectable.iter().all(|key| cycle.contains(key)),
            "all selectable groups are reachable: {cycle:?}"
        );
        assert!(
            refused.iter().all(|key| !cycle.contains(key)),
            "refused groups are never offered: {cycle:?}"
        );
    }

    #[test]
    fn gather_site_picker_filters_and_accepts_explicit_selection() {
        let dir = temp_dir("site-picker");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let schema = script::gatherer::settings::schema();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", schema, None);
        bag.insert("skill".into(), serde_json::json!("Woodcutting"));
        bag.insert("woodcuttingResources".into(), serde_json::json!(["normal"]));
        bag.insert("location".into(), serde_json::json!("Site"));
        bag.insert("site".into(), serde_json::json!("woodcutting.draynor"));
        let site_index = schema
            .iter()
            .position(|field| field.id == "site")
            .expect("Gatherer schema includes the named site setting");
        let visible_cursor = schema
            .iter()
            .filter(|field| script::setting_visible(field.show_if.as_deref(), &bag))
            .position(|field| field.id == "site")
            .expect("the Site setting is visible in Site mode");
        let options = frontend_core::scripts::resolve_parameter_options(
            &schema[site_index],
            &bag,
            &loadouts,
            Some(data.as_ref()),
        );
        assert_eq!(options.selectable().len(), 223);
        assert_eq!(
            options
                .selectable()
                .iter()
                .enumerate()
                .filter(|(index, _)| options.matches_query(*index, "varrock"))
                .count(),
            11
        );
        let original = bag.get("site").cloned().unwrap();
        let mut state = ParamsState {
            open: true,
            cursor: visible_cursor,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Gatherer"),
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            assert!(pane.state.choice_single);
            assert!(pane.state.choice_searchable);
            assert_eq!(
                pane.state.choice_cursor,
                options
                    .selectable()
                    .iter()
                    .position(|value| value == "woodcutting.draynor")
                    .unwrap()
            );
            assert_eq!(pane.on_key(KeyCode::Char('j')), ParamsKey::None);
            assert_eq!(pane.state.choice_search, "j");
            assert_eq!(pane.state.choice_cursor, 0, "j starts a query");
            assert_eq!(pane.on_key(KeyCode::Backspace), ParamsKey::None);
            for ch in "varrock".chars() {
                pane.on_key(KeyCode::Char(ch));
            }
            assert_eq!(pane.state.choice_search, "varrock");
            assert_eq!(pane.state.choice_cursor, 0);
            assert_eq!(pane.bag.get("site"), Some(&original));
        }

        let filtered = options
            .selectable()
            .iter()
            .enumerate()
            .filter(|(index, _)| options.matches_query(*index, "varrock"))
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        let selected = filtered[0].clone();
        let selected_label = options.label_for(&selected).to_owned();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Gatherer"),
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
        assert!(text.contains("search: varrock"), "{text:?}");
        assert!(text.contains(&selected_label), "{text:?}");

        {
            let mut pane = ParamsPane {
                schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Gatherer"),
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
            assert_eq!(pane.bag.get("site"), Some(&serde_json::json!(selected)));

            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            for ch in "no-such-place".chars() {
                pane.on_key(KeyCode::Char(ch));
            }
            assert!(pane.choice_indices(&options).is_empty());
            let saved = pane.bag.get("site").cloned();
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
            assert!(pane.state.editing);
            assert_eq!(pane.bag.get("site").cloned(), saved);
            assert_eq!(pane.on_key(KeyCode::Esc), ParamsKey::Cancel);
            assert_eq!(pane.bag.get("site").cloned(), saved);
        }
    }

    #[test]
    fn invalid_gather_site_remains_disabled_and_explains_why() {
        let dir = temp_dir("site-disabled");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let schema = script::gatherer::settings::schema();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", schema, None);
        bag.insert("skill".into(), serde_json::json!("Mining"));
        bag.insert("miningResources".into(), serde_json::json!(["rune stones"]));
        bag.insert("location".into(), serde_json::json!("Site"));
        bag.insert("site".into(), serde_json::json!("mining.varrock_east.se"));
        let visible_cursor = schema
            .iter()
            .filter(|field| script::setting_visible(field.show_if.as_deref(), &bag))
            .position(|field| field.id == "site")
            .expect("the Site setting is visible in Site mode");
        let original = bag.get("site").cloned();
        let mut state = ParamsState {
            open: true,
            cursor: visible_cursor,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Gatherer"),
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
            assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::None);
            assert!(!pane.state.editing);
            assert_eq!(pane.bag.get("site").cloned(), original);
        }
        for (width, height) in [(120, 40), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let pane = ParamsPane {
                        schema,
                        bag: &mut bag,
                        commit: &mut store_commit(&mut store, "Gatherer"),
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
                text.contains("options unavailable"),
                "{width}x{height}: {text:?}"
            );
            assert!(text.contains("Varrock East"), "{width}x{height}: {text:?}");
            let unwrapped: String = text.chars().filter(char::is_ascii_alphanumeric).collect();
            assert!(
                unwrapped.contains("notfortheselectedresources"),
                "{width}x{height}: {text:?}"
            );
        }
    }

    #[test]
    fn incompatible_saved_site_reason_is_visible_with_choices_at_80_columns() {
        let dir = temp_dir("site-reason-with-choices");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let schema = script::gatherer::settings::schema();
        let willow = data
            .gather_option("woodcutting", "willow")
            .expect("Willow resolves to a selected gathering key");
        let saved_site = data
            .gather_sites_for("woodcutting")
            .find(|site| !site.keys.iter().any(|key| key.key == willow.key))
            .expect("a named site does not offer Willow");
        let saved_site_id = saved_site.id.clone();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Gatherer", schema, None);
        bag.insert("skill".into(), serde_json::json!("Woodcutting"));
        bag.insert("woodcuttingResources".into(), serde_json::json!(["willow"]));
        bag.insert("location".into(), serde_json::json!("Site"));
        bag.insert("site".into(), serde_json::json!(saved_site_id));
        let site_index = schema
            .iter()
            .position(|field| field.id == "site")
            .expect("Gatherer schema includes the named site setting");
        let visible_cursor = schema
            .iter()
            .filter(|field| script::setting_visible(field.show_if.as_deref(), &bag))
            .position(|field| field.id == "site")
            .expect("the Site setting is visible in Site mode");
        let options = frontend_core::scripts::resolve_parameter_options(
            &schema[site_index],
            &bag,
            &loadouts,
            Some(data.as_ref()),
        );
        assert!(!options.is_empty(), "Willow has alternate named sites");
        assert_eq!(options.preserved, 1);
        let expected = options.label_for(&saved_site_id).to_owned();
        let expected_compact = expected
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        assert_ne!(visible_cursor, 0, "the site row is not the first field");
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Gatherer"),
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
        let compact = text
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        assert!(compact.contains(&expected_compact), "{text:?}");

        state.cursor = visible_cursor;

        {
            let mut pane = ParamsPane {
                schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Gatherer"),
                loadouts: &loadouts,
                game_data: Some(data.as_ref()),
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            assert!(pane.state.choice_single);
        }
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Gatherer"),
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
        let compact = text
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        assert!(text.contains("site choices"), "{text:?}");
        assert!(compact.contains(&expected_compact), "{text:?}");
    }

    #[test]
    fn quester_picker_keeps_unknown_path_visible_and_removable() {
        let dir = temp_dir("quest-unknown");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let mut quests = setting("quests", "string[]", None, Some("Quests"), &[]);
        quests.options = vec!["cook".into(), "sheep".into()];
        quests.option_labels = vec!["Cook's Assistant".into(), "Sheep Shearer".into()];
        quests.options_from = Some("released-paths".into());
        let schema = vec![quests];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Quester", &schema, None);
        bag.insert("quests".into(), serde_json::json!(["removed-path"]));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Quester"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            assert_eq!(pane.state.choice_selected, ["removed-path"]);
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Quester"),
                    loadouts: &loadouts,
                    game_data: None,
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
        assert!(text.contains("Unknown: removed-path"), "{text:?}");

        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Quester"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        let options = pane.resolved_options(&schema[0]);
        pane.state.choice_cursor = options
            .values
            .iter()
            .position(|value| value == "removed-path")
            .unwrap();
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert!(pane.state.choice_selected.is_empty());
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(pane.bag.get("quests"), Some(&serde_json::json!([])));
    }

    #[test]
    fn unavailable_quest_is_never_added_by_the_real_picker() {
        let dir = temp_dir("quest-unavailable-add");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = (script::quester::card::CARD.schema)();
        let quests_row = schema
            .iter()
            .position(|field| field.id == "quests")
            .unwrap();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Quester", schema, None);
        bag.insert("quests".into(), serde_json::json!(["cook"]));
        let mut state = ParamsState {
            open: true,
            cursor: quests_row,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema,
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Quester"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        let options = pane.resolved_options(&schema[quests_row]);
        pane.state.choice_cursor = options
            .values
            .iter()
            .position(|value| value == "hauntedmine")
            .unwrap();
        pane.on_key(KeyCode::Char(' '));
        assert_eq!(pane.state.choice_selected, ["cook"]);
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(pane.bag.get("quests"), Some(&serde_json::json!(["cook"])));
    }

    #[test]
    fn stored_unavailable_quest_reason_is_not_clipped_at_80_columns() {
        let dir = temp_dir("quest-unavailable-reason");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = (script::quester::card::CARD.schema)();
        let reason = script::quester::card::unavailable_quest("hauntedmine").unwrap();
        let full = format!("Haunted Mine — {reason}")
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        let quests_row = schema
            .iter()
            .position(|field| field.id == "quests")
            .unwrap();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Quester", schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: quests_row,
            ..Default::default()
        };

        let render = |bag: &mut serde_json::Map<String, serde_json::Value>,
                      state: &mut ParamsState,
                      store: &mut ScriptSettingsStore| {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| {
                    let pane = ParamsPane {
                        schema,
                        bag,
                        commit: &mut store_commit(store, "Quester"),
                        loadouts: &loadouts,
                        game_data: None,
                        state,
                    };
                    frame.render_widget(pane, frame.area());
                })
                .unwrap();
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        };
        bag.insert("quests".into(), serde_json::json!(["cook", "hauntedmine"]));
        let rows = render(&mut bag, &mut state, &mut store);
        assert!(rows.contains(&full), "rows view clips the reason: {rows}");
        {
            let mut pane = ParamsPane {
                schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Quester"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        }
        let picker = render(&mut bag, &mut state, &mut store);
        assert!(picker.contains(&full), "picker clips the reason: {picker}");
    }

    #[test]
    fn empty_loadout_source_shows_a_clear_disabled_state() {
        let dir = temp_dir("empty-loadout-options");
        let mut store = ScriptSettingsStore::at(dir.join("script-settings.json"));
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let mut loadout = setting("loadout", "string", None, Some("Loadout"), &[]);
        loadout.options_from = Some("loadouts".into());
        let schema = vec![loadout];
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Thiever", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: &schema,
                bag: &mut bag,
                commit: &mut store_commit(&mut store, "Thiever"),
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::None);
            assert!(
                !pane.state.editing,
                "an empty option source is not free text"
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let pane = ParamsPane {
                    schema: &schema,
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Thiever"),
                    loadouts: &loadouts,
                    game_data: None,
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
        assert!(text.contains("no loadouts available"), "{text:?}");
    }
}

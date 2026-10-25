//! Script parameter editors for the TUI script pane (schema-driven).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use frontend_core::scripts::{format_setting_value, Loadouts, SchemaRef, SettingView};

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
    /// Kept save errors, keyed by the field that submitted the write.
    pub save_errors: Vec<(String, String)>,
    /// Apply-to-all confirmation line (scope and the y/n keys) while one
    /// is prepared.
    pub sync_prompt: Option<String>,
    /// The last Apply-to-all result, shown until the next key.
    pub report: Option<String>,
    /// The selected profile's current card has saved settings awaiting a restart.
    pub restart_badge: Option<String>,
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
    pub schema: SchemaRef<'a>,
    pub bag: &'a mut serde_json::Map<String, serde_json::Value>,
    pub commit: &'a mut ParamsCommit<'a>,
    pub loadouts: &'a Loadouts,
    pub game_data: Option<&'a api::game_data::SelectedGameData>,
    pub state: &'a mut ParamsState,
}

impl<'a> ParamsPane<'a> {
    pub fn visible_rows(&self) -> Vec<SettingView<'a>> {
        self.schema.iter().filter(|d| d.visible(self.bag)).collect()
    }

    fn resolved_options(&self, def: SettingView<'_>) -> frontend_core::scripts::ParameterOptions {
        frontend_core::scripts::resolve_parameter_options(
            def,
            self.bag,
            self.loadouts,
            self.game_data,
        )
    }

    fn array_editor_enabled(
        &self,
        def: SettingView<'_>,
        options: &frontend_core::scripts::ParameterOptions,
    ) -> bool {
        def.ty() == "string[]"
            && options.can_edit_array(
                self.bag
                    .get(def.id())
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str),
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
        let n = self.schema.iter().filter(|d| d.visible(self.bag)).count();
        if n == 0 {
            self.state.cursor = 0;
        } else if self.state.cursor >= n {
            self.state.cursor = n - 1;
        }
    }

    fn on_edit_key(
        &mut self,
        code: crossterm::event::KeyCode,
        rows: &[SettingView<'_>],
    ) -> ParamsKey {
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

    fn refresh_text_error(&mut self, rows: &[SettingView<'_>]) {
        let Some(def) = rows.get(self.state.cursor).copied() else {
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
        rows: &[SettingView<'_>],
    ) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor).copied() else {
            return ParamsKey::None;
        };
        let opts = self.resolved_options(def);
        if opts.is_empty() && !self.array_editor_enabled(def, &opts) {
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
            options.values()
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

    fn activate(&mut self, rows: &[SettingView<'_>], enter: bool) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor).copied() else {
            return ParamsKey::None;
        };
        if def.ty() == "boolean" {
            if self.state.walk_permissions.permission_enabled(def.id()) == Some(true) {
                return ParamsKey::None;
            }
            let cur = self
                .bag
                .get(def.id())
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| def.default() == Some("true"));
            return if self.persist(def.id(), serde_json::json!(!cur)) {
                ParamsKey::Toggle
            } else {
                ParamsKey::None
            };
        }
        let opts = self.resolved_options(def);
        let array_editable = self.array_editor_enabled(def, &opts);
        if def.options_from().is_some() && opts.is_empty() && !array_editable {
            return ParamsKey::None;
        }
        if def.ty() == "string" && !opts.is_empty() {
            let stored = self
                .bag
                .get(def.id())
                .and_then(|v| v.as_str())
                .or(def.default())
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
            return if self.persist(def.id(), serde_json::json!(next)) {
                ParamsKey::Toggle
            } else {
                ParamsKey::None
            };
        }
        if !enter && def.ty() != "string[]" {
            return ParamsKey::None;
        }
        if def.ty() == "string[]" && array_editable {
            self.state.editing = true;
            self.state.multi_select = true;
            self.state.choice_single = false;
            self.state.choice_cursor = 0;
            self.state.choice_search.clear();
            self.state.choice_searchable = opts.values().len() > 16;
            self.state.choice_selected = current_string_list(self.bag.get(def.id()));
            self.state.scratch.clear();
            self.state.error = None;
            return ParamsKey::Edit;
        }
        if matches!(def.ty(), "number" | "string" | "tile" | "list" | "string[]") {
            self.state.editing = true;
            self.state.multi_select = false;
            self.state.choice_single = false;
            self.state.scratch = self
                .bag
                .get(def.id())
                .map(format_setting_value)
                .unwrap_or_else(|| def.default().unwrap_or_default().to_owned());
            self.state.choice_selected.clear();
            self.state.error = None;
            return ParamsKey::Edit;
        }
        ParamsKey::None
    }

    fn commit_text(&mut self, rows: &[SettingView<'_>]) -> ParamsKey {
        let Some(def) = rows.get(self.state.cursor).copied() else {
            return ParamsKey::None;
        };
        let options = self.resolved_options(def);
        match frontend_core::scripts::parse_parameter_text(
            def,
            &self.state.scratch,
            options.selectable(),
        ) {
            Ok(value) => {
                if self.persist(def.id(), value) {
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
        def: SettingView<'_>,
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
        if self.persist(def.id(), serde_json::json!(value)) {
            self.cancel_edit();
            ParamsKey::Saved
        } else {
            ParamsKey::None
        }
    }

    fn commit_choices(&mut self, def: SettingView<'_>) -> ParamsKey {
        let raw = serde_json::json!(self.state.choice_selected.clone());
        let value = def.coerce(&raw);
        if self.persist(def.id(), value) {
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
        if self.state.editing && (self.state.choice_single || self.state.multi_select) {
            if let Some(def) = self
                .schema
                .iter()
                .filter(|def| def.visible(self.bag))
                .nth(self.state.cursor)
            {
                if let Some((_, error)) = self
                    .state
                    .save_errors
                    .iter()
                    .find(|(field, _)| field == def.id())
                {
                    return format!("{}: {error}", def.display_label());
                }
            }
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
fn stored_quest_reason<'a>(
    bag: &'a serde_json::Map<String, serde_json::Value>,
    def: SettingView<'_>,
    options: &frontend_core::scripts::ParameterOptions,
) -> Option<&'a serde_json::Value> {
    if !matches!(
        def.options_from(),
        Some("released-paths" | "released-path-order")
    ) || options.preserved() == 0
    {
        return None;
    }
    let stored = bag.get(def.id())?;
    options
        .preserved_stored_labels(stored)
        .next()
        .map(|_| stored)
}

fn display_value(
    bag: &serde_json::Map<String, serde_json::Value>,
    def: SettingView<'_>,
    options: &frontend_core::scripts::ParameterOptions,
) -> String {
    let value = bag.get(def.id()).cloned().or_else(|| {
        def.default().map(|default| {
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

        let mut lines: Vec<String> = Vec::new();
        if let Some(badge) = self.state.restart_badge.as_deref() {
            lines.push(badge.to_owned());
        }
        let mut cursor_line = 0usize;
        let mut cursor_end_line = 1usize;
        let mut unavailable_current = None;
        let mut site_reason_detail = false;
        let mut quest_detail = None;
        if self.state.editing && (self.state.multi_select || self.state.choice_single) {
            if let Some(def) = rows.get(self.state.cursor).copied() {
                let opts = self.resolved_options(def);
                let mut stored_quest = None;
                if def.options_from() == Some("gather:sites") && opts.preserved() > 0 {
                    unavailable_current = Some(display_value(self.bag, def, &opts));
                    site_reason_detail = true;
                } else if let Some(stored) = stored_quest_reason(self.bag, def, &opts) {
                    unavailable_current = None;
                    site_reason_detail = false;
                    stored_quest = Some(stored);
                }
                let label = def.display_label();
                lines.push(format!("{label} choices"));
                if self.state.choice_searchable {
                    let query = if self.state.choice_search.is_empty() {
                        "type to filter"
                    } else {
                        &self.state.choice_search
                    };
                    lines.push(format!("search: {query}"));
                }
                let indices = self.choice_indices(&opts);
                let start = lines.len();
                if indices.is_empty() {
                    cursor_line = start;
                    lines.push("no matching options".to_owned());
                } else {
                    cursor_line = start
                        + self
                            .state
                            .choice_cursor
                            .min(indices.len().saturating_sub(1));
                    let values = if self.state.choice_single {
                        opts.selectable()
                    } else {
                        opts.values()
                    };
                    let stored = self
                        .bag
                        .get(def.id())
                        .and_then(serde_json::Value::as_str)
                        .or(def.default())
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
                        lines.push(format!("{mark}{status} {}", opts.label_for(opt)));
                    }
                }
                if let Some(stored) = stored_quest {
                    quest_detail = Some((opts, stored));
                }
                cursor_end_line = cursor_line + 1;
            }
        } else {
            let mut last_group: Option<&str> = None;
            for (i, def) in rows.iter().copied().enumerate() {
                let group = if self
                    .state
                    .walk_permissions
                    .permission_enabled(def.id())
                    .is_some()
                {
                    Some("Walk permissions")
                } else {
                    def.group()
                };
                if group != last_group {
                    last_group = group;
                    if let Some(g) = group {
                        lines.push(format!("— {g} —"));
                        if g == "Walk permissions" {
                            lines.push(
                                "Allow for this script even when the global setting is off."
                                    .to_owned(),
                            );
                        }
                    }
                }
                if i == self.state.cursor {
                    cursor_line = lines.len();
                }
                let label = def.display_label();
                let mark = if i == self.state.cursor { "> " } else { "  " };
                let value = if self.state.editing && i == self.state.cursor {
                    format!("{}_", self.state.scratch)
                } else if let Some(global) =
                    self.state.walk_permissions.permission_enabled(def.id())
                {
                    if global {
                        "On (inherited globally; script cannot veto)".into()
                    } else {
                        let enabled = self
                            .bag
                            .get(def.id())
                            .and_then(|value| value.as_bool())
                            .unwrap_or_else(|| def.default() == Some("true"));
                        if enabled {
                            "On (script opt-in)".into()
                        } else {
                            "Off".into()
                        }
                    }
                } else {
                    let options = self.resolved_options(def);
                    if def.options_from().is_some()
                        && options.is_empty()
                        && !self.array_editor_enabled(def, &options)
                    {
                        if def.options_from() == Some("loadouts") {
                            "no loadouts available".to_string()
                        } else if options.preserved() > 0 {
                            let current = display_value(self.bag, def, &options);
                            if i == self.state.cursor {
                                unavailable_current = Some(current);
                                site_reason_detail = def.options_from() == Some("gather:sites");
                                quest_detail = None;
                                "options unavailable".to_string()
                            } else {
                                if !self.state.editing && def.options_from() == Some("gather:sites")
                                {
                                    unavailable_current = Some(current.clone());
                                    site_reason_detail = true;
                                    quest_detail = None;
                                }
                                format!("options unavailable · {current}")
                            }
                        } else {
                            "options unavailable".to_string()
                        }
                    } else {
                        let current = display_value(self.bag, def, &options);
                        if !self.state.editing
                            && def.options_from() == Some("gather:sites")
                            && options.preserved() > 0
                        {
                            unavailable_current = Some(current.clone());
                            site_reason_detail = true;
                            quest_detail = None;
                        } else if !self.state.editing
                            && (i == self.state.cursor
                                || (unavailable_current.is_none() && quest_detail.is_none()))
                        {
                            if let Some(stored) = stored_quest_reason(self.bag, def, &options) {
                                unavailable_current = None;
                                site_reason_detail = false;
                                quest_detail = Some((options, stored));
                            }
                        }
                        current
                    }
                };
                let range = if def.ty() == "number" {
                    match def.bounds() {
                        (Some(min), Some(max), _) => format!(" ({min}–{max})"),
                        (Some(min), None, _) => format!(" (minimum {min})"),
                        (None, Some(max), _) => format!(" (maximum {max})"),
                        (None, None, _) => String::new(),
                    }
                } else {
                    String::new()
                };
                lines.push(format!("{mark}{label}{range}: {value}"));
                if let Some((_, error)) = self
                    .state
                    .save_errors
                    .iter()
                    .find(|(field, _)| field == def.id())
                {
                    lines.push(format!("  {label}: {error}"));
                }
                if i == self.state.cursor {
                    if let Some(error) = self.state.error.as_deref() {
                        lines.push(format!("  {label}: {error}"));
                    }
                    cursor_end_line = lines.len();
                }
            }
        }

        let (hint, reserve) = if let Some(current) = unavailable_current {
            // Keep the full saved detail visible beneath the row or picker.
            let reserve = if site_reason_detail {
                inner.height.min(3)
            } else {
                inner.height.saturating_sub(5).max(1)
            };
            (Some(current), reserve)
        } else if let Some((options, stored)) = quest_detail.as_ref() {
            // Reserve space for each borrowed label in the existing wrapped renderer.
            let width = usize::from(inner.width.max(1));
            let rows = options
                .preserved_stored_labels(stored)
                .map(|label| label.chars().count().div_ceil(width).saturating_add(1))
                .fold(0usize, usize::saturating_add);
            let reserve = u16::try_from(rows)
                .unwrap_or(u16::MAX)
                .min(inner.height / 2)
                .max(1);
            (None, reserve)
        } else {
            let hint = self.hint();
            let reserve = crate::settings::wrapped_rows(&hint, usize::from(inner.width))
                .min(inner.height.saturating_sub(1));
            (Some(hint), reserve)
        };
        // Scroll physical, wrapped rows rather than clipping logical rows.
        // Keep the selected field and its validation error in the same view.
        let wrapped_height =
            |line: &String| crate::settings::wrapped_lines(line, usize::from(inner.width)).count();
        let cursor_start = lines.iter().take(cursor_line).map(wrapped_height).sum();
        let cursor_end: usize = lines.iter().take(cursor_end_line).map(wrapped_height).sum();
        let content_h = usize::from(inner.height.saturating_sub(reserve));
        if cursor_start < self.state.scroll {
            self.state.scroll = cursor_start;
        } else if cursor_end > self.state.scroll + content_h {
            self.state.scroll = cursor_end.saturating_sub(content_h).min(cursor_start);
        }
        let total_rows: usize = lines.iter().map(wrapped_height).sum();
        self.state.scroll = self
            .state
            .scroll
            .min(total_rows.saturating_sub(content_h.max(1)));
        let list = Rect {
            height: inner.height.saturating_sub(reserve),
            ..inner
        };
        for (row, text) in lines
            .iter()
            .flat_map(|line| crate::settings::wrapped_lines(line, usize::from(inner.width)))
            .skip(self.state.scroll)
            .take(content_h)
            .enumerate()
        {
            buf.set_stringn(
                list.x,
                list.y + u16::try_from(row).unwrap_or(u16::MAX),
                text,
                usize::from(list.width),
                ratatui::style::Style::default(),
            );
        }
        let hint_area = Rect {
            y: list.y + list.height,
            height: inner.height - list.height,
            ..inner
        };
        if let Some(hint) = hint {
            crate::settings::draw_wrapped(
                buf,
                hint_area,
                hint_area.y,
                hint_area.bottom(),
                &hint,
                ratatui::style::Style::default(),
            );
        } else if let Some((options, stored)) = quest_detail.as_ref() {
            let mut y = hint_area.y;
            let bottom = hint_area.y.saturating_add(hint_area.height);
            let width = usize::from(hint_area.width.max(1));
            for label in options.preserved_stored_labels(stored) {
                let remaining = bottom.saturating_sub(y);
                if remaining == 0 {
                    break;
                }
                let rows = label.chars().count().div_ceil(width).saturating_add(1);
                let height = u16::try_from(rows).unwrap_or(u16::MAX).min(remaining);
                Paragraph::new(label)
                    .wrap(ratatui::widgets::Wrap { trim: true })
                    .render(
                        Rect {
                            y,
                            height,
                            ..hint_area
                        },
                        buf,
                    );
                y = y.saturating_add(height);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "js")]
    use std::sync::atomic::{AtomicU64, Ordering};

    use crossterm::event::KeyCode;
    #[cfg(feature = "js")]
    use frontend_core::scripts::CardSource;
    use frontend_core::test_support::scripts::TestSchema;
    #[cfg(feature = "js")]
    use frontend_core::test_support::scripts::{unavailable_quest, TestStore};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    #[cfg(feature = "js")]
    static TEST_DIRS: AtomicU64 = AtomicU64::new(0);
    #[cfg(feature = "js")]

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

    #[cfg(feature = "js")]
    fn bury_schema() -> TestSchema {
        TestSchema::setting(
            "buryBones",
            "boolean",
            Some("true"),
            Some("Bury bones"),
            &[],
        )
    }

    fn alcher_schema() -> TestSchema {
        let mut schema = TestSchema::setting("items", "string[]", None, Some("Items to alch"), &[]);
        schema.add_setting("alchs", "number", Some("27"), Some("Alchs per trip"), &[]);
        schema
    }

    #[cfg(feature = "js")]
    /// Commit sink over a legacy store, standing in for the coordinator.
    fn store_commit<'s>(
        store: &'s mut TestStore,
        name: &'static str,
    ) -> impl FnMut(&str, serde_json::Value, Option<serde_json::Value>) -> Result<(), String> + 's
    {
        move |id, value, _current| {
            store.set_value(CardSource::Catalog, name, id, value);
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

    #[cfg(feature = "js")]
    #[test]
    fn params_pane_space_toggles_a_bool_param() {
        let dir = temp_dir("bool");
        let path = dir.join("script-settings.json");
        let mut store = TestStore::at(path);
        let loadouts = Loadouts::default();
        let schema = bury_schema();
        let mut bag = store.merged_bag(CardSource::Catalog, "ChickenKiller", &schema, None);
        assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(true)));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "ChickenKiller"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(false)));
    }

    #[cfg(feature = "js")]
    #[test]
    fn global_walk_permission_is_inherited_and_script_additive_when_off() {
        let dir = temp_dir("walk-permissions");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let mut schema = TestSchema::setting(
            "allow_teleports",
            "boolean",
            Some("false"),
            Some("Allow teleports"),
            &[],
        );
        schema.set_group("Quester");
        let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
        assert_eq!(bag.get("allow_teleports"), Some(&serde_json::json!(false)));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            walk_permissions: frontend_core::WalkGlobalsView::new(
                frontend_core::WalkGlobals::new(true, false, false, false, false),
                false,
                false,
            ),
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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

        state.walk_permissions = frontend_core::WalkGlobalsView::new(
            frontend_core::WalkGlobals::new(false, false, false, false, false),
            false,
            false,
        );
        let outcome = {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn params_pane_space_cycles_loadout_combo_from_store() {
        let dir = temp_dir("loadout");
        let loadouts = Loadouts::detached(vec![
            api::loadout::Loadout::new("fish"),
            api::loadout::Loadout::new("mine"),
        ]);
        let mut schema =
            TestSchema::setting("loadout", "string", Some("fish"), Some("Loadout"), &[]);
        schema.set_options_from("loadouts");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let mut bag = store.merged_bag(CardSource::Catalog, "Thiever", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Thiever"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(bag.get("loadout"), Some(&serde_json::json!("mine")));
    }

    #[cfg(feature = "js")]
    #[test]
    fn alcher_numeric_enter_saves_and_esc_cancels() {
        let dir = temp_dir("alcher-num");
        let path = dir.join("script-settings.json");
        let mut store = TestStore::at(path.clone());
        let loadouts = Loadouts::default();
        let schema = alcher_schema();
        let mut bag = store.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 1,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
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
        let reloaded = TestStore::at(path);
        let start = reloaded.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        assert_eq!(
            start.get("alchs").and_then(|v| v.as_u64()),
            Some(40),
            "Start merge must load the saved integer"
        );
    }

    #[test]
    fn text_edits_submit_once_on_commit_and_skip_unchanged_or_invalid_values() {
        let loadouts = Loadouts::default();
        let schema = TestSchema::setting("count", "number", Some("1"), Some("Count"), &[]);
        let mut bag = schema.merged_bag(&serde_json::Map::new(), None);
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
            schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn alcher_string_array_csv_persists_as_array() {
        let dir = temp_dir("alcher-items");
        let path = dir.join("script-settings.json");
        let mut store = TestStore::at(path.clone());
        let loadouts = Loadouts::default();
        let schema = alcher_schema();
        let mut bag = store.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        assert!(bag.get("items").is_none());
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
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
        let reloaded = TestStore::at(path);
        let start = reloaded.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        assert_eq!(
            start.get("items"),
            Some(&serde_json::json!(["Maple longbow", "Yew longbow"]))
        );
    }

    #[cfg(feature = "js")]
    #[test]
    fn optioned_string_array_space_toggles_choice_and_cancel_keeps_old() {
        let dir = temp_dir("choices");
        let path = dir.join("script-settings.json");
        let mut store = TestStore::at(path);
        let loadouts = Loadouts::default();
        let schema = TestSchema::setting(
            "items",
            "string[]",
            Some("Maple longbow"),
            Some("Items to alch"),
            &["Maple longbow", "Yew longbow", "Custom"],
        );
        let mut bag = store.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn tile_and_list_coercion_rejects_invalid_without_replacing() {
        let dir = temp_dir("coerce");
        let path = dir.join("script-settings.json");
        let mut store = TestStore::at(path);
        let loadouts = Loadouts::default();
        let mut schema =
            TestSchema::setting("spot", "tile", Some("3200,3200,0"), Some("Spot"), &[]);
        schema.add_setting("drops", "list", Some("bones,shells"), Some("Drops"), &[]);
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
        let before_tile = bag.get("spot").cloned();
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
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
        let loadouts = Loadouts::default();
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
            schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn overlay_clears_background_and_keeps_cursor_visible_on_long_schema() {
        let dir = temp_dir("draw");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let mut schema = TestSchema::setting("n0", "number", Some("0"), Some("Field 00"), &[]);
        for i in 1..20 {
            schema.add_setting(
                &format!("n{i}"),
                "number",
                Some("0"),
                Some(&format!("Field {i:02}")),
                &[],
            );
        }
        let mut bag = store.merged_bag(CardSource::Catalog, "LongCard", &schema, None);
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
                    schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn editing_consumes_letters_instead_of_moving_the_row() {
        let dir = temp_dir("letters");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let schema =
            TestSchema::setting("customItem", "string", Some(""), Some("Custom item"), &[]);
        let mut bag = store.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn item_choices_paint_identity_label_and_persist_key() {
        let dir = temp_dir("item-labels");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let mut schema = TestSchema::setting("items", "string[]", None, Some("Items"), &[]);
        schema.set_item_options(
            &["custom"],
            &[("dragonhide_body", Some("Green d'hide body"))],
            false,
        );
        let mut bag = store.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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

        let mut bag = store.merged_bag(CardSource::Catalog, "Alcher", &schema, None);
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
                    schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn gatherer_dynamic_lists_keep_unknowns_and_allow_radius_edits() {
        let dir = temp_dir("gather-picker");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let mut schema = TestSchema::setting(
            "woodcuttingResources",
            "string[]",
            None,
            Some("Woodcutting resources"),
            &[],
        );
        schema.set_options_from("gather:woodcutting");
        schema.add_setting("radius", "number", Some("12"), Some("Radius"), &[]);
        schema.set_min("2").set_max("64");
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
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
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn multiselect_picker_searches_lists_larger_than_sixteen() {
        let dir = temp_dir("picker-search");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let options = (0..18)
            .map(|index| format!("quest-{index:02}"))
            .collect::<Vec<_>>();
        let labels = (0..18)
            .map(|index| format!("Quest {index:02}"))
            .collect::<Vec<_>>();
        let mut schema = TestSchema::setting(
            "quests",
            "string[]",
            None,
            Some("Quests"),
            &options.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        schema.set_option_labels(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn dynamic_picker_without_facts_is_disabled_not_a_text_box() {
        let dir = temp_dir("picker-unavailable");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let mut schema = TestSchema::setting(
            "woodcuttingResources",
            "string[]",
            None,
            Some("Woodcutting resources"),
            &[],
        );
        schema.set_options_from("gather:woodcutting");
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn scalar_fishing_cycle_skips_refused_groups_and_wraps() {
        let dir = temp_dir("fishing-cycle");
        let store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let mut schema =
            TestSchema::setting("fishingMethod", "string", None, Some("Fishing method"), &[]);
        schema.set_options_from("gather:fishing");
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
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
                schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn gather_site_picker_filters_and_accepts_explicit_selection() {
        let dir = temp_dir("site-picker");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let schema = TestSchema::gatherer();
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
        bag.insert("skill".into(), serde_json::json!("Woodcutting"));
        bag.insert("woodcuttingResources".into(), serde_json::json!(["normal"]));
        bag.insert("location".into(), serde_json::json!("Site"));
        bag.insert("site".into(), serde_json::json!("woodcutting.draynor"));
        let site_index = schema
            .schema_ref()
            .iter()
            .position(|field| field.id() == "site")
            .expect("Gatherer schema includes the named site setting");
        let visible_cursor = schema
            .schema_ref()
            .iter()
            .filter(|field| field.visible(&bag))
            .position(|field| field.id() == "site")
            .expect("the Site setting is visible in Site mode");
        let options = frontend_core::scripts::resolve_parameter_options(
            schema.schema_ref().field(site_index).unwrap(),
            &bag,
            &loadouts,
            Some(data.as_ref()),
        );
        let varrock = options
            .selectable()
            .iter()
            .enumerate()
            .filter(|(index, _)| options.matches_query(*index, "varrock"))
            .count();
        assert!(
            varrock > 0 && varrock < options.selectable().len(),
            "the filter narrows the site list to the rows that match: {varrock} of {}",
            options.selectable().len()
        );
        let original = bag.get("site").cloned().unwrap();
        let mut state = ParamsState {
            open: true,
            cursor: visible_cursor,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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
                schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn invalid_gather_site_remains_disabled_and_explains_why() {
        let dir = temp_dir("site-disabled");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let schema = TestSchema::gatherer();
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
        bag.insert("skill".into(), serde_json::json!("Mining"));
        bag.insert("miningResources".into(), serde_json::json!(["rune stones"]));
        bag.insert("location".into(), serde_json::json!("Site"));
        bag.insert("site".into(), serde_json::json!("mining.varrock_east.se"));
        let visible_cursor = schema
            .schema_ref()
            .iter()
            .filter(|field| field.visible(&bag))
            .position(|field| field.id() == "site")
            .expect("the Site setting is visible in Site mode");
        let original = bag.get("site").cloned();
        let mut state = ParamsState {
            open: true,
            cursor: visible_cursor,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                        schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn incompatible_saved_site_reason_is_visible_with_choices_at_80_columns() {
        let dir = temp_dir("site-reason-with-choices");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let schema = TestSchema::gatherer();
        let willow = data
            .gather_option("woodcutting", "willow")
            .expect("Willow resolves to a selected gathering key");
        let saved_site = data
            .gather_sites_for("woodcutting")
            .find(|site| !site.keys.iter().any(|key| key.key == willow.key))
            .expect("a named site does not offer Willow");
        let saved_site_id = saved_site.id.clone();
        let mut bag = store.merged_bag(CardSource::Catalog, "Gatherer", &schema, None);
        bag.insert("skill".into(), serde_json::json!("Woodcutting"));
        bag.insert("woodcuttingResources".into(), serde_json::json!(["willow"]));
        bag.insert("location".into(), serde_json::json!("Site"));
        bag.insert("site".into(), serde_json::json!(saved_site_id));
        let site_index = schema
            .schema_ref()
            .iter()
            .position(|field| field.id() == "site")
            .expect("Gatherer schema includes the named site setting");
        let visible_cursor = schema
            .schema_ref()
            .iter()
            .filter(|field| field.visible(&bag))
            .position(|field| field.id() == "site")
            .expect("the Site setting is visible in Site mode");
        let options = frontend_core::scripts::resolve_parameter_options(
            schema.schema_ref().field(site_index).unwrap(),
            &bag,
            &loadouts,
            Some(data.as_ref()),
        );
        assert!(!options.is_empty(), "Willow has alternate named sites");
        assert_eq!(options.preserved(), 1);
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
                    schema: schema.schema_ref(),
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
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn quester_picker_keeps_unknown_path_visible_and_removable() {
        let dir = temp_dir("quest-unknown");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let mut schema = TestSchema::setting(
            "quests",
            "string[]",
            None,
            Some("Quests"),
            &["cook", "sheep"],
        );
        schema.set_option_labels(&["Cook's Assistant", "Sheep Shearer"]);
        schema.set_options_from("released-paths");
        let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
        bag.insert("quests".into(), serde_json::json!(["removed-path"]));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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
            schema: schema.schema_ref(),
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Quester"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        let options = pane.resolved_options(schema.schema_ref().field(0).unwrap());
        pane.state.choice_cursor = options
            .values()
            .iter()
            .position(|value| value == "removed-path")
            .unwrap();
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert!(pane.state.choice_selected.is_empty());
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(pane.bag.get("quests"), Some(&serde_json::json!([])));
    }

    #[cfg(feature = "js")]
    #[test]
    fn unavailable_quest_is_never_added_by_the_real_picker() {
        let dir = temp_dir("quest-unavailable-add");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let schema = TestSchema::quester_card();
        let quests_row = schema
            .schema_ref()
            .iter()
            .position(|field| field.id() == "quests")
            .unwrap();
        let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
        bag.insert("quests".into(), serde_json::json!(["cook"]));
        let mut state = ParamsState {
            open: true,
            cursor: quests_row,
            ..Default::default()
        };
        let mut pane = ParamsPane {
            schema: schema.schema_ref(),
            bag: &mut bag,
            commit: &mut store_commit(&mut store, "Quester"),
            loadouts: &loadouts,
            game_data: None,
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
        let options = pane.resolved_options(schema.schema_ref().field(quests_row).unwrap());
        pane.state.choice_cursor = options
            .values()
            .iter()
            .position(|value| value == "hauntedmine")
            .unwrap();
        pane.on_key(KeyCode::Char(' '));
        assert_eq!(pane.state.choice_selected, ["cook"]);
        assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
        assert_eq!(pane.bag.get("quests"), Some(&serde_json::json!(["cook"])));
    }

    #[cfg(feature = "js")]
    #[test]
    fn stored_unavailable_quest_reason_is_not_clipped_at_80_columns() {
        let dir = temp_dir("quest-unavailable-reason");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let schema = TestSchema::quester_card();
        let reason = unavailable_quest("hauntedmine").unwrap();
        let full = format!("Haunted Mine — {reason}")
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        let quests_row = schema
            .schema_ref()
            .iter()
            .position(|field| field.id() == "quests")
            .unwrap();
        let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: quests_row,
            ..Default::default()
        };

        let render = |bag: &mut serde_json::Map<String, serde_json::Value>,
                      state: &mut ParamsState,
                      store: &mut TestStore| {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| {
                    let pane = ParamsPane {
                        schema: schema.schema_ref(),
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
                schema: schema.schema_ref(),
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

    #[cfg(feature = "js")]
    #[test]
    fn empty_loadout_source_shows_a_clear_disabled_state() {
        let dir = temp_dir("empty-loadout-options");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let mut schema = TestSchema::setting("loadout", "string", None, Some("Loadout"), &[]);
        schema.set_options_from("loadouts");
        let mut bag = store.merged_bag(CardSource::Catalog, "Thiever", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
            ..Default::default()
        };
        {
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
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
                    schema: schema.schema_ref(),
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
    #[cfg(feature = "js")]
    #[test]
    fn preserved_only_priority_array_opens_and_removal_is_persisted() {
        for (label, open_key) in [("enter", KeyCode::Enter), ("space", KeyCode::Char(' '))] {
            let dir = temp_dir(&format!("preserved-priority-{label}"));
            let path = dir.join("script-settings.json");
            let mut store = TestStore::at(path.clone());
            store.set_value(
                CardSource::Catalog,
                "Quester",
                "quests",
                serde_json::json!(["hauntedmine"]),
            );
            store.set_value(
                CardSource::Catalog,
                "Quester",
                "order_override",
                serde_json::json!(["hauntedmine"]),
            );
            store.save().unwrap();

            let loadouts = Loadouts::default();
            let schema = TestSchema::quester_card();
            let row = schema
                .schema_ref()
                .iter()
                .position(|field| field.id() == "order_override")
                .unwrap();
            let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
            let mut state = ParamsState {
                open: true,
                cursor: row,
                ..Default::default()
            };

            {
                let mut pane = ParamsPane {
                    schema: schema.schema_ref(),
                    bag: &mut bag,
                    commit: &mut store_commit(&mut store, "Quester"),
                    loadouts: &loadouts,
                    game_data: None,
                    state: &mut state,
                };
                assert_eq!(
                    pane.on_key(open_key),
                    ParamsKey::Edit,
                    "{label} should open the removal-only picker"
                );
                assert!(pane.state.multi_select);
                assert_eq!(pane.state.choice_selected, ["hauntedmine"]);

                let options = pane.resolved_options(schema.schema_ref().field(row).unwrap());
                let visible = pane.choice_indices(&options);
                pane.state.choice_cursor = visible
                    .iter()
                    .position(|index| options.values()[*index] == "hauntedmine")
                    .expect("the saved unavailable priority stays visible");
                assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
                assert!(pane.state.choice_selected.is_empty());
                assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Saved);
                assert_eq!(pane.bag.get("order_override"), Some(&serde_json::json!([])));
            }

            let reloaded = TestStore::at(path);
            let persisted = reloaded.merged_bag(CardSource::Catalog, "Quester", &schema, None);
            assert_eq!(
                persisted.get("order_override"),
                Some(&serde_json::json!([])),
                "{label} removal is durable"
            );
            assert_eq!(
                persisted.get("quests"),
                Some(&serde_json::json!(["hauntedmine"])),
                "removing the priority does not rewrite the quest selection"
            );
        }
    }

    #[cfg(feature = "js")]
    #[test]
    fn closed_removable_preserved_array_shows_its_value_and_reason() {
        let dir = temp_dir("preserved-priority-closed-row");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let schema = TestSchema::quester_card();
        let row = schema
            .schema_ref()
            .iter()
            .position(|field| field.id() == "order_override")
            .unwrap();
        let reason = unavailable_quest("hauntedmine").unwrap();
        let mut bag = store.merged_bag(CardSource::Catalog, "Quester", &schema, None);
        bag.insert("quests".into(), serde_json::json!(["hauntedmine"]));
        bag.insert("order_override".into(), serde_json::json!(["hauntedmine"]));
        let mut state = ParamsState {
            open: true,
            cursor: row,
            ..Default::default()
        };
        for (width, height) in [(80, 24), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let pane = ParamsPane {
                        schema: schema.schema_ref(),
                        bag: &mut bag,
                        commit: &mut store_commit(&mut store, "Quester"),
                        loadouts: &loadouts,
                        game_data: None,
                        state: &mut state,
                    };
                    frame.render_widget(pane, frame.area());
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                !text.contains("Order override: options unavailable"),
                "{width}x{height}: a removable row is painted as disabled: {text}"
            );
            let compact = text
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>();
            let detail = format!("Haunted Mine — {reason}")
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>();
            assert!(
                compact.contains(&detail),
                "{width}x{height}: the saved row's reason is missing: {text}"
            );
        }
    }

    #[cfg(feature = "js")]
    #[test]
    fn restart_badge_is_visible_at_reference_and_compact_sizes() {
        let dir = temp_dir("restart-badge");
        let mut store = TestStore::at(dir.join("script-settings.json"));
        let loadouts = Loadouts::default();
        let schema = TestSchema::setting("skill", "string", Some("Fishing"), Some("Skill"), &[]);
        let mut bag = serde_json::Map::new();
        let mut state = ParamsState {
            open: true,
            restart_badge: Some("Restart required to apply saved settings.".into()),
            ..Default::default()
        };
        for (width, height) in [(120, 40), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let pane = ParamsPane {
                        schema: schema.schema_ref(),
                        bag: &mut bag,
                        commit: &mut store_commit(&mut store, "Gatherer"),
                        loadouts: &loadouts,
                        game_data: None,
                        state: &mut state,
                    };
                    frame.render_widget(pane, frame.area());
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                text.contains("Restart required to apply saved settings."),
                "{width}x{height}: missing restart badge: {text}"
            );
        }
    }

    #[test]
    fn cookbot_labels_ranges_and_live_validation_are_readable_at_both_sizes() {
        let loadouts = Loadouts::default();
        let labels = [
            "Raw fish to cook",
            "Fire ground search radius",
            "Bank stand tile",
            "Range stand tile",
            "Openable obstacles",
            "Cook surface search radius",
        ];
        let mut schema = TestSchema::empty();
        for (index, label) in labels.iter().enumerate() {
            schema.add_setting(
                &format!("field{index}"),
                "number",
                Some("12"),
                Some(label),
                &[],
            );
            schema.set_min("2").set_max("20");
        }
        for (width, height) in [(80, 24), (120, 40)] {
            let mut bag = serde_json::Map::new();
            let mut state = ParamsState {
                open: true,
                cursor: 5,
                ..Default::default()
            };
            let mut commit = |_: &str, _: serde_json::Value, _: Option<serde_json::Value>| {
                panic!("an invalid draft must not be saved")
            };
            let mut pane = ParamsPane {
                schema: schema.schema_ref(),
                bag: &mut bag,
                commit: &mut commit,
                loadouts: &loadouts,
                game_data: None,
                state: &mut state,
            };
            assert_eq!(pane.on_key(KeyCode::Enter), ParamsKey::Edit);
            type_replace(&mut pane, "40");
            let error = pane.state.error.clone().expect("validate before Enter");
            assert!(
                pane.state.editing,
                "the form stays open on the invalid draft"
            );
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(pane, frame.area()))
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
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>();
            for label in labels {
                assert!(
                    compact.contains(
                        &label
                            .chars()
                            .filter(|ch| ch.is_alphanumeric())
                            .collect::<String>()
                    ),
                    "{width}x{height}: missing full label {label}: {text}"
                );
            }
            assert!(
                text.contains("2–20"),
                "{width}x{height}: bounds missing: {text}"
            );
            assert!(
                compact.contains(
                    &error
                        .chars()
                        .filter(|ch| ch.is_alphanumeric())
                        .collect::<String>()
                ),
                "{width}x{height}: live validation missing: {text}"
            );
        }
    }
}

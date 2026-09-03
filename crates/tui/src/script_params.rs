//! Script parameter editors for the TUI script pane (schema-driven).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use script::{
    resolve_setting_options, setting_visible, LoadoutsStore, ScriptSettingsStore, ScriptSource,
    SettingDef,
};

/// Mutable params-pane state: whether the form is open and the cursor row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamsState {
    pub open: bool,
    pub cursor: usize,
}

/// Outcome of a params key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamsKey {
    Toggle,
    Up,
    Down,
    Close,
    None,
}

/// The script params popup over a card's settings schema.
pub struct ParamsPane<'a> {
    pub schema: &'a [SettingDef],
    pub bag: &'a mut serde_json::Map<String, serde_json::Value>,
    pub store: &'a mut ScriptSettingsStore,
    pub loadouts: &'a LoadoutsStore,
    pub source: ScriptSource,
    pub name: &'a str,
    pub state: &'a mut ParamsState,
}

impl<'a> ParamsPane<'a> {
    pub fn visible_rows(&self) -> Vec<&'a SettingDef> {
        self.schema
            .iter()
            .filter(|d| setting_visible(d.show_if.as_deref(), self.bag))
            .collect()
    }

    pub fn on_key(&mut self, code: crossterm::event::KeyCode) -> ParamsKey {
        let rows = self.visible_rows();
        if rows.is_empty() {
            return ParamsKey::Close;
        }
        match code {
            crossterm::event::KeyCode::Esc => ParamsKey::Close,
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
            crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char(' ') => {
                if let Some(def) = rows.get(self.state.cursor) {
                    if def.ty == "boolean" {
                        let cur = self
                            .bag
                            .get(&def.id)
                            .and_then(|v| v.as_bool())
                            .unwrap_or_else(|| def.default.as_deref() == Some("true"));
                        let next = !cur;
                        self.store.set_bool(self.source, self.name, &def.id, next);
                        let _ = self.store.save();
                        self.bag.insert(def.id.clone(), serde_json::json!(next));
                        return ParamsKey::Toggle;
                    }
                    let opts = resolve_setting_options(def, self.loadouts);
                    if def.ty == "string" && !opts.is_empty() {
                        let cur = self
                            .bag
                            .get(&def.id)
                            .and_then(|v| v.as_str())
                            .or(def.default.as_deref())
                            .unwrap_or("")
                            .to_string();
                        let next = opts
                            .iter()
                            .position(|o| o == &cur)
                            .map(|i| opts[(i + 1) % opts.len()].clone())
                            .unwrap_or_else(|| opts[0].clone());
                        self.store.set_str(self.source, self.name, &def.id, &next);
                        let _ = self.store.save();
                        self.bag.insert(def.id.clone(), serde_json::json!(next));
                        return ParamsKey::Toggle;
                    }
                }
                ParamsKey::None
            }
            _ => ParamsKey::None,
        }
    }
}

impl Widget for &ParamsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rows = self.visible_rows();
        if rows.is_empty() {
            return;
        }
        let cursor = self.state.cursor.min(rows.len().saturating_sub(1));
        let mut lines = vec![Line::from("parameters (space toggles bool)")];
        let mut last_group: Option<&str> = None;
        for (i, def) in rows.iter().enumerate() {
            if def.group.as_deref() != last_group {
                last_group = def.group.as_deref();
                if let Some(g) = last_group {
                    lines.push(Line::from(Span::raw(format!("— {g} —"))));
                }
            }
            let label = def.label.as_deref().unwrap_or(&def.id);
            let mark = if i == cursor { "> " } else { "  " };
            let value = match def.ty.as_str() {
                "boolean" => {
                    let v = self
                        .bag
                        .get(&def.id)
                        .and_then(|x| x.as_bool())
                        .unwrap_or_else(|| def.default.as_deref() == Some("true"));
                    v.to_string()
                }
                _ => self
                    .bag
                    .get(&def.id)
                    .map(|x| x.to_string())
                    .unwrap_or_else(|| "—".into()),
            };
            lines.push(Line::from(format!("{mark}{label}: {value}")));
        }
        let block = Block::default().borders(Borders::ALL).title("parameters");
        Paragraph::new(lines).block(block).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use script::SettingDef;

    fn bury_schema() -> Vec<SettingDef> {
        vec![SettingDef {
            id: "buryBones".into(),
            ty: "boolean".into(),
            default: Some("true".into()),
            label: Some("Bury bones".into()),
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
        }]
    }

    #[test]
    fn params_pane_space_toggles_a_bool_param() {
        let dir =
            std::env::temp_dir().join(format!("274bot-tui-params-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script-settings.json");
        let mut store = ScriptSettingsStore::at(path);
        let loadouts = LoadoutsStore::at(dir.join("loadouts.json"));
        let schema = bury_schema();
        let mut bag = store.merged_bag(ScriptSource::Catalog, "ChickenKiller", &schema, None);
        assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(true)));
        let mut state = ParamsState {
            open: true,
            cursor: 0,
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            store: &mut store,
            loadouts: &loadouts,
            source: ScriptSource::Catalog,
            name: "ChickenKiller",
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(bag.get("buryBones"), Some(&serde_json::json!(false)));
    }

    #[test]
    fn params_pane_space_cycles_loadout_combo_from_store() {
        let dir =
            std::env::temp_dir().join(format!("274bot-tui-loadout-combo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings_path = dir.join("script-settings.json");
        let loadouts_path = dir.join("loadouts.json");
        let mut loadouts = LoadoutsStore::at(loadouts_path);
        loadouts.upsert(script::Loadout {
            name: "fish".into(),
            worn: vec![],
            carry: vec![],
        });
        loadouts.upsert(script::Loadout {
            name: "mine".into(),
            worn: vec![],
            carry: vec![],
        });
        loadouts.save().unwrap();
        let schema = vec![SettingDef {
            id: "loadout".into(),
            ty: "string".into(),
            default: Some("fish".into()),
            label: Some("Loadout".into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: Some("loadouts".into()),
            csv_toggle: None,
            help: None,
        }];
        let mut store = ScriptSettingsStore::at(settings_path);
        let mut bag = store.merged_bag(ScriptSource::Catalog, "Thiever", &schema, None);
        let mut state = ParamsState {
            open: true,
            cursor: 0,
        };
        let mut pane = ParamsPane {
            schema: &schema,
            bag: &mut bag,
            store: &mut store,
            loadouts: &loadouts,
            source: ScriptSource::Catalog,
            name: "Thiever",
            state: &mut state,
        };
        assert_eq!(pane.on_key(KeyCode::Char(' ')), ParamsKey::Toggle);
        assert_eq!(bag.get("loadout"), Some(&serde_json::json!("mine")));
    }
}

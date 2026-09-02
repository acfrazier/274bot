//! Loadouts popup: CRUD for `{ name, worn, carry }` presets.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use script::{Loadout, LoadoutsStore};

/// Mutable loadouts-pane state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadoutsState {
    pub open: bool,
    /// 0 = list row, 1 = name, 2 = worn, 3 = carry, 4 = add, 5 = delete.
    pub row: usize,
    pub sel: usize,
    pub name_scratch: String,
    pub worn_scratch: String,
    pub carry_scratch: String,
}

/// Outcome of one loadouts key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadoutsKey {
    Changed,
    Consumed,
    Ignored,
}

pub struct LoadoutsPane<'a> {
    pub store: &'a mut LoadoutsStore,
    pub state: &'a mut LoadoutsState,
}

impl<'a> LoadoutsPane<'a> {
    pub fn sync_scratch_from_selection(&mut self) {
        if let Some(loadout) = self.store.loadouts().get(self.state.sel) {
            self.state.name_scratch = loadout.name.clone();
            self.state.worn_scratch = loadout.worn.join(", ");
            self.state.carry_scratch = loadout.carry.join(", ");
        } else {
            self.state.name_scratch.clear();
            self.state.worn_scratch.clear();
            self.state.carry_scratch.clear();
        }
    }

    fn apply_scratch(&mut self) {
        let name = self.state.name_scratch.trim();
        if name.is_empty() {
            return;
        }
        let worn = split_csv(&self.state.worn_scratch);
        let carry = split_csv(&self.state.carry_scratch);
        self.store.upsert(Loadout {
            name: name.to_string(),
            worn,
            carry,
        });
        let _ = self.store.save();
    }

    pub fn on_key(&mut self, key: KeyEvent) -> LoadoutsKey {
        let count = self.store.loadouts().len();
        match key.code {
            KeyCode::Esc => {
                self.state.open = false;
                LoadoutsKey::Consumed
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.row = self.state.row.saturating_sub(1);
                LoadoutsKey::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.row = (self.state.row + 1).min(5);
                LoadoutsKey::Consumed
            }
            KeyCode::Tab => {
                self.state.row = (self.state.row + 1).min(5);
                LoadoutsKey::Consumed
            }
            KeyCode::BackTab => {
                self.state.row = self.state.row.saturating_sub(1);
                LoadoutsKey::Consumed
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.state.row {
                0 if count > 0 => {
                    self.state.sel = (self.state.sel + 1) % count;
                    self.sync_scratch_from_selection();
                    LoadoutsKey::Consumed
                }
                1..=3 => {
                    self.apply_scratch();
                    LoadoutsKey::Changed
                }
                4 => {
                    let next = format!("loadout-{}", count + 1);
                    self.store.upsert(Loadout {
                        name: next.clone(),
                        worn: vec![],
                        carry: vec![],
                    });
                    let _ = self.store.save();
                    self.state.sel = self.store.loadouts().len().saturating_sub(1);
                    self.sync_scratch_from_selection();
                    LoadoutsKey::Changed
                }
                5 => {
                    if let Some(loadout) = self.store.loadouts().get(self.state.sel) {
                        let name = loadout.name.clone();
                        self.store.remove(&name);
                        let _ = self.store.save();
                        if self.state.sel >= self.store.loadouts().len() {
                            self.state.sel = self.store.loadouts().len().saturating_sub(1);
                        }
                        self.sync_scratch_from_selection();
                        LoadoutsKey::Changed
                    } else {
                        LoadoutsKey::Consumed
                    }
                }
                _ => LoadoutsKey::Consumed,
            },
            KeyCode::Char(c) if self.state.row == 1 => {
                self.state.name_scratch.push(c);
                LoadoutsKey::Consumed
            }
            KeyCode::Char(c) if self.state.row == 2 => {
                self.state.worn_scratch.push(c);
                LoadoutsKey::Consumed
            }
            KeyCode::Char(c) if self.state.row == 3 => {
                self.state.carry_scratch.push(c);
                LoadoutsKey::Consumed
            }
            KeyCode::Backspace if (1..=3).contains(&self.state.row) => {
                match self.state.row {
                    1 => {
                        self.state.name_scratch.pop();
                    }
                    2 => {
                        self.state.worn_scratch.pop();
                    }
                    _ => {
                        self.state.carry_scratch.pop();
                    }
                }
                LoadoutsKey::Consumed
            }
            _ => LoadoutsKey::Ignored,
        }
    }

    pub fn popup_rect(area: Rect) -> Rect {
        let w = area.width.min(44);
        let h = 12.min(area.height);
        Rect {
            x: area.x + area.width.saturating_sub(w) / 2,
            y: area.y + area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl Widget for LoadoutsPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.open {
            return;
        }
        let popup = Self::popup_rect(area);
        Clear.render(popup, buf);
        let block = Block::default().borders(Borders::ALL).title("loadouts");
        let inner = block.inner(popup);
        block.render(popup, buf);
        let names: Vec<String> = self.store.names();
        let current = names
            .get(self.state.sel)
            .cloned()
            .unwrap_or_else(|| "(none)".into());
        let rows = [
            ("selected", current),
            ("name", self.state.name_scratch.clone()),
            ("worn", self.state.worn_scratch.clone()),
            ("carry", self.state.carry_scratch.clone()),
            ("", "[add loadout]".into()),
            ("", "[delete selected]".into()),
        ];
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .map(|(i, (label, value))| {
                let marker = if i == self.state.row { "> " } else { "  " };
                if label.is_empty() {
                    Line::from(format!("{marker}{value}"))
                } else {
                    Line::from(format!("{marker}{label}: {value}"))
                }
            })
            .collect();
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use script::{Loadout, LoadoutsStore};

    use super::{LoadoutsKey, LoadoutsPane, LoadoutsState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn tmp_store() -> LoadoutsStore {
        let dir = std::env::temp_dir().join(format!(
            "274bot-tui-loadouts-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loadouts.json");
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout {
            name: "fish".into(),
            worn: vec![],
            carry: vec!["net".into()],
        });
        store.save().unwrap();
        store
    }

    #[test]
    fn loadouts_popup_adds_a_preset_and_lists_names() {
        let mut store = tmp_store();
        let mut state = LoadoutsState {
            open: true,
            row: 4,
            ..Default::default()
        };
        {
            let mut pane = LoadoutsPane {
                store: &mut store,
                state: &mut state,
            };
            pane.sync_scratch_from_selection();
            assert_eq!(pane.on_key(key(KeyCode::Enter)), LoadoutsKey::Changed);
        }
        assert_eq!(store.names().len(), 2);
        assert!(store.names().iter().any(|n| n.starts_with("loadout-")));
    }

    #[test]
    fn loadouts_popup_draws_while_open() {
        let mut store = tmp_store();
        let mut state = LoadoutsState {
            open: true,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        {
            let mut pane = LoadoutsPane {
                store: &mut store,
                state: &mut state,
            };
            pane.sync_scratch_from_selection();
            terminal
                .draw(|frame| frame.render_widget(pane, frame.area()))
                .unwrap();
        }
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("loadouts"), "popup paints: {text:?}");
        assert!(text.contains("fish"), "selected loadout paints: {text:?}");
    }
}

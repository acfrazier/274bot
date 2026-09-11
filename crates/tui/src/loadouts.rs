//! Loadouts popup: CRUD for slot-keyed worn gear and quantity carry.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use script::{
    unique_loadout_name, worn_slot_label, CarryEntry, Loadout, LoadoutsStore, WORN_SLOTS,
};

/// Mutable loadouts-pane state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadoutsState {
    pub open: bool,
    /// 0 selected, 1 name, 2 worn slots, 3 unassigned, 4 carry, 5 add,
    /// 6 duplicate, 7 delete, 8 save.
    pub row: usize,
    pub sel: usize,
    pub name_scratch: String,
    pub worn_scratch: String,
    pub unassigned_scratch: String,
    pub carry_scratch: String,
    pub status: String,
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
            self.state.worn_scratch = format_worn(loadout);
            self.state.unassigned_scratch = loadout.unassigned.join(", ");
            self.state.carry_scratch = format_carry(loadout);
        } else {
            self.state.name_scratch.clear();
            self.state.worn_scratch.clear();
            self.state.unassigned_scratch.clear();
            self.state.carry_scratch.clear();
        }
    }

    fn commit_scratch(&mut self) -> bool {
        let name = self.state.name_scratch.trim();
        if name.is_empty() {
            self.state.status = "Name is required.".into();
            return false;
        }
        let backup = self.store.snapshot();
        let loadout = Loadout {
            name: name.to_string(),
            worn: parse_worn(&self.state.worn_scratch),
            unassigned: split_csv(&self.state.unassigned_scratch),
            carry: parse_carry(&self.state.carry_scratch),
        };
        if self.state.sel < self.store.loadouts().len() {
            self.store.replace_at(self.state.sel, loadout);
        } else {
            self.store.upsert(loadout);
        }
        match self.store.save() {
            Ok(()) => {
                self.state.status = "Saved.".into();
                true
            }
            Err(err) => {
                self.store.restore(backup);
                self.state.status = format!("Save failed: {err}");
                false
            }
        }
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
                self.state.row = (self.state.row + 1).min(8);
                LoadoutsKey::Consumed
            }
            KeyCode::Tab => {
                self.state.row = (self.state.row + 1).min(8);
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
                1..=4 => {
                    if self.commit_scratch() {
                        LoadoutsKey::Changed
                    } else {
                        LoadoutsKey::Consumed
                    }
                }
                5 => {
                    let next = unique_loadout_name(self.store.loadouts(), "loadout");
                    let backup = self.store.snapshot();
                    self.store.upsert(Loadout::new(next));
                    match self.store.save() {
                        Ok(()) => {
                            self.state.sel = self.store.loadouts().len().saturating_sub(1);
                            self.sync_scratch_from_selection();
                            self.state.status = "Saved.".into();
                            LoadoutsKey::Changed
                        }
                        Err(err) => {
                            self.store.restore(backup);
                            self.state.status = format!("Save failed: {err}");
                            LoadoutsKey::Consumed
                        }
                    }
                }
                6 => {
                    if let Some(src) = self.store.loadouts().get(self.state.sel).cloned() {
                        let mut dup = src.clone();
                        dup.name = unique_loadout_name(
                            self.store.loadouts(),
                            &format!("{} copy", src.name),
                        );
                        let backup = self.store.snapshot();
                        self.store.upsert(dup);
                        match self.store.save() {
                            Ok(()) => {
                                self.state.sel = self.store.loadouts().len().saturating_sub(1);
                                self.sync_scratch_from_selection();
                                self.state.status = "Saved.".into();
                                LoadoutsKey::Changed
                            }
                            Err(err) => {
                                self.store.restore(backup);
                                self.state.status = format!("Save failed: {err}");
                                LoadoutsKey::Consumed
                            }
                        }
                    } else {
                        LoadoutsKey::Consumed
                    }
                }
                7 => {
                    if let Some(loadout) = self.store.loadouts().get(self.state.sel) {
                        let name = loadout.name.clone();
                        let backup = self.store.snapshot();
                        self.store.remove(&name);
                        match self.store.save() {
                            Ok(()) => {
                                if self.state.sel >= self.store.loadouts().len() {
                                    self.state.sel = self.store.loadouts().len().saturating_sub(1);
                                }
                                self.sync_scratch_from_selection();
                                self.state.status = "Saved.".into();
                                LoadoutsKey::Changed
                            }
                            Err(err) => {
                                self.store.restore(backup);
                                self.state.status = format!("Save failed: {err}");
                                LoadoutsKey::Consumed
                            }
                        }
                    } else {
                        LoadoutsKey::Consumed
                    }
                }
                8 => {
                    if self.commit_scratch() {
                        LoadoutsKey::Changed
                    } else {
                        LoadoutsKey::Consumed
                    }
                }
                _ => LoadoutsKey::Consumed,
            },
            KeyCode::Char(c) if (1..=4).contains(&self.state.row) => {
                match self.state.row {
                    1 => self.state.name_scratch.push(c),
                    2 => self.state.worn_scratch.push(c),
                    3 => self.state.unassigned_scratch.push(c),
                    _ => self.state.carry_scratch.push(c),
                }
                LoadoutsKey::Consumed
            }
            KeyCode::Backspace if (1..=4).contains(&self.state.row) => {
                match self.state.row {
                    1 => {
                        self.state.name_scratch.pop();
                    }
                    2 => {
                        self.state.worn_scratch.pop();
                    }
                    3 => {
                        self.state.unassigned_scratch.pop();
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
        let w = area.width.min(56);
        let h = 16.min(area.height);
        Rect {
            x: area.x + area.width.saturating_sub(w) / 2,
            y: area.y + area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }
}

fn format_worn(loadout: &Loadout) -> String {
    WORN_SLOTS
        .iter()
        .filter_map(|slot| loadout.worn.get(*slot).map(|item| format!("{slot}={item}")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_worn(raw: &str) -> std::collections::BTreeMap<String, String> {
    let mut worn = std::collections::BTreeMap::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((slot, item)) = part.split_once('=') else {
            continue;
        };
        let slot = slot.trim();
        let item = item.trim();
        if WORN_SLOTS.contains(&slot) && !item.is_empty() {
            worn.insert(slot.to_string(), item.to_string());
        }
    }
    worn
}

fn format_carry(loadout: &Loadout) -> String {
    loadout
        .carry
        .iter()
        .map(|entry| format!("{}:{}", entry.item, entry.qty))
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_carry(raw: &str) -> Vec<CarryEntry> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|part| {
            if let Some((item, qty)) = part.split_once(':') {
                let item = item.trim();
                if item.is_empty() {
                    return None;
                }
                let qty = qty.trim().parse::<u32>().ok().filter(|n| *n > 0)?;
                Some(CarryEntry::new(item, qty))
            } else {
                Some(CarryEntry::new(part, 1))
            }
        })
        .collect()
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
        let slot_hint = WORN_SLOTS
            .iter()
            .map(|slot| worn_slot_label(slot))
            .collect::<Vec<_>>()
            .join("/");
        let rows = [
            ("selected", current),
            ("name", self.state.name_scratch.clone()),
            ("worn slot=item", self.state.worn_scratch.clone()),
            ("unassigned", self.state.unassigned_scratch.clone()),
            ("carry item:qty", self.state.carry_scratch.clone()),
            ("", "[new]".into()),
            ("", "[duplicate]".into()),
            ("", "[delete selected]".into()),
            ("", "[save]".into()),
        ];
        let mut lines: Vec<Line> = rows
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
        if !self.state.status.is_empty() {
            lines.push(Line::from(format!("  {}", self.state.status)));
        }
        lines.push(Line::from(format!("  slots {slot_hint}")));
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use script::{CarryEntry, Loadout, LoadoutsStore};

    use super::{
        format_carry, format_worn, parse_carry, parse_worn, LoadoutsKey, LoadoutsPane,
        LoadoutsState,
    };

    static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn tmp_store() -> LoadoutsStore {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-tui-loadouts-{n}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loadouts.json");
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout::new("fish").with_carry("net", 1));
        store.save().unwrap();
        store
    }

    #[test]
    fn loadouts_popup_adds_a_preset_and_lists_names() {
        let mut store = tmp_store();
        let mut state = LoadoutsState {
            open: true,
            row: 5,
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
        assert!(store.names().iter().any(|n| n.starts_with("loadout")));
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

    #[test]
    fn apply_scratch_renames_in_place() {
        let mut store = LoadoutsStore::at({
            let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "274bot-tui-loadout-rename-{n}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir.join("loadouts.json")
        });
        store.upsert(Loadout::new("melee"));

        let mut state = LoadoutsState {
            open: true,
            sel: 0,
            row: 1,
            name_scratch: "melee2".into(),
            ..Default::default()
        };
        let changed = {
            let mut pane = LoadoutsPane {
                store: &mut store,
                state: &mut state,
            };
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(changed, LoadoutsKey::Changed);
        assert_eq!(store.loadouts().len(), 1);
        assert_eq!(store.loadouts()[0].name, "melee2");
        assert_eq!(state.sel, 0);
        assert_eq!(state.status, "Saved.");
    }

    #[test]
    fn slot_and_quantity_text_round_trips() {
        let loadout = Loadout::new("melee")
            .with_slot("righthand", "Rune scimitar")
            .with_carry("Lobster", 10);
        let worn = format_worn(&loadout);
        let carry = format_carry(&loadout);
        assert_eq!(
            parse_worn(&worn).get("righthand").map(String::as_str),
            Some("Rune scimitar")
        );
        assert_eq!(parse_carry(&carry), vec![CarryEntry::new("Lobster", 10)]);
    }

    #[test]
    fn invalid_empty_name_does_not_clobber_store() {
        let mut store = tmp_store();
        let mut state = LoadoutsState {
            open: true,
            sel: 0,
            row: 1,
            name_scratch: " ".into(),
            ..Default::default()
        };
        let result = {
            let mut pane = LoadoutsPane {
                store: &mut store,
                state: &mut state,
            };
            pane.on_key(key(KeyCode::Enter))
        };
        assert_eq!(result, LoadoutsKey::Consumed);
        assert_eq!(store.loadouts()[0].name, "fish");
        assert_eq!(state.status, "Name is required.");
    }
}

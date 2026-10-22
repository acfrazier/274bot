//! Loadouts popup: CRUD for slot-keyed worn gear and quantity carry.
//!
//! The popup edits the core's loadout form
//! ([`frontend_core::scripts::LoadoutForm`], lent to the command seam). The
//! text rows are the popup's own typed text: when the operator commits one,
//! the text is parsed into a draft, staged in the form and saved by the
//! `SaveLoadout` row. A key returns what it asks of the session
//! ([`LoadoutsIntent`]); the session stages and submits it
//! (`TuiSession::loadouts_key`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use api::loadout::{CarryEntry, Loadout, WORN_SLOTS};
use frontend_core::scripts::{LoadoutForm, WornSlot};

/// Mutable loadouts-pane state: the popup's cursor and its typed text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadoutsState {
    pub open: bool,
    /// 0 selected, 1 name, 2 worn slots, 3 unassigned, 4 carry, 5 add,
    /// 6 duplicate, 7 delete, 8 save.
    pub row: usize,
    pub name_scratch: String,
    pub worn_scratch: String,
    pub unassigned_scratch: String,
    pub carry_scratch: String,
    /// The popup was just opened: the session's next look selects its first
    /// row and fills the text from it.
    pub reopened: bool,
}

/// What one loadouts key asks of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadoutsIntent {
    /// Nothing: the cursor moved, text was typed or erased, the popup
    /// closed, or the key is not the popup's.
    None,
    /// Select the next loadout (the `selected` row).
    SelectNext,
    /// Save the text rows as the selected loadout (Enter on a text row or
    /// on `[save]`).
    Save,
    /// Create a loadout (`[new]`).
    Create,
    /// Duplicate the selected loadout (`[duplicate]`).
    Duplicate,
    /// Delete the selected loadout (`[delete selected]`).
    Delete,
}

impl LoadoutsState {
    /// Open the popup: the text rows are blank and the session's next look
    /// selects the first loadout and fills them from it.
    pub fn reopen(&mut self) {
        self.open = true;
        self.reopened = true;
        self.name_scratch.clear();
        self.worn_scratch.clear();
        self.unassigned_scratch.clear();
        self.carry_scratch.clear();
    }

    /// Read the text rows again from the loadout `form` has selected (blank
    /// when it selects none).
    pub fn fill(&mut self, form: &LoadoutForm) {
        if let Some(loadout) = form.selected_row() {
            self.name_scratch.clone_from(&loadout.name);
            self.worn_scratch = format_worn(loadout);
            self.unassigned_scratch = loadout.unassigned.join(", ");
            self.carry_scratch = format_carry(loadout);
        } else {
            self.name_scratch.clear();
            self.worn_scratch.clear();
            self.unassigned_scratch.clear();
            self.carry_scratch.clear();
        }
    }

    /// The loadout the text rows spell.
    pub fn text_loadout(&self) -> Loadout {
        Loadout {
            name: self.name_scratch.trim().to_string(),
            worn: parse_worn(&self.worn_scratch),
            unassigned: split_csv(&self.unassigned_scratch),
            carry: parse_carry(&self.carry_scratch),
        }
    }

    /// Apply `key`: the cursor and the typed text change here; what the key
    /// asks of the session comes back. `count` is how many loadouts are
    /// saved.
    pub fn on_key(&mut self, key: KeyEvent, count: usize) -> LoadoutsIntent {
        match key.code {
            KeyCode::Esc => {
                self.open = false;
                LoadoutsIntent::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.row = self.row.saturating_sub(1);
                LoadoutsIntent::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.row = (self.row + 1).min(8);
                LoadoutsIntent::None
            }
            KeyCode::Tab => {
                self.row = (self.row + 1).min(8);
                LoadoutsIntent::None
            }
            KeyCode::BackTab => {
                self.row = self.row.saturating_sub(1);
                LoadoutsIntent::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.row {
                0 if count > 0 => LoadoutsIntent::SelectNext,
                1..=4 | 8 => LoadoutsIntent::Save,
                5 => LoadoutsIntent::Create,
                6 => LoadoutsIntent::Duplicate,
                7 => LoadoutsIntent::Delete,
                _ => LoadoutsIntent::None,
            },
            KeyCode::Char(c) if (1..=4).contains(&self.row) => {
                match self.row {
                    1 => self.name_scratch.push(c),
                    2 => self.worn_scratch.push(c),
                    3 => self.unassigned_scratch.push(c),
                    _ => self.carry_scratch.push(c),
                }
                LoadoutsIntent::None
            }
            KeyCode::Backspace if (1..=4).contains(&self.row) => {
                match self.row {
                    1 => {
                        self.name_scratch.pop();
                    }
                    2 => {
                        self.worn_scratch.pop();
                    }
                    3 => {
                        self.unassigned_scratch.pop();
                    }
                    _ => {
                        self.carry_scratch.pop();
                    }
                }
                LoadoutsIntent::None
            }
            _ => LoadoutsIntent::None,
        }
    }
}

/// The popup as drawn: the form's rows and status over the popup's own
/// state.
pub struct LoadoutsPane<'a> {
    pub form: &'a LoadoutForm,
    pub state: &'a LoadoutsState,
}

impl LoadoutsPane<'_> {
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
        let current = self
            .form
            .selected_row()
            .map(|row| row.name.clone())
            .unwrap_or_else(|| "(none)".into());
        let slot_hint = WornSlot::ALL
            .iter()
            .map(|slot| slot.label())
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
        if !self.form.status().is_empty() {
            lines.push(Line::from(format!("  {}", self.form.status())));
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
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use api::loadout::{CarryEntry, Loadout};
    use frontend_core::scripts::{LoadoutForm, Loadouts};
    use host_play::local_state::{LocalStateService, StateRoot};

    use super::{
        format_carry, format_worn, parse_carry, parse_worn, LoadoutsIntent, LoadoutsPane,
        LoadoutsState,
    };

    static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn scratch_service(label: &str) -> Arc<LocalStateService> {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-tui-loadouts-{label}-{n}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        LocalStateService::open_process(StateRoot::new(dir.join(".274bot")))
    }

    fn tmp_form() -> LoadoutForm {
        let service = scratch_service("store");
        let mut store = Loadouts::operator(&service);
        store.upsert_for_test(Loadout::new("fish").with_carry("net", 1));
        store.save_for_test().unwrap();
        LoadoutForm::over(store)
    }

    #[test]
    fn loadouts_popup_draws_while_open() {
        let form = tmp_form();
        let mut state = LoadoutsState {
            open: true,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        state.fill(&form);
        terminal
            .draw(|frame| {
                frame.render_widget(
                    LoadoutsPane {
                        form: &form,
                        state: &state,
                    },
                    frame.area(),
                )
            })
            .unwrap();
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

    /// The popup's text rows read from the selected loadout and spell the
    /// same loadout back; a loadout with nothing selected reads blank.
    #[test]
    fn the_text_rows_fill_from_the_selection_and_spell_it_back() {
        let service = scratch_service("fill");
        let mut store = Loadouts::operator(&service);
        let melee = Loadout::new("melee")
            .with_slot("righthand", "Rune scimitar")
            .with_carry("Lobster", 10);
        store.upsert_for_test(melee.clone());
        store.save_for_test().unwrap();
        let mut form = LoadoutForm::over(store);
        let mut state = LoadoutsState::default();

        state.fill(&form);
        assert_eq!(state.name_scratch, "melee");
        assert_eq!(state.worn_scratch, "righthand=Rune scimitar");
        assert_eq!(state.carry_scratch, "Lobster:10");
        assert_eq!(state.text_loadout(), melee);

        form.stage_selection(5);
        state.fill(&form);
        assert!(state.name_scratch.is_empty() && state.carry_scratch.is_empty());
    }

    #[test]
    fn keys_move_the_cursor_edit_text_and_name_what_they_ask() {
        let mut state = LoadoutsState {
            open: true,
            ..Default::default()
        };
        // Row 0 selects the next loadout only while there is one.
        assert_eq!(state.on_key(key(KeyCode::Enter), 0), LoadoutsIntent::None);
        assert_eq!(
            state.on_key(key(KeyCode::Enter), 2),
            LoadoutsIntent::SelectNext
        );

        // Typing goes into the row under the cursor; Backspace erases.
        assert_eq!(state.on_key(key(KeyCode::Down), 2), LoadoutsIntent::None);
        assert_eq!(state.row, 1);
        for c in "abc".chars() {
            assert_eq!(state.on_key(key(KeyCode::Char(c)), 2), LoadoutsIntent::None);
        }
        assert_eq!(
            state.on_key(key(KeyCode::Backspace), 2),
            LoadoutsIntent::None
        );
        assert_eq!(state.name_scratch, "ab");
        assert_eq!(state.on_key(key(KeyCode::Enter), 2), LoadoutsIntent::Save);

        // The buttons.
        for (row, intent) in [
            (5, LoadoutsIntent::Create),
            (6, LoadoutsIntent::Duplicate),
            (7, LoadoutsIntent::Delete),
            (8, LoadoutsIntent::Save),
        ] {
            state.row = row;
            assert_eq!(state.on_key(key(KeyCode::Enter), 2), intent, "row {row}");
        }

        // Esc closes the popup and asks for nothing.
        assert_eq!(state.on_key(key(KeyCode::Esc), 2), LoadoutsIntent::None);
        assert!(!state.open);
    }
}

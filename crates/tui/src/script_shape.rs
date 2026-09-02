//! Script pane (spec `2026-09-01-headless-tui-design.md`): the
//! Browse/Start/Pause/Stop/Load widgets, the Browse picker over the
//! registry card names, and the Load path input. The pane is a plain
//! widget over owned view data — it never calls [`SlotScript`]; clicks
//! map to [`crate::app::AppAction`]s that `tui-play` dispatches onto
//! `Play::script_start_load` / pause / stop.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use script::{RunState, SlotScript};

/// The script button labels, left to right.
pub const SCRIPT_BUTTONS: [&str; 5] = ["Browse", "Start", "Pause", "Stop", "Load"];

/// The lifecycle text for a [`RunState`].
pub fn run_state_text(state: RunState) -> &'static str {
    match state {
        RunState::Idle => "idle",
        RunState::Running => "running",
        RunState::Paused => "paused",
        RunState::Stopping => "stopping",
        RunState::Error => "error",
    }
}

/// One script-pane click result. The app maps buttons to [`AppAction`]s
/// and `Pick`s to the Browse selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptClick {
    /// A button row hit: one of [`SCRIPT_BUTTONS`].
    Button(&'static str),
    /// A Browse picker row hit: the row's index into `names`.
    Pick(usize),
    /// A miss.
    None,
}

/// The script pane. `slot` is a read-only handle the TUI never calls —
/// it exists so a test can prove clicking a button does not mutate a
/// [`SlotScript`]; the app only emits actions the binary dispatches.
pub struct ScriptPane<'a> {
    /// The displayed lifecycle state (from `Play::script_state`).
    pub state: RunState,
    /// The Browse-selected card name (marked in the picker list).
    pub sel: Option<&'a str>,
    /// The registry card names the Browse picker lists.
    pub names: &'a [String],
    /// The Browse picker is open.
    pub browse_open: bool,
    /// The Load path input is open.
    pub load_open: bool,
    /// The Load path typed so far.
    pub load_path: &'a str,
    /// A shape-only slot handle the TUI never touches (the spec's proof:
    /// clicking Start must not change it).
    pub slot: Option<&'a SlotScript>,
}

impl<'a> ScriptPane<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: RunState,
        sel: Option<&'a str>,
        names: &'a [String],
        browse_open: bool,
        load_open: bool,
        load_path: &'a str,
        slot: Option<&'a SlotScript>,
    ) -> Self {
        Self {
            state,
            sel,
            names,
            browse_open,
            load_open,
            load_path,
            slot,
        }
    }

    /// The click inside the pane. Buttons live on the second inner line;
    /// Browse picker rows follow the state + buttons lines when the
    /// picker is open.
    pub fn on_click(&self, area: Rect, col: u16, row: u16) -> ScriptClick {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        // Buttons live on the second inner line.
        if row == inner.y + 1 {
            let mut cursor = inner.x;
            for label in SCRIPT_BUTTONS {
                // `[label] ` is label.len() + 3 cells.
                if col >= cursor && col < cursor + label.len() as u16 + 3 {
                    return ScriptClick::Button(label);
                }
                cursor += label.len() as u16 + 3;
            }
        }
        if self.browse_open {
            // Card rows start after the state and buttons lines.
            let list_top = inner.y + 2;
            if row >= list_top {
                let idx = (row - list_top) as usize;
                if idx < self.names.len() {
                    return ScriptClick::Pick(idx);
                }
            }
        }
        ScriptClick::None
    }
}

impl Widget for ScriptPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = if self.load_open {
            "script — load"
        } else if self.browse_open {
            "script — browse"
        } else {
            "script"
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        let state = run_state_text(self.state);
        let sel = self.sel.unwrap_or("—");
        let buttons = SCRIPT_BUTTONS
            .iter()
            .map(|b| format!("[{b}]"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut lines = vec![
            Line::from(format!("script: {state}   sel: {sel}")),
            Line::from(buttons),
        ];
        if self.load_open {
            lines.push(Line::from(format!("path: {}_", self.load_path)));
        }
        if self.browse_open {
            if self.names.is_empty() {
                lines.push(Line::from("browse: (no cards — Load a JS file first)"));
            } else {
                for name in self.names.iter() {
                    let mark = if Some(name.as_str()) == self.sel {
                        "> "
                    } else {
                        "  "
                    };
                    lines.push(Line::from(format!("{mark}{name}")));
                }
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    use script::SlotScript;

    use super::{run_state_text, ScriptClick, ScriptPane, SCRIPT_BUTTONS};

    fn render(pane: ScriptPane<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(pane, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn the_script_widgets_render() {
        let text = render(
            ScriptPane::new(script::RunState::Idle, None, &[], false, false, "", None),
            60,
            4,
        );
        for label in SCRIPT_BUTTONS {
            assert!(
                text.contains(&format!("[{label}]")),
                "button {label} paints: {text:?}"
            );
        }
        assert!(text.contains("script: idle"), "state line: {text:?}");
    }

    /// The shape test from the spec: Start is present and a click on it
    /// must NOT change a dummy [`SlotScript`] — the TUI never calls it.
    #[test]
    fn clicking_start_does_not_change_a_dummy_slot_script() {
        let slot = SlotScript::new();
        let area = Rect::new(0, 0, 60, 4);
        let pane = ScriptPane::new(slot.state(), None, &[], false, false, "", Some(&slot));
        // The buttons row is the second inner line (row 2: border row 0,
        // script status row 1, buttons row 2). Inner starts at x=1, so
        // `[Browse] ` spans cols 1..10 and `[Start] ` starts at col 10.
        let start = pane.on_click(area, 10, 2);
        assert_eq!(
            start,
            ScriptClick::Button("Start"),
            "Start is clickable (the app maps it to ScriptStart)"
        );
        assert_eq!(slot.state(), script::RunState::Idle, "state unchanged");
        assert!(!slot.want_run, "want_run unchanged");
        assert_eq!(run_state_text(slot.state()), "idle");
    }

    #[test]
    fn click_between_buttons_misses() {
        let pane = ScriptPane::new(script::RunState::Idle, None, &[], false, false, "", None);
        let area = Rect::new(0, 0, 60, 4);
        // One row below the buttons line.
        assert_eq!(pane.on_click(area, 9, 3), ScriptClick::None);
    }

    #[test]
    fn browse_open_picks_a_card_row() {
        let names = vec!["BoneBurier".to_string(), "MineRobber".to_string()];
        let pane = ScriptPane::new(
            script::RunState::Idle,
            None,
            &names,
            true,
            false,
            "",
            None,
        );
        let area = Rect::new(0, 0, 60, 7);
        // With the picker open the card rows start after the state and
        // buttons lines: inner.y + 2 = area.y + 3.
        assert_eq!(
            pane.on_click(area, 2, 3),
            ScriptClick::Pick(0),
            "the first card row picks card 0"
        );
        assert_eq!(
            pane.on_click(area, 2, 4),
            ScriptClick::Pick(1),
            "the second card row picks card 1"
        );
        assert_eq!(
            pane.on_click(area, 2, 5),
            ScriptClick::None,
            "past the list is a miss"
        );
        let text = render(
            ScriptPane::new(
                script::RunState::Idle,
                Some("BoneBurier"),
                &names,
                true,
                false,
                "",
                None,
            ),
            60,
            7,
        );
        assert!(text.contains("BoneBurier"), "card rows paint: {text:?}");
        assert!(text.contains('>'), "the selected card is marked: {text:?}");
    }

    #[test]
    fn load_open_renders_the_typed_path() {
        let text = render(
            ScriptPane::new(
                script::RunState::Idle,
                None,
                &[],
                false,
                true,
                "/tmp/digbot.js",
                None,
            ),
            60,
            5,
        );
        assert!(
            text.contains("/tmp/digbot.js"),
            "the typed load path paints: {text:?}"
        );
    }
}

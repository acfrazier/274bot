//! Script pane, shape only (spec `2026-09-01-headless-tui-design.md`):
//! the Browse/Start/Pause/Stop/Load widgets exist and are clickable, but
//! wired scripts are not in this tag — the TUI never calls [`SlotScript`]
//! or `Play::script_start`. The pane shows the slot's script lifecycle
//! state (read-only) and answers clicks with the button label; `tui-play`
//! ignores the answer.

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

/// The script shape pane. `slot` is a read-only handle the TUI never
/// calls — it exists so a test can prove clicking Start does not mutate a
/// [`SlotScript`]; `state` is the displayed lifecycle.
pub struct ScriptPane<'a> {
    /// The displayed lifecycle state (from `Play::script_state`).
    pub state: RunState,
    /// A shape-only slot handle the TUI never touches (the spec's proof:
    /// clicking Start must not change it).
    pub slot: Option<&'a SlotScript>,
}

impl<'a> ScriptPane<'a> {
    pub fn new(state: RunState, slot: Option<&'a SlotScript>) -> Self {
        Self { state, slot }
    }

    /// The button clicked inside the pane, `None` for a miss. The caller
    /// does nothing with the label this tag — wired scripts are 0.1.5.
    pub fn on_click(&self, area: Rect, col: u16, row: u16) -> Option<&'static str> {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        // Buttons live on the second inner line.
        if row != inner.y + 1 {
            return None;
        }
        let mut cursor = inner.x;
        for label in SCRIPT_BUTTONS {
            // `[label] ` is label.len() + 3 cells.
            if col >= cursor && col < cursor + label.len() as u16 + 3 {
                return Some(label);
            }
            cursor += label.len() as u16 + 3;
        }
        None
    }
}

impl Widget for ScriptPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default().borders(Borders::ALL).title("script");
        let inner = block.inner(area);
        block.render(area, buf);
        let state = run_state_text(self.state);
        let buttons = SCRIPT_BUTTONS
            .iter()
            .map(|b| format!("[{b}]"))
            .collect::<Vec<_>>()
            .join(" ");
        let lines = vec![
            Line::from(format!("script: {state}")),
            Line::from(format!("{buttons}   (not wired this tag)")),
        ];
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

    use super::{run_state_text, ScriptPane, SCRIPT_BUTTONS};

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
        let text = render(ScriptPane::new(script::RunState::Idle, None), 60, 4);
        for label in SCRIPT_BUTTONS {
            assert!(
                text.contains(&format!("[{label}]")),
                "button {label} paints: {text:?}"
            );
        }
        assert!(text.contains("not wired"), "shape-only hint: {text:?}");
        assert!(text.contains("script: idle"), "state line: {text:?}");
    }

    /// The shape test from the spec: Start is present and a click on it
    /// must NOT change a dummy [`SlotScript`] — the TUI never calls it.
    #[test]
    fn clicking_start_does_not_change_a_dummy_slot_script() {
        let slot = SlotScript::new();
        let area = Rect::new(0, 0, 60, 4);
        let pane = ScriptPane::new(slot.state(), Some(&slot));
        // The buttons row is the second inner line (row 2: border row 0,
        // script status row 1, buttons row 2). Inner starts at x=1, so
        // `[Browse] ` spans cols 1..10 and `[Start] ` starts at col 10.
        let start = pane.on_click(area, 10, 2);
        assert_eq!(start, Some("Start"), "Start is clickable");
        assert_eq!(slot.state(), script::RunState::Idle, "state unchanged");
        assert!(!slot.want_run, "want_run unchanged");
        assert_eq!(run_state_text(slot.state()), "idle");
    }

    #[test]
    fn click_between_buttons_misses() {
        let pane = ScriptPane::new(script::RunState::Idle, None);
        let area = Rect::new(0, 0, 60, 4);
        // One row below the buttons line.
        assert_eq!(pane.on_click(area, 9, 3), None);
    }
}

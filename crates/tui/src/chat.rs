//! The game chat / NPC dialogue pane (spec
//! `2026-09-01-headless-tui-design.md`). Below the map it shows the public
//! chat ring — the snapshot's `chat_lines`, newest first, client-shaped —
//! until a chat modal is open; then it shows `chat_modal_texts` plus any
//! `chat_options`, and Space/Enter (or a click) continues the dialog
//! (`continue_dialog`) or answers the focused choice (`answer_choice`) to
//! unstick NPC chat the guardian is not handling. The send hook is a
//! `FnMut(ChatAction)` like the map's walk hook — nothing is queued until
//! `tui-play` wires it to [`host_play::WireCmd`].

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use api::snapshot::{ChatLineView, ChatOptionView};

/// One chat interaction the operator triggered. `Answer` carries the
/// 1-based option index, matching `Interactions::answer_choice`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAction {
    /// Press the chat modal's Continue button.
    Continue,
    /// Press the chat modal's `option`-th BUTTON_OK choice (1-based).
    Answer(usize),
    /// The key/click was ignored.
    None,
}

/// Mutable chat-pane state kept across frames: the focused option row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatState {
    /// The operator's focused option (0-based into `chat_options`).
    pub choice: usize,
}

/// The chat pane's snapshot-backed read view.
#[derive(Debug, Clone, Copy)]
pub struct ChatView<'a> {
    /// The public chat ring, newest first.
    pub lines: &'a [ChatLineView],
    /// The chat modal's text pages.
    pub modal_texts: &'a [String],
    /// The chat modal's BUTTON_OK choices.
    pub options: &'a [ChatOptionView],
    /// A BUTTON_CONTINUE component is up.
    pub has_continue: bool,
}

/// A chat modal is open when the snapshot shows dialog text, choices, or
/// a Continue button — any of the three means the pane switches from the
/// public ring to the dialogue.
pub fn chat_modal_open(view: &ChatView<'_>) -> bool {
    !view.modal_texts.is_empty() || !view.options.is_empty() || view.has_continue
}

/// The chat pane widget. Cheap to rebuild each frame (borrows only); the
/// send hook is called from `on_key` / `on_click`, not from the render.
pub struct Chat<'a, F> {
    pub view: ChatView<'a>,
    pub state: &'a mut ChatState,
    /// The interaction hook: `Continue` / `Answer` when the operator
    /// advances a dialog.
    pub send: F,
}

impl<'a, F: FnMut(ChatAction)> Chat<'a, F> {
    pub fn new(view: ChatView<'a>, state: &'a mut ChatState, send: F) -> Self {
        Self { view, state, send }
    }

    /// Space / Enter continue the dialog (answering the focused option
    /// when one is up); Up/Down and j/k move the option focus; everything
    /// else is ignored.
    pub fn on_key(&mut self, key: KeyEvent) -> ChatAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.view.options.len() > 1 {
                    self.state.choice = self
                        .state
                        .choice
                        .saturating_sub(1)
                        .min(self.view.options.len() - 1);
                }
                ChatAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.view.options.len() > 1 {
                    self.state.choice = (self.state.choice + 1).min(self.view.options.len() - 1);
                }
                ChatAction::None
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                let action = self.confirm();
                (self.send)(action);
                action
            }
            _ => ChatAction::None,
        }
    }

    /// A click inside the pane: an option row answers that choice, any
    /// other row continues the dialog. `area` is the pane's buffer rect;
    /// `(col, row)` is the click's buffer position.
    pub fn on_click(&mut self, area: Rect, col: u16, row: u16) -> ChatAction {
        if !area.contains(Position::new(col, row)) {
            return ChatAction::None;
        }
        let action = if self.view.options.is_empty() {
            ChatAction::Continue
        } else {
            // Options start after the border, the modal text lines, and
            // one blank row (mirrors the render layout).
            let text_lines: usize = self
                .view
                .modal_texts
                .iter()
                .map(|t| t.split('\n').count())
                .sum();
            let options_start = area.y + 1 + text_lines as u16 + 1;
            let offset = row.saturating_sub(options_start) as usize;
            if offset < self.view.options.len() {
                self.state.choice = offset;
                ChatAction::Answer(offset + 1)
            } else {
                ChatAction::None
            }
        };
        if action != ChatAction::None {
            (self.send)(action);
        }
        action
    }

    /// The confirm action for Space/Enter: the focused option when options
    /// are up, else Continue when a Continue button is up, else nothing.
    fn confirm(&mut self) -> ChatAction {
        if !self.view.options.is_empty() {
            let choice = self.state.choice.min(self.view.options.len() - 1);
            self.state.choice = choice;
            ChatAction::Answer(choice + 1)
        } else if self.view.has_continue {
            ChatAction::Continue
        } else {
            ChatAction::None
        }
    }
}

impl<'a, F: FnMut(ChatAction)> Widget for Chat<'a, F> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = if chat_modal_open(&self.view) {
            "dialogue"
        } else {
            "chat"
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if chat_modal_open(&self.view) {
            let mut lines: Vec<Line> = Vec::new();
            for text in self.view.modal_texts {
                for piece in text.split('\n') {
                    lines.push(Line::from(piece.to_string()));
                }
            }
            if !self.view.options.is_empty() {
                lines.push(Line::from(""));
                let focus = self.state.choice.min(self.view.options.len() - 1);
                for (i, option) in self.view.options.iter().enumerate() {
                    let marker = if i == focus { "> " } else { "  " };
                    lines.push(Line::from(format!("{marker}{}", option.text)));
                }
                lines.push(Line::from(""));
                lines.push(Line::from("Space/Enter: choose"));
            } else if self.view.has_continue {
                lines.push(Line::from(""));
                lines.push(Line::from("— Space/Enter to continue —"));
            }
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(inner, buf);
        } else {
            let mut lines: Vec<Line> = Vec::new();
            for line in self
                .view
                .lines
                .iter()
                .take(area.height.saturating_sub(2) as usize)
            {
                let who = line
                    .username
                    .as_deref()
                    .map(|u| format!("{u}: "))
                    .unwrap_or_default();
                lines.push(Line::from(format!("{who}{}", line.text)));
            }
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(inner, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use api::snapshot::{ChatLineView, ChatOptionView};

    use super::{chat_modal_open, Chat, ChatAction, ChatState, ChatView};

    fn line(text: &str) -> ChatLineView {
        ChatLineView {
            type_: 0,
            username: Some("npc".into()),
            text: text.into(),
            sequence: 0,
        }
    }

    fn option(text: &str) -> ChatOptionView {
        ChatOptionView {
            component_id: 1,
            text: text.into(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn render(view: ChatView<'_>, w: u16, h: u16) -> String {
        let mut state = ChatState::default();
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Chat::new(view, &mut state, |_| {}), frame.area()))
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
    fn chat_widget_shows_the_snapshot_ring_lines() {
        let lines = vec![line("welcome to 274"), line("a second line")];
        let view = ChatView {
            lines: &lines,
            modal_texts: &[],
            options: &[],
            has_continue: false,
        };
        let text = render(view, 60, 5);
        assert!(text.contains("welcome to 274"), "ring text: {text:?}");
        assert!(
            text.contains("npc: a second line"),
            "username prefix: {text:?}"
        );
        assert!(!chat_modal_open(&view));
    }

    #[test]
    fn modal_open_shows_dialogue_text_and_continue_prompt() {
        let lines = vec![line("public ring, hidden")];
        let texts = vec!["Greetings traveller.".into()];
        let view = ChatView {
            lines: &lines,
            modal_texts: &texts,
            options: &[],
            has_continue: true,
        };
        assert!(chat_modal_open(&view));
        let text = render(view, 60, 6);
        assert!(
            text.contains("Greetings traveller."),
            "modal text paints: {text:?}"
        );
        assert!(
            text.contains("continue"),
            "the Continue prompt paints: {text:?}"
        );
        assert!(
            !text.contains("public ring"),
            "the ring is replaced by the dialogue: {text:?}"
        );
    }

    #[test]
    fn enter_sends_continue_dialog_through_the_hook() {
        let texts = vec!["The stranger waits.".into()];
        let view = ChatView {
            lines: &[],
            modal_texts: &texts,
            options: &[],
            has_continue: true,
        };
        let mut state = ChatState::default();
        let mut sent: Vec<ChatAction> = Vec::new();
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            chat.on_key(key(KeyCode::Enter))
        };
        assert_eq!(action, ChatAction::Continue);
        assert_eq!(sent, vec![ChatAction::Continue], "Enter → continue_dialog");
    }

    #[test]
    fn space_answers_the_focused_option() {
        let opts = vec![option("Yes"), option("No thanks")];
        let view = ChatView {
            lines: &[],
            modal_texts: &["Which way?".into()],
            options: &opts,
            has_continue: false,
        };
        let mut state = ChatState::default();
        let mut sent: Vec<ChatAction> = Vec::new();
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            // Focus the second option, then confirm.
            chat.on_key(key(KeyCode::Down));
            chat.on_key(key(KeyCode::Char(' ')))
        };
        assert_eq!(state.choice, 1);
        assert_eq!(action, ChatAction::Answer(2), "Space answers option 2");
        assert_eq!(sent, vec![ChatAction::Answer(2)]);
    }

    #[test]
    fn click_on_an_option_row_answers_that_choice() {
        let opts = vec![option("Yes"), option("No thanks")];
        let view = ChatView {
            lines: &[],
            modal_texts: &["Which way?".into()],
            options: &opts,
            has_continue: false,
        };
        let mut state = ChatState::default();
        let mut sent: Vec<ChatAction> = Vec::new();
        // Pane: border row 0, text row 1, blank row 2, options at rows
        // 3 and 4 (mirrors the render layout).
        let area = ratatui::layout::Rect::new(0, 0, 60, 8);
        let (a1, a2) = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            (chat.on_click(area, 10, 3), chat.on_click(area, 10, 4))
        };
        assert_eq!(
            a1,
            ChatAction::Answer(1),
            "the first option row answers option 1"
        );
        assert_eq!(
            a2,
            ChatAction::Answer(2),
            "the second option row answers option 2"
        );
        assert_eq!(sent, vec![ChatAction::Answer(1), ChatAction::Answer(2)]);
    }

    #[test]
    fn click_below_the_options_continues_when_a_continue_is_up() {
        let opts = vec![option("Yes")];
        let view = ChatView {
            lines: &[],
            modal_texts: &["Which way?".into()],
            options: &opts,
            has_continue: true,
        };
        let mut state = ChatState::default();
        let mut sent: Vec<ChatAction> = Vec::new();
        let area = ratatui::layout::Rect::new(0, 0, 60, 8);
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            // A click below the option rows is not an option — it does not
            // continue through the choices.
            chat.on_click(area, 10, 7)
        };
        assert_eq!(action, ChatAction::None);
        assert!(sent.is_empty());
    }

    #[test]
    fn modal_options_render_with_the_focused_marker() {
        let opts = vec![option("Yes"), option("No thanks")];
        let view = ChatView {
            lines: &[],
            modal_texts: &["Which way?".into()],
            options: &opts,
            has_continue: false,
        };
        let text = render(view, 60, 8);
        assert!(text.contains("Which way?"), "dialogue text: {text:?}");
        assert!(text.contains("No thanks"), "options paint: {text:?}");
    }
}

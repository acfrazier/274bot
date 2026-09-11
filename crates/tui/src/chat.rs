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
    /// Press the `index`-th advertised script paint button (0-based).
    PaintButton(usize),
    /// The key/click was ignored.
    None,
}

/// Mutable chat-pane state kept across frames: the focused option row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatState {
    /// The operator's focused option (0-based into `chat_options`).
    pub choice: usize,
    /// The operator's focused paint button (0-based into `buttons`).
    pub paint_choice: usize,
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
    /// The focused slot's script paint frame; the pane shows it instead
    /// of the ring while it is non-empty (see [`ChatView::paint_showing`]).
    pub script_paint: Option<&'a script::shim::ScriptPaint>,
    /// Operator toggle: show the game chat even while the script paints
    /// (the `p` key flips it).
    pub show_game_chat: bool,
}

/// A chat modal is open when the snapshot shows dialog text, choices, or
/// a Continue button — any of the three means the pane switches from the
/// public ring to the dialogue.
pub fn chat_modal_open(view: &ChatView<'_>) -> bool {
    !view.modal_texts.is_empty() || !view.options.is_empty() || view.has_continue
}

impl<'a> ChatView<'a> {
    /// A recorded script paint is showing when the pane has one and the
    /// operator has not toggled back to the game chat. A modal always
    /// wins (the operator must answer the dialogue).
    fn paint_showing(&self) -> bool {
        !self.show_game_chat
            && self
                .script_paint
                .is_some_and(|p| p.title.is_some() || !p.lines.is_empty() || !p.buttons.is_empty())
    }

    fn paint_buttons(&self) -> &[script::shim::ScriptPaintButton] {
        self.script_paint
            .map(|p| p.buttons.as_slice())
            .unwrap_or(&[])
    }

    /// Preferred outer height for the ordinary app layout. Paint buttons
    /// get their advertised rows instead of competing with the historic
    /// six-row chat allocation; the cap keeps the rest of the TUI useful.
    pub(crate) fn preferred_height(&self) -> u16 {
        const DEFAULT_HEIGHT: u16 = 6;
        const MAX_PAINT_HEIGHT: u16 = 14;

        if chat_modal_open(self) || !self.paint_showing() || self.paint_buttons().is_empty() {
            return DEFAULT_HEIGHT;
        }
        let paint = self.script_paint.expect("paint_showing requires paint");
        let content_rows = usize::from(paint.title.is_some()) + paint.lines.len();
        let rows = 2usize
            .saturating_add(content_rows)
            .saturating_add(1)
            .saturating_add(paint.buttons.len());
        u16::try_from(rows)
            .unwrap_or(u16::MAX)
            .clamp(DEFAULT_HEIGHT, MAX_PAINT_HEIGHT)
    }
}

#[derive(Debug)]
struct PaintButtonLayout {
    index: usize,
    row: Rect,
    hit: Rect,
    text: String,
}

#[derive(Debug)]
struct PaintLayout {
    content: Rect,
    buttons: Vec<PaintButtonLayout>,
}

/// Compute the paint rows once for both rendering and pointer dispatch.
/// Buttons are pinned to the bottom of the inner pane; if they cannot all
/// fit, the visible window always contains the focused button.
fn paint_layout(view: &ChatView<'_>, state: &ChatState, area: Rect) -> PaintLayout {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let buttons = view.paint_buttons();
    let visible_count = buttons.len().min(usize::from(inner.height));
    if visible_count == 0 || inner.width == 0 {
        return PaintLayout {
            content: inner,
            buttons: Vec::new(),
        };
    }

    let focus = state.paint_choice.min(buttons.len() - 1);
    let first = focus
        .saturating_add(1)
        .saturating_sub(visible_count)
        .min(buttons.len() - visible_count);
    let buttons_y = inner.y + inner.height - visible_count as u16;
    let content = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        buttons_y.saturating_sub(inner.y).saturating_sub(1),
    );
    let buttons = buttons[first..first + visible_count]
        .iter()
        .enumerate()
        .map(|(visible_index, button)| {
            let index = first + visible_index;
            let marker = if index == focus { "> " } else { "  " };
            let text = format!("{marker}[{}] {}", index + 1, button.label);
            let row = Rect::new(inner.x, buttons_y + visible_index as u16, inner.width, 1);
            let hit_width = Line::from(text.as_str())
                .width()
                .min(usize::from(inner.width)) as u16;
            PaintButtonLayout {
                index,
                row,
                hit: Rect::new(row.x, row.y, hit_width, 1),
                text,
            }
        })
        .collect();
    PaintLayout { content, buttons }
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
    /// when one is up); Up/Down and j/k move the option focus; paint
    /// buttons use digits / j/k+Enter while paint-showing. Modal wins.
    pub fn on_key(&mut self, key: KeyEvent) -> ChatAction {
        if chat_modal_open(&self.view) {
            return self.on_modal_key(key);
        }
        if self.view.paint_showing() && !self.view.paint_buttons().is_empty() {
            return self.on_paint_key(key);
        }
        ChatAction::None
    }

    fn on_modal_key(&mut self, key: KeyEvent) -> ChatAction {
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

    fn on_paint_key(&mut self, key: KeyEvent) -> ChatAction {
        let n = self.view.paint_buttons().len();
        if n == 0 {
            return ChatAction::None;
        }
        self.state.paint_choice = self.state.paint_choice.min(n - 1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.paint_choice = self.state.paint_choice.saturating_sub(1);
                ChatAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.paint_choice = (self.state.paint_choice + 1).min(n - 1);
                ChatAction::None
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = (c as u8 - b'1') as usize;
                if index < n {
                    let action = ChatAction::PaintButton(index);
                    (self.send)(action);
                    action
                } else {
                    ChatAction::None
                }
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                let action = ChatAction::PaintButton(self.state.paint_choice);
                (self.send)(action);
                action
            }
            _ => ChatAction::None,
        }
    }

    /// A click inside the pane: an option row answers that choice, any
    /// other row continues the dialog. Paint-showing clicks hit advertised
    /// buttons instead of `WireCmd`. `area` is the pane's buffer rect;
    /// `(col, row)` is the click's buffer position.
    pub fn on_click(&mut self, area: Rect, col: u16, row: u16) -> ChatAction {
        if !area.contains(Position::new(col, row)) {
            return ChatAction::None;
        }
        let action = if chat_modal_open(&self.view) {
            if self.view.options.is_empty() {
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
            }
        } else if self.view.paint_showing() {
            let layout = paint_layout(&self.view, self.state, area);
            layout
                .buttons
                .iter()
                .find(|button| button.hit.contains(Position::new(col, row)))
                .map_or(ChatAction::None, |button| {
                    self.state.paint_choice = button.index;
                    ChatAction::PaintButton(button.index)
                })
        } else if self.view.options.is_empty() {
            ChatAction::Continue
        } else {
            ChatAction::None
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
        } else if self.view.paint_showing() {
            "script paint"
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
        } else if let Some(paint) = self.view.script_paint.filter(|_| self.view.paint_showing()) {
            // Paint-as-chat: the script's frame replaces the game ring
            // (title + rows, rs2b0t paint shape). `p` toggles back. Controls
            // are laid out separately so wrapped body text cannot clip or
            // shift their hit targets.
            let layout = paint_layout(&self.view, self.state, area);
            let mut lines: Vec<Line> = Vec::new();
            if let Some(t) = &paint.title {
                lines.push(Line::from(t.clone()));
            }
            for row in &paint.lines {
                lines.push(Line::from(row.clone()));
            }
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(layout.content, buf);
            for button in layout.buttons {
                Paragraph::new(button.text).render(button.row, buf);
            }
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
            script_paint: None,
            show_game_chat: false,
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
            script_paint: None,
            show_game_chat: false,
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
            script_paint: None,
            show_game_chat: false,
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
            script_paint: None,
            show_game_chat: false,
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
            script_paint: None,
            show_game_chat: false,
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
            script_paint: None,
            show_game_chat: false,
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
            script_paint: None,
            show_game_chat: false,
        };
        let text = render(view, 60, 8);
        assert!(text.contains("Which way?"), "dialogue text: {text:?}");
        assert!(text.contains("No thanks"), "options paint: {text:?}");
    }

    /// Task 13: a recorded script paint replaces the chat ring (title +
    /// rows), and the toggle re-shows the ring.
    #[test]
    fn script_paint_replaces_the_ring_until_toggled() {
        let lines = vec![line("a game chat line")];
        let paint = script::shim::ScriptPaint {
            title: Some("BoneBurier — digging".into()),
            accent: None,
            lines: vec!["Runtime: 1.2m | Buried: 3".into()],
            buttons: Vec::new(),
            generation: 0,
        };
        let view = ChatView {
            lines: &lines,
            modal_texts: &[],
            options: &[],
            has_continue: false,
            script_paint: Some(&paint),
            show_game_chat: false,
        };
        let text = render(view, 60, 5);
        assert!(
            text.contains("BoneBurier — digging"),
            "paint title paints: {text:?}"
        );
        assert!(
            text.contains("Runtime: 1.2m | Buried: 3"),
            "paint rows paint: {text:?}"
        );
        assert!(
            !text.contains("a game chat line"),
            "the ring is replaced: {text:?}"
        );
        let toggled = ChatView {
            script_paint: Some(&paint),
            show_game_chat: true,
            ..view
        };
        let text = render(toggled, 60, 5);
        assert!(
            text.contains("a game chat line"),
            "the toggle shows the game chat: {text:?}"
        );
    }

    fn paint_with_button() -> script::shim::ScriptPaint {
        script::shim::ScriptPaint {
            title: Some("NatureCrafter".into()),
            accent: None,
            lines: vec!["status".into()],
            buttons: vec![script::shim::ScriptPaintButton {
                id: "gobank".into(),
                label: "Go bank".into(),
            }],
            generation: 0,
        }
    }

    #[test]
    fn paint_digit_and_enter_dispatch_paint_button() {
        let paint = paint_with_button();
        let view = ChatView {
            lines: &[],
            modal_texts: &[],
            options: &[],
            has_continue: false,
            script_paint: Some(&paint),
            show_game_chat: false,
        };
        let text = render(view, 60, 8);
        assert!(text.contains("Go bank"), "reachable control: {text:?}");
        let mut state = ChatState::default();
        let mut sent: Vec<ChatAction> = Vec::new();
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            chat.on_key(key(KeyCode::Char('1')))
        };
        assert_eq!(action, ChatAction::PaintButton(0));
        assert_eq!(sent, vec![ChatAction::PaintButton(0)]);
        sent.clear();
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            chat.on_key(key(KeyCode::Enter))
        };
        assert_eq!(action, ChatAction::PaintButton(0));
    }

    #[test]
    fn paint_modal_still_wins_over_paint_buttons() {
        let mut paint = paint_with_button();
        paint
            .lines
            .extend(["second row".into(), "third row".into()]);
        let texts = vec!["The stranger waits.".into()];
        let view = ChatView {
            lines: &[],
            modal_texts: &texts,
            options: &[],
            has_continue: true,
            script_paint: Some(&paint),
            show_game_chat: false,
        };
        assert_eq!(
            view.preferred_height(),
            6,
            "the dialogue layout, not hidden paint, controls modal height"
        );
        let mut state = ChatState::default();
        let mut sent: Vec<ChatAction> = Vec::new();
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            chat.on_key(key(KeyCode::Enter))
        };
        assert_eq!(action, ChatAction::Continue);
        assert_eq!(sent, vec![ChatAction::Continue]);
        sent.clear();
        let action = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            chat.on_key(key(KeyCode::Char('1')))
        };
        assert_eq!(action, ChatAction::None);
        assert!(sent.is_empty());
    }

    #[test]
    fn paint_click_rejects_rows_above_wrapped_button_and_hits_rendered_row() {
        let paint = script::shim::ScriptPaint {
            title: Some("NatureCrafter".into()),
            accent: None,
            lines: vec!["a status row long enough to wrap in this compact pane".into()],
            buttons: vec![script::shim::ScriptPaintButton {
                id: "gobank".into(),
                label: "Go bank".into(),
            }],
            generation: 0,
        };
        let view = ChatView {
            lines: &[],
            modal_texts: &[],
            options: &[],
            has_continue: false,
            script_paint: Some(&paint),
            show_game_chat: false,
        };
        let area = ratatui::layout::Rect::new(0, 0, 24, 7);
        let mut state = ChatState::default();
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Chat::new(view, &mut state, |_| {}), area))
            .unwrap();
        let button_row = terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(area.width))
            .enumerate()
            .find_map(|(row, cells)| {
                let text: String = cells.iter().map(|cell| cell.symbol()).collect();
                text.contains("[1] Go bank").then_some(row as u16)
            })
            .expect("the focused button must remain visible in the compact pane");
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            !rendered.contains("compact pane"),
            "the overflowing tail of the wrapped body is clipped: {rendered:?}"
        );

        let mut sent = Vec::new();
        let (title, body, spacer, border, trailing, button) = {
            let mut chat = Chat::new(view, &mut state, |a| sent.push(a));
            (
                chat.on_click(area, 2, 1),
                chat.on_click(area, 2, 2),
                chat.on_click(area, 2, button_row - 1),
                chat.on_click(area, 0, button_row),
                chat.on_click(area, area.width - 2, button_row),
                chat.on_click(area, 3, button_row),
            )
        };
        assert_eq!(title, ChatAction::None);
        assert_eq!(body, ChatAction::None);
        assert_eq!(spacer, ChatAction::None);
        assert_eq!(border, ChatAction::None);
        assert_eq!(trailing, ChatAction::None);
        assert_eq!(button, ChatAction::PaintButton(0));
        assert_eq!(sent, vec![ChatAction::PaintButton(0)]);
    }
}

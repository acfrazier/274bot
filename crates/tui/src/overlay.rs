//! App-owned overlays: help, the command palette, confirmations, context
//! menus, the full-message viewer and the armed manual-walk box. An open
//! overlay takes every key and every click (a click outside a menu only
//! closes it), so nothing leaks to the panes or to the bots behind it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

use nav::tile::Tile;

use crate::app::{wasd_target, AppAction, TuiApp};
use crate::commands::Command;
use crate::fleet_card::CardPicker;
use crate::help::{filter_rows, help_rows, render_help, HelpState};
use crate::layout::{contains, Screen};
use crate::palette::{palette_commands, render_palette, PaletteState};
use crate::profile_import::{ImportAction, ImportView};
use crate::world_picker::WorldPicker;

#[derive(Debug, PartialEq, Eq)]
pub enum Modal {
    Help(HelpState),
    Palette(PaletteState),
    Confirm(Confirm),
    Context(ContextMenu),
    /// The full text of the message line.
    Message {
        scroll: u16,
    },
    /// WASD walking is armed for the selected bot until Esc.
    Manual,
    /// Import rs2b0t's profile export: path, preview, report.
    ProfileImport(ImportView),
    /// Pick the card the fleet actions use (Enter picks, Esc keeps).
    FleetCard(CardPicker),
    /// Pick the world a bot (or the marked bots) moves to (Enter picks, Esc
    /// cancels). Picking asks core for a plan; it moves nothing itself.
    WorldPicker(WorldPicker),
}

impl Modal {
    /// The keyboard scope the footer names while this overlay is open.
    pub fn scope(&self) -> &'static str {
        match self {
            Modal::Help(_) => "Help",
            Modal::Palette(_) => "Command palette",
            Modal::Confirm(_) => "Confirm",
            Modal::Context(_) => "Menu",
            Modal::Message { .. } => "Message",
            Modal::Manual => "Manual walk",
            Modal::ProfileImport(_) => "rs2b0t import",
            Modal::FleetCard(_) => "Fleet card",
            Modal::WorldPicker(_) => "World picker",
        }
    }

    pub fn hints(&self) -> &'static str {
        match self {
            Modal::Help(_) => "type to search · Up/Down scroll · Esc close",
            Modal::Palette(_) => "type to filter · Up/Down · Enter run · Esc close",
            Modal::Confirm(_) => "Enter/y confirm · Esc/n cancel",
            Modal::Context(_) => "Up/Down · Enter run · Esc close",
            Modal::Message { .. } => "Up/Down scroll · Esc close",
            Modal::Manual => "W A S D / arrows step one tile · Esc disarm",
            Modal::ProfileImport(view) => view.hints(),
            Modal::FleetCard(_) => "Up/Down · Enter pick · Esc keep the card",
            Modal::WorldPicker(_) => "Up/Down · Enter pick · Esc cancel",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Confirm {
    pub kind: ConfirmKind,
    /// Shown above the buttons (for example after the scope changed).
    pub note: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    /// Remove this member from the fleet (the target is frozen here).
    Remove(String),
    Quit,
    /// Relog this member now so a memory-mode switch reaches the server.
    /// Offered when running script work would be interrupted or a queued
    /// Start would be cancelled.
    MemoryRelog(String),
    /// A fleet action core froze (`PlanFleetAction`) that asks first:
    /// `prompt` is core's own text, shown as it is. Confirm runs the plan,
    /// Cancel drops it.
    FleetPlan {
        action: frontend_core::FleetAction,
        prompt: String,
    },
    /// A settings write saved successfully but its running script needs a restart.
    RestartSettings {
        op: frontend_core::OperationId,
        /// Live prompt length when this summary was built. Compared on poll
        /// so an in-place target removal redraws without allocating.
        target_count: usize,
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenu {
    pub title: String,
    pub items: Vec<ContextItem>,
    pub cursor: usize,
    /// Screen cell the menu opened at.
    pub at: (u16, u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextItem {
    Select(String),
    Mark(String),
    /// Select the bot and show one of its tabs.
    Open(String, Screen),
    Run(Command),
}

impl ContextItem {
    fn label(&self, app: &TuiApp) -> String {
        match self {
            ContextItem::Select(name) => format!("Select {name}"),
            ContextItem::Mark(name) if app.is_marked_name(name) => {
                format!("Unselect row {name}")
            }
            ContextItem::Mark(name) => format!("Select row {name} (group)"),
            ContextItem::Open(name, screen) => format!("{} of {name}", screen.label()),
            ContextItem::Run(command) => {
                let mut label = command.label(app).to_string();
                if let Err(reason) = command.availability(app) {
                    label.push_str(" - ");
                    label.push_str(reason);
                }
                label
            }
        }
    }
}

/// Rows `line` occupies when word-wrapped at `width` cells, mirroring
/// the confirmation [`Paragraph`]'s `Wrap { trim: false }`: words move
/// whole to the next row, a word longer than a row starts on a fresh
/// row and is then split across rows, and the separating space stays
/// on the row it ends. Used to size the confirm popup so wrapped
/// text is never clipped; the popup keeps one further row of slack
/// below the text.
fn wrapped_rows(line: &str, width: usize) -> usize {
    let mut rows = 1;
    let mut col = 0;
    for (index, word) in line.split(' ').enumerate() {
        if index > 0 {
            if col >= width {
                rows += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        let mut rest = Line::from(word).width();
        if col > 0 && col + rest > width {
            rows += 1;
            col = 0;
        }
        while rest > 0 {
            if col >= width {
                rows += 1;
                col = 0;
            }
            let take = rest.min(width - col);
            col += take;
            rest -= take;
        }
    }
    rows
}

impl Confirm {
    fn title(&self) -> &'static str {
        match &self.kind {
            ConfirmKind::Remove(_) => "Remove from fleet",
            ConfirmKind::Quit => "Quit tui-play",
            ConfirmKind::MemoryRelog(_) => "Relog now",
            ConfirmKind::FleetPlan { action, .. } => action.verb(),
            ConfirmKind::RestartSettings { .. } => "Restart script settings",
        }
    }

    fn lines(&self, app: &TuiApp) -> Vec<String> {
        let mut lines = match &self.kind {
            ConfirmKind::Remove(name) => vec![
                format!("Remove BOT {name} from the fleet?"),
                "It logs out cleanly, then its slot stops.".into(),
                "The vault profile is kept: this is not Delete profile.".into(),
            ],
            ConfirmKind::MemoryRelog(name) => vec![
                format!("Relog BOT {name} now?"),
                "It logs out and back in through the login queue, so the".into(),
                "memory switch reaches the server tabs and sound.".into(),
                "Running script work is interrupted; a queued Start is cancelled.".into(),
            ],
            ConfirmKind::Quit => {
                let counts = app.counts;
                vec![
                    "Quit tui-play?".into(),
                    format!(
                        "Loaded bots: {} ({} in game). They log out and stop.",
                        counts.loaded, counts.ready
                    ),
                ]
            }
            ConfirmKind::RestartSettings { summary, .. } => vec![
                summary.clone(),
                "The current script keeps running if you choose Later.".into(),
            ],
            ConfirmKind::FleetPlan { prompt, .. } => vec![prompt.clone()],
        };
        if let Some(note) = self.note {
            lines.push(note.into());
        }
        lines
    }

    fn yes(&self) -> &'static str {
        match self.kind {
            ConfirmKind::Remove(_) => "[Remove y]",
            ConfirmKind::Quit => "[Quit y]",
            ConfirmKind::MemoryRelog(_) => "[Relog y]",
            ConfirmKind::FleetPlan { .. } => "[Run y]",
            ConfirmKind::RestartSettings { .. } => "[Restart y]",
        }
    }

    fn no(&self) -> &'static str {
        match self.kind {
            ConfirmKind::RestartSettings { .. } => "[Later n]",
            _ => "[Cancel n]",
        }
    }
}

fn is_text(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

impl TuiApp {
    /// Open an app overlay, closing any other overlay or popup first: only
    /// one overlay ever owns the keyboard.
    pub fn open_modal(&mut self, modal: Modal) {
        self.settings_state.open = false;
        self.loadouts_state.open = false;
        self.params_state.open = false;
        self.modal = Some(modal);
    }

    /// Ask to confirm `kind`; nothing runs until the operator confirms.
    pub fn confirm(&mut self, kind: ConfirmKind) {
        self.open_modal(Modal::Confirm(Confirm { kind, note: None }));
    }

    pub(crate) fn modal_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(modal) = self.modal.take() else {
            return AppAction::None;
        };
        let (keep, action) = match modal {
            Modal::Help(state) => (self.help_key(state, key), AppAction::None),
            Modal::Palette(state) => self.palette_key(state, key),
            Modal::Confirm(confirm) => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.run_confirm(confirm)
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    let action = match confirm.kind {
                        ConfirmKind::RestartSettings { .. } => AppAction::ScriptRestartLater,
                        ConfirmKind::FleetPlan { .. } => AppAction::CancelFleetPlan,
                        _ => AppAction::None,
                    };
                    (None, action)
                }
                _ => (Some(Modal::Confirm(confirm)), AppAction::None),
            },
            Modal::FleetCard(mut picker) => match key.code {
                KeyCode::Enter => match picker.highlighted() {
                    Some(card) => (None, AppAction::PickFleetCard(card)),
                    None => (Some(Modal::FleetCard(picker)), AppAction::None),
                },
                KeyCode::Esc => (None, AppAction::None),
                other => {
                    match other {
                        KeyCode::Up | KeyCode::Char('k') => picker.step(-1),
                        KeyCode::Down | KeyCode::Char('j') => picker.step(1),
                        KeyCode::PageUp => picker.step(-10),
                        KeyCode::PageDown => picker.step(10),
                        KeyCode::Home => picker.jump(false),
                        KeyCode::End => picker.jump(true),
                        _ => {}
                    }
                    (Some(Modal::FleetCard(picker)), AppAction::None)
                }
            },
            Modal::WorldPicker(mut picker) => match key.code {
                KeyCode::Enter => (None, picker.pick(picker.cursor())),
                KeyCode::Esc => (None, AppAction::None),
                other => {
                    match other {
                        KeyCode::Up | KeyCode::Char('k') => picker.step(-1),
                        KeyCode::Down | KeyCode::Char('j') => picker.step(1),
                        KeyCode::PageUp => picker.step(-10),
                        KeyCode::PageDown => picker.step(10),
                        KeyCode::Home => picker.jump(false),
                        KeyCode::End => picker.jump(true),
                        _ => {}
                    }
                    (Some(Modal::WorldPicker(picker)), AppAction::None)
                }
            },
            Modal::Context(menu) => self.context_key(menu, key),
            Modal::Message { scroll } => match key.code {
                KeyCode::Up | KeyCode::Char('k') => (
                    Some(Modal::Message {
                        scroll: scroll.saturating_sub(1),
                    }),
                    AppAction::None,
                ),
                KeyCode::Down | KeyCode::Char('j') => (
                    Some(Modal::Message {
                        scroll: scroll.saturating_add(1),
                    }),
                    AppAction::None,
                ),
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => (None, AppAction::None),
                _ => (Some(Modal::Message { scroll }), AppAction::None),
            },
            Modal::Manual => self.manual_key(key),
            Modal::ProfileImport(mut view) => match view.on_key(key) {
                Some(ImportAction::Close) => (None, AppAction::ProfileImport(ImportAction::Close)),
                Some(action) => (
                    Some(Modal::ProfileImport(view)),
                    AppAction::ProfileImport(action),
                ),
                None => (Some(Modal::ProfileImport(view)), AppAction::None),
            },
        };
        if self.modal.is_none() {
            self.modal = keep;
        }
        action
    }

    fn help_key(&mut self, mut state: HelpState, key: KeyEvent) -> Option<Modal> {
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => return None,
            KeyCode::Up => state.scroll = state.scroll.saturating_sub(1),
            KeyCode::Down => state.scroll = state.scroll.saturating_add(1),
            KeyCode::PageUp => state.scroll = state.scroll.saturating_sub(10),
            KeyCode::PageDown => state.scroll = state.scroll.saturating_add(10),
            KeyCode::Home => state.scroll = 0,
            KeyCode::Backspace => {
                state.query.pop();
                state.scroll = 0;
            }
            KeyCode::Char(c) if is_text(&key) => {
                state.query.push(c);
                state.scroll = 0;
            }
            _ => {}
        }
        Some(Modal::Help(state))
    }

    fn palette_key(
        &mut self,
        mut state: PaletteState,
        key: KeyEvent,
    ) -> (Option<Modal>, AppAction) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return (None, AppAction::None),
            KeyCode::Enter => {
                let commands = palette_commands(self, &state.query);
                let Some(&command) = commands.get(state.cursor) else {
                    return (Some(Modal::Palette(state)), AppAction::None);
                };
                return match command.availability(self) {
                    Ok(()) => (None, self.run_command(command)),
                    Err(reason) => {
                        state.note = Some(reason);
                        (Some(Modal::Palette(state)), AppAction::None)
                    }
                };
            }
            KeyCode::Up => state.cursor = state.cursor.saturating_sub(1),
            KeyCode::Char('p') if ctrl => state.cursor = state.cursor.saturating_sub(1),
            KeyCode::Down => state.cursor = state.cursor.saturating_add(1),
            KeyCode::Char('n') if ctrl => state.cursor = state.cursor.saturating_add(1),
            KeyCode::PageUp => state.cursor = state.cursor.saturating_sub(10),
            KeyCode::PageDown => state.cursor = state.cursor.saturating_add(10),
            KeyCode::Home => state.cursor = 0,
            KeyCode::End => state.cursor = usize::MAX,
            KeyCode::Backspace => {
                state.query.pop();
                state.cursor = 0;
                state.note = None;
            }
            KeyCode::Char(c) if is_text(&key) => {
                state.query.push(c);
                state.cursor = 0;
                state.note = None;
            }
            _ => {}
        }
        let len = palette_commands(self, &state.query).len();
        state.cursor = state.cursor.min(len.saturating_sub(1));
        (Some(Modal::Palette(state)), AppAction::None)
    }

    fn run_confirm(&mut self, confirm: Confirm) -> (Option<Modal>, AppAction) {
        match confirm.kind {
            ConfirmKind::Remove(name) => (None, AppAction::Remove(name)),
            ConfirmKind::MemoryRelog(name) => (None, AppAction::MemoryRelogNow(name)),
            ConfirmKind::Quit => {
                self.quit = true;
                (None, AppAction::Quit)
            }
            ConfirmKind::RestartSettings { .. } => (None, AppAction::ScriptRestartSettings),
            ConfirmKind::FleetPlan { .. } => (None, AppAction::RunFleetPlan),
        }
    }

    fn context_key(&mut self, mut menu: ContextMenu, key: KeyEvent) -> (Option<Modal>, AppAction) {
        match key.code {
            KeyCode::Esc => return (None, AppAction::None),
            KeyCode::Up | KeyCode::Char('k') => menu.cursor = menu.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                menu.cursor = (menu.cursor + 1).min(menu.items.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                let Some(item) = menu.items.get(menu.cursor).cloned() else {
                    return (None, AppAction::None);
                };
                return (None, self.run_context_item(item));
            }
            _ => {}
        }
        (Some(Modal::Context(menu)), AppAction::None)
    }

    fn run_context_item(&mut self, item: ContextItem) -> AppAction {
        match item {
            ContextItem::Select(name) => self.select_bot(&name),
            ContextItem::Mark(name) => self
                .fleet_model
                .index_of_name(&name)
                .and_then(|index| crate::fleet::toggled_mark(&self.fleet_model, index))
                .map_or(AppAction::None, |(name, on)| AppAction::SetMark(name, on)),
            ContextItem::Open(name, screen) => {
                let select = self.select_bot(&name);
                let show = self.show_screen(screen);
                AppAction::batch(select, show)
            }
            ContextItem::Run(command) => match command.availability(self) {
                Ok(()) => self.run_command(command),
                Err(reason) => {
                    self.error = Some(format!("{}: {reason}", command.label(self)));
                    AppAction::None
                }
            },
        }
    }

    fn manual_key(&mut self, key: KeyEvent) -> (Option<Modal>, AppAction) {
        let code = match key.code {
            KeyCode::Esc => return (None, AppAction::None),
            KeyCode::Up => KeyCode::Char('w'),
            KeyCode::Down => KeyCode::Char('s'),
            KeyCode::Left => KeyCode::Char('a'),
            KeyCode::Right => KeyCode::Char('d'),
            other => other,
        };
        let action = match self.here {
            Some(here) => wasd_target((here.x, here.z, here.level), code)
                .map_or(AppAction::None, |(x, z, level)| {
                    AppAction::WalkTile(Tile { x, z, level })
                }),
            None => AppAction::None,
        };
        (Some(Modal::Manual), action)
    }

    pub(crate) fn modal_click(&mut self, col: u16, row: u16) -> AppAction {
        let inside = contains(self.regions.modal, col, row);
        let item = self.regions.modal_item_at(col, row);
        let Some(modal) = self.modal.take() else {
            return AppAction::None;
        };
        let (keep, action) = match modal {
            Modal::Palette(mut state) => match item {
                Some(index) => {
                    state.cursor = index;
                    self.palette_key(state, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                }
                None if inside => (Some(Modal::Palette(state)), AppAction::None),
                None => (None, AppAction::None),
            },
            Modal::Context(menu) => match item.and_then(|i| menu.items.get(i).cloned()) {
                Some(entry) => (None, self.run_context_item(entry)),
                None if inside => (Some(Modal::Context(menu)), AppAction::None),
                None => (None, AppAction::None),
            },
            Modal::Confirm(confirm) => match item {
                Some(0) => self.run_confirm(confirm),
                Some(_) => {
                    let action = match &confirm.kind {
                        ConfirmKind::RestartSettings { .. } => AppAction::ScriptRestartLater,
                        ConfirmKind::FleetPlan { .. } => AppAction::CancelFleetPlan,
                        _ => AppAction::None,
                    };
                    (None, action)
                }
                None => (Some(Modal::Confirm(confirm)), AppAction::None),
            },
            Modal::FleetCard(picker) => match item {
                Some(index) => match picker.card_at(index) {
                    Some(card) => (None, AppAction::PickFleetCard(card)),
                    None => (Some(Modal::FleetCard(picker)), AppAction::None),
                },
                None if inside => (Some(Modal::FleetCard(picker)), AppAction::None),
                // A click outside leaves the card as it was, like Esc.
                None => (None, AppAction::None),
            },
            Modal::WorldPicker(picker) => match item {
                Some(index) => (None, picker.pick(index)),
                None if inside => (Some(Modal::WorldPicker(picker)), AppAction::None),
                // A click outside leaves everything as it was, like Esc.
                None => (None, AppAction::None),
            },
            Modal::Help(state) if inside => (Some(Modal::Help(state)), AppAction::None),
            Modal::Help(_) | Modal::Message { .. } => (None, AppAction::None),
            Modal::Manual => (Some(Modal::Manual), AppAction::None),
            // A click never closes the import: the typed path or the
            // report would be lost. Esc closes it.
            Modal::ProfileImport(view) => (Some(Modal::ProfileImport(view)), AppAction::None),
        };
        if self.modal.is_none() {
            self.modal = keep;
        }
        action
    }

    pub(crate) fn modal_scroll(&mut self, delta: i32) {
        match self.modal.as_mut() {
            Some(Modal::Help(state)) => {
                state.scroll = state.scroll.saturating_add_signed(delta as isize * 3);
            }
            Some(Modal::Palette(state)) => {
                state.cursor = state.cursor.saturating_add_signed(delta as isize);
            }
            Some(Modal::Context(menu)) => {
                menu.cursor = menu
                    .cursor
                    .saturating_add_signed(delta as isize)
                    .min(menu.items.len().saturating_sub(1));
            }
            Some(Modal::Message { scroll }) => {
                *scroll = scroll.saturating_add_signed(delta as i16 * 3);
            }
            Some(Modal::ProfileImport(view)) => view.scroll_by(delta),
            Some(Modal::FleetCard(picker)) => picker.step(delta as isize),
            Some(Modal::WorldPicker(picker)) => picker.step(delta as isize),
            _ => {}
        }
    }

    /// Draw the open overlay over everything and record its hit regions.
    pub(crate) fn draw_modal(&mut self, frame: &mut Frame<'_>) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        let area = frame.area();
        let buf = frame.buffer_mut();
        match &mut modal {
            Modal::Help(state) => {
                let rows = filter_rows(help_rows(self), &state.query);
                self.regions.modal = render_help(state, &rows, area, buf);
            }
            Modal::Palette(state) => {
                let commands = palette_commands(self, &state.query);
                let (rect, hits) = render_palette(self, state, &commands, area, buf);
                self.regions.modal = rect;
                self.regions.modal_items = hits;
            }
            Modal::Confirm(confirm) => self.draw_confirm(confirm, area, buf),
            Modal::Context(menu) => self.draw_context(menu, area, buf),
            Modal::FleetCard(picker) => {
                let (rect, hits) = picker.render(area, buf);
                self.regions.modal = rect;
                self.regions.modal_items = hits;
            }
            Modal::WorldPicker(picker) => {
                self.regions.modal = picker.render(area, buf, &mut self.regions.modal_items);
            }
            Modal::Message { scroll } => self.draw_message(*scroll, area, buf),
            Modal::Manual => self.draw_manual(area, buf),
            Modal::ProfileImport(view) => self.regions.modal = view.render(area, buf),
        }
        self.modal = Some(modal);
    }

    fn draw_confirm(&mut self, confirm: &Confirm, area: Rect, buf: &mut Buffer) {
        let lines = confirm.lines(self);
        let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
        let width = (longest + 4).clamp(30, 90).min(area.width);
        // Size by wrapped rows, not logical lines: a line longer than
        // the popup wraps onto extra rows, and a popup sized by line
        // count would cut the overflow (and any line after it) off.
        let text_width = usize::from(width.saturating_sub(2).max(1));
        let wrapped: usize = lines
            .iter()
            .map(|line| wrapped_rows(line, text_width))
            .sum();
        let height = (wrapped as u16 + 4).min(area.height);
        let paragraph = Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
            .wrap(Wrap { trim: false });
        let popup = centered(area, width, height);
        Clear.render(popup, buf);
        let title = format!(" {} ", confirm.title());
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);
        paragraph.render(
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(1),
            ),
            buf,
        );
        self.regions.modal = popup;
        if inner.height == 0 {
            return;
        }
        let y = inner.y + inner.height - 1;
        let yes = confirm.yes();
        let no = confirm.no();
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let (end, _) = buf.set_stringn(inner.x, y, yes, usize::from(inner.width), bold);
        self.regions
            .modal_items
            .push((Rect::new(inner.x, y, end - inner.x, 1), 0));
        let no_x = end + 2;
        if no_x < inner.x + inner.width {
            let (no_end, _) = buf.set_stringn(
                no_x,
                y,
                no,
                usize::from(inner.x + inner.width - no_x),
                Style::default(),
            );
            self.regions
                .modal_items
                .push((Rect::new(no_x, y, no_end - no_x, 1), 1));
        }
    }

    fn draw_context(&mut self, menu: &mut ContextMenu, area: Rect, buf: &mut Buffer) {
        let labels: Vec<String> = menu.items.iter().map(|item| item.label(self)).collect();
        let longest = labels
            .iter()
            .map(|l| l.chars().count())
            .chain(std::iter::once(menu.title.chars().count() + 2))
            .max()
            .unwrap_or(0) as u16;
        let width = (longest + 4).min(area.width);
        let height = (labels.len() as u16 + 2).min(area.height);
        let x = menu.at.0.min(area.x + area.width - width);
        let y = menu.at.1.min(area.y + area.height - height);
        let popup = Rect::new(x, y, width, height);
        Clear.render(popup, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", menu.title));
        let inner = block.inner(popup);
        block.render(popup, buf);
        self.regions.modal = popup;
        for (index, label) in labels.iter().enumerate().take(usize::from(inner.height)) {
            let row_y = inner.y + index as u16;
            let mut style = Style::default();
            if index == menu.cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let text = format!("{}{label}", if index == menu.cursor { ">" } else { " " });
            buf.set_stringn(inner.x, row_y, &text, usize::from(inner.width), style);
            self.regions
                .modal_items
                .push((Rect::new(inner.x, row_y, inner.width, 1), index));
        }
    }

    fn draw_message(&mut self, scroll: u16, area: Rect, buf: &mut Buffer) {
        let popup = centered(
            area,
            area.width.saturating_sub(8).max(20).min(area.width),
            area.height.saturating_sub(6).max(5).min(area.height),
        );
        Clear.render(popup, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Message · Up/Down scroll · Esc close ");
        let text = self.error.clone().unwrap_or_else(|| "no message".into());
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(block)
            .render(popup, buf);
        self.regions.modal = popup;
    }

    fn draw_manual(&mut self, area: Rect, buf: &mut Buffer) {
        let bot = self.focused_name().unwrap_or_else(|| "—".into());
        let at = match self.here {
            Some(here) => format!("plane {} at {},{}", here.level, here.x, here.z),
            None => "no observed position".into(),
        };
        let lines = vec![
            Line::from(format!("BOT {bot} · {at}")),
            Line::from("W A S D / arrows: walk one tile · Esc: disarm"),
        ];
        let width = 56.min(area.width);
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + area.height.saturating_sub(6),
            width,
            4.min(area.height),
        );
        Clear.render(popup, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" MANUAL WALK (armed) "),
            )
            .render(popup, buf);
        self.regions.modal = popup;
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows ratatui actually uses for `line` wrapped at `width` with
    /// the confirmation's `Wrap { trim: false }` (last non-blank row
    /// + 1); the test lines carry no trailing space.
    fn rendered_rows(line: &str, width: u16) -> usize {
        let height = 400;
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        Paragraph::new(vec![Line::from(line.to_string())])
            .wrap(Wrap { trim: false })
            .render(area, &mut buf);
        let mut last = 0;
        for y in 0..height {
            let blank = (0..width).all(|x| buf[(x, y)].symbol() == " ");
            if !blank {
                last = usize::from(y) + 1;
            }
        }
        last.max(1)
    }

    /// `wrapped_rows` mirrors ratatui even when a word is wider than
    /// the row: ratatui starts such a word on a fresh row and then
    /// splits it, so counting the split from the current column
    /// under-counts and the popup would clip the overflow.
    #[test]
    fn wrapped_rows_matches_ratatui_when_a_word_is_wider_than_the_row() {
        let name27 = "a".repeat(27);
        let name41 = "b".repeat(41);
        let cases: Vec<(String, u16)> = vec![
            // The smallest over-wide case: 2 rows counted, 3 drawn.
            ("zt jmfuk".to_string(), 4),
            // Confirmation-shaped lines at narrow popup widths.
            (
                format!("Stop and restart 3 running bots: {name27}, {name27}, {name27}"),
                26,
            ),
            (
                format!("Start the card on 3 idle bots: {name41}, {name41}, {name41}"),
                38,
            ),
            // Words that fit a row keep their exact count too.
            (
                "Stop and restart 2 running bots: alice, bob".to_string(),
                38,
            ),
            ("hello world again".to_string(), 5),
        ];
        for (line, width) in &cases {
            assert_eq!(
                wrapped_rows(line, usize::from(*width)),
                rendered_rows(line, *width),
                "width {width}: {line:?}"
            );
        }
    }
}

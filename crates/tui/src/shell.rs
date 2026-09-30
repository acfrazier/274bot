//! Shell drawing: the two header rows (title, fleet counts, resources; the
//! selected bot and the tabs), the fleet and detail panes, the log drawer,
//! the footer that always names the keyboard scope, then the open overlay.
//! Every draw records the hit regions the router reads, so a resize never
//! leaves a stale click target.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui::Frame;

use frontend_core::views::run_state_label;
use frontend_core::Phase;

use crate::app::TuiApp;
use crate::commands::{shortcut, Command};
use crate::fleet::{member_row, state_label, FleetTable};
use crate::layout::{shell_rects, Pane, Screen, SizeClass, Tab};
use crate::log_pane::{render_rows, LogPane};
use crate::settings::SettingsPane;

const OVERVIEW_BUTTONS: [Command; 7] = [
    Command::Login,
    Command::Logout,
    Command::Remove,
    Command::Settings,
    Command::Loadouts,
    Command::ManualWalk,
    Command::DismissNotice,
];
const FLEET_BUTTONS: [Command; 2] = [Command::LoadLoginAll, Command::LogoutAll];
const MAP_BUTTONS: [Command; 5] = [
    Command::MapWalk,
    Command::MapTeleport,
    Command::MapGroup,
    Command::MapSearch,
    Command::MapWilderness,
];

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn put(buf: &mut Buffer, x: &mut u16, y: u16, right: u16, text: &str, style: Style) {
    if *x < right {
        let (next, _) = buf.set_stringn(*x, y, text, usize::from(right - *x), style);
        *x = next;
    }
}

/// Emphasise the border of the pane that has keyboard focus and write a
/// plain `[keys]` label into it (focus is never shown by colour alone).
fn mark_focus(buf: &mut Buffer, area: Rect) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let right = area.x + area.width - 1;
    let bottom = area.y + area.height - 1;
    for x in area.x..=right {
        buf[(x, area.y)].set_style(style);
        buf[(x, bottom)].set_style(style);
    }
    for y in area.y..=bottom {
        buf[(area.x, y)].set_style(style);
        buf[(right, y)].set_style(style);
    }
    let label = "[keys]";
    if area.width > label.len() as u16 + 4 {
        buf.set_string(right - label.len() as u16, area.y, label, style);
    }
}

impl TuiApp {
    /// Render the whole shell for the frame's size.
    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let class = SizeClass::of(area);
        // A pane that is not on screen cannot hold the keyboard.
        if self.key_focus == Pane::Drawer
            && (class == SizeClass::Compact || self.screen == Screen::Logs)
        {
            self.key_focus = Pane::Detail;
        }
        let fleet_main = class == SizeClass::Compact && self.key_focus == Pane::Fleet;
        let rects = shell_rects(area, class, fleet_main, self.screen);
        self.regions.begin(area, class, rects);
        self.chat_area = Rect::default();
        self.script_area = Rect::default();
        let focused = self.focused_name();
        self.log.refresh(focused.as_deref());
        if self.screen == Screen::Logs && self.key_focus == Pane::Detail {
            self.log.mark_seen();
        }
        self.draw_header(frame.buffer_mut(), rects.header, class);
        if !rects.fleet.is_empty() {
            self.draw_fleet(frame.buffer_mut(), rects.fleet, class);
        }
        if !rects.detail.is_empty() {
            self.draw_detail(frame, rects.detail, class);
        }
        if !rects.side.is_empty() {
            self.draw_side(frame, rects.side);
        }
        self.draw_drawer(frame.buffer_mut(), rects.drawer, class);
        self.draw_footer(frame.buffer_mut(), rects.footer);
        if self.settings_state.open {
            let mut pane = SettingsPane::new(
                &mut self.settings,
                &mut self.nav,
                &mut self.map_bake,
                &mut self.settings_state,
            );
            pane.title = &self.settings_title;
            pane.notice = self.settings_save.notice();
            frame.render_widget(pane, area);
        }
        self.draw_modal(frame);
    }

    /// `BOT <name> | world | phase | script …`, the line above the detail
    /// (compact: `BOT <name> @world <status>`).
    pub(crate) fn bot_line(&self, compact: bool) -> String {
        let Some(index) = self.focused.filter(|&i| i < self.names.len()) else {
            return "BOT none: select one in the Fleet (F2, Enter)".into();
        };
        let name = &self.names[index];
        let row = member_row(&self.names, &self.fleet, index);
        let world = row
            .and_then(|row| row.world)
            .map_or("local".to_string(), |w| format!("w{w}"));
        let mut line = if compact {
            format!("BOT {name} @{world} {}", state_label(row))
        } else {
            format!(
                "BOT {name} │ {world} │ {} │ script {}",
                row.map_or(Phase::Offline, |row| row.phase).label(),
                run_state_label(self.script_state)
            )
        };
        if self.chat_data.is_modal_open() {
            line.push_str(if compact {
                " DIALOGUE"
            } else {
                " │ DIALOGUE open (Chat F6)"
            });
        }
        line
    }

    fn draw_header(&mut self, buf: &mut Buffer, area: Rect, class: SizeClass) {
        if area.height == 0 {
            return;
        }
        let right = area.x + area.width;
        let counts = self.counts;
        let compact = class == SizeClass::Compact;
        // The full meter (with its units and peak) is in the status pane;
        // the header carries the core's narrow line.
        let tail = if compact {
            format!(
                " │ {}/{} ready Q:{} Err:{} │ {}",
                counts.ready, counts.loaded, counts.queued, counts.failed, self.resources.brief
            )
        } else {
            format!(
                " │ loaded {} ready {} queued {} failed {} │ {}",
                counts.loaded, counts.ready, counts.queued, counts.failed, self.resources.brief
            )
        };
        let tail_w = tail.chars().count() as u16;
        let title_w = area.width.saturating_sub(tail_w).max(area.width.min(12));
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let mut x = area.x;
        let title = self.title();
        if title.chars().count() as u16 > title_w {
            let cut: String = title
                .chars()
                .take(usize::from(title_w.saturating_sub(1)))
                .collect();
            put(buf, &mut x, area.y, right, &cut, bold);
            put(buf, &mut x, area.y, right, "…", bold);
        } else {
            put(buf, &mut x, area.y, right, title, bold);
        }
        put(buf, &mut x, area.y, right, &tail, Style::default());
        if area.height < 2 {
            return;
        }
        let y = area.y + 1;
        let mut x = area.x;
        if compact {
            put(buf, &mut x, y, right, &self.bot_line(true), bold);
            put(buf, &mut x, y, right, " │", dim());
        }
        // Tabs: the detail tab shown is bracketed; the fleet tab is
        // bracketed while the fleet drawer holds the 80x24 main pane.
        let fleet_shown = compact && self.key_focus == Pane::Fleet;
        let tabs = std::iter::once(Tab::Fleet).chain(Screen::ALL.iter().map(|s| Tab::Screen(*s)));
        for tab in tabs {
            let (label, key, active) = match tab {
                Tab::Fleet => ("Fleet", "F2", fleet_shown),
                Tab::Screen(screen) => {
                    let label = if screen == Screen::Chat && self.chat_data.is_modal_open() {
                        "Chat!"
                    } else {
                        screen.label()
                    };
                    (label, screen.key(), !fleet_shown && self.screen == screen)
                }
            };
            let text = match (active, compact) {
                (true, true) => format!("[{label}]"),
                (false, true) => format!(" {label} "),
                (true, false) => format!("[{label} {key}]"),
                (false, false) => format!(" {label} {key} "),
            };
            let style = if active {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::default()
            };
            let start = x;
            put(buf, &mut x, y, right, &text, style);
            if x > start {
                self.regions
                    .tabs
                    .push((Rect::new(start, y, x - start, 1), tab));
            }
        }
        // Help and palette buttons at the right edge.
        let help = "[? help]";
        let palette = "[: cmd]";
        let needed = (help.len() + palette.len() + 1) as u16;
        if x + needed < right {
            let mut bx = right - needed;
            let start = bx;
            put(buf, &mut bx, y, right, palette, Style::default());
            self.regions
                .buttons
                .push((Rect::new(start, y, bx - start, 1), Command::Palette));
            bx += 1;
            let start = bx;
            put(buf, &mut bx, y, right, help, Style::default());
            self.regions
                .buttons
                .push((Rect::new(start, y, bx - start, 1), Command::Help));
        }
    }

    fn button_text(&self, command: Command) -> String {
        let label = command.button(self);
        match shortcut(command) {
            Some(key) => format!("[{label} {key}]"),
            None => format!("[{label}]"),
        }
    }

    /// Rows `buttons` take when wrapped into `width` columns.
    fn button_rows(&self, width: u16, buttons: &[Command]) -> u16 {
        let mut rows = 1u16;
        let mut used = 0u16;
        for &command in buttons {
            let w = self.button_text(command).chars().count() as u16;
            if used > 0 && used + w > width {
                rows += 1;
                used = 0;
            }
            used += w + 1;
        }
        rows
    }

    /// Draw `[Label k]` buttons, wrapping inside `area`, and record them
    /// for clicks. Unavailable buttons are dim; a click shows the reason.
    fn draw_buttons(&mut self, buf: &mut Buffer, area: Rect, buttons: &[Command]) {
        let right = area.x + area.width;
        let bottom = area.y + area.height;
        let mut x = area.x;
        let mut y = area.y;
        for &command in buttons {
            let text = self.button_text(command);
            let w = text.chars().count() as u16;
            if x > area.x && x + w > right {
                x = area.x;
                y += 1;
            }
            if y >= bottom {
                break;
            }
            let style = if command.availability(self).is_ok() {
                Style::default()
            } else {
                dim()
            };
            let start = x;
            put(buf, &mut x, y, right, &text, style);
            if x > start {
                self.regions
                    .buttons
                    .push((Rect::new(start, y, x - start, 1), command));
            }
            x += 1;
        }
    }

    fn draw_fleet(&mut self, buf: &mut Buffer, area: Rect, class: SizeClass) {
        let keys = self.key_focus == Pane::Fleet;
        let block = Block::default().borders(Borders::ALL).title(" FLEET ");
        let inner = block.inner(area);
        block.render(area, buf);
        let rows = self
            .button_rows(inner.width, &FLEET_BUTTONS)
            .min(inner.height);
        let hits = FleetTable {
            names: &self.names,
            ids: &self.profile_ids,
            rows: &self.fleet,
            state: &mut self.table,
            selected: self.focused,
            keys,
            class,
        }
        .render(inner, buf, rows);
        self.regions.fleet_rows = hits.rows;
        self.regions.fleet_first = hits.first;
        let buttons = Rect::new(inner.x, inner.y + inner.height - rows, inner.width, rows);
        self.draw_buttons(buf, buttons, &FLEET_BUTTONS);
        if keys {
            mark_focus(buf, area);
        }
    }

    fn draw_detail(&mut self, frame: &mut Frame<'_>, area: Rect, class: SizeClass) {
        let keys = self.key_focus == Pane::Detail;
        let mut body = area;
        if class != SizeClass::Compact && area.height > 1 {
            let buf = frame.buffer_mut();
            let right = area.x + area.width;
            let mut x = area.x;
            let style = if keys {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            put(buf, &mut x, area.y, right, &self.bot_line(false), style);
            body = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
        }
        let primary = match self.screen {
            Screen::Overview => self.draw_overview(frame, body),
            Screen::Map => {
                self.draw_map(frame, body);
                body
            }
            Screen::Script => {
                self.script_area = body;
                self.draw_script(frame, body);
                body
            }
            Screen::Chat => {
                self.chat_area = body;
                self.draw_chat(frame, body);
                body
            }
            Screen::Logs => {
                let focused = self.focused_name();
                frame.render_widget(
                    LogPane {
                        state: &self.log,
                        focused: focused.as_deref(),
                    },
                    body,
                );
                self.regions.log_rows = LogPane::rows_area(body);
                body
            }
        };
        if keys {
            mark_focus(frame.buffer_mut(), primary);
        }
    }

    /// Overview: the selected bot's buttons, status rows and inventory.
    /// Returns the rect that shows keyboard focus.
    fn draw_overview(&mut self, frame: &mut Frame<'_>, area: Rect) -> Rect {
        let buttons: Vec<Command> = OVERVIEW_BUTTONS
            .into_iter()
            .filter(|c| *c != Command::DismissNotice || self.background_notice.is_some())
            .collect();
        let rows = self.button_rows(area.width, &buttons).min(area.height);
        let button_area = Rect::new(area.x, area.y, area.width, rows);
        self.draw_buttons(frame.buffer_mut(), button_area, &buttons);
        let rest = Rect::new(area.x, area.y + rows, area.width, area.height - rows);
        if rest.width >= 70 {
            let [status, inventory] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(rest);
            self.draw_status(frame, status);
            self.draw_inv_locs(frame, inventory);
            status
        } else {
            let inventory_h = 5.min(rest.height / 3);
            let [status, inventory] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(inventory_h)]).areas(rest);
            self.draw_status(frame, status);
            if inventory.height > 0 {
                self.draw_inv_locs(frame, inventory);
            }
            status
        }
    }

    /// Large: the selected bot's status and chat beside the detail tab
    /// (whichever the tab itself is not showing).
    fn draw_side(&mut self, frame: &mut Frame<'_>, area: Rect) {
        match self.screen {
            Screen::Overview => {
                self.chat_area = area;
                self.draw_chat(frame, area);
            }
            Screen::Chat => self.draw_status(frame, area),
            _ => {
                // Chat keeps its preferred rows (paint buttons included);
                // status takes the rest.
                let chat_h = self
                    .chat_data
                    .view()
                    .preferred_height()
                    .min(area.height.saturating_sub(8));
                let [status, chat] =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(chat_h)]).areas(area);
                self.draw_status(frame, status);
                if chat.height >= 3 {
                    self.chat_area = chat;
                    self.draw_chat(frame, chat);
                }
            }
        }
    }

    /// The message line: the last report or error, clickable for the full
    /// text.
    fn draw_message_line(&mut self, buf: &mut Buffer, area: Rect) {
        let right = area.x + area.width;
        let mut x = area.x;
        match &self.error {
            Some(message) => {
                put(buf, &mut x, area.y, right, "msg: ", dim());
                put(buf, &mut x, area.y, right, message, Style::default());
                self.regions.message = Rect::new(area.x, area.y, area.width, 1);
            }
            None => put(buf, &mut x, area.y, right, "msg: —", dim()),
        }
    }

    fn draw_drawer(&mut self, buf: &mut Buffer, area: Rect, class: SizeClass) {
        if area.height == 0 {
            return;
        }
        if area.height == 1 || class == SizeClass::Compact {
            self.draw_message_line(buf, Rect::new(area.x, area.y, area.width, 1));
            let right = area.x + area.width;
            if area.height >= 2 {
                let y = area.y + 1;
                let mut x = area.x;
                put(buf, &mut x, y, right, "log: ", dim());
                if self.log.newest().is_some() {
                    render_rows(&self.log, Rect::new(x, y, right - x, 1), buf, 0);
                } else {
                    put(buf, &mut x, y, right, "(empty)", dim());
                }
            }
            if area.height >= 3 {
                let y = area.y + 2;
                let (unread, loud) = self.log.unread();
                let text = format!(
                    "logs: {unread} new, {loud} warn/error · F7 opens Logs · {}",
                    self.log.header(self.focused_name().as_deref())
                );
                let mut x = area.x;
                put(buf, &mut x, y, right, &text, dim());
            }
            return;
        }
        let keys = self.key_focus == Pane::Drawer;
        let focused = self.focused_name();
        let title = format!(" LOG {} ", self.log.header(focused.as_deref()));
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 {
            return;
        }
        self.draw_message_line(buf, Rect::new(inner.x, inner.y, inner.width, 1));
        let rows = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
        render_rows(&self.log, rows, buf, self.log.scroll);
        self.regions.log_rows = rows;
        if keys {
            mark_focus(buf, area);
        }
    }

    fn draw_footer(&mut self, buf: &mut Buffer, area: Rect) {
        if area.height == 0 {
            return;
        }
        let right = area.x + area.width;
        let mut x = area.x;
        let scope = format!("KEYS {} ▸ ", self.key_scope());
        put(
            buf,
            &mut x,
            area.y,
            right,
            &scope,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        let mouse = if self.mouse_capture { "on" } else { "off" };
        let tail = if area.width >= 100 {
            format!(" │ Tab pane · ^P cmd · ? help · mouse {mouse}")
        } else {
            format!(" │ ? help · mouse {mouse}")
        };
        let tail_w = tail.chars().count() as u16;
        let hints_right = right.saturating_sub(tail_w).max(x);
        put(
            buf,
            &mut x,
            area.y,
            hints_right,
            self.key_hints(),
            Style::default(),
        );
        let mut tx = hints_right.max(x);
        put(buf, &mut tx, area.y, right, &tail, dim());
    }

    /// Map buttons over the map info rows (the second, explicit step after
    /// a click or Enter selects a tile).
    pub(crate) fn draw_map_buttons(&mut self, buf: &mut Buffer, area: Rect) {
        self.draw_buttons(buf, area, &MAP_BUTTONS);
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

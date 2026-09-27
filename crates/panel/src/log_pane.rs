//! Panel log section over the shared `frontend_core::log` store: timestamp
//! column, level colours, level/source/scope filters, text search, follow,
//! Copy and Save log…. The section fills the side panel's leftover height
//! when it is the last section, and is a resizable box otherwise.

use dear_imgui_rs::{ChildFlags, Ui};
use frontend_core::log::{
    default_save_path, global, save_text, Level, LogEntry, LogScope, LogView, SaveTicket, Source,
};
use frontend_core::log_file::apply_session_log;

use crate::session::Session;
use crate::theme::{ERROR, TEXT, TEXT_DIM, WARN};

/// Smallest log box height (px) when it fills the leftover panel height.
const MIN_FILL_HEIGHT: f32 = 120.0;
/// Initial height (px) of the resizable box when other sections follow it.
const RESIZABLE_HEIGHT: f32 = 160.0;
const SAVE_POPUP: &str = "Save log";

/// Scope choice in the section; `Bot` follows the focused bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneScope {
    Bot,
    Process,
    All,
}

/// Renderer-local log state: the filtered view, the search buffer, the
/// Save log… dialog and its outcome.
pub struct LogPane {
    pub view: LogView,
    pub scope: PaneScope,
    pub search: String,
    save_path: String,
    save: Option<SaveTicket>,
    /// Last Copy/Save/session-file outcome shown under the controls.
    status: Option<(bool, String)>,
    clipboard: Option<arboard::Clipboard>,
}

impl Default for LogPane {
    fn default() -> Self {
        Self {
            view: LogView::new(LogScope::Process),
            scope: PaneScope::Bot,
            search: String::new(),
            save_path: String::new(),
            save: None,
            status: None,
            clipboard: None,
        }
    }
}

impl LogPane {
    /// Point the view at the focused bot (Bot scope) and pull new lines.
    /// Steady state: one atomic load, no allocation.
    pub fn refresh(&mut self, focused: Option<&str>) {
        match self.scope {
            PaneScope::Bot => self.view.follow_slot(focused),
            PaneScope::Process => self.view.set_scope(LogScope::Process),
            PaneScope::All => self.view.set_scope(LogScope::All),
        }
        global().refresh(&mut self.view);
        if let Some(ticket) = &self.save {
            if let Some(result) = ticket.poll() {
                self.save = None;
                self.status = Some(match result {
                    Ok(path) => (true, format!("saved {}", path.display())),
                    Err(e) => (false, e),
                });
            }
        }
    }

    fn copy(&mut self) {
        let text = self.view.to_text();
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        let copied = self
            .clipboard
            .as_mut()
            .is_some_and(|c| c.set_text(text).is_ok());
        self.status = Some(if copied {
            (true, format!("copied {} line(s)", self.view.len()))
        } else {
            (false, "copy: clipboard unavailable".into())
        });
    }

    fn scope_label(&self, focused: Option<&str>) -> String {
        match self.scope {
            PaneScope::Bot => focused.unwrap_or("process").to_string(),
            PaneScope::Process => "process".into(),
            PaneScope::All => "all".into(),
        }
    }
}

/// Text colour for a level.
pub fn level_color(level: Level) -> [f32; 4] {
    match level {
        Level::Debug => TEXT_DIM,
        Level::Info => TEXT,
        Level::Warn => WARN,
        Level::Error => ERROR,
    }
}

/// Stick to the bottom of the log when the last frame was already there
/// (1 px slack for float layout). Scrolling up to read history stays put.
pub fn log_follow_bottom(scroll_y: f32, scroll_max_y: f32) -> bool {
    scroll_y >= scroll_max_y - 1.0
}

/// The log section body. `last` is whether it is the last visible panel
/// section (then it fills the leftover height).
pub fn log_body(ui: &Ui, session: &mut Session, last: bool) {
    let focused = session.core.selected();
    let pane = &mut session.log_pane;
    pane.refresh(focused);
    filter_row(ui, pane);
    action_row(ui, pane, focused);
    if let Some((ok, text)) = &pane.status {
        ui.text_colored(if *ok { TEXT_DIM } else { ERROR }, text);
    }
    if pane.view.dropped() > 0 {
        ui.text_colored(TEXT_DIM, "older lines rolled off (ring bound)");
    }
    let height = if last {
        ui.content_region_avail()[1].max(MIN_FILL_HEIGHT)
    } else {
        RESIZABLE_HEIGHT
    };
    let flags = if last {
        ChildFlags::BORDERS
    } else {
        ChildFlags::BORDERS | ChildFlags::RESIZE_Y
    };
    let all = matches!(pane.view.scope(), LogScope::All);
    ui.child_window("panel-log")
        .size([0.0, height])
        .child_flags(flags)
        .build(ui, || {
            let at_bottom = log_follow_bottom(ui.scroll_y(), ui.scroll_max_y());
            let wheel = ui.io().mouse_wheel();
            if ui.is_window_hovered() && wheel > 0.0 {
                pane.view.follow = false;
            } else if at_bottom && wheel < 0.0 {
                pane.view.follow = true;
            }
            for entry in pane.view.rows() {
                log_row(ui, entry, all);
            }
            if pane.view.follow {
                ui.set_scroll_here_y(1.0);
            }
        });
}

fn log_row(ui: &Ui, entry: &LogEntry, all: bool) {
    ui.text_colored(TEXT_DIM, entry.clock.as_str());
    if ui.is_item_hovered() {
        ui.tooltip(|| {
            ui.text(entry.slot.as_deref().unwrap_or("process"));
            ui.text(entry.source.label());
            ui.text(entry.level.label());
            if let Some(tick) = entry.tick {
                ui.text(format!("tick {tick}"));
            }
        });
    }
    if all {
        ui.same_line();
        ui.text_colored(TEXT_DIM, entry.slot.as_deref().unwrap_or("*"));
    }
    ui.same_line();
    ui.text_colored(TEXT_DIM, entry.source.label());
    ui.same_line();
    let _color = ui.push_style_color(dear_imgui_rs::StyleColor::Text, level_color(entry.level));
    ui.text_wrapped(&*entry.message);
}

fn filter_row(ui: &Ui, pane: &mut LogPane) {
    let third = ((ui.content_region_avail()[0] - 8.0) / 3.0).max(40.0);
    ui.set_next_item_width(third);
    let min = pane.view.filter().min_level;
    if let Some(_open) = ui.begin_combo("##log-level", min.label()) {
        for level in Level::ALL {
            if ui
                .selectable_config(level.label())
                .selected(level == min)
                .build()
            {
                pane.view.edit_filter(|f| f.min_level = level);
            }
        }
    }
    ui.set_item_tooltip("minimum level");
    ui.same_line();
    ui.set_next_item_width(third);
    let source = pane.view.filter().single_source();
    if let Some(_open) = ui.begin_combo("##log-source", source.map_or("all sources", Source::label))
    {
        if ui
            .selectable_config("all sources")
            .selected(source.is_none())
            .build()
        {
            pane.view.edit_filter(|f| f.set_source(None));
        }
        for s in Source::ALL {
            if ui
                .selectable_config(s.label())
                .selected(source == Some(s))
                .build()
            {
                pane.view.edit_filter(|f| f.set_source(Some(s)));
            }
        }
    }
    ui.set_item_tooltip("source");
    ui.same_line();
    ui.set_next_item_width(-1.0);
    let scope_label = match pane.scope {
        PaneScope::Bot => "bot",
        PaneScope::Process => "process",
        PaneScope::All => "all bots",
    };
    if let Some(_open) = ui.begin_combo("##log-scope", scope_label) {
        for (scope, label) in [
            (PaneScope::Bot, "bot"),
            (PaneScope::Process, "process"),
            (PaneScope::All, "all bots"),
        ] {
            if ui
                .selectable_config(label)
                .selected(pane.scope == scope)
                .build()
            {
                pane.scope = scope;
            }
        }
    }
    ui.set_item_tooltip("focused bot, process lines, or every bot");
    ui.set_next_item_width(-70.0);
    if ui
        .input_text("##log-search", &mut pane.search)
        .hint("search")
        .build()
    {
        let text = pane.search.clone();
        pane.view.edit_filter(|f| f.text = text);
    }
    ui.same_line();
    ui.checkbox("follow", &mut pane.view.follow);
}

fn action_row(ui: &Ui, pane: &mut LogPane, focused: Option<&str>) {
    let half = ((ui.content_region_avail()[0] - 8.0) / 2.0).max(40.0);
    if ui.button_with_size("Copy", [half, 0.0]) {
        pane.copy();
    }
    ui.same_line();
    let saving = pane.save.is_some();
    let _disabled = saving.then(|| ui.begin_disabled());
    if ui.button_with_size(if saving { "Saving…" } else { "Save log…" }, [half, 0.0]) {
        pane.save_path = default_save_path(&pane.scope_label(focused))
            .display()
            .to_string();
        ui.open_popup(SAVE_POPUP);
    }
    drop(_disabled);
    ui.modal_popup(SAVE_POPUP, || {
        ui.text(format!(
            "{} line(s) with the current filters",
            pane.view.len()
        ));
        ui.set_next_item_width(420.0);
        ui.input_text("##save-log-path", &mut pane.save_path)
            .build();
        if ui.button("Save") && !pane.save_path.trim().is_empty() {
            let path = std::path::PathBuf::from(pane.save_path.trim());
            pane.save = Some(save_text(path, pane.view.to_text()));
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel") {
            ui.close_current_popup();
        }
    });
}

/// General config row: the shared per-session log file preference.
pub fn session_log_row(ui: &Ui, session: &mut Session) {
    let mut on = session.ui.session_log_file;
    if ui.checkbox("session log file", &mut on) {
        session.ui.session_log_file = on;
        crate::ui_state::save(&session.ui);
        let path = apply_session_log(on);
        session.log_pane.status = path.map(|p| (true, format!("writing {}", p.display())));
    }
    ui.set_item_tooltip("write this session's log to ~/.274bot/logs/ (rotated, off by default)");
}

/// Start the session file at launch when the saved preference is on.
pub fn apply_session_log_pref(session: &Session) {
    if session.ui.session_log_file {
        apply_session_log(true);
    }
}

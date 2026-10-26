//! Panel log section over the shared `frontend_core::log` store: timestamp
//! column, level colours, level/source/scope filters, text search, follow,
//! Copy and Save log…. The same body appears inline and in its dockable tab.

use std::path::PathBuf;

use dear_imgui_rs::{ChildFlags, Ui};
use frontend_core::log::{
    default_save_path, global, Level, LogEntry, LogScope, LogView, SaveTicket, Source,
};

use crate::session::Session;
use crate::theme::{scale_px, ERROR, TEXT, TEXT_DIM, WARN};

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
    /// The Save button was pressed: the next body draw submits `SaveLog`.
    save_requested: bool,
    save: Option<SaveTicket>,
    /// Last Copy/Save/session-file outcome shown under the controls.
    status: Option<(bool, String)>,
    clipboard: Option<arboard::Clipboard>,
    /// The stage and file the session-file label was built for, and the
    /// label (`writing <path>` / `opening <path>`); rebuilt when either
    /// moved, so a steady frame builds nothing.
    session_file_label_for: Option<(frontend_core::log_file::SessionFilePhase, u64)>,
    session_file_label: String,
}

impl Default for LogPane {
    fn default() -> Self {
        Self {
            view: LogView::new(LogScope::Process),
            scope: PaneScope::Bot,
            search: String::new(),
            save_path: String::new(),
            save_requested: false,
            save: None,
            status: None,
            clipboard: None,
            session_file_label_for: None,
            session_file_label: String::new(),
        }
    }
}

impl LogPane {
    /// The session-file label for `phase` (`Writing` or `Opening`) of the
    /// file with `generation`: `writing <path>` or `opening <path>`. Built
    /// again only when the stage or the file changed.
    fn session_file_label(
        &mut self,
        phase: frontend_core::log_file::SessionFilePhase,
        generation: u64,
    ) -> &str {
        use frontend_core::log_file::SessionFilePhase;
        use std::fmt::Write as _;
        if self.session_file_label_for != Some((phase, generation)) {
            self.session_file_label_for = Some((phase, generation));
            self.session_file_label.clear();
            let status = frontend_core::log_file::session_file_status();
            if let Some(path) = status.path() {
                let verb = if phase == SessionFilePhase::Writing {
                    "writing"
                } else {
                    "opening"
                };
                let _ = write!(self.session_file_label, "{verb} {}", path.display());
            }
        }
        &self.session_file_label
    }
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

/// The log body is shared by the inline section and the dockable Log tab.
/// `last` is whether the body fills the remaining panel height.
pub fn log_body(ui: &Ui, session: &mut Session, last: bool) {
    if std::mem::take(&mut session.log_pane.save_requested) {
        session.save_log();
    }
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
        ui.content_region_avail()[1].max(scale_px(ui, MIN_FILL_HEIGHT))
    } else {
        scale_px(ui, RESIZABLE_HEIGHT)
    };
    let flags = if last {
        ChildFlags::BORDERS
    } else {
        ChildFlags::BORDERS | ChildFlags::RESIZE_Y
    };
    let all = pane.view.scope().is_all();
    ui.child_window("panel-log")
        .size([0.0, height])
        .child_flags(flags)
        .build(ui, || {
            let at_bottom = log_follow_bottom(ui.scroll_y(), ui.scroll_max_y());
            let wheel = ui.io().mouse_wheel();
            if ui.is_window_hovered() && wheel > 0.0 {
                pane.view.set_follow(false);
            } else if at_bottom && wheel < 0.0 {
                pane.view.set_follow(true);
            }
            for entry in pane.view.rows() {
                log_row(ui, entry, all);
            }
            if pane.view.follow() {
                ui.set_scroll_here_y(1.0);
            }
        });
}

fn log_row(ui: &Ui, entry: &LogEntry, all: bool) {
    ui.text_colored(TEXT_DIM, entry.clock().as_str());
    if ui.is_item_hovered() {
        ui.tooltip(|| {
            ui.text(entry.slot().unwrap_or("process"));
            ui.text(entry.source().label());
            ui.text(entry.level().label());
            if let Some(tick) = entry.tick() {
                ui.text(format!("tick {tick}"));
            }
        });
    }
    if all {
        ui.same_line();
        ui.text_colored(TEXT_DIM, entry.slot().unwrap_or("*"));
    }
    ui.same_line();
    ui.text_colored(TEXT_DIM, entry.source().label());
    ui.same_line();
    let _color = ui.push_style_color(dear_imgui_rs::StyleColor::Text, level_color(entry.level()));
    ui.text_wrapped(entry.message());
}

fn filter_row(ui: &Ui, pane: &mut LogPane) {
    let third = ((ui.content_region_avail()[0] - scale_px(ui, 8.0)) / 3.0).max(scale_px(ui, 40.0));
    ui.set_next_item_width(third);
    let min = pane.view.filter().min_level();
    if let Some(_open) = ui.begin_combo("##log-level", min.label()) {
        for level in Level::ALL {
            if ui
                .selectable_config(level.label())
                .selected(level == min)
                .build()
            {
                pane.view.edit_filter(|f| f.set_min_level(level));
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
    ui.set_next_item_width(-scale_px(ui, 70.0));
    if ui
        .input_text("##log-search", &mut pane.search)
        .hint("search")
        .build()
    {
        let text = pane.search.clone();
        pane.view.edit_filter(|f| f.set_text(text));
    }
    ui.same_line();
    let mut follow = pane.view.follow();
    ui.checkbox("follow", &mut follow);
    pane.view.set_follow(follow);
}

fn action_row(ui: &Ui, pane: &mut LogPane, focused: Option<&str>) {
    let half = ((ui.content_region_avail()[0] - scale_px(ui, 8.0)) / 2.0).max(scale_px(ui, 40.0));
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
        ui.set_next_item_width(scale_px(ui, 420.0));
        ui.input_text("##save-log-path", &mut pane.save_path)
            .build();
        if ui.button("Save") && !pane.save_path.trim().is_empty() {
            pane.save_requested = true;
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
    let mut on = session.session_log_shown();
    if ui.checkbox("session log file", &mut on) {
        session.set_session_log(on);
    }
    ui.set_item_tooltip("write this session's log to ~/.274bot/logs/ (rotated, off by default)");
    crate::app::setting_feedback_line(ui, session, frontend_core::Setting::SessionLog);
    if on
        && session
            .core
            .setting_failure(frontend_core::Setting::SessionLog)
            .is_none()
    {
        // The file is the sink's own state, not the saved choice.
        let phase = frontend_core::log_file::session_file_phase();
        if matches!(
            phase,
            frontend_core::log_file::SessionFilePhase::Writing
                | frontend_core::log_file::SessionFilePhase::Opening
        ) {
            let generation = frontend_core::log_file::session_file_generation();
            let label = session.log_pane.session_file_label(phase, generation);
            if !label.is_empty() {
                ui.text_wrapped(label);
            }
        }
    }
}

impl Session {
    /// Save log…: the rows the view shows, to the dialog's path, through
    /// the `SaveLog` command; the pane follows its write for the status.
    pub(crate) fn save_log(&mut self) {
        let path = PathBuf::from(self.log_pane.save_path.trim());
        let text = self.log_pane.view.to_text();
        self.core.stage_log_save(path, text);
        if let Some(submitted) = self.submit(
            frontend_core::OperatorCommand::SaveLog,
            frontend_core::Targets::Application,
        ) {
            match self.core.command_failure(submitted.request()) {
                Some(error) => self.log_pane.status = Some((false, error.to_string())),
                None => self.log_pane.save = self.core.log_save(submitted.request()),
            }
        }
    }

    /// The "session log file" preference, through `SetSessionLog`.
    /// Durable-only: the core opens or closes the file when the saved value
    /// holds it. The checkbox shows the choice while the write is pending
    /// ([`Self::session_log_shown`]); a failed write, or a file that could
    /// not be opened, shows beside the checkbox.
    pub(crate) fn set_session_log(&mut self, on: bool) {
        self.setting_drafts.session_log = Some(on);
        self.submit(
            frontend_core::OperatorCommand::SetSessionLog(on),
            frontend_core::Targets::Application,
        );
        self.settle_setting_drafts();
    }

    /// What the session-log checkbox shows: the operator's choice while its
    /// write is pending, else the saved value.
    pub(crate) fn session_log_shown(&self) -> bool {
        self.setting_drafts
            .session_log
            .unwrap_or(self.ui.session_log_file)
    }
}

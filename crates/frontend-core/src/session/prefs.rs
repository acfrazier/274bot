//! Preference and notice rows of the command seam: the draw policy, the
//! shared navigation preferences and danger level, the session log file,
//! the map-bake choice, the Quester Paths settings and reload, the one-time
//! notices and Save log.
//!
//! Every operator preference is written here, through the process
//! local-state service's preferences document; a front end writes only its
//! own presentation state ([`crate::UiPreferences::update`]).

use std::path::PathBuf;
use std::sync::Arc;

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{DrawSwitch, FailureKind, OperatorCommand, Rejection, SkipReason};
use crate::map_bake::MapBakeChoice;
use crate::nav_prefs::NavPreference;
use crate::quester_paths::QuesterPathsView;
use crate::surface::SlotSurface;
use crate::walk_permissions::{DangerLevel, WalkGlobalsView};
use crate::OperatorSession;

/// The panel preference key of "capture input" (absent = on).
pub const CAPTURE_KEY: &str = "capture";
/// The panel preference key of "sidecar 50 fps" (absent = off).
pub const SIDECAR_50_KEY: &str = "sidecar_50";
/// The panel preference key of "focused 50 fps" (absent = on).
pub const FOCUSED_50_KEY: &str = "focused_50";
/// The panel preference key of the render-all warning's "Don't show again".
pub const RENDER_ALL_WARNING_ACK_KEY: &str = "render_all_warning_ack";

/// One preference row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefRow {
    Draw(DrawSwitch, bool),
    Nav(NavPreference, bool),
    Danger(DangerLevel),
    SessionLog(bool),
    MapBake(MapBakeChoice),
    QuesterPaths,
    ReloadQuesterPaths,
    AcknowledgeNotice,
    SaveLog,
}

impl PrefRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Draw(switch, on) => OperatorCommand::SetDraw(switch, on),
            Self::Nav(pref, on) => OperatorCommand::SetNavPreference(pref, on),
            Self::Danger(level) => OperatorCommand::SetDangerLevel(level),
            Self::SessionLog(on) => OperatorCommand::SetSessionLog(on),
            Self::MapBake(choice) => OperatorCommand::SetMapBakeChoice(choice),
            Self::QuesterPaths => OperatorCommand::SetQuesterPaths,
            Self::ReloadQuesterPaths => OperatorCommand::ReloadQuesterPaths,
            Self::AcknowledgeNotice => OperatorCommand::AcknowledgeNotice,
            Self::SaveLog => OperatorCommand::SaveLog,
        }
    }
}

/// The one-time notices [`OperatorCommand::AcknowledgeNotice`] answers
/// (staged with [`OperatorSession::stage_notice`]). The walk-permission
/// and danger-routing notices are navigation preferences
/// ([`NavPreference::ScriptScopeNoticeAck`],
/// [`NavPreference::SurvivableRoutingNoticeAck`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// "Other profiles keep running in the background": don't show again.
    BackgroundBots,
    /// The render-all warning: don't show again.
    RenderAllWarning,
    /// The Quester Paths status line: dismiss it.
    QuesterPaths,
}

/// The panel's draw policy: the saved draw switches and the run-only full
/// rate. A front end reads it to mirror the switches onto its renderer; it
/// changes only through [`OperatorCommand::SetDraw`].
#[derive(Debug, PartialEq, Eq)]
pub struct DrawPolicy {
    capture: bool,
    sidecar_50: bool,
    focused_50: bool,
    live_full_rate: bool,
    persist: bool,
}

impl DrawPolicy {
    /// A policy with these saved values. With `persist` false (a live
    /// harness run) changes are not written.
    pub fn new(capture: bool, sidecar_50: bool, focused_50: bool, persist: bool) -> Self {
        Self {
            capture,
            sidecar_50,
            focused_50,
            live_full_rate: false,
            persist,
        }
    }

    /// The saved policy in `preferences` (absent keys take their
    /// defaults: capture on, sidecar 50 fps off, focused 50 fps on).
    pub fn restore(preferences: &crate::UiPreferences, persist: bool) -> Self {
        let read = |key: &str, default: bool| {
            preferences
                .value(key)
                .and_then(|value| value.as_bool())
                .unwrap_or(default)
        };
        Self::new(
            read(CAPTURE_KEY, true),
            read(SIDECAR_50_KEY, false),
            read(FOCUSED_50_KEY, true),
            persist,
        )
    }

    /// Whether changes are written (off for a live harness run, whose
    /// draw setup is never the operator's).
    pub fn set_persist(&mut self, persist: bool) {
        self.persist = persist;
    }

    /// "capture input".
    pub fn capture(&self) -> bool {
        self.capture
    }

    /// "sidecar 50 fps".
    pub fn sidecar_50(&self) -> bool {
        self.sidecar_50
    }

    /// "focused 50 fps".
    pub fn focused_50(&self) -> bool {
        self.focused_50
    }

    /// "full rate (this run)".
    pub fn live_full_rate(&self) -> bool {
        self.live_full_rate
    }

    /// The saved preference key of `switch`; `None` for a switch that is
    /// not saved (the per-member renderer, this run's full rate).
    pub const fn key(switch: DrawSwitch) -> Option<&'static str> {
        match switch {
            DrawSwitch::CaptureInput => Some(CAPTURE_KEY),
            DrawSwitch::SidecarFullRate => Some(SIDECAR_50_KEY),
            DrawSwitch::FocusedFullRate => Some(FOCUSED_50_KEY),
            DrawSwitch::GameRenderer | DrawSwitch::LiveFullRate => None,
        }
    }

    /// Set `switch`; `true` when the value changed.
    fn set(&mut self, switch: DrawSwitch, on: bool) -> bool {
        let slot = match switch {
            DrawSwitch::CaptureInput => &mut self.capture,
            DrawSwitch::SidecarFullRate => &mut self.sidecar_50,
            DrawSwitch::FocusedFullRate => &mut self.focused_50,
            DrawSwitch::LiveFullRate => &mut self.live_full_rate,
            DrawSwitch::GameRenderer => return true,
        };
        std::mem::replace(slot, on) != on
    }
}

impl<Io> OperatorSession<Io> {
    /// Stage the notice the next [`OperatorCommand::AcknowledgeNotice`]
    /// answers.
    pub fn stage_notice(&mut self, notice: Notice) {
        self.commands.stage_notice(notice);
    }

    /// Stage the next [`OperatorCommand::SaveLog`]: write `text` (the
    /// rows a log view shows) to `path`.
    pub fn stage_log_save(&mut self, path: PathBuf, text: String) {
        self.commands.stage_log_save(path, text);
    }

    /// Stage the Quester Paths settings the next
    /// [`OperatorCommand::SetQuesterPaths`] applies, with the bound
    /// profile's game data a changed source reloads with; with `persist`
    /// false (a live harness run) they are applied and not written.
    pub fn stage_quester_paths(
        &mut self,
        after: QuesterPathsView,
        persist: bool,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        self.commands.stage_quester_paths(after, persist, selected);
    }

    /// Stage the bound profile's game data the next
    /// [`OperatorCommand::ReloadQuesterPaths`] reloads with.
    pub fn stage_quester_reload(
        &mut self,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        self.commands.stage_quester_game_data(selected);
    }

    /// Startup: project the bound operator preferences onto the session
    /// (the durable walk globals and their store, the manual-movement
    /// pause) and return the walk-permission view the front end shows.
    pub fn adopt_operator_preferences(&mut self) -> WalkGlobalsView {
        let Some(preferences) = self.preferences() else {
            return WalkGlobalsView::default();
        };
        let view = WalkGlobalsView::read(&preferences);
        self.set_walk_globals(view.globals());
        self.set_walk_globals_store(preferences.share());
        let pause = crate::nav_prefs::nav_preference(
            &preferences,
            NavPreference::PauseScriptOnManualWalkAbort,
        )
        .unwrap_or(true);
        self.set_pause_script_on_manual_walk_abort(pause);
        view
    }

    /// Startup: this front end draws no game image (the TUI), so the
    /// capacity warning names the render policy `Headless`. A property of
    /// the front end, not an operator decision; `SetRenderAll` is the
    /// panel's choice between rendering every member and only the focused
    /// one.
    pub fn declare_headless_render(&mut self) {
        self.set_capacity_render(crate::capacity::RenderPolicyLabel::Headless);
    }

    /// Re-read the durable walk globals (another process may have changed
    /// them) and project them onto the play session; the view for the
    /// front end. A pump: it adopts the stored decision, it makes none.
    pub fn refresh_walk_globals(&mut self) -> WalkGlobalsView {
        let Some(preferences) = self.preferences() else {
            return WalkGlobalsView::default();
        };
        let view = WalkGlobalsView::read(&preferences);
        self.set_walk_globals(view.globals());
        view
    }

    /// A handle on the running write of the SaveLog command `request`, for
    /// the front end's status line ([`crate::log::SaveTicket::poll`]).
    pub fn log_save(&self, request: RequestId) -> Option<crate::log::SaveTicket> {
        self.commands.log_save_ticket(request)
    }

    /// The saved value of a navigation preference, read from the bound
    /// preferences document now; `None` without one.
    pub fn nav_preference(&self, preference: NavPreference) -> Option<bool> {
        let preferences = self.preferences()?;
        crate::nav_prefs::nav_preference(&preferences, preference).ok()
    }

    /// The saved session-log-file preference; `None` without a bound
    /// preferences document.
    pub fn session_log_preference(&self) -> Option<bool> {
        self.preferences()
            .map(|preferences| crate::log_file::session_log_setting(&preferences))
    }

    /// The saved map-bake choice; `None` without a bound preferences
    /// document.
    pub fn map_bake_preference(&self) -> Option<MapBakeChoice> {
        self.preferences()
            .map(|preferences| crate::map_bake::load_map_bake_choice(&preferences))
    }

    /// Advance the Quester Paths reload `paths` owns with `selected` game
    /// data (once per front-end frame) and settle the
    /// [`OperatorCommand::ReloadQuesterPaths`] it finished.
    pub fn advance_quester_paths(
        &mut self,
        paths: &mut crate::QuesterPathsController,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        paths.advance(selected);
        if let Some((request, outcome)) = paths.take_settled_reload() {
            let (terminal, detail) = match outcome {
                Ok(()) => (
                    api::identity::Settlement::Completed(crate::command::Completion::Done),
                    None,
                ),
                Err(reason) => (
                    api::identity::Settlement::Failed(crate::command::CommandFailure::Failed(
                        FailureKind::JobFailed,
                    )),
                    Some(reason),
                ),
            };
            self.settle_later(
                request,
                crate::command::Subject::Application,
                terminal,
                detail.as_deref(),
            );
        }
    }

    pub(super) fn pref_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: PrefRow,
        request: RequestId,
        admitted: bool,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        if !admitted {
            self.settle_app(request, command, Applied::NotAdmitted, submitted);
            return;
        }
        let applied = match row {
            PrefRow::Draw(switch, on) => self.set_draw(switch, on, surface),
            PrefRow::Nav(preference, on) => self.set_nav_preference(preference, on, surface),
            PrefRow::Danger(level) => self.set_danger_level(level, surface),
            PrefRow::SessionLog(on) => self.set_session_log(on),
            PrefRow::MapBake(choice) => self.set_map_bake_choice(choice, surface),
            PrefRow::QuesterPaths => self.set_quester_paths(surface),
            PrefRow::ReloadQuesterPaths => self.reload_quester_paths(request, surface),
            PrefRow::AcknowledgeNotice => self.acknowledge_notice(surface),
            PrefRow::SaveLog => self.save_log(request),
        };
        self.settle_app(request, command, applied, submitted);
    }

    fn set_draw<S: SlotSurface<Io = Io>>(
        &mut self,
        switch: DrawSwitch,
        on: bool,
        surface: &mut S,
    ) -> Applied {
        let persist = {
            let mut models = surface.models();
            let Some(draw) = models.draw.as_deref_mut() else {
                return no_model("draw policy");
            };
            draw.set(switch, on);
            draw.persist
        };
        let write = match (persist, DrawPolicy::key(switch)) {
            (true, Some(key)) => Some(self.write_preference(key, serde_json::Value::Bool(on))),
            _ => None,
        };
        surface.draw_changed(switch, on);
        match switch {
            DrawSwitch::GameRenderer | DrawSwitch::CaptureInput => {
                if let Some(name) = self.selected.clone() {
                    self.wake_slot(&name);
                }
            }
            DrawSwitch::SidecarFullRate
            | DrawSwitch::FocusedFullRate
            | DrawSwitch::LiveFullRate => self.wake_all_slots(),
        }
        if let Some(name) = DrawPolicy::key(switch).filter(|_| persist) {
            crate::log::global().process_line(
                crate::log::Source::Host,
                crate::log::Level::Info,
                &format!("panel preference {name}: {}", on),
            );
        }
        match write {
            Some(Err(applied)) => applied,
            Some(Ok(())) | None => Applied::Done,
        }
    }

    fn set_nav_preference<S: SlotSurface<Io = Io>>(
        &mut self,
        preference: NavPreference,
        on: bool,
        surface: &mut S,
    ) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        if let Err(error) = crate::nav_prefs::write_nav_preference(&preferences, preference, on) {
            return write_failed("navigation preference", &error);
        }
        if preference == NavPreference::PauseScriptOnManualWalkAbort {
            self.set_pause_script_on_manual_walk_abort(on);
        }
        self.adopt_walk_globals(&preferences, surface);
        Applied::Done
    }

    fn set_danger_level<S: SlotSurface<Io = Io>>(
        &mut self,
        level: DangerLevel,
        surface: &mut S,
    ) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        let before = WalkGlobalsView::read(&preferences);
        let mut globals = before.globals();
        globals.set_danger_level(level);
        let after = WalkGlobalsView::new(
            globals,
            before.script_scope_notice_ack(),
            before.survivable_routing_notice_ack(),
        );
        if let Err(error) = WalkGlobalsView::persist_changed(&preferences, before, after) {
            return write_failed("danger routing", &error);
        }
        self.adopt_walk_globals(&preferences, surface);
        Applied::Done
    }

    /// Read the durable walk globals back and hand them to the play
    /// session and the front end's lent view.
    fn adopt_walk_globals<S: SlotSurface<Io = Io>>(
        &mut self,
        preferences: &crate::UiPreferences,
        surface: &mut S,
    ) {
        let view = WalkGlobalsView::read(preferences);
        self.set_walk_globals(view.globals());
        if let Some(lent) = surface.models().walk_globals.as_deref_mut() {
            *lent = view;
        }
    }

    fn set_session_log(&mut self, on: bool) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        if let Err(error) = crate::log_file::persist_session_log_setting(&preferences, on) {
            return write_failed("session file setting", &error);
        }
        crate::log_file::apply_session_log(on);
        Applied::Done
    }

    fn set_map_bake_choice<S: SlotSurface<Io = Io>>(
        &mut self,
        choice: MapBakeChoice,
        surface: &mut S,
    ) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        if let Err(error) = crate::map_bake::persist_map_bake_choice(&preferences, choice) {
            return write_failed("map bake choice", &error);
        }
        if let Some(desk) = surface.models().map.as_deref_mut() {
            desk.bake_gate_mut().set_choice(choice);
        }
        Applied::Done
    }

    fn set_quester_paths<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) -> Applied {
        let Some((after, persist)) = self.commands.take_quester_paths() else {
            return not_staged("Quest Paths settings");
        };
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        let selected = self.commands.take_quester_game_data();
        let mut models = surface.models();
        let Some(paths) = models.quester.as_deref_mut() else {
            return no_model("Quest Paths settings");
        };
        match paths.apply_changed(&preferences, &after, selected, persist) {
            Ok(()) => Applied::Done,
            Err(reason) => Applied::Failed(FailureKind::WriteFailed, Some(reason)),
        }
    }

    fn reload_quester_paths<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
    ) -> Applied {
        let selected = self.commands.take_quester_game_data();
        let mut models = surface.models();
        let Some(paths) = models.quester.as_deref_mut() else {
            return no_model("Quest Paths");
        };
        match paths.request_reload_for(request, selected) {
            Ok(()) => Applied::Later,
            Err(crate::quester_paths::ReloadRefused::Running) => {
                Applied::Skipped(SkipReason::NotReady)
            }
            Err(crate::quester_paths::ReloadRefused::NoGameData) => Applied::Rejected(
                Rejection::NotApplicable,
                Some("Load a profile before reloading quest Paths.".into()),
            ),
        }
    }

    fn acknowledge_notice<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) -> Applied {
        let Some(notice) = self.commands.take_notice() else {
            return not_staged("notice");
        };
        match notice {
            Notice::BackgroundBots => {
                let Some(preferences) = self.preferences() else {
                    return no_store();
                };
                match host_play::persist_background_bots_ack(preferences.inner()) {
                    Ok(()) => Applied::Done,
                    Err(error) => Applied::Failed(
                        FailureKind::WriteFailed,
                        Some(host_play::background_bots_ack_error(&error)),
                    ),
                }
            }
            Notice::RenderAllWarning => {
                match self.write_preference(RENDER_ALL_WARNING_ACK_KEY, true.into()) {
                    Ok(()) => Applied::Done,
                    Err(applied) => applied,
                }
            }
            Notice::QuesterPaths => match surface.models().quester.as_deref_mut() {
                Some(paths) => {
                    paths.clear_notice();
                    Applied::Done
                }
                None => no_model("Quest Paths"),
            },
        }
    }

    fn save_log(&mut self, request: RequestId) -> Applied {
        let Some((path, text)) = self.commands.take_log_save() else {
            return not_staged("log save");
        };
        let ticket = crate::log::save_text(path, text);
        self.commands.watch_log_save(request, ticket);
        Applied::Later
    }

    /// Write one top-level preference; the failure as the member's terminal.
    fn write_preference(&self, key: &str, value: serde_json::Value) -> Result<(), Applied> {
        let preferences = self.preferences().ok_or_else(no_store)?;
        preferences
            .set(key, value)
            .map_err(|error| write_failed(key, &error))
    }
}

fn no_model(what: &str) -> Applied {
    Applied::Rejected(
        Rejection::NoModel,
        Some(format!("{what}: this front end holds none")),
    )
}

fn no_store() -> Applied {
    Applied::Rejected(
        Rejection::NotApplicable,
        Some("preferences: no preferences store is bound".into()),
    )
}

fn not_staged(what: &str) -> Applied {
    Applied::Rejected(
        Rejection::NotStaged,
        Some(format!("{what}: nothing staged")),
    )
}

fn write_failed(what: &str, error: &std::io::Error) -> Applied {
    Applied::Failed(FailureKind::WriteFailed, Some(format!("{what}: {error}")))
}

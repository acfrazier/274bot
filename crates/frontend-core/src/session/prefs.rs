//! Preference and notice rows of the command seam: the draw policy, the
//! shared navigation preferences and danger level, the session log file,
//! the map-bake choice, the Quester Paths settings and reload, the one-time
//! notices and Save log.
//!
//! Every operator preference is written here, through the process
//! local-state service's preferences document; a front end writes only its
//! own presentation state ([`crate::UiPreferences::begin_patch`]).

use std::path::PathBuf;
use std::sync::Arc;

use api::identity::{RequestId, Settlement};
use host_play::local_state::{QueuedUpdate, Update};

use super::commands::{Applied, Submitted};
use crate::command::{
    CommandFailure, Completion, DrawSwitch, FailureKind, OperatorCommand, Rejection, SkipReason,
    Subject,
};
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

/// Maximum saved-setting transitions kept for a watching front end. If a
/// producer reaches this bound, the drained batch reports that it is
/// incomplete.
pub const SETTING_EVENTS_CAP: usize = 256;

/// The save state one [`SettingEvent`] reports for its setting. Owners
/// report accepted writes, durable writes, failures, and explicit failure
/// removal. A failure that still belongs to its owner stays failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingEventState {
    /// An accepted write has not reached its terminal result.
    Pending,
    /// A write failed; the event's error text says why.
    Failed,
    /// The owner removed a retained failure, including by expiry.
    Cleared,
    /// A write became durable.
    Saved,
}

/// One saved-setting transition, recorded only while a front end watches
/// ([`OperatorSession::watch_setting_events`]) and drained with
/// [`OperatorSession::take_setting_events`]. `setting` is a stable identity,
/// separate from the descriptive `what` used in command failure details.
#[derive(Debug, PartialEq, Eq)]
pub struct SettingEvent {
    setting: String,
    state: SettingEventState,
    error: Option<String>,
}

impl SettingEvent {
    /// One transition of `setting` into `state`; `error` is the failure
    /// text for [`SettingEventState::Failed`], `None` otherwise.
    pub fn new(setting: String, state: SettingEventState, error: Option<String>) -> Self {
        Self {
            setting,
            state,
            error,
        }
    }

    /// The setting the write saves.
    pub fn setting(&self) -> &str {
        &self.setting
    }

    /// The state the write entered.
    pub fn state(&self) -> SettingEventState {
        self.state
    }

    /// The failure text; `Some` only for [`SettingEventState::Failed`].
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
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
        if preferences.ready() && !self.preferences_adopted {
            self.preferences_adopted = true;
            self.adopt_profile_order(&preferences);
        }
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

    /// The walk view a queued Nav or danger write adopted when it settled
    /// since the last call, for the headless driver's lent model (the
    /// panel and the TUI re-read the durable view each frame).
    pub(crate) fn take_settled_walk_globals(&mut self) -> Option<WalkGlobalsView> {
        self.commands.take_settled_walk_globals()
    }

    /// Whether an accepted preferences save still awaits its terminal result.
    pub fn preferences_pending(&self) -> bool {
        self.commands.preferences_pending() || !self.order_writes.is_empty()
    }

    /// Ask the session to record saved-setting transitions for this front
    /// end (drained with [`Self::take_setting_events`]). Off by default:
    /// while off, a transition costs one bool check and records nothing.
    /// Turning it off drops any undrained transitions.
    pub fn watch_setting_events(&mut self, watch: bool) {
        self.commands.set_watch_setting_events(watch);
    }

    /// Drain saved-setting transitions and report whether the bounded
    /// queue dropped any transitions before this call. Empty while watching
    /// is off.
    pub fn take_setting_events(&mut self) -> (Vec<SettingEvent>, bool) {
        self.commands.take_setting_events()
    }

    /// Record that `setting`'s write entered `state` (no error text).
    pub(super) fn note_setting(&mut self, setting: &str, state: SettingEventState) {
        self.commands.note_setting_event(setting, state, None);
    }

    /// Record that `setting`'s write failed with `error`. The text is
    /// formatted only while a front end watches.
    pub(super) fn note_setting_failed(&mut self, setting: &str, error: &dyn std::fmt::Display) {
        if self.commands.watching_setting_events() {
            let text = error.to_string();
            self.commands
                .note_setting_event(setting, SettingEventState::Failed, Some(text));
        }
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
            PrefRow::Draw(switch, on) => self.set_draw(request, switch, on, surface),
            PrefRow::Nav(preference, on) => {
                self.set_nav_preference(request, preference, on, surface)
            }
            PrefRow::Danger(level) => self.set_danger_level(request, level, surface),
            PrefRow::SessionLog(on) => self.set_session_log(request, on),
            PrefRow::MapBake(choice) => self.set_map_bake_choice(request, choice, surface),
            PrefRow::QuesterPaths => self.set_quester_paths(request, surface),
            PrefRow::ReloadQuesterPaths => self.reload_quester_paths(request, surface),
            PrefRow::AcknowledgeNotice => self.acknowledge_notice(request, surface),
            PrefRow::SaveLog => self.save_log(request),
        };
        self.settle_app(request, command, applied, submitted);
    }

    /// The terminal of a preference write that did not fail: `Done` with
    /// `effect` applied when it is durable; when the bounded wait ended
    /// with it still queued, the member stays pending and settles from
    /// the writer's terminal ([`Self::settle_pref_writes`]). Either way
    /// this is now the newest command for its setting, so an older queued
    /// write of that setting no longer applies its value when it settles.
    fn after_write(
        &mut self,
        request: RequestId,
        write: Update,
        setting: &'static str,
        what: &'static str,
        effect: PrefEffect,
    ) -> Applied {
        for older in self.commands.pref_writes().iter_mut() {
            older.effect = older.effect.superseded_by(effect);
        }
        match write {
            Update::Durable => {
                self.apply_pref_effect(effect);
                self.note_setting(setting, SettingEventState::Saved);
                Applied::Done
            }
            Update::Queued(queued) => {
                self.commands.watch_pref_write(PrefWriteWatch {
                    request,
                    queued,
                    setting,
                    what,
                    effect,
                    order: None,
                });
                self.note_setting(setting, SettingEventState::Pending);
                Applied::Later
            }
        }
    }

    /// What a durable preference write changes in this session beyond the
    /// file (the effects that need no lent model). Returns the walk view a
    /// walk effect adopted.
    fn apply_pref_effect(&mut self, effect: PrefEffect) -> Option<WalkGlobalsView> {
        match effect {
            PrefEffect::None => None,
            PrefEffect::Walk(pause) => {
                if let Some(on) = pause {
                    self.set_pause_script_on_manual_walk_abort(on);
                }
                Some(self.refresh_walk_globals())
            }
            PrefEffect::SessionLog(on) => {
                crate::log_file::apply_session_log(on);
                None
            }
            PrefEffect::ProfileOrder => None,
        }
    }

    /// Poll every preference write still queued on the writer thread and
    /// settle the command of each one that ended: `Done` (its effect
    /// applied) when durable, `Failed(WriteFailed)` with the reason
    /// otherwise. Nothing to do, and nothing allocated, while none is
    /// queued.
    pub(super) fn settle_pref_writes(&mut self) {
        let mut index = 0;
        while index < self.commands.pref_writes().len() {
            let Some(outcome) = self.commands.pref_writes()[index].queued.poll() else {
                index += 1;
                continue;
            };
            let watch = self.commands.pref_writes().swap_remove(index);
            let (terminal, detail) = match outcome {
                Ok(()) => {
                    self.note_setting(watch.setting, SettingEventState::Saved);
                    if let Some(view) = self.apply_pref_effect(watch.effect) {
                        self.commands.settle_walk_globals(view);
                    }
                    if let Some(order) = watch.order {
                        if order.seq > self.durable_order_seq {
                            self.durable_order_seq = order.seq;
                            self.durable_order = order.order;
                        }
                    }
                    (Settlement::Completed(Completion::Done), None)
                }
                Err(error) => {
                    self.note_setting_failed(watch.setting, &error);
                    // The newest reorder's write failed: show the last
                    // durable order. An older one's failure changes
                    // nothing (a newer write carries the current order).
                    if matches!(watch.effect, PrefEffect::ProfileOrder) {
                        self.revert_profile_order();
                    }
                    (
                        Settlement::Failed(CommandFailure::Failed(FailureKind::WriteFailed)),
                        Some(format!("{}: {error}", watch.what)),
                    )
                }
            };
            self.settle_later(
                watch.request,
                Subject::Application,
                terminal,
                detail.as_deref(),
            );
        }
    }

    fn set_draw<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
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
            (true, Some(key)) => Some((
                key,
                self.write_preference(key, serde_json::Value::Bool(on), key),
            )),
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
        match write {
            Some((_, Err(applied))) => applied,
            Some((key, Ok(write))) => self.after_write(request, write, key, key, PrefEffect::None),
            None => Applied::Done,
        }
    }

    fn set_nav_preference<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        preference: NavPreference,
        on: bool,
        surface: &mut S,
    ) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        let setting = preference.setting_identity();
        let write = match crate::nav_prefs::write_nav_preference(&preferences, preference, on) {
            Ok(write) => write,
            Err(error) => {
                self.note_setting_failed(setting, &error);
                return write_failed("navigation preference", &error);
            }
        };
        let pause = (preference == NavPreference::PauseScriptOnManualWalkAbort).then_some(on);
        let durable = matches!(write, Update::Durable);
        let applied = self.after_write(
            request,
            write,
            setting,
            "navigation preference",
            PrefEffect::Walk(pause),
        );
        if durable {
            self.adopt_walk_globals(&preferences, surface);
        }
        applied
    }

    fn set_danger_level<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        level: DangerLevel,
        surface: &mut S,
    ) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        let write = match WalkGlobalsView::persist_danger(&preferences, level) {
            Ok(write) => write,
            Err(error) => {
                self.note_setting_failed("danger routing", &error);
                return write_failed("danger routing", &error);
            }
        };
        let durable = matches!(write, Update::Durable);
        let applied = self.after_write(
            request,
            write,
            "danger routing",
            "danger routing",
            PrefEffect::Walk(None),
        );
        if durable {
            self.adopt_walk_globals(&preferences, surface);
        }
        applied
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

    fn set_session_log(&mut self, request: RequestId, on: bool) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        match crate::log_file::persist_session_log_setting(&preferences, on) {
            Ok(write) => self.after_write(
                request,
                write,
                "session file setting",
                "session file setting",
                PrefEffect::SessionLog(on),
            ),
            Err(error) => {
                self.note_setting_failed("session file setting", &error);
                write_failed("session file setting", &error)
            }
        }
    }

    /// The bake gate (a lent model) takes the choice once the write is
    /// accepted, durable or still queued: the gate is this run's consent,
    /// and the command's terminal reports the save.
    fn set_map_bake_choice<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        choice: MapBakeChoice,
        surface: &mut S,
    ) -> Applied {
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        let write = match crate::map_bake::persist_map_bake_choice(&preferences, choice) {
            Ok(write) => write,
            Err(error) => {
                self.note_setting_failed("map bake choice", &error);
                return write_failed("map bake choice", &error);
            }
        };
        if let Some(desk) = surface.models().map.as_deref_mut() {
            desk.bake_gate_mut().set_choice(choice);
        }
        self.after_write(
            request,
            write,
            "map bake choice",
            "map bake choice",
            PrefEffect::None,
        )
    }

    fn set_quester_paths<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
    ) -> Applied {
        let Some((after, persist)) = self.commands.take_quester_paths() else {
            return not_staged("Quest Paths settings");
        };
        let Some(preferences) = self.preferences() else {
            return no_store();
        };
        let selected = self.commands.take_quester_game_data();
        let applied = {
            let mut models = surface.models();
            let Some(paths) = models.quester.as_deref_mut() else {
                return no_model("Quest Paths settings");
            };
            paths.apply_changed(&preferences, &after, selected, persist)
        };
        match applied {
            Ok(write) => self.after_write(
                request,
                write,
                "Quest Paths settings",
                "Quest Paths settings",
                PrefEffect::None,
            ),
            Err(reason) => {
                self.note_setting_failed("Quest Paths settings", &reason);
                Applied::Failed(FailureKind::WriteFailed, Some(reason))
            }
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

    fn acknowledge_notice<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
    ) -> Applied {
        let Some(notice) = self.commands.take_notice() else {
            return not_staged("notice");
        };
        match notice {
            Notice::BackgroundBots => {
                let Some(preferences) = self.preferences() else {
                    return no_store();
                };
                match host_play::begin_persist_background_bots_ack(preferences.inner()) {
                    Ok(write) => self.after_write(
                        request,
                        write,
                        "background bots",
                        "background bots",
                        PrefEffect::None,
                    ),
                    Err(error) => {
                        let text = host_play::background_bots_ack_error(&error);
                        self.note_setting_failed("background bots", &text);
                        Applied::Failed(FailureKind::WriteFailed, Some(text))
                    }
                }
            }
            Notice::RenderAllWarning => {
                match self.write_preference(
                    RENDER_ALL_WARNING_ACK_KEY,
                    true.into(),
                    RENDER_ALL_WARNING_ACK_KEY,
                ) {
                    Ok(write) => self.after_write(
                        request,
                        write,
                        RENDER_ALL_WARNING_ACK_KEY,
                        RENDER_ALL_WARNING_ACK_KEY,
                        PrefEffect::None,
                    ),
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

    /// Write one top-level preference; the failure as the member's
    /// terminal.
    fn write_preference(
        &mut self,
        key: &str,
        value: serde_json::Value,
        setting: &str,
    ) -> Result<Update, Applied> {
        let preferences = self.preferences().ok_or_else(no_store)?;
        preferences.begin_set(key, value).map_err(|error| {
            self.note_setting_failed(setting, &error);
            write_failed(key, &error)
        })
    }

    /// Write the saved Load order.
    /// `Done` once durable; a queued write keeps the member pending
    /// ([`Self::settle_pref_writes`]). A refused or failed write restores
    /// the last durable order, never a failed preview.
    pub(super) fn write_profile_order(&mut self, request: RequestId) -> Applied {
        let write = match self.write_preference(
            crate::profile_order::PROFILE_ORDER_KEY,
            self.profile_order.to_value(),
            "profile order",
        ) {
            Ok(write) => write,
            Err(refused) => {
                self.revert_profile_order();
                return refused;
            }
        };
        for older in self.commands.pref_writes().iter_mut() {
            older.effect = older.effect.superseded_by(PrefEffect::ProfileOrder);
        }
        self.order_write_seq += 1;
        match write {
            Update::Durable => {
                self.durable_order_seq = self.order_write_seq;
                self.durable_order.assign_from(&self.profile_order);
                self.note_setting("profile order", SettingEventState::Saved);
                Applied::Done
            }
            Update::Queued(queued) => {
                self.commands.watch_pref_write(PrefWriteWatch {
                    request,
                    queued,
                    setting: "profile order",
                    what: "profile order",
                    effect: PrefEffect::ProfileOrder,
                    order: Some(OrderWrite {
                        seq: self.order_write_seq,
                        order: self.profile_order.snapshot(),
                    }),
                });
                self.note_setting("profile order", SettingEventState::Pending);
                Applied::Later
            }
        }
    }

    /// Put back the last durable saved Load order (a reorder whose write
    /// failed), reconciled with the vault.
    fn revert_profile_order(&mut self) {
        self.profile_order.assign_from(&self.durable_order);
        self.order_reconciled_at = None;
        self.profiles_generation = self.profiles_generation.wrapping_add(1);
        self.reconcile_profile_order();
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

/// What a durable preference write changes in the session beyond the
/// file, applied when the write is durable (at once, or when a queued
/// write settles).
#[derive(Debug, Clone, Copy)]
pub(super) enum PrefEffect {
    None,
    /// Re-adopt the durable walk globals; with `Some`, also the manual-walk
    /// pause.
    Walk(Option<bool>),
    /// Open or close the session file.
    SessionLog(bool),
    /// The saved Load order: on a failure, put back the order the file
    /// holds.
    ProfileOrder,
}

impl PrefEffect {
    /// This queued effect once `newer`, a later command's effect, was
    /// accepted: the value it would set for the same setting is dropped,
    /// so the newest command for a setting wins. Re-adopting the durable
    /// walk globals stays, because it reads the file and not a captured
    /// value.
    fn superseded_by(self, newer: Self) -> Self {
        match (self, newer) {
            (Self::Walk(Some(_)), Self::Walk(Some(_))) => Self::Walk(None),
            (Self::SessionLog(_), Self::SessionLog(_)) => Self::None,
            (Self::ProfileOrder, Self::ProfileOrder) => Self::None,
            (effect, _) => effect,
        }
    }
}

/// A preference command whose write is still queued on the writer
/// thread.
pub(super) struct PrefWriteWatch {
    pub(super) request: RequestId,
    pub(super) queued: QueuedUpdate,
    /// Stable sidecar identity for the setting.
    pub(super) setting: &'static str,
    /// Descriptive label for the write's failure detail.
    pub(super) what: &'static str,
    pub(super) effect: PrefEffect,
    /// durable order, unless a newer write already is) when it is durable.
    pub(super) order: Option<OrderWrite>,
}

/// The Load order one reorder writes.
pub(super) struct OrderWrite {
    /// The write's number: a higher one began later.
    seq: u64,
    order: crate::profile_order::ProfileOrder,
}

fn write_failed(what: &str, error: &crate::prefs::PreferenceError) -> Applied {
    Applied::Failed(FailureKind::WriteFailed, Some(format!("{what}: {error}")))
}

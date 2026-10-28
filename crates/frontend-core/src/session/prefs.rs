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
use std::time::Instant;

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
use crate::setting::{Setting, SettingStatus};
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

    /// The current value of a saved `switch`; `None` for one that is not
    /// saved.
    fn value(&self, switch: DrawSwitch) -> Option<bool> {
        match switch {
            DrawSwitch::CaptureInput => Some(self.capture),
            DrawSwitch::SidecarFullRate => Some(self.sidecar_50),
            DrawSwitch::FocusedFullRate => Some(self.focused_50),
            DrawSwitch::GameRenderer | DrawSwitch::LiveFullRate => None,
        }
    }

    /// The value `document` holds for a saved `switch` (absent keys take
    /// their defaults: capture on, sidecar 50 fps off, focused 50 fps on).
    fn saved_in(document: &serde_json::Map<String, serde_json::Value>, switch: DrawSwitch) -> bool {
        let default = !matches!(switch, DrawSwitch::SidecarFullRate);
        Self::key(switch)
            .and_then(|key| document.get(key))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(default)
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

    /// Startup: bind the operator preferences to the session (the durable
    /// walk globals' store) and return the walk-permission view the front
    /// end shows. A readable store is adopted at once; a cold one adopts
    /// from [`Self::poll_preferences`] when its first snapshot is ready,
    /// and the view shown until then is provisional.
    pub fn adopt_operator_preferences(&mut self) -> WalkGlobalsView {
        let Some(preferences) = self.preferences() else {
            return WalkGlobalsView::default();
        };
        self.set_walk_globals_store(preferences.share());
        self.sync_snapshot();
        let view = WalkGlobalsView::read(&preferences);
        self.set_walk_globals(view.globals());
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

    /// What a front end shows beside `setting`: saved, pending (an
    /// accepted write has no terminal result yet) or the retained failure.
    /// Core state: closing and reopening a window or popup reconstructs it.
    pub fn setting_status(&self, setting: Setting) -> SettingStatus<'_> {
        self.settings.status(setting)
    }

    /// The retained failure text of `setting`, if any.
    pub fn setting_failure(&self, setting: Setting) -> Option<&str> {
        self.settings.failure(setting)
    }

    /// Clear the retained failure of `setting`: the action that resolves it
    /// happened (the operator closed the notice that showed it, or fixed
    /// its cause). Otherwise it expires by itself after
    /// [`crate::ERROR_MESSAGE_TIMEOUT`].
    pub fn dismiss_setting_failure(&mut self, setting: Setting) {
        let had_failure = self.settings.failure(setting).is_some();
        self.settings.clear_failure(setting);
        if had_failure {
            self.note_setting(setting, SettingEventState::Cleared);
        }
    }

    /// When the earliest retained setting failure expires, for a front end
    /// that sleeps between frames.
    pub fn next_setting_expiry(&self) -> Option<Instant> {
        self.settings.next_expiry()
    }

    /// The one reconciliation owner of saved settings. Call it once per
    /// frame or tick, after [`Self::poll`] (or [`Self::poll_host`]) and
    /// before the front end reads its models, with the surface that lends
    /// them. It settles every accepted preference write by its own request
    /// (retaining a failure at its setting), then lets every model follow
    /// the latest durable snapshot once per durable version: the first
    /// readable snapshot is the startup adoption (nothing is saved over a
    /// cold store), a later one carries another process's change or this
    /// session's own write landing. A saved setting never takes effect from
    /// a submit: a draw switch, the bake gate, the Quester source, the walk
    /// grants, the manual-walk pause and the session file change here, from
    /// the durable value, so a pending or failed write changes nothing.
    pub fn poll_preferences<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) {
        self.poll_preferences_at(surface, Instant::now());
    }

    /// [`Self::poll_preferences`] at `now`.
    pub fn poll_preferences_at<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S, now: Instant) {
        self.settle_pref_writes();
        self.poll_order_writes();
        if self.commands.watching_setting_events() {
            let settings = Setting::ALL;
            let failures = settings.map(|setting| self.settings.failure(setting).is_some());
            self.settings.expire(now);
            for (setting, had_failure) in settings.into_iter().zip(failures) {
                if had_failure && self.settings.failure(setting).is_none() {
                    self.note_setting(setting, SettingEventState::Cleared);
                }
            }
        } else {
            self.settings.expire(now);
        }
        let Some(snapshot) = self.sync_snapshot() else {
            return;
        };
        if self.models_version != Some(snapshot.version()) {
            let first = self.models_version.is_none();
            self.models_version = Some(snapshot.version());
            self.follow_models(&snapshot, first, surface);
        }
        self.poll_session_sink(now);
    }

    /// The session-owned state follows a new durable snapshot: the walk
    /// grants and their play projection, the manual-walk pause, the Load
    /// order and the session file.
    pub(super) fn follow_snapshot(
        &mut self,
        preferences: &crate::UiPreferences,
        snapshot: &crate::prefs::PreferenceSnapshot,
        first: bool,
    ) {
        let view = WalkGlobalsView::read(preferences);
        self.set_walk_globals(view.globals());
        let pause = crate::nav_prefs::nav_preference(
            preferences,
            NavPreference::PauseScriptOnManualWalkAbort,
        )
        .unwrap_or(true);
        self.set_pause_script_on_manual_walk_abort(pause);
        self.follow_profile_order(snapshot, first);
        self.follow_session_log(snapshot);
        self.preferences_adopted = true;
    }

    /// Open or close the session file when the durable preference says so.
    /// A sink that is off at startup is left alone.
    fn follow_session_log(&mut self, snapshot: &crate::prefs::PreferenceSnapshot) {
        let wanted = snapshot
            .document()
            .get(crate::log_file::SESSION_LOG_KEY)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let first = self.session_log_wanted.is_none();
        if self.session_log_wanted == Some(wanted) {
            return;
        }
        self.session_log_wanted = Some(wanted);
        self.session_sink_reported = false;
        if first && !wanted {
            return;
        }
        crate::log_file::apply_session_log(wanted);
    }

    /// A saved choice to write the session file is not yet a file that
    /// writes: the sink opens on its own thread. A failure to open it is a
    /// runtime-effect failure of a durable choice, and says so (once per
    /// choice; the preference itself stays saved).
    fn poll_session_sink(&mut self, now: Instant) {
        if self.session_log_wanted != Some(true) || self.session_sink_reported {
            return;
        }
        if let Some(cause) = crate::log_file::global_file_failure() {
            self.session_sink_reported = true;
            let error =
                format!("Saved, but the session file could not be opened or written: {cause}");
            self.settings
                .fail_now(Setting::SessionLog, error.clone(), now);
            self.note_setting_failed(Setting::SessionLog, &error);
        }
    }

    /// Every lent model follows the durable snapshot: the draw policy (a
    /// changed saved switch tells the surface and wakes the slots), the
    /// bake gate, the Quester source and the walk view. A model that is
    /// absent is skipped; a draw policy that does not persist (a live
    /// harness run) never follows the file.
    fn follow_models<S: SlotSurface<Io = Io>>(
        &mut self,
        snapshot: &crate::prefs::PreferenceSnapshot,
        first: bool,
        surface: &mut S,
    ) {
        let document = snapshot.document();
        for switch in [
            DrawSwitch::CaptureInput,
            DrawSwitch::SidecarFullRate,
            DrawSwitch::FocusedFullRate,
        ] {
            let saved = DrawPolicy::saved_in(document, switch);
            let differs = surface
                .models()
                .draw
                .as_deref()
                .is_some_and(|draw| draw.persist && draw.value(switch) != Some(saved));
            if differs {
                self.apply_draw(switch, saved, surface);
            }
        }
        let choice = crate::map_bake::choice_in(document);
        if let Some(desk) = surface.models().map.as_deref_mut() {
            if desk.bake_gate().choice() != choice {
                desk.bake_gate_mut().set_choice(choice);
            }
        }
        if let Some(paths) = surface.models().quester.as_deref_mut() {
            paths.adopt_durable(QuesterPathsView::read_document(document), first);
        }
        if let Some(preferences) = self.preferences() {
            let view = WalkGlobalsView::read(&preferences);
            if let Some(lent) = surface.models().walk_globals.as_deref_mut() {
                *lent = view;
            }
        }
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

    pub(super) fn note_setting(&mut self, setting: Setting, state: SettingEventState) {
        self.commands
            .note_setting_event(setting.identity(), state, None);
    }

    /// Record a failed setting write with its stable identity.
    pub(super) fn note_setting_failed(&mut self, setting: Setting, error: &dyn std::fmt::Display) {
        if self.commands.watching_setting_events() {
            let text = error.to_string();
            self.commands.note_setting_event(
                setting.identity(),
                SettingEventState::Failed,
                Some(text),
            );
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
            PrefRow::Nav(preference, on) => self.set_nav_preference(request, preference, on),
            PrefRow::Danger(level) => self.set_danger_level(request, level),
            PrefRow::SessionLog(on) => self.set_session_log(request, on),
            PrefRow::MapBake(choice) => self.set_map_bake_choice(request, choice),
            PrefRow::QuesterPaths => self.set_quester_paths(request, surface),
            PrefRow::ReloadQuesterPaths => self.reload_quester_paths(request, surface),
            PrefRow::AcknowledgeNotice => self.acknowledge_notice(request, surface),
            PrefRow::SaveLog => self.save_log(request),
        };
        self.settle_app(request, command, applied, submitted);
    }

    /// The terminal of a saved-setting write that was accepted: `Done` for
    /// a write with nothing to wait for; otherwise the setting is pending
    /// and its command settles from the writer's terminal
    /// ([`Self::settle_pref_writes`]). Nothing is applied here: a saved
    /// setting takes effect from the latest durable snapshot
    /// ([`Self::poll_preferences`]), never from the submit.
    fn after_write(
        &mut self,
        request: RequestId,
        setting: Setting,
        write: Update,
        what: &'static str,
    ) -> Applied {
        match write {
            Update::Durable => {
                let had_failure = self.settings.failure(setting).is_some();
                self.settings.clear_failure(setting);
                if had_failure {
                    self.note_setting(setting, SettingEventState::Cleared);
                }
                self.note_setting(setting, SettingEventState::Saved);
                Applied::Done
            }
            Update::Queued(queued) => {
                let had_failure = self.settings.failure(setting).is_some();
                self.settings.begin(setting);
                if had_failure {
                    self.note_setting(setting, SettingEventState::Cleared);
                }
                self.commands.watch_pref_write(PrefWriteWatch {
                    request,
                    queued,
                    setting,
                    what,
                });
                self.note_setting(setting, SettingEventState::Pending);
                Applied::Later
            }
        }
    }

    /// A write for `setting` was refused before it was accepted: the
    /// command's own failure, and the failure the setting shows.
    fn refused(&mut self, setting: Setting, applied: Applied) -> Applied {
        if let Applied::Failed(_, Some(text)) | Applied::Rejected(_, Some(text)) = &applied {
            self.settings
                .fail_now(setting, format!("Not saved: {text}"), Instant::now());
            self.note_setting_failed(setting, text);
        }
        applied
    }

    /// Poll every saved-setting write still queued on the writer thread and
    /// settle the command of each one that ended: `Done` when durable,
    /// `Failed(WriteFailed)` with the reason otherwise. A failure stays on
    /// its setting ([`Self::setting_status`]) until an action resolves it
    /// or the error timeout passes. Nothing to do, and nothing allocated,
    /// while none is queued.
    pub(super) fn settle_pref_writes(&mut self) {
        let now = Instant::now();
        let mut index = 0;
        while index < self.commands.pref_writes().len() {
            let Some(outcome) = self.commands.pref_writes()[index].queued.poll() else {
                index += 1;
                continue;
            };
            let watch = self.commands.pref_writes().swap_remove(index);
            let (terminal, detail) = match outcome {
                Ok(()) => {
                    let had_failure = self.settings.failure(watch.setting).is_some();
                    self.settings.succeed(watch.setting);
                    if had_failure {
                        self.note_setting(watch.setting, SettingEventState::Cleared);
                    }
                    self.note_setting(watch.setting, SettingEventState::Saved);
                    (Settlement::Completed(Completion::Done), None)
                }
                Err(error) => {
                    self.settings
                        .fail(watch.setting, format!("Not saved: {error}"), now);
                    self.note_setting_failed(watch.setting, &error);
                    // A reorder whose write failed shows the stored order
                    // again, unless a newer reorder is still pending: it
                    // carries the current order.
                    if watch.setting == Setting::ProfileOrder
                        && !matches!(
                            self.settings.status(Setting::ProfileOrder),
                            SettingStatus::Pending
                        )
                    {
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

    /// Apply `switch` to the lent draw policy and tell the surface; the
    /// slots wake so a parked worker re-reads its draw state within a
    /// frame. `false` when this front end holds no draw model.
    fn apply_draw<S: SlotSurface<Io = Io>>(
        &mut self,
        switch: DrawSwitch,
        on: bool,
        surface: &mut S,
    ) -> bool {
        {
            let mut models = surface.models();
            let Some(draw) = models.draw.as_deref_mut() else {
                return false;
            };
            draw.set(switch, on);
        }
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
        true
    }

    /// A saved draw switch is durable-only: the write is accepted and the
    /// policy changes when the latest snapshot holds the value. A run-only
    /// switch (the per-member renderer, this run's full rate) and every
    /// switch of a run that does not persist (a live harness run) apply at
    /// once.
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
            draw.persist
        };
        match (persist, DrawPolicy::key(switch)) {
            (true, Some(key)) => {
                let setting = Setting::Draw(switch);
                match self.write_preference(key, serde_json::Value::Bool(on)) {
                    Ok(write) => self.after_write(request, setting, write, key),
                    Err(applied) => self.refused(setting, applied),
                }
            }
            _ => {
                self.apply_draw(switch, on, surface);
                Applied::Done
            }
        }
    }

    fn set_nav_preference(
        &mut self,
        request: RequestId,
        preference: NavPreference,
        on: bool,
    ) -> Applied {
        let setting = Setting::Nav(preference);
        let Some(preferences) = self.preferences() else {
            return self.refused(setting, no_store());
        };
        match crate::nav_prefs::write_nav_preference(&preferences, preference, on) {
            Ok(write) => self.after_write(request, setting, write, "navigation preference"),
            Err(error) => self.refused(setting, not_accepted("navigation preference", &error)),
        }
    }

    fn set_danger_level(&mut self, request: RequestId, level: DangerLevel) -> Applied {
        let setting = Setting::DangerRouting;
        let Some(preferences) = self.preferences() else {
            return self.refused(setting, no_store());
        };
        match WalkGlobalsView::persist_danger(&preferences, level) {
            Ok(write) => self.after_write(request, setting, write, "danger routing"),
            Err(error) => self.refused(setting, not_accepted("danger routing", &error)),
        }
    }

    fn set_session_log(&mut self, request: RequestId, on: bool) -> Applied {
        let setting = Setting::SessionLog;
        let Some(preferences) = self.preferences() else {
            return self.refused(setting, no_store());
        };
        match crate::log_file::persist_session_log_setting(&preferences, on) {
            Ok(write) => self.after_write(request, setting, write, "session file setting"),
            Err(error) => self.refused(setting, not_accepted("session file setting", &error)),
        }
    }

    /// The bake gate (a lent model) takes the choice from the durable
    /// snapshot; the command's terminal reports the save.
    fn set_map_bake_choice(&mut self, request: RequestId, choice: MapBakeChoice) -> Applied {
        let setting = Setting::MapBake;
        let Some(preferences) = self.preferences() else {
            return self.refused(setting, no_store());
        };
        match crate::map_bake::persist_map_bake_choice(&preferences, choice) {
            Ok(write) => self.after_write(request, setting, write, "map bake choice"),
            Err(error) => self.refused(setting, not_accepted("map bake choice", &error)),
        }
    }

    /// The Quester source is durable-only: the folder members are written,
    /// and the controller applies and reloads the source when the latest
    /// snapshot holds it. A failed write leaves the prior source active. A
    /// run that does not persist applies the source at once.
    fn set_quester_paths<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
    ) -> Applied {
        let setting = Setting::QuesterPaths;
        let Some((after, persist)) = self.commands.take_quester_paths() else {
            return not_staged("Quest Paths settings");
        };
        let _ = self.commands.take_quester_game_data();
        let active = {
            let models = surface.models();
            let Some(paths) = models.quester.as_deref() else {
                return no_model("Quest Paths settings");
            };
            QuesterPathsView::new(paths.settings().enabled(), paths.settings().folder().into())
        };
        if !persist {
            if let Some(paths) = surface.models().quester.as_deref_mut() {
                paths.apply_unsaved(&after);
            }
            return Applied::Done;
        }
        let Some(preferences) = self.preferences() else {
            return self.refused(setting, no_store());
        };
        // The saved source is what becomes active: a folder the document
        // cannot hold (not valid UTF-8) would come back as another path.
        if after.folder().to_str().is_none() {
            return self.refused(
                setting,
                Applied::Rejected(
                    Rejection::NotApplicable,
                    Some(
                        "Quest Paths settings: the folder is not valid UTF-8 and cannot be saved"
                            .into(),
                    ),
                ),
            );
        }
        // The change is made against what the last accepted write asked
        // for while it is pending, so toggling back and forth never skips a
        // write the durable value still needs.
        let before = match self.quester_intent.take() {
            Some(intent) if matches!(self.settings.status(setting), SettingStatus::Pending) => {
                intent
            }
            _ => active,
        };
        match crate::QuesterPathsController::submit_changed(&preferences, &before, &after) {
            Ok(write) => {
                self.quester_intent = Some(after);
                self.after_write(request, setting, write, "Quest Paths settings")
            }
            Err(error) => self.refused(
                setting,
                write_refused(format!("Quest Paths settings: {error}")),
            ),
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

    /// A notice's "don't show again" is durable-only: the front end hides
    /// the notice when the snapshot holds the acknowledgement, so a write
    /// that is pending or failed never dismisses it for good.
    fn acknowledge_notice<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
    ) -> Applied {
        let Some(notice) = self.commands.take_notice() else {
            return not_staged("notice");
        };
        let setting = Setting::Notice(notice);
        match notice {
            Notice::BackgroundBots => {
                let Some(preferences) = self.preferences() else {
                    return self.refused(setting, no_store());
                };
                match host_play::begin_persist_background_bots_ack(preferences.inner()) {
                    Ok(write) => self.after_write(request, setting, write, "background bots"),
                    Err(error) => self.refused(
                        setting,
                        write_refused(host_play::background_bots_ack_error(&error)),
                    ),
                }
            }
            Notice::RenderAllWarning => {
                match self.write_preference(RENDER_ALL_WARNING_ACK_KEY, true.into()) {
                    Ok(write) => {
                        self.after_write(request, setting, write, RENDER_ALL_WARNING_ACK_KEY)
                    }
                    Err(applied) => self.refused(setting, applied),
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
    fn write_preference(&self, key: &str, value: serde_json::Value) -> Result<Update, Applied> {
        let preferences = self.preferences().ok_or_else(no_store)?;
        preferences
            .begin_set(key, value)
            .map_err(|error| not_accepted(key, &error))
    }

    /// Write the saved Load order. `Done` when nothing waits; a queued
    /// write keeps the member pending ([`Self::settle_pref_writes`]). A
    /// refused or failed write shows the stored order again, never a failed
    /// preview.
    pub(super) fn write_profile_order(&mut self, request: RequestId) -> Applied {
        let write = match self.write_preference(
            crate::profile_order::PROFILE_ORDER_KEY,
            self.profile_order.to_value(),
        ) {
            Ok(write) => write,
            Err(refused) => {
                self.revert_profile_order();
                return self.refused(Setting::ProfileOrder, refused);
            }
        };
        self.order_saved = true;
        self.after_write(request, Setting::ProfileOrder, write, "profile order")
    }

    /// Put back the stored Load order (a reorder whose write failed),
    /// reconciled with the vault.
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

/// A saved-setting command whose write is still queued on the writer
/// thread.
pub(super) struct PrefWriteWatch {
    pub(super) request: RequestId,
    pub(super) queued: QueuedUpdate,
    /// The setting that shows this write's pending and failed state.
    pub(super) setting: Setting,
    /// What the write saves, for a failure's detail.
    pub(super) what: &'static str,
}

/// A saved-setting write the preference queue refused before accepting it:
/// the command is refused (one `info` line at the command boundary), not
/// failed.
fn write_refused(text: String) -> Applied {
    Applied::Rejected(Rejection::WriteRefused, Some(text))
}

fn not_accepted(what: &str, error: &crate::prefs::PreferenceError) -> Applied {
    write_refused(format!("{what}: {error}"))
}
#[cfg(test)]
mod setting_event_outcome_tests {
    use super::*;
    use host_play::InstancePermit;

    fn session() -> OperatorSession<u32> {
        OperatorSession::new(InstancePermit::SkipLock)
    }

    #[test]
    fn an_immediate_durable_write_emits_saved() {
        let mut session = session();
        session.watch_setting_events(true);

        assert!(matches!(
            session.after_write(
                RequestId::from_raw(7).unwrap(),
                Setting::ProfileOrder,
                Update::Durable,
                "profile order",
            ),
            Applied::Done
        ));
        let (events, incomplete) = session.take_setting_events();
        assert!(!incomplete);
        assert_eq!(
            events.iter().map(SettingEvent::state).collect::<Vec<_>>(),
            [SettingEventState::Saved]
        );
        assert_eq!(events[0].setting(), "profile order");
    }

    #[test]
    fn an_immediate_durable_retry_emits_cleared_then_saved() {
        let mut session = session();
        let setting = Setting::ProfileOrder;
        session
            .settings
            .fail_now(setting, "old failure".into(), Instant::now());
        session.watch_setting_events(true);

        assert!(matches!(
            session.after_write(
                RequestId::from_raw(8).unwrap(),
                setting,
                Update::Durable,
                "profile order",
            ),
            Applied::Done
        ));
        let (events, incomplete) = session.take_setting_events();
        assert!(!incomplete);
        assert_eq!(
            events.iter().map(SettingEvent::state).collect::<Vec<_>>(),
            [SettingEventState::Cleared, SettingEventState::Saved]
        );
        assert!(events
            .iter()
            .all(|event| event.setting() == "profile order"));
    }
}

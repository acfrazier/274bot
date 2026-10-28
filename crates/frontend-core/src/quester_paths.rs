//! Durable operator configuration and asynchronous reload for Quester Paths.

use api::identity::{Scope, WorkIdentity};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::prefs::{PreferenceError, PreferencePatch, UiPreferences};
use host_play::local_state::Update;

/// Shared settings label used by panel and TUI.
pub const LOAD_PATHS_LABEL: &str = "Load quest Paths from folder";
/// Shared explicit reload label used by panel and TUI.
pub const RELOAD_PATHS_LABEL: &str = "Reload Paths";
/// Heading for the Path-folder settings, which apply to every bot.
pub const QUEST_PATHS_HEADING: &str = "Quest Paths (all bots)";
/// The Quester settings heading that the Path-folder settings sit under.
pub const QUESTER_HEADING: &str = "Quester";

/// Persisted host-wide folder source for the Quester Path registry.
#[derive(Debug, PartialEq, Eq)]
pub struct QuesterPathsView {
    enabled: bool,
    folder: PathBuf,
}

impl Default for QuesterPathsView {
    fn default() -> Self {
        Self {
            enabled: false,
            folder: script::bot_file("quester/paths/289"),
        }
    }
}

impl QuesterPathsView {
    /// Whether Paths load from the configured folder.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Set whether Paths load from the configured folder.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The configured Path folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Replace the configured Path folder.
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.folder = folder;
    }

    /// A Path-folder view for a form draft or a persist.
    pub fn new(enabled: bool, folder: PathBuf) -> Self {
        Self { enabled, folder }
    }

    /// Read the host-wide settings object without touching any Path files.
    fn read(preferences: &UiPreferences) -> Self {
        match preferences.document() {
            Ok(document) => Self::read_document(&document),
            Err(_) => Self::default(),
        }
    }

    /// The host-wide settings a preference document holds (the defaults
    /// for a member that is absent or the wrong type).
    pub(crate) fn read_document(document: &serde_json::Map<String, serde_json::Value>) -> Self {
        let Some(settings) = document
            .get("quester_paths")
            .and_then(serde_json::Value::as_object)
        else {
            return Self::default();
        };
        Self {
            enabled: settings
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            folder: settings
                .get("folder")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| Self::default().folder),
        }
    }

    /// Write only fields changed by this editor through the shared
    /// preferences. A write still queued when the bounded wait ends comes
    /// back [`Update::Queued`].
    fn persist_changed(
        preferences: &UiPreferences,
        before: &Self,
        after: &Self,
    ) -> Result<Update, PreferenceError> {
        let enabled_changed = before.enabled != after.enabled;
        let folder_changed = before.folder != after.folder;
        if !enabled_changed && !folder_changed {
            return Ok(Update::Durable);
        }
        let mut patch = PreferencePatch::default();
        if enabled_changed {
            patch.set_member(
                &["quester_paths", "enabled"],
                Some(serde_json::Value::Bool(after.enabled)),
            )?;
        }
        if folder_changed {
            patch.set_member(
                &["quester_paths", "folder"],
                Some(serde_json::Value::String(
                    after.folder.to_string_lossy().into_owned(),
                )),
            )?;
        }
        preferences.begin_patch(patch)
    }

    /// Apply the durable source to the process-wide registry configuration.
    fn apply(&self) {
        script::quester::registry::set_source(script::quester::registry::FolderSource {
            enabled: self.enabled,
            folder: self.folder.clone(),
        });
    }
}

enum ReloadValue {
    Prepared(script::quester::registry::PreparedRegistry),
}

struct PendingReload {
    selected: Arc<api::game_data::SelectedGameData>,
    source: script::quester::registry::FolderSource,
}

/// Non-blocking owner for one Path-registry reload. Only a consumed
/// WorkAdmission terminal may publish the current snapshot.
#[derive(Default)]
struct ReloadPaths {
    worker: Option<script::preparation::PreparedTask<Result<ReloadValue, String>>>,
    pending: Option<PendingReload>,
    current: Option<WorkIdentity>,
    revision: u64,
}
impl ReloadPaths {
    fn start(
        &mut self,
        selected: Arc<api::game_data::SelectedGameData>,
        source: script::quester::registry::FolderSource,
    ) -> Result<(), String> {
        if self.is_running() {
            return Err("a Path reload is already running".into());
        }
        let pending = PendingReload { selected, source };
        match self.submit(&pending) {
            Ok(()) => Ok(()),
            Err(SubmissionError::Full) => {
                self.pending = Some(pending);
                Ok(())
            }
            Err(SubmissionError::Failed(error)) => Err(error),
        }
    }

    fn retry_pending(&mut self) -> Result<(), String> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        match self.submit(&pending) {
            Ok(()) => Ok(()),
            Err(SubmissionError::Full) => {
                self.pending = Some(pending);
                Ok(())
            }
            Err(SubmissionError::Failed(error)) => Err(error),
        }
    }

    fn submit(&mut self, pending: &PendingReload) -> Result<(), SubmissionError> {
        use script::preparation::CooperativePreparationStep as Step;
        use script::quester::registry::RegistryPreparation;

        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| SubmissionError::Failed("Path reload revision exhausted".into()))?;
        let settings =
            script::preparation::PreparationInput::new(revision, 0, 0, 0).map_err(|error| {
                SubmissionError::Failed(format!("could not admit Path reload settings: {error}"))
            })?;
        let input = script::preparation::PreparationInput::for_quester_folder(
            revision,
            settings,
            &pending.source,
        )
        .map_err(|error| {
            SubmissionError::Failed(format!("could not admit Path reload: {error}"))
        })?;
        let selected = Arc::clone(&pending.selected);
        let mut source = Some(pending.source.clone());
        let mut preparation = None;
        let mut actions = 0_u32;
        let worker = script::preparation::submit_cooperative(
            script::preparation::PreparationKind::Catalogue,
            Scope::Process,
            input,
            move |context| {
                actions = actions.checked_add(1).expect("bounded folder action count");
                if preparation.is_none() {
                    match RegistryPreparation::from_source_without_catalog(
                        source.take().expect("first registry initialization"),
                        Arc::clone(&selected),
                    ) {
                        Ok(candidate) => preparation = Some(candidate),
                        Err(error) => {
                            return Step::Complete {
                                output: Err(compile_error_message(&error)),
                                completed: actions,
                                total: Some(actions),
                            };
                        }
                    }
                }
                let candidate = preparation.as_mut().expect("registry initialized");
                let result = match candidate.step(context) {
                    Ok(Some(candidate)) => Some(Ok(ReloadValue::Prepared(candidate))),
                    Ok(None) => None,
                    Err(error) => Some(Err(compile_error_message(&error))),
                };
                match result {
                    Some(output) => Step::Complete {
                        output,
                        completed: actions,
                        total: Some(actions),
                    },
                    None => Step::Continue {
                        completed: actions,
                        total: None,
                    },
                }
            },
        )
        .map_err(|error| match error {
            script::preparation::PreparationError::Refused(api::work::SubmissionRefusal::Full) => {
                SubmissionError::Full
            }
            error => SubmissionError::Failed(format!("could not start Path reload: {error}")),
        })?;
        self.current = Some(worker.identity());
        self.worker = Some(worker);
        self.revision = revision;
        Ok(())
    }

    /// Take a completed summary or error without blocking the UI/pump.
    fn poll(&mut self) -> Option<Result<Arc<str>, String>> {
        if !self.worker.as_ref()?.is_finished() {
            return None;
        }
        let worker = self.worker.take().expect("finished Path preparation");
        let completed = worker.identity();
        let current = self.current.take().expect("current Path preparation");
        let result = worker
            .join()
            .map_err(|error| format!("Path reload worker: {error}"))
            .and_then(|result| result)
            .and_then(|candidate| match candidate {
                ReloadValue::Prepared(candidate) => {
                    let registry =
                        script::quester::registry::commit_snapshot(candidate, completed, current)
                            .map_err(|error| compile_error_message(&error))?;
                    Ok(registry.report().cloned().unwrap_or_else(|| {
                        Arc::from(format!(
                            "Reload Paths: bundled release registry active ({} rows)",
                            registry.rows().len()
                        ))
                    }))
                }
            });
        Some(result)
    }

    fn is_running(&self) -> bool {
        self.worker.is_some() || self.pending.is_some()
    }
}

enum SubmissionError {
    Full,
    Failed(String),
}

/// Owns the shared settings, reload lifecycle, notice state and user-facing
/// messages for both front ends.
#[derive(Default)]
pub struct QuesterPathsController {
    settings: QuesterPathsView,
    reload: ReloadPaths,
    notice: Option<Result<Arc<str>, Arc<str>>>,
    reload_when_ready: bool,
    /// A changed source asked for a reload before any game data was
    /// supplied: [`Self::advance`] says so once.
    profile_notice_due: bool,
    /// The `ReloadQuesterPaths` command the running reload answers.
    reload_request: Option<api::identity::RequestId>,
    /// That command's outcome, once the reload finished, until the
    /// session settles it.
    settled_reload: Option<(api::identity::RequestId, Result<(), String>)>,
}

/// Why an explicit Reload Paths was not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReloadRefused {
    /// A reload is already running.
    #[error("a Path reload is already running")]
    Running,
    /// No profile's game data is loaded yet.
    #[error("load a profile before reloading quest Paths")]
    NoGameData,
}

impl QuesterPathsController {
    /// Restore and apply saved settings. An enabled source is reloaded the
    /// first time selected game data is supplied to [`Self::advance`].
    pub fn restore(&mut self, preferences: &UiPreferences) {
        self.settings = QuesterPathsView::read(preferences);
        self.settings.apply();
        self.reload_when_ready = self.settings.enabled;
        self.notice = None;
    }

    pub fn settings(&self) -> &QuesterPathsView {
        &self.settings
    }

    /// Submit the source members that differ between `before` (the source
    /// the last accepted write asked for, or the active one) and `after`.
    /// Nothing is applied here: the source takes effect when the write is
    /// durable ([`Self::adopt_durable`]), and a failed write leaves the prior
    /// source active.
    pub(crate) fn submit_changed(
        preferences: &UiPreferences,
        before: &QuesterPathsView,
        after: &QuesterPathsView,
    ) -> Result<Update, PreferenceError> {
        QuesterPathsView::persist_changed(preferences, before, after)
    }

    /// Apply and reload `after` at once without saving it (a live harness
    /// run, whose source is never the operator's).
    pub(crate) fn apply_unsaved(&mut self, after: &QuesterPathsView) {
        if after == &self.settings {
            return;
        }
        self.settings.enabled = after.enabled;
        self.settings.folder.clone_from(&after.folder);
        self.settings.apply();
        self.reload_when_ready = true;
        self.profile_notice_due = true;
    }

    /// Follow the durable source: apply it to the process registry and
    /// reload it when it differs from the active one. The first adoption
    /// is the restore at startup (an enabled source reloads once game data
    /// is supplied to [`Self::advance`]).
    pub(crate) fn adopt_durable(&mut self, durable: QuesterPathsView, first: bool) {
        if !first && durable == self.settings {
            return;
        }
        self.settings = durable;
        self.settings.apply();
        if first {
            self.reload_when_ready = self.settings.enabled;
            self.notice = None;
        } else {
            self.reload_when_ready = true;
            self.profile_notice_due = true;
        }
    }

    /// Handle an explicit Reload Paths action (`ReloadQuesterPaths`
    /// `request`): its outcome is kept for [`Self::take_settled_reload`].
    pub(crate) fn request_reload_for(
        &mut self,
        request: api::identity::RequestId,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) -> Result<(), ReloadRefused> {
        if self.reload.is_running() {
            self.set_notice(Ok(Arc::from("A Path reload is already running.")));
            Err(ReloadRefused::Running)
        } else if let Some(selected) = selected {
            self.reload_when_ready = false;
            self.start_reload(selected);
            if self.reload.is_running() || self.reload.pending.is_some() {
                self.reload_request = Some(request);
            } else {
                let error = self
                    .notice_text()
                    .unwrap_or("Path reload refused")
                    .to_string();
                self.settled_reload = Some((request, Err(error)));
            }
            Ok(())
        } else {
            self.set_notice(Err(Arc::from(
                "Load a profile before reloading quest Paths.",
            )));
            Err(ReloadRefused::NoGameData)
        }
    }

    /// The explicit reload command that finished since the last call, with
    /// its outcome.
    pub(crate) fn take_settled_reload(
        &mut self,
    ) -> Option<(api::identity::RequestId, Result<(), String>)> {
        self.settled_reload.take()
    }

    /// Start a saved-setting reload as soon as game data is ready and poll
    /// the worker without blocking the caller.
    pub fn advance(&mut self, selected: Option<Arc<api::game_data::SelectedGameData>>) {
        if let Some(result) = self.reload.poll() {
            let outcome = match &result {
                Ok(_) => Ok(()),
                Err(error) => Err(error.clone()),
            };
            if let Some(request) = self.reload_request.take() {
                self.settled_reload = Some((request, outcome));
            }
            match result {
                Ok(message) => self.set_notice(Ok(message)),
                Err(error) => self.set_notice(Err(reload_error(error))),
            }
        }
        if self.reload_when_ready && !self.reload.is_running() {
            if let Some(selected) = selected {
                self.reload_when_ready = false;
                self.profile_notice_due = false;
                self.start_reload(selected);
                return;
            }
            if std::mem::take(&mut self.profile_notice_due) {
                self.set_notice(Err(Arc::from(
                    "Load a profile before reloading quest Paths.",
                )));
            }
        }
        if self.reload.worker.is_none() {
            let was_pending = self.reload.pending.is_some();
            match self.reload.retry_pending() {
                Ok(()) if was_pending && self.reload.pending.is_none() => {
                    self.set_notice(Ok(Arc::from("Reloading quest Paths…")));
                }
                Ok(()) => {}
                Err(error) => {
                    if let Some(request) = self.reload_request.take() {
                        self.settled_reload = Some((request, Err(error.clone())));
                    }
                    self.set_notice(Err(reload_error(error)));
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.reload.is_running()
    }

    pub fn notice(&self) -> Option<&Result<Arc<str>, Arc<str>>> {
        self.notice.as_ref()
    }

    pub fn notice_text(&self) -> Option<&str> {
        self.notice.as_ref().map(|notice| match notice {
            Ok(message) | Err(message) => message.as_ref(),
        })
    }

    pub fn notice_is_error(&self) -> bool {
        matches!(self.notice, Some(Err(_)))
    }

    pub(crate) fn clear_notice(&mut self) {
        self.notice = None;
    }

    fn start_reload(&mut self, selected: Arc<api::game_data::SelectedGameData>) {
        let source = script::quester::registry::source_for_preparation();
        match self.reload.start(selected, source) {
            Ok(()) if self.reload.pending.is_some() => self.set_notice(Ok(Arc::from(
                "Reload Paths queued until background capacity is available.",
            ))),
            Ok(()) => self.set_notice(Ok(Arc::from("Reloading quest Paths…"))),
            Err(error) => self.set_notice(Err(reload_error(error))),
        }
    }

    fn set_notice(&mut self, notice: Result<Arc<str>, Arc<str>>) {
        let (level, message) = match &notice {
            Ok(message) => (crate::log::Level::Info, message.as_ref()),
            Err(message) => (crate::log::Level::Error, message.as_ref()),
        };
        crate::log::global().process_line(crate::log::Source::Host, level, message);
        self.notice = Some(notice);
    }
}

fn reload_error(error: String) -> Arc<str> {
    if error.starts_with("Path reload failed:") {
        error.into()
    } else {
        format!("Path reload failed: {error}").into()
    }
}

fn compile_error_message(error: &script::quester::compile::CompileError) -> String {
    let step = error.step.as_ref().map_or("-", |step| step.0.as_ref());
    let detail = error.detail.as_deref().unwrap_or("Path reload failed");
    format!(
        "Path reload failed: path={} step={} code={} detail={detail}",
        error.path.0, step, error.code
    )
}

/// Serializes the crate's tests that set the process-wide quester folder
/// source (directly, or by restoring a controller from preferences).
#[cfg(test)]
pub(crate) fn registry_source_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{QuesterPathsView, ReloadPaths};
    use crate::prefs::UiPreferences as Preferences;
    use api::selected::{ClientRevision, FamilyPreparation};
    use host_play::local_state::{LocalStateService, StateRoot};
    use script::quester::registry::FolderSource;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct TempStore {
        dir: PathBuf,
        preferences: Preferences,
    }

    impl TempStore {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "frontend-core-quester-paths-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self {
                preferences: Preferences::new(
                    LocalStateService::open_process(StateRoot::new(dir.clone())).preferences(),
                ),
                dir,
            }
        }

        fn prefs(&self) -> &Preferences {
            &self.preferences
        }

        fn dir(&self) -> &Path {
            &self.dir
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    struct RestoreRegistrySource;

    impl Drop for RestoreRegistrySource {
        fn drop(&mut self) {
            script::quester::registry::set_source(FolderSource::default());
        }
    }

    struct ReleaseHeldWork(Arc<AtomicBool>);

    impl Drop for ReleaseHeldWork {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    fn write_cook(folder: &Path, id: &str, display_name: &str, invalid: bool) {
        let mut document: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../script/paths/289/cook.json"
        )))
        .unwrap();
        document["id"] = serde_json::json!(id);
        document["display_name"] = serde_json::json!(display_name);
        document["roles"][0]["sequences"][0]["steps"][0]["comment"] =
            serde_json::json!("folder edit");
        if invalid {
            document["roles"][0]["sequences"][0]["steps"][0]["args"]["not_a_real_argument"] =
                serde_json::json!(true);
        }
        std::fs::write(
            folder.join(format!("{id}.json")),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
    }

    fn finish_reload(reload: &mut ReloadPaths) -> Result<Arc<str>, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            reload.retry_pending().expect("retry pending Path reload");
            if let Some(result) = reload.poll() {
                return result;
            }
            assert!(Instant::now() < deadline, "Path reload did not finish");
            std::thread::yield_now();
        }
    }

    #[test]
    fn missing_preferences_disable_folder_paths_and_use_product_data_directory() {
        let store = TempStore::new();
        let view = QuesterPathsView::read(store.prefs());
        assert!(!view.enabled);
        assert_eq!(view.folder, script::bot_file("quester/paths/289"));
    }

    #[test]
    fn changed_preferences_round_trip_without_replacing_unrelated_values() {
        let store = TempStore::new();
        let path = store.prefs().path();
        vault::write_private_file(
            &path,
            br#"{"unrelated":{"keep":true},"nav":{"show_nav_path":true,"custom":"preserve"},"quester_paths":{"enabled":false,"folder":"old","future":"preserve"}}"#,
        )
        .unwrap();
        store.prefs().reload().unwrap();
        let before = QuesterPathsView::read(store.prefs());
        let after = QuesterPathsView {
            enabled: true,
            folder: store.dir().join("checkout").join("quest-paths"),
        };

        crate::prefs::settled_for_test(QuesterPathsView::persist_changed(
            store.prefs(),
            &before,
            &after,
        ))
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["unrelated"]["keep"], true);
        assert_eq!(saved["nav"]["show_nav_path"], true);
        assert_eq!(saved["nav"]["custom"], "preserve");
        assert_eq!(saved["quester_paths"]["enabled"], true);
        assert_eq!(
            saved["quester_paths"]["folder"],
            after.folder.to_string_lossy().as_ref()
        );
        assert_eq!(saved["quester_paths"]["future"], "preserve");
        assert_eq!(QuesterPathsView::read(store.prefs()), after);
    }

    #[test]
    fn async_reload_retries_temporary_full_and_publishes_edited_folder_files() {
        let _source = super::registry_source_test_lock();
        let store = TempStore::new();
        let folder = store.dir().join("paths");
        std::fs::create_dir_all(&folder).unwrap();
        write_cook(&folder, "cook", "Folder Cook", false);
        write_cook(&folder, "fresh-draft", "Fresh Draft", false);
        write_cook(&folder, "broken-draft", "Broken Draft", true);
        std::fs::write(
            folder.join("index.json"),
            r#"{
                "schema": 1,
                "paths": [{
                    "id": "broken-draft",
                    "file": "broken-draft.json",
                    "name": "Broken Draft",
                    "unavailable": "Authoring in progress"
                }]
            }"#,
        )
        .unwrap();

        let selected = FamilyPreparation::run(|_| {
            api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
        })
        .expect("spawn family preparation")
        .join()
        .expect("selected data worker");
        let source = FolderSource {
            enabled: true,
            folder: folder.clone(),
        };
        let zero = script::preparation::PreparationInput::new(0, 0, 0, 0).unwrap();
        let folder_charge =
            script::preparation::PreparationInput::for_quester_folder(0, zero, &source)
                .unwrap()
                .charge()
                .total_bytes()
                .unwrap();
        assert!(folder_charge > 48 * 1024 * 1024);

        let started = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(false));
        let _release_on_panic = ReleaseHeldWork(Arc::clone(&released));
        // Keep retrying when unrelated work causes Full. Stop only when our
        // own held jobs leave too little room for the folder reload.
        let service = script::work_admission::WorkService::shared().expect("WorkAdmission service");
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut held = Vec::new();
        let mut index = 0_u64;
        loop {
            assert!(
                Instant::now() < deadline,
                "fixture could not reserve enough WorkAdmission capacity"
            );
            let observed = service.lock().publication_epoch();
            let worker_started = Arc::clone(&started);
            let worker_released = Arc::clone(&released);
            let request = script::preparation::submit_cooperative(
                script::preparation::PreparationKind::Catalogue,
                api::identity::Scope::Process,
                script::preparation::PreparationInput::new(777_001 + index, 48 * 1024 * 1024, 0, 0)
                    .expect("bounded temporary admission load"),
                move |_| {
                    worker_started.store(true, Ordering::Release);
                    if worker_released.load(Ordering::Acquire) {
                        script::preparation::CooperativePreparationStep::Complete {
                            output: (),
                            completed: 1,
                            total: Some(1),
                        }
                    } else {
                        script::preparation::CooperativePreparationStep::Continue {
                            completed: 0,
                            total: Some(1),
                        }
                    }
                },
            );
            index += 1;
            match request {
                Ok(job) => held.push(job),
                Err(script::preparation::PreparationError::Refused(
                    api::work::SubmissionRefusal::Full,
                )) => {
                    let (jobs, epoch) = {
                        let service = service.lock();
                        let jobs = service.capacity().jobs();
                        (jobs, service.publication_epoch())
                    };
                    if epoch == observed && jobs == held.len() {
                        break;
                    }
                    match script::work_admission::WorkService::wait_for_publication_change(
                        &service,
                        epoch,
                        Duration::from_secs(1),
                    ) {
                        script::work_admission::PublicationWaitOutcome::Changed
                        | script::work_admission::PublicationWaitOutcome::TimedOut => {}
                        script::work_admission::PublicationWaitOutcome::Unavailable => {
                            panic!("WorkAdmission became unavailable during the fixture")
                        }
                    }
                }
                Err(error) => panic!("temporary capacity fixture: {error}"),
            }
        }
        assert!(
            !held.is_empty(),
            "fixture must reserve WorkAdmission capacity"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !started.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "capacity fixture did not start");
            std::thread::yield_now();
        }
        script::quester::registry::set_source(source.clone());
        let _restore_source = RestoreRegistrySource;
        let mut reload = ReloadPaths::default();
        let queued = reload.start(Arc::clone(&selected), source.clone());
        assert!(
            reload.pending.is_some(),
            "Path reload must encounter temporary Full"
        );
        assert!(
            reload.is_running(),
            "a reload waiting for WorkAdmission capacity is running"
        );
        let replacement = FolderSource {
            enabled: true,
            folder: store.dir().join("replacement"),
        };
        assert!(
            reload.start(Arc::clone(&selected), replacement).is_err(),
            "queued Path reload demand must not be replaced"
        );
        assert_eq!(
            reload
                .pending
                .as_ref()
                .map(|pending| &pending.source.folder),
            Some(&folder),
            "the original queued demand remains intact"
        );
        released.store(true, Ordering::Release);
        for job in held {
            job.join().expect("temporary capacity worker completes");
        }
        queued.expect("Full Path reload remains pending");
        assert!(reload.pending.is_some());
        let mut controller = super::QuesterPathsController {
            reload,
            ..Default::default()
        };
        controller.advance(Some(Arc::clone(&selected)));
        assert!(
            controller.reload.pending.is_none(),
            "controller retries queued demand"
        );
        assert!(controller.is_running());
        let summary = finish_reload(&mut controller.reload).expect("folder reload report");
        assert!(summary.contains("Reload Paths:"), "{summary}");
        let published = script::quester::registry::snapshot();
        for (id, label) in [
            ("cook", "Folder Cook [folder]"),
            ("fresh-draft", "Fresh Draft [draft]"),
        ] {
            assert!(
                published
                    .rows()
                    .iter()
                    .any(|row| row.id == id && row.label == label),
                "published registry missing {id} [{label}]: {published:?}"
            );
        }
        let diagnostics = published
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for expected in [
            "broken-draft.json",
            "step=start",
            "code=invalid-args",
            "not_a_real_argument",
        ] {
            assert!(
                diagnostics.contains(expected),
                "missing {expected} in {diagnostics}"
            );
        }

        write_cook(&folder, "cook", "Edited Again", false);
        controller
            .reload
            .start(Arc::clone(&selected), source.clone())
            .expect("restart asynchronous reload");
        finish_reload(&mut controller.reload).expect("second folder reload report");
        let edited = script::quester::registry::snapshot();
        assert!(edited
            .rows()
            .iter()
            .any(|row| row.id == "cook" && row.label == "Edited Again [folder]"));
        assert!(!edited
            .rows()
            .iter()
            .any(|row| row.id == "cook" && row.label == "Folder Cook [folder]"));
        assert!(edited
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.to_string().contains("invalid-args")));
        // The restore guard resets the process-global source even on panic.
    }
}

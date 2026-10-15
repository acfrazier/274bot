//! Durable operator configuration and asynchronous reload for Quester Paths.

use api::identity::{Scope, WorkIdentity};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Shared settings label used by panel and TUI.
pub const LOAD_PATHS_LABEL: &str = "Load quest Paths from folder";
/// Shared explicit reload label used by panel and TUI.
pub const RELOAD_PATHS_LABEL: &str = "Reload Paths";
/// Heading for the Path-folder settings, which apply to every bot.
pub const QUEST_PATHS_HEADING: &str = "Quest Paths (all bots)";
/// The Quester settings heading that the Path-folder settings sit under.
pub const QUESTER_HEADING: &str = "Quester";

/// Persisted host-wide folder source for the Quester Path registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuesterPathsView {
    pub enabled: bool,
    pub folder: PathBuf,
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
    /// Read the host-wide settings object without touching any Path files.
    fn read_at(prefs_path: &Path) -> Self {
        let Some(value) = host_play::panel_ui_value_at(prefs_path, "quester_paths") else {
            return Self::default();
        };
        let Some(settings) = value.as_object() else {
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

    /// Write only fields changed by this editor through the shared panel-ui transaction.
    fn persist_changed_at(prefs_path: &Path, before: &Self, after: &Self) -> io::Result<()> {
        let enabled_changed = before.enabled != after.enabled;
        let folder_changed = before.folder != after.folder;
        if !enabled_changed && !folder_changed {
            return Ok(());
        }

        host_play::update_panel_ui_at(prefs_path, |document, _, _| {
            let root = document.as_object_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "panel-ui.json is not an object")
            })?;
            let settings = root
                .entry("quester_paths")
                .or_insert_with(|| serde_json::json!({}));
            if !settings.is_object() {
                *settings = serde_json::json!({});
            }
            let settings = settings
                .as_object_mut()
                .expect("replacement quester_paths value is an object");
            if enabled_changed {
                settings.insert("enabled".into(), serde_json::Value::Bool(after.enabled));
            }
            if folder_changed {
                settings.insert(
                    "folder".into(),
                    serde_json::Value::String(after.folder.to_string_lossy().into_owned()),
                );
            }
            Ok(None)
        })
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
        self.worker.is_some()
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
}

impl QuesterPathsController {
    /// Restore and apply saved settings. An enabled source is reloaded the
    /// first time selected game data is supplied to [`Self::advance`].
    pub fn restore_at(&mut self, prefs_path: &Path) {
        self.settings = QuesterPathsView::read_at(prefs_path);
        self.settings.apply();
        self.reload_when_ready = self.settings.enabled;
        self.notice = None;
    }

    pub fn settings(&self) -> &QuesterPathsView {
        &self.settings
    }

    /// Persist (when requested), apply and reload a changed source.
    pub fn apply_changed_at(
        &mut self,
        prefs_path: &Path,
        after: QuesterPathsView,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
        persist: bool,
    ) -> Result<(), String> {
        if after == self.settings {
            return Ok(());
        }
        if persist {
            if let Err(error) =
                QuesterPathsView::persist_changed_at(prefs_path, &self.settings, &after)
            {
                let message = format!("Quest Paths settings were not saved: {error}");
                self.set_notice(Err(Arc::from(message.clone())));
                return Err(message);
            }
        }

        self.settings = after;
        self.settings.apply();
        self.reload_when_ready = true;
        if selected.is_none() {
            self.set_notice(Err(Arc::from(
                "Load a profile before reloading quest Paths.",
            )));
        }
        self.advance(selected);
        Ok(())
    }

    /// Handle an explicit Reload Paths action.
    pub fn request_reload(&mut self, selected: Option<Arc<api::game_data::SelectedGameData>>) {
        if self.reload.is_running() {
            self.set_notice(Ok(Arc::from("A Path reload is already running.")));
        } else if let Some(selected) = selected {
            self.reload_when_ready = false;
            self.start_reload(selected);
        } else {
            self.set_notice(Err(Arc::from(
                "Load a profile before reloading quest Paths.",
            )));
        }
    }

    /// Start a saved-setting reload as soon as game data is ready and poll
    /// the worker without blocking the caller.
    pub fn advance(&mut self, selected: Option<Arc<api::game_data::SelectedGameData>>) {
        if let Some(result) = self.reload.poll() {
            match result {
                Ok(message) => self.set_notice(Ok(message)),
                Err(error) => self.set_notice(Err(reload_error(error))),
            }
        }
        if self.reload_when_ready && !self.reload.is_running() {
            if let Some(selected) = selected {
                self.reload_when_ready = false;
                self.start_reload(selected);
                return;
            }
        }
        if !self.reload.is_running() {
            let was_pending = self.reload.pending.is_some();
            match self.reload.retry_pending() {
                Ok(()) if was_pending && self.reload.pending.is_none() => {
                    self.set_notice(Ok(Arc::from("Reloading quest Paths…")));
                }
                Ok(()) => {}
                Err(error) => self.set_notice(Err(reload_error(error))),
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

    pub fn clear_notice(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::{QuesterPathsView, ReloadPaths};
    use api::selected::{ClientRevision, FamilyPreparation};
    use script::quester::registry::FolderSource;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct TempStore {
        dir: PathBuf,
        prefs: PathBuf,
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
                prefs: dir.join("panel-ui.json"),
                dir,
            }
        }

        fn prefs(&self) -> &Path {
            &self.prefs
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
        let view = QuesterPathsView::read_at(store.prefs());
        assert!(!view.enabled);
        assert_eq!(view.folder, script::bot_file("quester/paths/289"));
    }

    #[test]
    fn changed_preferences_round_trip_without_replacing_unrelated_values() {
        let store = TempStore::new();
        std::fs::write(
            store.prefs(),
            r#"{"unrelated":{"keep":true},"nav":{"show_nav_path":true,"custom":"preserve"},"quester_paths":{"enabled":false,"folder":"old","future":"preserve"}}"#,
        )
        .unwrap();
        let before = QuesterPathsView::read_at(store.prefs());
        let after = QuesterPathsView {
            enabled: true,
            folder: store.dir().join("checkout").join("quest-paths"),
        };

        QuesterPathsView::persist_changed_at(store.prefs(), &before, &after).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.prefs()).unwrap()).unwrap();
        assert_eq!(saved["unrelated"]["keep"], true);
        assert_eq!(saved["nav"]["show_nav_path"], true);
        assert_eq!(saved["nav"]["custom"], "preserve");
        assert_eq!(saved["quester_paths"]["enabled"], true);
        assert_eq!(
            saved["quester_paths"]["folder"],
            after.folder.to_string_lossy().as_ref()
        );
        assert_eq!(saved["quester_paths"]["future"], "preserve");
        assert_eq!(QuesterPathsView::read_at(store.prefs()), after);
    }

    #[test]
    fn async_reload_retries_temporary_full_and_publishes_edited_folder_files() {
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
        // Reserve bounded jobs until the byte ledger reports Full. One
        // 48 MiB reservation is smaller than this folder reload's charge,
        // so the next attempt must wait rather than refuse the reload.
        let mut held = Vec::new();
        for index in 0..64_u64 {
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
            match request {
                Ok(job) => held.push(job),
                Err(script::preparation::PreparationError::Refused(
                    api::work::SubmissionRefusal::Full,
                )) => break,
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
        released.store(true, Ordering::Release);
        for job in held {
            job.join().expect("temporary capacity worker completes");
        }
        queued.expect("Full Path reload remains pending");
        assert!(reload.pending.is_some());
        reload
            .retry_pending()
            .expect("retry Path reload after temporary Full");
        assert!(reload.pending.is_none());
        assert!(reload.is_running());
        let summary = finish_reload(&mut reload).expect("folder reload report");
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
        reload
            .start(Arc::clone(&selected), source.clone())
            .expect("restart asynchronous reload");
        finish_reload(&mut reload).expect("second folder reload report");
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

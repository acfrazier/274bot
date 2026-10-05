//! Durable operator configuration and asynchronous reload for Quester Paths.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

/// Shared settings label used by panel and TUI.
pub const LOAD_PATHS_LABEL: &str = "Load quest Paths from folder";
/// Shared explicit reload label used by panel and TUI.
pub const RELOAD_PATHS_LABEL: &str = "Reload Paths";

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

/// Non-blocking owner for one Path-registry reload.
#[derive(Default)]
struct ReloadPaths {
    receiver: Option<Receiver<Result<Arc<str>, String>>>,
}

impl ReloadPaths {
    /// Re-read the configured source and publish its snapshot on a
    /// FamilyPreparation worker, never on the caller.
    fn start(&mut self, selected: Arc<api::game_data::SelectedGameData>) -> Result<(), String> {
        self.start_with(move || {
            let registry = script::quester::registry::reload(&selected)
                .map_err(|error| compile_error_message(&error))?;
            Ok(registry.report().cloned().unwrap_or_else(|| {
                Arc::from(format!(
                    "Reload Paths: bundled release registry active ({} rows)",
                    registry.rows().len()
                ))
            }))
        })
    }

    fn start_with(
        &mut self,
        work: impl FnOnce() -> Result<Arc<str>, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.is_running() {
            return Err("a Path reload is already running".into());
        }
        let (sender, receiver) = mpsc::channel();
        let worker = api::selected::FamilyPreparation::run(move |_| {
            let _ = sender.send(work());
        })
        .map_err(|error| format!("could not start Path reload worker: {error}"))?;
        drop(worker);
        self.receiver = Some(receiver);
        Ok(())
    }

    /// Take a completed summary or error without blocking the UI/pump.
    fn poll(&mut self) -> Option<Result<Arc<str>, String>> {
        let receiver = self.receiver.as_ref()?;
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver = None;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.receiver = None;
                Some(Err("Path reload worker ended without a result".into()))
            }
        }
    }

    fn is_running(&self) -> bool {
        self.receiver.is_some()
    }
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
        match self.reload.start(selected) {
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
    use script::quester::registry::{FolderSource, PathRegistry};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    fn registry_summary(
        source: &FolderSource,
        selected: &api::game_data::SelectedGameData,
    ) -> Arc<str> {
        let quests = api::quest_facts::QuestCatalog::from_identity(selected.quest_identity())
            .expect("quest facts");
        let registry = PathRegistry::load(source, selected, &quests).expect("folder Path load");
        let mut summary = registry
            .report()
            .map_or_else(|| "bundled Paths".to_owned(), ToString::to_string);
        for row in registry
            .rows()
            .iter()
            .filter(|row| matches!(row.id.as_str(), "cook" | "fresh-draft" | "broken-draft"))
        {
            summary.push_str(&format!(
                "\n{} source={} label={} unavailable={}",
                row.id,
                row.source.label(),
                row.label,
                row.unavailable.as_deref().unwrap_or("-")
            ));
        }
        summary.into()
    }

    fn finish_reload(reload: &mut ReloadPaths) -> Result<Arc<str>, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
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
    fn async_reload_uses_edited_folder_files_and_keeps_draft_diagnostics() {
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

        let mut reload = ReloadPaths::default();
        let first_source = source.clone();
        let first_selected = Arc::clone(&selected);
        reload
            .start_with(move || Ok(registry_summary(&first_source, &first_selected)))
            .expect("start asynchronous reload");
        let summary = finish_reload(&mut reload).expect("folder reload report");
        assert!(summary.contains("Folder Cook [folder]"), "{summary}");
        assert!(summary.contains("Fresh Draft [draft]"), "{summary}");
        for expected in [
            "broken-draft.json",
            "Broken Draft [draft]",
            "step=start",
            "code=invalid-args",
            "not_a_real_argument",
        ] {
            assert!(
                summary.contains(expected),
                "missing {expected} in {summary}"
            );
        }

        write_cook(&folder, "cook", "Edited Again", false);
        let second_source = source.clone();
        reload
            .start_with(move || Ok(registry_summary(&second_source, &selected)))
            .expect("restart asynchronous reload");
        let edited = finish_reload(&mut reload).expect("second folder reload report");
        assert!(edited.contains("Edited Again [folder]"), "{edited}");
        assert!(!edited.contains("Folder Cook [folder]"), "{edited}");
        assert!(edited.contains("code=invalid-args"), "{edited}");
    }
}

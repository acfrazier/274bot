//! One immutable registry snapshot, fed by bundled documents and an optional folder.
//! Folder validation runs on a preparation worker; active runs keep their snapshot.
#[cfg(test)]
use super::compile::CompiledPath;
use super::compile::{self, CompileError};
use super::path::PathDocument;
use super::queue::{ReleaseIndex, ReleasePath};
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FamilyPreparation};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

pub const MAX_PATH_BYTES: u64 = 1024 * 1024;
const MAX_ROWS: usize = 256;
const MAX_DIAGNOSTIC_LINES: usize = 32;
const MAX_LISTED_DIAGNOSTICS: usize = MAX_DIAGNOSTIC_LINES - 1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderSource {
    pub enabled: bool,
    pub folder: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum PathSource {
    Bundled,
    Folder,
    Draft,
}
impl PathSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Folder => "folder",
            Self::Draft => "draft",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PathRow {
    pub id: String,
    pub label: String,
    pub unavailable: Option<String>,
    pub source: PathSource,
}

#[derive(Clone, Debug)]
pub struct PathDiagnostic {
    pub file: PathBuf,
    pub error: CompileError,
}
impl std::fmt::Display for PathDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} step={} code={} detail={}",
            self.file.display(),
            self.error.step.as_ref().map_or("-", |step| step.0.as_ref()),
            self.error.code,
            self.error
                .detail
                .as_deref()
                .unwrap_or("Path validation failed")
        )
    }
}

#[derive(Default)]
struct Diagnostics {
    lines: Vec<PathDiagnostic>,
    total: usize,
    omitted: usize,
}

impl Diagnostics {
    fn push(&mut self, diagnostic: impl FnOnce() -> PathDiagnostic) {
        self.total += 1;
        if self.lines.len() < MAX_LISTED_DIAGNOSTICS {
            self.lines.push(diagnostic());
        } else {
            self.omitted += 1;
        }
    }

    fn finish(mut self, folder: &Path) -> (Vec<PathDiagnostic>, usize) {
        if self.omitted > 0 {
            self.lines.push(diagnostic(
                folder,
                "more-diagnostics",
                format!("{} more validation diagnostics omitted", self.omitted),
            ));
        }
        (self.lines, self.total)
    }
}

#[derive(Debug)]
struct FolderDocument {
    bytes: Arc<Vec<u8>>,
    display: String,
}

#[derive(Debug)]
pub struct FolderPaths {
    index: ReleaseIndex,
    documents: HashMap<String, FolderDocument>,
    rows: Vec<PathRow>,
    diagnostics: Vec<PathDiagnostic>,
    report: Option<Arc<str>>,
}

/// Bundled snapshots remain allocation-free and their documents remain static.
#[derive(Clone, Debug, Default)]
pub enum PathRegistry {
    #[default]
    Bundled,
    Folder(Arc<FolderPaths>),
}

impl std::ops::Deref for PathRegistry {
    type Target = ReleaseIndex;
    fn deref(&self) -> &Self::Target {
        self.index()
    }
}

pub static BUNDLED_INDEX: LazyLock<ReleaseIndex> =
    LazyLock::new(|| serde_json::from_str(compile::INDEX_JSON).expect("bundled Path index"));

pub fn bundled_path(id: &str) -> Option<&'static [u8]> {
    bundled_in(&BUNDLED_INDEX, id)
        .then(|| compile::path_bytes(id))
        .flatten()
}

pub(super) fn bundled_in(index: &ReleaseIndex, id: &str) -> bool {
    index.schema == 1
        && index.paths.iter().any(|row| {
            row.id == id
                && row.unavailable.is_none()
                && row
                    .file
                    .as_deref()
                    .and_then(|file| file.strip_suffix(".json"))
                    == Some(id)
                && compile::path_bytes(id).is_some()
        })
}

fn bundled_rows() -> &'static [PathRow] {
    static ROWS: LazyLock<Vec<PathRow>> = LazyLock::new(|| {
        BUNDLED_INDEX
            .paths
            .iter()
            .map(|entry| {
                let display = entry.name.clone().unwrap_or_else(|| {
                    let document: PathDocument = serde_json::from_slice(
                        bundled_path(&entry.id).expect("bundled roster document"),
                    )
                    .expect("bundled Path document");
                    document.display_name
                });
                PathRow {
                    id: entry.id.clone(),
                    label: entry
                        .unavailable
                        .as_ref()
                        .map_or_else(|| display.clone(), |reason| format!("{display} — {reason}")),
                    unavailable: entry.unavailable.clone(),
                    source: PathSource::Bundled,
                }
            })
            .collect()
    });
    &ROWS
}

#[derive(Default)]
struct State {
    source: FolderSource,
    snapshot: PathRegistry,
}
static STATE: Mutex<State> = Mutex::new(State {
    source: FolderSource {
        enabled: false,
        folder: PathBuf::new(),
    },
    snapshot: PathRegistry::Bundled,
});

pub fn set_source(source: FolderSource) {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    if state.source != source {
        state.source = source;
        state.snapshot = PathRegistry::Bundled;
    }
}
pub fn source() -> FolderSource {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .source
        .clone()
}
pub fn snapshot() -> PathRegistry {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .snapshot
        .clone()
}

/// Called for Reload and every Start. Disabled reads never clone the folder path.
pub fn reload(
    selected: &SelectedGameData,
    worker: &mut FamilyPreparation,
) -> Result<PathRegistry, CompileError> {
    reload_with_catalog(selected, None, worker)
}

pub(super) fn reload_with_catalog(
    selected: &SelectedGameData,
    provided: Option<&QuestCatalog>,
    worker: &mut FamilyPreparation,
) -> Result<PathRegistry, CompileError> {
    let source = {
        let state = STATE.lock().unwrap_or_else(|error| error.into_inner());
        if !state.source.enabled {
            return Ok(PathRegistry::Bundled);
        }
        state.source.clone()
    };
    let catalog;
    let quests = if let Some(quests) = provided {
        quests
    } else {
        catalog = QuestCatalog::from_identity(selected.quest_identity())
            .map_err(|error| CompileError::code("quest-facts").with_detail(format!("{error:?}")))?;
        &catalog
    };
    let registry = PathRegistry::load(&source, selected, quests, worker)?;
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    if state.source != source {
        return Err(CompileError::code("stale-folder-source")
            .with_detail("Path folder setting changed during Reload; reload again"));
    }
    state.snapshot = registry.clone();
    drop(state);
    for diagnostic in registry.diagnostics() {
        api::host_log!(
            api::hostlog::Category::Lifecycle,
            api::hostlog::Level::Warn,
            "Quester Path validation: {diagnostic}"
        );
    }
    Ok(registry)
}

/// Owned bytes are shared with a run, never copied per bot.
#[derive(Clone)]
pub enum PathBytes {
    Bundled(&'static [u8]),
    Folder(Arc<Vec<u8>>),
}
impl AsRef<[u8]> for PathBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Bundled(bytes) => bytes,
            Self::Folder(bytes) => bytes.as_slice(),
        }
    }
}

impl PathRegistry {
    #[cfg(test)]
    pub(super) fn from_index(index: ReleaseIndex) -> Self {
        Self::Folder(Arc::new(FolderPaths {
            index,
            documents: HashMap::new(),
            rows: Vec::new(),
            diagnostics: Vec::new(),
            report: None,
        }))
    }
    pub fn index(&self) -> &ReleaseIndex {
        match self {
            Self::Bundled => &BUNDLED_INDEX,
            Self::Folder(folder) => &folder.index,
        }
    }
    pub fn rows(&self) -> &[PathRow] {
        match self {
            Self::Bundled => bundled_rows(),
            Self::Folder(folder) => &folder.rows,
        }
    }
    pub fn path_source(&self, id: &str) -> PathSource {
        match self {
            Self::Bundled => PathSource::Bundled,
            Self::Folder(folder) => classify_source(id, &folder.index, &folder.documents),
        }
    }
    pub fn bytes(&self, id: &str) -> Option<PathBytes> {
        if self
            .index()
            .paths
            .iter()
            .find(|row| row.id == id)?
            .unavailable
            .is_some()
        {
            return None;
        }
        if let Self::Folder(folder) = self {
            if let Some(document) = folder.documents.get(id) {
                return Some(PathBytes::Folder(Arc::clone(&document.bytes)));
            }
        }
        bundled_path(id).map(PathBytes::Bundled)
    }
    #[cfg(test)]
    pub fn compile(
        &self,
        id: &str,
        selected: &SelectedGameData,
        quests: &QuestCatalog,
        worker: &mut FamilyPreparation,
    ) -> Result<Arc<CompiledPath>, CompileError> {
        let bytes = self
            .bytes(id)
            .ok_or_else(|| CompileError::code("unavailable-path"))?;
        compile::compile_path(bytes.as_ref(), selected, quests, worker)
    }
    pub fn report(&self) -> Option<&Arc<str>> {
        match self {
            Self::Bundled => None,
            Self::Folder(folder) => folder.report.as_ref(),
        }
    }
    pub fn diagnostics(&self) -> &[PathDiagnostic] {
        match self {
            Self::Bundled => &[],
            Self::Folder(folder) => &folder.diagnostics,
        }
    }
    #[cfg(test)]
    pub fn ids(&self) -> Vec<String> {
        self.index()
            .paths
            .iter()
            .map(|row| row.id.clone())
            .collect()
    }

    pub fn load(
        source: &FolderSource,
        selected: &SelectedGameData,
        quests: &QuestCatalog,
        worker: &mut FamilyPreparation,
    ) -> Result<Self, CompileError> {
        if !source.enabled {
            return Ok(Self::Bundled);
        }
        if selected
            .selected_pin()
            .map_err(|error| CompileError::code("missing-pin").with_detail(format!("{error:?}")))?
            .revision
            != ClientRevision::R289
        {
            return Err(CompileError::code("unsupported-revision")
                .with_detail("Folder Paths support revision 289 only"));
        }
        let mut diagnostics = Diagnostics::default();
        let mut candidates = BTreeMap::new();
        let mut omitted_candidates = 0;
        let mut omitted_indexed = HashSet::new();
        match std::fs::read_dir(&source.folder) {
            Ok(entries) => {
                for entry in entries {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            diagnostics.push(|| {
                                diagnostic(&source.folder, "read-folder", error.to_string())
                            });
                            continue;
                        }
                    };
                    let path = entry.path();
                    if path.extension().is_none_or(|extension| extension != "json")
                        || path.file_name().is_some_and(|name| name == "index.json")
                    {
                        continue;
                    }
                    let Some(id) = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_owned)
                    else {
                        diagnostics.push(|| {
                            diagnostic(&path, "invalid-filename", "Path filename must be UTF-8")
                        });
                        continue;
                    };
                    if !valid_id(&id) {
                        diagnostics.push(|| {
                            diagnostic(
                                &path,
                                "invalid-filename",
                                "Use a quest id containing letters, digits, '-' or '_'",
                            )
                        });
                        continue;
                    }
                    insert_candidate(&mut candidates, &id, path, &mut omitted_candidates);
                }
            }
            Err(error) => {
                diagnostics.push(|| diagnostic(&source.folder, "read-folder", error.to_string()));
            }
        }
        if omitted_candidates > 0 {
            diagnostics.push(|| {
                diagnostic(
                    &source.folder,
                    "too-many-paths",
                    format!(
                        "{} more filename-sorted Path files omitted after the 256-file cap",
                        omitted_candidates
                    ),
                )
            });
        }
        let folder_index = read_index(&source.folder.join("index.json"), &mut diagnostics);
        let mut ordered = Vec::new();
        if let Some(index) = &folder_index {
            for row in &index.paths {
                if !valid_id(&row.id)
                    || row
                        .file
                        .as_deref()
                        .is_some_and(|file| file != format!("{}.json", row.id))
                {
                    diagnostics.push(|| {
                        diagnostic(
                            &source.folder.join("index.json"),
                            "invalid-index-row",
                            format!("{} must use <id>.json in this folder", row.id),
                        )
                    });
                    continue;
                }
                ordered.push(row.id.clone());
                if let Some(file) = &row.file {
                    let path = source.folder.join(file);
                    if !candidates.contains_key(&row.id) {
                        if omitted_candidates > 0 && path.is_file() {
                            omitted_indexed.insert(row.id.clone());
                        } else {
                            candidates.insert(row.id.clone(), path);
                        }
                    }
                }
            }
        }
        for id in candidates.keys() {
            if !ordered.contains(id) {
                ordered.push(id.clone());
            }
        }
        let mut index = BUNDLED_INDEX.clone();
        let mut documents = HashMap::new();
        for id in ordered {
            let bundled = index.paths.iter().position(|row| row.id == id);
            if bundled.is_none() && index.paths.len() >= MAX_ROWS {
                diagnostics.push(|| {
                    diagnostic(
                        &source.folder.join(format!("{id}.json")),
                        "too-many-paths",
                        "Combined registry contains more than 256 rows",
                    )
                });
                continue;
            }
            let authored = folder_index
                .as_ref()
                .and_then(|index| index.paths.iter().find(|row| row.id == id));
            let mut draft = ReleasePath {
                id: id.clone(),
                file: Some(format!("{id}.json")),
                name: None,
                unavailable: None,
            };
            if omitted_indexed.contains(&id) {
                if bundled.is_none() {
                    draft.name = Some(
                        authored
                            .and_then(|row| row.name.clone())
                            .unwrap_or_else(|| id.clone()),
                    );
                    draft.unavailable = Some("Omitted by the 256-file filename-sorted cap".into());
                    index.paths.push(draft);
                }
                continue;
            }
            if let Some(row) =
                authored.filter(|row| row.file.is_none() && !candidates.contains_key(&id))
            {
                if bundled.is_none() {
                    index.paths.push(row.clone());
                }
                continue;
            }
            let path = candidates
                .get(&id)
                .cloned()
                .unwrap_or_else(|| source.folder.join(format!("{id}.json")));
            let result = read_document(&path, &id, selected, quests, worker);
            match result {
                Ok(document) => {
                    if let Some(reason) = authored.and_then(|row| row.unavailable.as_ref()) {
                        draft.name = Some(document.display.clone());
                        draft.unavailable = Some(reason.clone());
                    }
                    documents.insert(id.clone(), document);
                }
                Err(error) => {
                    let issue = PathDiagnostic { file: path, error };
                    if bundled.is_none() {
                        draft.name = Some(
                            authored
                                .and_then(|row| row.name.clone())
                                .unwrap_or_else(|| id.clone()),
                        );
                        draft.unavailable = Some(issue.to_string());
                    }
                    diagnostics.push(move || issue);
                }
            }
            if bundled.is_none() {
                index.paths.push(draft);
            }
        }
        let (diagnostics, diagnostic_count) = diagnostics.finish(&source.folder);
        let mut rows = bundled_rows().to_vec();
        for entry in &index.paths {
            let display = documents
                .get(&entry.id)
                .map(|document| document.display.as_str())
                .or(entry.name.as_deref())
                .unwrap_or(&entry.id);
            let source = classify_source(&entry.id, &index, &documents);
            if source == PathSource::Bundled && entry.unavailable.is_none() {
                continue;
            }
            let mut label = if source == PathSource::Bundled {
                display.to_owned()
            } else {
                format!("{display} [{}]", source.label())
            };
            if let Some(reason) = &entry.unavailable {
                label.push_str(&format!(" — {reason}"));
            }
            let row = PathRow {
                id: entry.id.clone(),
                label,
                unavailable: entry.unavailable.clone(),
                source,
            };
            if let Some(position) = rows.iter().position(|row| row.id == entry.id) {
                rows[position] = row;
            } else {
                rows.push(row);
            }
        }
        let mut report = format!(
            "Reload Paths: {} folder documents, {} registry rows, {} validation errors ({})",
            documents.len(),
            index.paths.len(),
            diagnostic_count,
            source.folder.display()
        );
        for issue in &diagnostics {
            report.push('\n');
            report.push_str(&issue.to_string());
        }
        Ok(Self::Folder(Arc::new(FolderPaths {
            index,
            documents,
            rows,
            diagnostics,
            report: Some(report.into()),
        })))
    }
}

fn classify_source(
    id: &str,
    index: &ReleaseIndex,
    documents: &HashMap<String, FolderDocument>,
) -> PathSource {
    if BUNDLED_INDEX.paths.iter().any(|row| row.id == id) {
        if documents.contains_key(id) {
            PathSource::Folder
        } else {
            PathSource::Bundled
        }
    } else if index.paths.iter().any(|row| row.id == id) {
        PathSource::Draft
    } else {
        PathSource::Bundled
    }
}

pub fn digest_text(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for &byte in digest {
        text.push(HEX[usize::from(byte >> 4)] as char);
        text.push(HEX[usize::from(byte & 15)] as char);
    }
    text
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn insert_candidate(
    candidates: &mut BTreeMap<String, PathBuf>,
    id: &str,
    path: PathBuf,
    omitted: &mut usize,
) {
    if candidates.contains_key(id) {
        return;
    }
    if candidates.len() < MAX_ROWS {
        candidates.insert(id.to_owned(), path);
        return;
    }

    *omitted += 1;
    if candidates
        .last_key_value()
        .is_some_and(|(last_id, _)| id < last_id.as_str())
    {
        let last_id = candidates
            .last_key_value()
            .expect("the full candidate set has a last filename")
            .0
            .clone();
        candidates.remove(&last_id);
        candidates.insert(id.to_owned(), path);
    }
}
fn diagnostic(path: &Path, code: &'static str, detail: impl Into<Arc<str>>) -> PathDiagnostic {
    PathDiagnostic {
        file: path.to_owned(),
        error: CompileError::code(code).with_detail(detail),
    }
}
fn open_nonblocking(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options.open(path)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CompileError> {
    read_bounded_with_open(path, open_nonblocking)
}
fn read_bounded_with_open(
    path: &Path,
    open_file: impl FnOnce(&Path) -> io::Result<std::fs::File>,
) -> Result<Vec<u8>, CompileError> {
    let file = open_file(path)
        .map_err(|error| CompileError::code("read-file").with_detail(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| CompileError::code("read-file").with_detail(error.to_string()))?;
    if !metadata.is_file() {
        return Err(CompileError::code("not-a-file").with_detail("Path must be a regular file"));
    }
    if metadata.len() > MAX_PATH_BYTES {
        return Err(CompileError::code("file-too-large").with_detail("Path file exceeds 1 MiB"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PATH_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CompileError::code("read-file").with_detail(error.to_string()))?;
    if bytes.len() as u64 > MAX_PATH_BYTES {
        return Err(CompileError::code("file-too-large").with_detail("Path file exceeds 1 MiB"));
    }
    Ok(bytes)
}
fn read_index(path: &Path, diagnostics: &mut Diagnostics) -> Option<ReleaseIndex> {
    if !path.exists() {
        return None;
    }
    let result = read_bounded(path).and_then(|bytes| {
        let index: ReleaseIndex = serde_json::from_slice(&bytes)
            .map_err(|error| CompileError::code("invalid-index").with_detail(error.to_string()))?;
        super::queue::validate_index(&index)
            .map_err(|error| CompileError::code("invalid-index").with_detail(error.to_string()))?;
        Ok(index)
    });
    match result {
        Ok(index) => Some(index),
        Err(error) => {
            diagnostics.push(move || PathDiagnostic {
                file: path.to_owned(),
                error,
            });
            None
        }
    }
}
fn read_document(
    file: &Path,
    id: &str,
    selected: &SelectedGameData,
    quests: &QuestCatalog,
    worker: &mut FamilyPreparation,
) -> Result<FolderDocument, CompileError> {
    let bytes = read_bounded(file)?;
    let document: PathDocument = serde_json::from_slice(&bytes)
        .map_err(|error| CompileError::code("invalid-json").with_detail(error.to_string()))?;
    if document.id.0.as_ref() != id {
        return Err(CompileError::code("path-id-mismatch")
            .with_path(document.id)
            .with_detail(format!("Filename id {id:?} differs from document id")));
    }
    // Revalidate every file, including one whose digest is already in the active cache.
    let gathering = compile::prepare_gathering(&document, selected, worker)?;
    compile::compile_uncached(
        &document,
        compile::digest_bytes(&bytes),
        selected,
        quests,
        gathering,
        None,
    )?;
    Ok(FolderDocument {
        bytes: Arc::new(bytes),
        display: document.display_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{ClientRevision, FamilyPreparation};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "quester-registry-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn source(&self, enabled: bool) -> FolderSource {
            FolderSource {
                enabled,
                folder: self.0.clone(),
            }
        }
        fn cook(&self, id: &str, comment: &str) {
            let mut value: serde_json::Value =
                serde_json::from_slice(compile::cook_bytes()).unwrap();
            value["id"] = serde_json::json!(id);
            value["roles"][0]["sequences"][0]["steps"][0]["comment"] = serde_json::json!(comment);
            std::fs::write(
                self.0.join(format!("{id}.json")),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn with_data(
        test: impl FnOnce(Arc<SelectedGameData>, QuestCatalog, &mut FamilyPreparation) + Send + 'static,
    ) {
        FamilyPreparation::run(move |worker| {
            let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
            test(selected, quests, worker);
        })
        .unwrap()
        .join()
        .unwrap();
    }

    #[test]
    fn folder_gather_progress_reader_prepares_catalog_during_validation() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            let id = "gather-reader-draft";
            let mut document: serde_json::Value =
                serde_json::from_slice(compile::cook_bytes()).unwrap();
            document["id"] = serde_json::json!(id);
            let mut reader = document["roles"][0]["sequences"][0]["steps"][0].clone();
            reader["id"] = serde_json::json!("folder-gather-reader");
            reader["kind"] = serde_json::json!("gather");
            reader["advances"] = serde_json::json!(false);
            reader["args"] = serde_json::json!({
                "skill": "mining",
                "resource": "copper",
                "until": {"obj": "copper_ore", "qty": 1}
            });
            reader["settle"] = serde_json::json!({
                "Fact": {
                    "kind": "item_count_at_least",
                    "version": 1,
                    "args": {"obj": "copper_ore", "qty": 1}
                }
            });
            document["roles"][0]["progress_reader"] = reader;
            std::fs::write(
                folder.0.join(format!("{id}.json")),
                serde_json::to_vec(&document).unwrap(),
            )
            .unwrap();

            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            assert!(
                registry.bytes(id).is_some(),
                "folder gather reader was rejected: {:?}",
                registry.report()
            );
            let path = registry.compile(id, &selected, &quests, worker).unwrap();
            assert_eq!(
                path.progress_reader.as_ref().unwrap().id.0.as_ref(),
                "folder-gather-reader"
            );
        });
    }

    #[test]
    fn folder_override_replaces_bundled_document_on_start_load() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("cook", "edited folder comment");
            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            let path = registry
                .compile("cook", &selected, &quests, worker)
                .unwrap();
            assert_eq!(
                path.sequences[0].steps[0].comment.as_deref(),
                Some("edited folder comment")
            );
        });
    }

    #[test]
    fn folder_draft_is_a_queue_candidate_and_compiles() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("cook-draft", "draft comment");
            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            assert!(registry.ids().iter().any(|id| id == "cook-draft"));
            assert_eq!(
                registry
                    .compile("cook-draft", &selected, &quests, worker)
                    .unwrap()
                    .id
                    .0
                    .as_ref(),
                "cook-draft"
            );
            let queue = super::super::queue::Queue::from_registry(
                registry,
                super::super::queue::QueueSettings {
                    quests: vec!["cook-draft".into()],
                    ..Default::default()
                },
            )
            .unwrap();
            let position = queue.next_candidate().unwrap();
            assert_eq!(queue.id(position), Some("cook-draft"));
            assert_eq!(queue.path_source(position), PathSource::Draft);
        });
    }

    #[test]
    fn invalid_override_reports_compile_error_and_keeps_other_rows() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("cook", "invalid override");
            let path = folder.0.join("cook.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value["roles"][0]["sequences"][0]["steps"][0]["args"]["not_an_argument"] =
                serde_json::json!(true);
            std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
            folder.cook("cook-draft", "valid draft");
            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            let report = registry.report().expect("per-file validation report");
            for expected in ["cook.json", "start", "invalid-args", "not_an_argument"] {
                assert!(report.contains(expected), "missing {expected}: {report}");
            }
            let fallback = registry
                .compile("cook", &selected, &quests, worker)
                .unwrap();
            assert_eq!(
                fallback.digest,
                compile::digest_bytes(compile::cook_bytes())
            );
            assert!(registry
                .compile("cook-draft", &selected, &quests, worker)
                .is_ok());
        });
    }

    #[test]
    fn reload_observes_edited_document_without_restart() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("cook", "first edit");
            let first = PathRegistry::load(&folder.source(true), &selected, &quests, worker)
                .unwrap()
                .compile("cook", &selected, &quests, worker)
                .unwrap();
            folder.cook("cook", "second edit");
            let second = PathRegistry::load(&folder.source(true), &selected, &quests, worker)
                .unwrap()
                .compile("cook", &selected, &quests, worker)
                .unwrap();
            assert_ne!(first.digest, second.digest);
            assert_eq!(
                second.sequences[0].steps[0].comment.as_deref(),
                Some("second edit")
            );
        });
    }

    #[test]
    fn folder_off_ignores_files_and_keeps_bundled_bytes() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("cook", "ignored comment");
            folder.cook("cook-draft", "ignored draft");
            let registry =
                PathRegistry::load(&folder.source(false), &selected, &quests, worker).unwrap();
            assert!(matches!(registry, PathRegistry::Bundled));
            assert_eq!(
                std::mem::size_of::<PathRegistry>(),
                std::mem::size_of::<&ReleaseIndex>()
            );
            assert!(registry.report().is_none());
            assert!(!registry.ids().iter().any(|id| id == "cook-draft"));
            assert_eq!(
                registry
                    .compile("cook", &selected, &quests, worker)
                    .unwrap()
                    .digest,
                compile::digest_bytes(compile::cook_bytes())
            );
            let Some(PathBytes::Bundled(bytes)) = registry.bytes("cook") else {
                panic!("disabled folder must keep static bundled storage");
            };
            assert!(std::ptr::eq(bytes.as_ptr(), compile::cook_bytes().as_ptr()));
        });
    }

    #[test]
    fn optional_index_orders_drafts_and_preserves_server_unavailable_rows() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("draft-a", "A");
            folder.cook("cook", "folder override");
            folder.cook("draft-z", "Z");
            std::fs::write(folder.0.join("index.json"),
            r#"{"schema":1,"paths":[{"id":"draft-z","file":"draft-z.json"},{"id":"draft-a","file":"draft-a.json"},{"id":"hauntedmine","name":"Haunted Mine","unavailable":"no content"}]}"#
        ).unwrap();
            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            let ids = registry.ids();
            assert!(
                ids.iter().position(|id| id == "draft-z")
                    < ids.iter().position(|id| id == "draft-a")
            );
            assert_eq!(
                &ids[..BUNDLED_INDEX.paths.len()],
                &PathRegistry::Bundled.ids()
            );
            assert!(registry.diagnostics().is_empty());
            for id in ["cook", "draft-a", "draft-z", "hauntedmine"] {
                let row = registry.rows().iter().find(|row| row.id == id).unwrap();
                assert_eq!(row.source, registry.path_source(id));
            }
            assert!(registry.bytes("hauntedmine").is_none());
        });
    }

    #[test]
    fn invalid_draft_stays_unavailable_and_oversized_file_is_bounded() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            std::fs::write(folder.0.join("broken.json"), b"not JSON").unwrap();
            std::fs::File::create(folder.0.join("huge.json"))
                .unwrap()
                .set_len(MAX_PATH_BYTES + 1)
                .unwrap();
            folder.cook("valid", "still valid");
            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            for (id, code) in [("broken", "invalid-json"), ("huge", "file-too-large")] {
                let row = registry.rows().iter().find(|row| row.id == id).unwrap();
                assert_eq!(row.source, PathSource::Draft);
                assert_eq!(registry.path_source(id), row.source);
                assert!(row.unavailable.as_deref().unwrap().contains(code));
                assert!(registry.bytes(id).is_none());
            }
            assert!(registry
                .compile("valid", &selected, &quests, worker)
                .is_ok());
        });
    }

    #[test]
    fn candidates_are_sorted_before_the_256_file_cap() {
        let mut candidates = BTreeMap::new();
        let mut omitted = 0;
        for number in (0..MAX_ROWS + 4).rev() {
            let id = format!("draft-{number:03}");
            insert_candidate(
                &mut candidates,
                &id,
                PathBuf::from(format!("{id}.json")),
                &mut omitted,
            );
        }

        assert_eq!(omitted, 4);
        for (number, (id, _)) in candidates.iter().enumerate() {
            assert_eq!(id, &format!("draft-{number:03}"));
        }
    }

    #[test]
    fn folder_diagnostics_are_capped_with_an_omission_summary() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            for number in 0..300 {
                std::fs::write(
                    folder.0.join(format!("draft-{number:03}.json")),
                    b"not JSON",
                )
                .unwrap();
            }

            let registry =
                PathRegistry::load(&folder.source(true), &selected, &quests, worker).unwrap();
            let report = registry.report().unwrap();
            assert_eq!(registry.diagnostics().len(), MAX_DIAGNOSTIC_LINES);
            assert!(
                registry
                    .diagnostics()
                    .last()
                    .unwrap()
                    .to_string()
                    .contains(" more "),
                "the final emitted diagnostic is the omission summary"
            );
            assert!(report.contains(" more "), "{report}");
            let queue = super::super::queue::Queue::from_registry(
                registry,
                super::super::queue::QueueSettings::default(),
            )
            .unwrap();
            assert!(
                queue.path_report().unwrap().contains(" more "),
                "the status field's report keeps the omission summary"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn read_bounded_rechecks_metadata_after_a_swapped_fifo_is_opened() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let folder = Folder::new();
        let path = folder.0.join("swapped.json");
        std::fs::write(&path, b"[]").unwrap();

        let result = read_bounded_with_open(&path, |path| {
            std::fs::remove_file(path)?;
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: `c_path` is a valid, NUL-terminated pathname.
            if unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            open_nonblocking(path)
        });

        let error = result.expect_err("the opened FIFO must be rejected");
        assert_eq!(error.code.as_ref(), "not-a-file");
    }

    #[cfg(unix)]
    #[test]
    fn read_bounded_opens_fifo_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;

        let folder = Folder::new();
        let path = folder.0.join("fifo.json");
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c_path` is a valid, NUL-terminated pathname.
        if unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } != 0 {
            panic!("could not create FIFO: {}", std::io::Error::last_os_error());
        }

        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            let _ = sender.send(read_bounded(&worker_path));
        });
        match receiver.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(result) => {
                worker.join().unwrap();
                assert_eq!(
                    result
                        .expect_err("the opened FIFO must be rejected")
                        .code
                        .as_ref(),
                    "not-a-file"
                );
            }
            Err(_) => {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).custom_flags(libc::O_NONBLOCK);
                let _writer = options
                    .open(&path)
                    .expect("release a regression that blocks while opening the FIFO");
                let result = receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .expect("the blocked FIFO opener did not resume");
                assert_eq!(
                    result
                        .expect_err("the opened FIFO must be rejected")
                        .code
                        .as_ref(),
                    "not-a-file"
                );
                worker.join().unwrap();
                panic!("reading the FIFO blocked; the Path loader must open with O_NONBLOCK");
            }
        }
    }
}

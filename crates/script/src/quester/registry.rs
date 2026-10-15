//! One immutable registry snapshot, fed by bundled documents and an optional folder.
//! Folder validation runs on a preparation worker; active runs keep their snapshot.
#[cfg(test)]
use super::compile::CompiledPath;
use super::compile::{self, CompileError};
use super::path::PathDocument;
use super::queue::{ReleaseIndex, ReleasePath, ReleasePathStatus};
use api::gather_methods::GatherCatalog;
use api::game_data::SelectedGameData;
use api::identity::WorkIdentity;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FamilyPreparation};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, ReadDir};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

pub const MAX_PATH_BYTES: u64 = 1024 * 1024;

/// Maximum raw bytes from successfully validated folder documents retained
/// in one registry snapshot.
///
/// This is the document-byte contribution to the cooperative job's result
/// reservation; other registry rows, diagnostics, and report bytes are
/// additional bounded result state.
pub const MAX_RETAINED_FOLDER_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
/// Maximum source bytes read for any single index or Path document.
pub const MAX_REGISTRY_FILE_BYTES: usize = MAX_PATH_BYTES as usize;
/// Maximum rows in the combined release registry and scanned file candidates.
pub const MAX_REGISTRY_ROWS: usize = 256;
/// Maximum candidates retained after adding every validated index entry.
pub const MAX_REGISTRY_CANDIDATES: usize = 2 * MAX_REGISTRY_ROWS;
/// Maximum observed entries in the optional folder before scanning fails closed.
pub const MAX_FOLDER_ENTRIES: usize = 4096;
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
    /// Release-roster status, independent of an optional folder override.
    pub status: Option<ReleasePathStatus>,
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

fn bundled_row(entry: &ReleasePath) -> PathRow {
    let display = entry.name.clone().unwrap_or_else(|| {
        let document: PathDocument =
            serde_json::from_slice(bundled_path(&entry.id).expect("bundled roster document"))
                .expect("bundled Path document");
        document.display_name
    });
    let mut label = entry
        .unavailable
        .as_ref()
        .map_or_else(|| display.clone(), |reason| format!("{display} — {reason}"));
    if entry.status == Some(ReleasePathStatus::Draft) {
        label.push_str(" [draft]");
    }
    PathRow {
        id: entry.id.clone(),
        label,
        unavailable: entry.unavailable.clone(),
        source: PathSource::Bundled,
        status: entry.status,
    }
}

fn bundled_rows() -> &'static [PathRow] {
    static ROWS: LazyLock<Vec<PathRow>> =
        LazyLock::new(|| BUNDLED_INDEX.paths.iter().map(bundled_row).collect());
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

/// A prepared Path snapshot holds no registry lock and publishes nothing.
/// Its exact source is checked again when the current WorkAdmission ticket
/// consumes the result; a stale/cancelled worker cannot replace the registry.
pub struct PreparedRegistry {
    source: FolderSource,
    registry: PathRegistry,
}

impl PreparedRegistry {
    pub fn registry(&self) -> &PathRegistry {
        &self.registry
    }
}

/// Publish only the exact current, successfully consumed worker ticket and
/// only while the source is still the source that was prepared.
pub fn commit_snapshot(
    prepared: PreparedRegistry,
    completed: WorkIdentity,
    current: WorkIdentity,
) -> Result<PathRegistry, CompileError> {
    if completed != current {
        return Err(CompileError::code("stale-preparation")
            .with_detail("Path preparation no longer belongs to the current reload"));
    }
    commit_prepared(prepared)
}

fn commit_prepared(prepared: PreparedRegistry) -> Result<PathRegistry, CompileError> {
    let PreparedRegistry { source, registry } = prepared;
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    if state.source != source && (source.enabled || state.source.enabled) {
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
    let mut preparation = if let Some(quests) = provided {
        RegistryPreparation::from_refs(source, selected, quests)?
    } else {
        RegistryPreparation::from_refs_without_catalog(source, selected)?
    };
    let prepared = loop {
        if let Some(prepared) = preparation.step(worker)? {
            break prepared;
        }
    };
    commit_prepared(prepared)
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
        let mut preparation =
            RegistryPreparation::from_refs(source.clone(), selected, quests)?;
        loop {
            if let Some(prepared) = preparation.step(worker)? {
                return Ok(prepared.registry);
            }
        }
    }
}

/// The bounded unit that the next cooperative registry call will perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryPreparationPhase {
    BuildQuestCatalog,
    CheckPin,
    ScanFolder,
    ReadIndex,
    ParseIndex,
    MergeIndex,
    CollectCandidates,
    OrderCandidates,
    BuildBaseIndex,
    PrepareDocument,
    ReadDocument,
    ParseDocument,
    PrepareGatheringFamily,
    CompileDocument,
    BuildBaseRows,
    ReduceRows,
    BuildReport,
    Complete,
}

/// Observable bounded progress and raw document retention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistryPreparationProgress {
    pub phase: RegistryPreparationPhase,
    pub completed_documents: usize,
    pub total_documents: Option<usize>,
    /// Successful folder-document bytes retained in the eventual snapshot.
    pub retained_document_bytes: usize,
    /// Bytes read for the current index or Path file, when one is in progress.
    pub pending_file_bytes: usize,
}

enum SelectedOwner<'a> {
    Borrowed(&'a SelectedGameData),
    Shared(Arc<SelectedGameData>),
}
impl SelectedOwner<'_> {
    fn get(&self) -> &SelectedGameData {
        match self {
            Self::Borrowed(value) => value,
            Self::Shared(value) => value,
        }
    }
}

enum QuestOwner<'a> {
    Borrowed(&'a QuestCatalog),
    Shared(Arc<QuestCatalog>),
    Deferred,
}
impl QuestOwner<'_> {
    fn get(&self) -> &QuestCatalog {
        match self {
            Self::Borrowed(value) => value,
            Self::Shared(value) => value,
            Self::Deferred => unreachable!("quest catalog is built before document compilation"),
        }
    }
}

struct PendingDocument {
    id: String,
    path: PathBuf,
    bundled: bool,
    draft: ReleasePath,
    authored_unavailable: Option<String>,
    reader: Option<BoundedRead>,
    bytes: Option<Arc<Vec<u8>>>,
    document: Option<PathDocument>,
    gathering: Option<Arc<GatherCatalog>>,
}

/// A resumable folder-registry reducer. The source and selected owners are
/// captured once; each call performs one scan, read, parse, compile, or
/// reduction unit and retains bounded state for the next call.
pub struct RegistryPreparation<'a> {
    source: FolderSource,
    selected: SelectedOwner<'a>,
    quests: QuestOwner<'a>,
    phase: RegistryPreparationPhase,
    registry: Option<PathRegistry>,
    scan_started: bool,
    scanned_entries: usize,
    entries: Option<ReadDir>,
    candidates: BTreeMap<String, PathBuf>,
    omitted_candidates: usize,
    omitted_indexed: HashSet<String>,
    diagnostics: Diagnostics,
    final_diagnostics: Vec<PathDiagnostic>,
    diagnostic_count: usize,
    index_opened: bool,
    index_reader: Option<BoundedRead>,
    index_bytes: Option<Vec<u8>>,
    folder_index: Option<ReleaseIndex>,
    index_row_position: usize,
    candidate_ids: Vec<String>,
    candidate_copy_position: usize,
    candidate_position: usize,
    ordered: Vec<String>,
    total_documents: Option<usize>,
    document_position: usize,
    completed_documents: usize,
    index: Option<ReleaseIndex>,
    base_index_position: usize,
    documents: HashMap<String, FolderDocument>,
    base_row_position: usize,
    retained_document_bytes: usize,
    pending: Option<PendingDocument>,
    rows: Vec<PathRow>,
    row_position: usize,
    report: String,
    report_position: usize,
}

impl RegistryPreparation<'static> {
    /// Capture an explicit folder source and shared immutable input owners.
    pub fn from_source(
        source: FolderSource,
        selected: Arc<SelectedGameData>,
        quests: Arc<QuestCatalog>,
    ) -> Result<Self, CompileError> {
        Ok(Self::new(
            source,
            SelectedOwner::Shared(selected),
            QuestOwner::Shared(quests),
        ))
    }

    /// Capture an explicit source and defer quest-catalog construction to a
    /// cooperative step.
    pub fn from_source_without_catalog(
        source: FolderSource,
        selected: Arc<SelectedGameData>,
    ) -> Result<Self, CompileError> {
        Ok(Self::new(
            source,
            SelectedOwner::Shared(selected),
            QuestOwner::Deferred,
        ))
    }

    /// Capture the process-current folder source once, before the first step.
    pub fn from_current_source(
        selected: Arc<SelectedGameData>,
        quests: Arc<QuestCatalog>,
    ) -> Result<Self, CompileError> {
        Self::from_source(source(), selected, quests)
    }

    /// Capture the process-current folder source and selected owner. No
    /// filesystem I/O or quest-catalog decoding occurs before `step`.
    pub fn from_current(selected: Arc<SelectedGameData>) -> Result<Self, CompileError> {
        Self::from_source_without_catalog(source(), selected)
    }
}

impl<'a> RegistryPreparation<'a> {
    fn from_refs(
        source: FolderSource,
        selected: &'a SelectedGameData,
        quests: &'a QuestCatalog,
    ) -> Result<Self, CompileError> {
        Ok(Self::new(
            source,
            SelectedOwner::Borrowed(selected),
            QuestOwner::Borrowed(quests),
        ))
    }

    fn from_refs_without_catalog(
        source: FolderSource,
        selected: &'a SelectedGameData,
    ) -> Result<Self, CompileError> {
        Ok(Self::new(
            source,
            SelectedOwner::Borrowed(selected),
            QuestOwner::Deferred,
        ))
    }

    fn new(
        source: FolderSource,
        selected: SelectedOwner<'a>,
        quests: QuestOwner<'a>,
    ) -> Self {
        let source = if source.enabled {
            source
        } else {
            FolderSource::default()
        };
        let enabled = source.enabled;
        let phase = if !enabled {
            RegistryPreparationPhase::Complete
        } else if matches!(&quests, QuestOwner::Deferred) {
            RegistryPreparationPhase::BuildQuestCatalog
        } else {
            RegistryPreparationPhase::CheckPin
        };
        Self {
            source,
            selected,
            quests,
            phase,
            registry: (!enabled).then_some(PathRegistry::Bundled),
            scan_started: false,
            scanned_entries: 0,
            entries: None,
            candidates: BTreeMap::new(),
            omitted_candidates: 0,
            omitted_indexed: HashSet::new(),
            diagnostics: Diagnostics::default(),
            final_diagnostics: Vec::new(),
            diagnostic_count: 0,
            index_opened: false,
            index_reader: None,
            index_bytes: None,
            folder_index: None,
            index_row_position: 0,
            candidate_ids: Vec::new(),
            candidate_copy_position: 0,
            candidate_position: 0,
            ordered: Vec::new(),
            total_documents: None,
            document_position: 0,
            completed_documents: 0,
            index: None,
            documents: HashMap::new(),
            retained_document_bytes: 0,
            pending: None,
            base_index_position: 0,
            base_row_position: 0,
            rows: Vec::new(),
            row_position: 0,
            report: String::new(),
            report_position: 0,
        }
    }

    /// Report the next bounded unit before calling [`Self::step`].
    pub fn phase(&self) -> RegistryPreparationPhase {
        self.phase
    }

    /// Progress suitable for WorkAdmission progress publication.
    pub fn progress(&self) -> RegistryPreparationProgress {
        RegistryPreparationProgress {
            phase: self.phase,
            completed_documents: self.completed_documents,
            total_documents: self.total_documents,
            retained_document_bytes: self.retained_document_bytes,
            pending_file_bytes: self
                .index_reader
                .as_ref()
                .map_or(0, BoundedRead::len)
                .max(self.index_bytes.as_ref().map_or(0, Vec::len))
                .max(
                    self.pending
                        .as_ref()
                        .and_then(|pending| pending.reader.as_ref())
                        .map_or_else(
                            || {
                                self.pending
                                    .as_ref()
                                    .and_then(|pending| pending.bytes.as_ref())
                                    .map_or(0, |bytes| bytes.len())
                            },
                            BoundedRead::len,
                        ),
                ),
        }
    }

    /// Perform one bounded preparation unit. `Some` is the unpublished
    /// snapshot; the caller must still pass it through `commit_snapshot`.
    pub fn step(
        &mut self,
        worker: &mut FamilyPreparation,
    ) -> Result<Option<PreparedRegistry>, CompileError> {
        use RegistryPreparationPhase as Phase;
        match self.phase {
            Phase::BuildQuestCatalog => {
                let quests = QuestCatalog::from_identity(self.selected.get().quest_identity())
                    .map_err(|error| {
                        CompileError::code("quest-facts").with_detail(format!("{error:?}"))
                    })?;
                self.quests = QuestOwner::Shared(Arc::new(quests));
                self.phase = Phase::CheckPin;
            }
            Phase::CheckPin => {
                let revision = self
                    .selected
                    .get()
                    .selected_pin()
                    .map_err(|error| {
                        CompileError::code("missing-pin").with_detail(format!("{error:?}"))
                    })?
                    .revision;
                if revision != ClientRevision::R289 {
                    return Err(CompileError::code("unsupported-revision")
                        .with_detail("Folder Paths support revision 289 only"));
                }
                self.phase = Phase::ScanFolder;
            }
            Phase::ScanFolder => {
                if !self.scan_started {
                    self.scan_started = true;
                    match std::fs::read_dir(&self.source.folder) {
                        Ok(entries) => self.entries = Some(entries),
                        Err(error) => {
                            let folder = self.source.folder.clone();
                            self.diagnostics.push(|| {
                                diagnostic(&folder, "read-folder", error.to_string())
                            });
                            self.finish_scan();
                        }
                    }
                    return Ok(None);
                }
                let next = self.entries.as_mut().and_then(|entries| entries.next());
                if next.is_some() {
                    if self.scanned_entries == MAX_FOLDER_ENTRIES {
                        return Err(CompileError::code("too-many-folder-entries").with_detail(
                            format!("Folder contains more than {MAX_FOLDER_ENTRIES} entries"),
                        ));
                    }
                    self.scanned_entries += 1;
                }
                match next {
                    Some(Ok(entry)) => {
                        let path = entry.path();
                        if path.extension().is_none_or(|extension| extension != "json")
                            || path.file_name().is_some_and(|name| name == "index.json")
                        {
                            return Ok(None);
                        }
                        let Some(id) = path
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .map(str::to_owned)
                        else {
                            self.diagnostics.push(|| {
                                diagnostic(
                                    &path,
                                    "invalid-filename",
                                    "Path filename must be UTF-8",
                                )
                            });
                            return Ok(None);
                        };
                        if !valid_id(&id) {
                            self.diagnostics.push(|| {
                                diagnostic(
                                    &path,
                                    "invalid-filename",
                                    "Use a quest id containing letters, digits, '-' or '_'",
                                )
                            });
                            return Ok(None);
                        }
                        insert_candidate(
                            &mut self.candidates,
                            &id,
                            path,
                            &mut self.omitted_candidates,
                        );
                    }
                    Some(Err(error)) => {
                        let folder = self.source.folder.clone();
                        self.diagnostics.push(|| {
                            diagnostic(&folder, "read-folder", error.to_string())
                        });
                    }
                    None => self.finish_scan(),
                }
            }
            Phase::ReadIndex => {
                if !self.index_opened {
                    self.index_opened = true;
                    let path = self.source.folder.join("index.json");
                    if !path.exists() {
                        self.phase = Phase::MergeIndex;
                        return Ok(None);
                    }
                    match open_bounded(&path) {
                        Ok(reader) => self.index_reader = Some(reader),
                        Err(error) => {
                            self.push_index_diagnostic(error);
                            self.phase = Phase::MergeIndex;
                        }
                    }
                    return Ok(None);
                }
                let Some(result) = self.index_reader.as_mut().map(BoundedRead::read_chunk) else {
                    self.phase = Phase::MergeIndex;
                    return Ok(None);
                };
                match result {
                    Ok(Some(bytes)) => {
                        self.index_reader = None;
                        self.index_bytes = Some(bytes);
                        self.phase = Phase::ParseIndex;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.index_reader = None;
                        self.push_index_diagnostic(error);
                        self.phase = Phase::MergeIndex;
                    }
                }
            }
            Phase::ParseIndex => {
                let path = self.source.folder.join("index.json");
                let bytes = self.index_bytes.take().expect("index bytes were read");
                let parsed = serde_json::from_slice::<ReleaseIndex>(&bytes)
                    .map_err(|error| {
                        CompileError::code("invalid-index").with_detail(error.to_string())
                    })
                    .and_then(|index| {
                        super::queue::validate_index(&index)
                            .map_err(|error| {
                                CompileError::code("invalid-index")
                                    .with_detail(error.to_string())
                            })?;
                        Ok(index)
                    });
                match parsed {
                    Ok(index) => self.folder_index = Some(index),
                    Err(error) => self.push_index_diagnostic(error),
                }
                self.phase = Phase::MergeIndex;
            }
            Phase::MergeIndex => {
                let Some((id, file)) = self
                    .folder_index
                    .as_ref()
                    .and_then(|index| index.paths.get(self.index_row_position))
                    .map(|row| (row.id.clone(), row.file.clone()))
                else {
                    self.candidate_ids = Vec::with_capacity(self.candidates.len());
                    self.phase = Phase::CollectCandidates;
                    return Ok(None);
                };
                self.index_row_position += 1;
                if !valid_id(&id)
                    || file
                        .as_deref()
                        .is_some_and(|file| file != format!("{id}.json"))
                {
                    let path = self.source.folder.join("index.json");
                    self.diagnostics.push(|| {
                        diagnostic(
                            &path,
                            "invalid-index-row",
                            format!("{id} must use <id>.json in this folder"),
                        )
                    });
                    return Ok(None);
                }
                self.ordered.push(id.clone());
                if let Some(file) = file {
                    let path = self.source.folder.join(file);
                    if !self.candidates.contains_key(&id) {
                        if self.omitted_candidates > 0 && path.is_file() {
                            self.omitted_indexed.insert(id);
                        } else {
                            if self.candidates.len() >= MAX_REGISTRY_CANDIDATES {
                                return Err(CompileError::code("registry-candidate-limit")
                                    .with_detail("Folder registry candidate bound exceeded"));
                            }
                            self.candidates.insert(id, path);
                        }
                    }
                }
            }
            Phase::CollectCandidates => {
                let id = self
                    .candidates
                    .iter()
                    .nth(self.candidate_copy_position)
                    .map(|(id, _)| id.clone());
                if let Some(id) = id {
                    self.candidate_ids.push(id);
                    self.candidate_copy_position += 1;
                } else {
                    self.phase = Phase::OrderCandidates;
                }
            }
            Phase::OrderCandidates => {
                let Some(id) = self.candidate_ids.get(self.candidate_position) else {
                    self.total_documents = Some(self.ordered.len());
                    self.index = Some(ReleaseIndex {
                        schema: BUNDLED_INDEX.schema,
                        paths: Vec::with_capacity(MAX_REGISTRY_ROWS),
                    });
                    self.base_index_position = 0;
                    self.phase = Phase::BuildBaseIndex;
                    return Ok(None);
                };
                if !self.ordered.contains(id) {
                    self.ordered.push(id.clone());
                }
                self.candidate_position += 1;
            }
            Phase::BuildBaseIndex => {
                let Some(row) = BUNDLED_INDEX.paths.get(self.base_index_position) else {
                    self.phase = Phase::PrepareDocument;
                    return Ok(None);
                };
                self.index
                    .as_mut()
                    .expect("combined index is initialized")
                    .paths
                    .push(row.clone());
                self.base_index_position += 1;
            }
            Phase::PrepareDocument => {
                let Some(id) = self.ordered.get(self.document_position).cloned() else {
                    let (diagnostics, count) =
                        std::mem::take(&mut self.diagnostics).finish(&self.source.folder);
                    self.final_diagnostics = diagnostics;
                    self.diagnostic_count = count;
                    self.rows = Vec::with_capacity(BUNDLED_INDEX.paths.len());
                    self.base_row_position = 0;
                    self.phase = Phase::BuildBaseRows;
                    return Ok(None);
                };
                let index = self.index.as_ref().expect("combined index is prepared");
                let bundled = index.paths.iter().any(|row| row.id == id);
                if !bundled && index.paths.len() >= MAX_REGISTRY_ROWS {
                    let path = self.source.folder.join(format!("{id}.json"));
                    self.diagnostics.push(|| {
                        diagnostic(
                            &path,
                            "too-many-paths",
                            "Combined registry contains more than 256 rows",
                        )
                    });
                    self.document_position += 1;
                    self.completed_documents += 1;
                    return Ok(None);
                }
                let authored = self
                    .folder_index
                    .as_ref()
                    .and_then(|index| index.paths.iter().find(|row| row.id == id))
                    .cloned();
                let mut draft = ReleasePath {
                    id: id.clone(),
                    file: Some(format!("{id}.json")),
                    name: None,
                    unavailable: None,
                    status: authored.as_ref().and_then(|row| row.status),
                };
                if self.omitted_indexed.contains(&id) {
                    if !bundled {
                        draft.name = Some(
                            authored
                                .as_ref()
                                .and_then(|row| row.name.clone())
                                .unwrap_or_else(|| id.clone()),
                        );
                        draft.unavailable =
                            Some("Omitted by the 256-file filename-sorted cap".into());
                        self.index
                            .as_mut()
                            .expect("combined index is prepared")
                            .paths
                            .push(draft);
                    }
                    self.document_position += 1;
                    self.completed_documents += 1;
                    return Ok(None);
                }
                if let Some(row) = authored
                    .as_ref()
                    .filter(|row| row.file.is_none() && !self.candidates.contains_key(&id))
                    .cloned()
                {
                    if !bundled {
                        self.index
                            .as_mut()
                            .expect("combined index is prepared")
                            .paths
                            .push(row);
                    }
                    self.document_position += 1;
                    self.completed_documents += 1;
                    return Ok(None);
                }
                let path = self
                    .candidates
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| self.source.folder.join(format!("{id}.json")));
                self.pending = Some(PendingDocument {
                    id,
                    path,
                    bundled,
                    draft,
                    authored_unavailable: authored.and_then(|row| row.unavailable),
                    reader: None,
                    bytes: None,
                    document: None,
                    gathering: None,
                });
                self.phase = Phase::ReadDocument;
            }
            Phase::BuildBaseRows => {
                if let Some(entry) = BUNDLED_INDEX.paths.get(self.base_row_position) {
                    self.rows.push(bundled_row(entry));
                    self.base_row_position += 1;
                } else {
                    self.phase = Phase::ReduceRows;
                }
            }
            Phase::ReadDocument => {
                let needs_open = self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.reader.is_none() && pending.bytes.is_none());
                if needs_open {
                    let path = self
                        .pending
                        .as_ref()
                        .expect("document is selected")
                        .path
                        .clone();
                    match open_bounded(&path) {
                        Ok(reader) => {
                            self.pending
                                .as_mut()
                                .expect("document is selected")
                                .reader = Some(reader);
                        }
                        Err(error) => self.finish_document(Err(error))?,
                    }
                    return Ok(None);
                }
                let result = self
                    .pending
                    .as_mut()
                    .expect("document is selected")
                    .reader
                    .as_mut()
                    .expect("document reader is open")
                    .read_chunk();
                match result {
                    Ok(Some(bytes)) => {
                        let pending = self.pending.as_mut().expect("document is selected");
                        pending.reader = None;
                        pending.bytes = Some(Arc::new(bytes));
                        self.phase = Phase::ParseDocument;
                    }
                    Ok(None) => {}
                    Err(error) => self.finish_document(Err(error))?,
                }
            }
            Phase::ParseDocument => {
                let pending = self.pending.as_mut().expect("document is selected");
                let bytes = Arc::clone(pending.bytes.as_ref().expect("document bytes were read"));
                let document = serde_json::from_slice::<PathDocument>(bytes.as_slice())
                    .map_err(|error| {
                        CompileError::code("invalid-json").with_detail(error.to_string())
                    })
                    .and_then(|document| {
                        if document.id.0.as_ref() != pending.id {
                            Err(CompileError::code("path-id-mismatch")
                                .with_path(document.id.clone())
                                .with_detail(format!(
                                    "Filename id {:?} differs from document id",
                                    pending.id
                                )))
                        } else {
                            Ok(document)
                        }
                    });
                match document {
                    Ok(document) => {
                        let needs_gathering = document_uses_gathering(&document);
                        self.pending
                            .as_mut()
                            .expect("document is selected")
                            .document = Some(document);
                        self.phase = if needs_gathering {
                            Phase::PrepareGatheringFamily
                        } else {
                            Phase::CompileDocument
                        };
                    }
                    Err(error) => self.finish_document(Err(error))?,
                }
            }
            Phase::PrepareGatheringFamily => {
                let result = {
                    let pending = self.pending.as_ref().expect("document is selected");
                    compile::prepare_gathering(
                        pending.document.as_ref().expect("document was parsed"),
                        self.selected.get(),
                        worker,
                    )
                };
                match result {
                    Ok(gathering) => {
                        self.pending
                            .as_mut()
                            .expect("document is selected")
                            .gathering = gathering;
                        self.phase = Phase::CompileDocument;
                    }
                    Err(error) => self.finish_document(Err(error))?,
                }
            }
            Phase::CompileDocument => {
                let result = {
                    let pending = self.pending.as_mut().expect("document is selected");
                    let bytes = pending.bytes.as_ref().expect("document bytes were read");
                    let document = pending.document.as_ref().expect("document was parsed");
                    let gathering = pending.gathering.clone();
                    let digest = compile::digest_bytes(bytes.as_slice());
                    if let Some(partner) = &document.partner {
                        partner.roles.iter().try_for_each(|role| {
                            compile::compile_uncached(
                                document,
                                digest,
                                self.selected.get(),
                                self.quests.get(),
                                gathering.clone(),
                                Some(role.gang),
                            )
                            .map(|_| ())
                            .map_err(|mut error| {
                                let detail = format!(
                                    "gang {:?}: {}",
                                    role.gang,
                                    error.detail.as_deref().unwrap_or("Path validation failed")
                                );
                                error.role.get_or_insert_with(|| role.id.clone());
                                error.with_detail(detail)
                            })
                        })
                    } else {
                        compile::compile_uncached(
                            document,
                            digest,
                            self.selected.get(),
                            self.quests.get(),
                            gathering,
                            None,
                        )
                        .map(|_| ())
                    }
                };
                match result {
                    Ok(()) => {
                        let pending = self.pending.as_mut().expect("document is selected");
                        let document = pending.document.as_mut().expect("document was parsed");
                        let folder_document = FolderDocument {
                            bytes: Arc::clone(
                                pending.bytes.as_ref().expect("document bytes were read"),
                            ),
                            display: std::mem::take(&mut document.display_name),
                        };
                        self.finish_document(Ok(folder_document))?;
                    }
                    Err(error) => self.finish_document(Err(error))?,
                }
            }
            Phase::ReduceRows => {
                let index = self.index.as_ref().expect("combined index is prepared");
                let Some(entry) = index.paths.get(self.row_position) else {
                    self.report = format!(
                        "Reload Paths: {} folder documents, {} registry rows, {} validation errors ({})",
                        self.documents.len(),
                        index.paths.len(),
                        self.diagnostic_count,
                        self.source.folder.display()
                    );
                    self.phase = Phase::BuildReport;
                    return Ok(None);
                };
                let display = self
                    .documents
                    .get(&entry.id)
                    .map(|document| document.display.as_str())
                    .or(entry.name.as_deref())
                    .unwrap_or(&entry.id);
                let source = classify_source(&entry.id, index, &self.documents);
                self.row_position += 1;
                if source == PathSource::Bundled && entry.unavailable.is_none() {
                    return Ok(None);
                }
                let mut label = if source == PathSource::Bundled {
                    display.to_owned()
                } else {
                    format!("{display} [{}]", source.label())
                };
                if let Some(reason) = &entry.unavailable {
                    label.push_str(&format!(" — {reason}"));
                }
                if entry.status == Some(ReleasePathStatus::Draft)
                    && source != PathSource::Draft
                {
                    label.push_str(" [draft]");
                }
                let row = PathRow {
                    id: entry.id.clone(),
                    label,
                    unavailable: entry.unavailable.clone(),
                    source,
                    status: entry.status,
                };
                if let Some(position) = self.rows.iter().position(|row| row.id == entry.id) {
                    self.rows[position] = row;
                } else {
                    self.rows.push(row);
                }
            }
            Phase::BuildReport => {
                if let Some(issue) = self.final_diagnostics.get(self.report_position) {
                    self.report.push('\n');
                    self.report.push_str(&issue.to_string());
                    self.report_position += 1;
                } else {
                    let index = self.index.take().expect("combined index is prepared");
                    self.registry = Some(PathRegistry::Folder(Arc::new(FolderPaths {
                        index,
                        documents: std::mem::take(&mut self.documents),
                        diagnostics: std::mem::take(&mut self.final_diagnostics),
                        report: Some(std::mem::take(&mut self.report).into()),
                    })));
                    self.phase = Phase::Complete;
                }
            }
            Phase::Complete => {
                let registry = self.registry.take().ok_or_else(|| {
                    CompileError::code("preparation-complete")
                        .with_detail("Path registry preparation was already consumed")
                })?;
                return Ok(Some(PreparedRegistry {
                    source: self.source.clone(),
                    registry,
                }));
            }
        }
        Ok(None)
    }

    fn finish_scan(&mut self) {
        self.entries = None;
        if self.omitted_candidates > 0 {
            let omitted = self.omitted_candidates;
            let folder = self.source.folder.clone();
            self.diagnostics.push(|| {
                diagnostic(
                    &folder,
                    "too-many-paths",
                    format!(
                        "{omitted} more filename-sorted Path files omitted after the 256-file cap"
                    ),
                )
            });
        }
        self.phase = RegistryPreparationPhase::ReadIndex;
    }

    fn push_index_diagnostic(&mut self, error: CompileError) {
        let path = self.source.folder.join("index.json");
        self.diagnostics.push(|| PathDiagnostic { file: path, error });
    }

    fn finish_document(
        &mut self,
        result: Result<FolderDocument, CompileError>,
    ) -> Result<(), CompileError> {
        let mut pending = self.pending.take().expect("document is selected");
        match result {
            Ok(document) => {
                let retained = self
                    .retained_document_bytes
                    .checked_add(document.bytes.len())
                    .filter(|bytes| *bytes <= MAX_RETAINED_FOLDER_DOCUMENT_BYTES)
                    .ok_or_else(|| {
                        CompileError::code("registry-retention-limit").with_detail(format!(
                            "Folder documents exceed the {}-byte retained registry bound",
                            MAX_RETAINED_FOLDER_DOCUMENT_BYTES
                        ))
                    })?;
                self.retained_document_bytes = retained;
                if let Some(reason) = pending.authored_unavailable.as_ref() {
                    pending.draft.name = Some(document.display.clone());
                    pending.draft.unavailable = Some(reason.clone());
                }
                self.documents.insert(pending.id.clone(), document);
            }
            Err(error) => {
                let issue = PathDiagnostic {
                    file: pending.path,
                    error,
                };
                if !pending.bundled {
                    pending.draft.name = Some(
                        self.folder_index
                            .as_ref()
                            .and_then(|index| index.paths.iter().find(|row| row.id == pending.id))
                            .and_then(|row| row.name.clone())
                            .unwrap_or_else(|| pending.id.clone()),
                    );
                    pending.draft.unavailable = Some(issue.to_string());
                }
                self.diagnostics.push(move || issue);
            }
        }
        if !pending.bundled {
            self.index
                .as_mut()
                .expect("combined index is prepared")
                .paths
                .push(pending.draft);
        }
        self.document_position += 1;
        self.completed_documents += 1;
        self.phase = RegistryPreparationPhase::PrepareDocument;
        Ok(())
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
    if candidates.len() < MAX_REGISTRY_ROWS {
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

#[cfg(test)]
fn read_bounded(path: &Path) -> Result<Vec<u8>, CompileError> {
    read_bounded_with_open(path, open_nonblocking)
}
#[cfg(test)]
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
const FILE_READ_CHUNK_BYTES: usize = 64 * 1024;

struct BoundedRead {
    file: File,
    bytes: Vec<u8>,
}

impl BoundedRead {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Read at most one fixed-size chunk. `Some` means EOF and transfers the
    /// bounded bytes; `None` means another cooperative read unit is required.
    fn read_chunk(&mut self) -> Result<Option<Vec<u8>>, CompileError> {
        let mut chunk = [0_u8; FILE_READ_CHUNK_BYTES];
        let remaining = MAX_REGISTRY_FILE_BYTES + 1 - self.bytes.len();
        let count = self
            .file
            .read(&mut chunk[..remaining.min(chunk.len())])
            .map_err(|error| CompileError::code("read-file").with_detail(error.to_string()))?;
        if count == 0 {
            return Ok(Some(std::mem::take(&mut self.bytes)));
        }
        self.bytes.extend_from_slice(&chunk[..count]);
        if self.bytes.len() > MAX_REGISTRY_FILE_BYTES {
            return Err(
                CompileError::code("file-too-large").with_detail("Path file exceeds 1 MiB")
            );
        }
        Ok(None)
    }
}

fn open_bounded(path: &Path) -> Result<BoundedRead, CompileError> {
    let file = open_nonblocking(path)
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
    Ok(BoundedRead {
        file,
        bytes: Vec::with_capacity(metadata.len() as usize),
    })
}

fn document_uses_gathering(document: &PathDocument) -> bool {
    document.roles.iter().any(|role| {
        role.prelude
            .iter()
            .chain(role.sequences.iter().flat_map(|sequence| &sequence.steps))
            .chain(role.progress_reader.iter())
            .any(|step| step.kind == "gather")
    }) || document.quest.as_ref().is_some_and(|header| {
        header
            .acquire
            .values()
            .flatten()
            .any(|step| step.kind == "gather")
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

    fn cooperative_registry(
        preparation: &mut RegistryPreparation<'_>,
        worker: &mut FamilyPreparation,
        phases: &mut std::collections::HashSet<RegistryPreparationPhase>,
    ) -> PreparedRegistry {
        loop {
            phases.insert(preparation.phase());
            assert!(
                preparation.progress().pending_file_bytes <= MAX_REGISTRY_FILE_BYTES,
                "the cooperative reader exceeded its per-file retention ceiling"
            );
            if let Some(prepared) = preparation.step(worker).unwrap() {
                return prepared;
            }
        }
    }

    fn prepare_current_registry(
        selected: Arc<SelectedGameData>,
        worker: &mut FamilyPreparation,
    ) -> PreparedRegistry {
        let mut preparation = RegistryPreparation::from_current(selected).unwrap();
        loop {
            if let Some(prepared) = preparation.step(worker).unwrap() {
                return prepared;
            }
        }
    }

    #[test]
    fn cooperative_reload_matches_filesystem_after_edit_and_keeps_diagnostics() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            folder.cook("cook", "before cooperative preparation");
            std::fs::write(folder.0.join("invalid-draft.json"), b"not JSON").unwrap();
            std::fs::write(
                folder.0.join("index.json"),
                r#"{"schema":1,"paths":[{"id":"missing-draft","file":"missing-draft.json"},{"id":"offline-draft","name":"Offline Path","unavailable":"server has not released it"}]}"#,
            )
            .unwrap();

            let source = folder.source(true);
            let mut preparation = RegistryPreparation::from_source(
                source.clone(),
                Arc::clone(&selected),
                Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap()),
            )
            .unwrap();
            assert_eq!(preparation.phase(), RegistryPreparationPhase::CheckPin);
            assert!(preparation.step(worker).unwrap().is_none());

            // The source path is captured, while files are read by the staged
            // worker. An edit after construction is therefore observed.
            folder.cook("cook", "edited after cooperative preparation began");
            let synchronous =
                PathRegistry::load(&source, &selected, &quests, worker).unwrap();
            let mut phases = std::collections::HashSet::new();
            let prepared = cooperative_registry(&mut preparation, worker, &mut phases);
            let cooperative = prepared.registry();

            assert_eq!(prepared.source, source);
            assert!(phases.contains(&RegistryPreparationPhase::ScanFolder));
            assert!(phases.contains(&RegistryPreparationPhase::ReadIndex));
            assert!(phases.contains(&RegistryPreparationPhase::ParseIndex));
            assert!(phases.contains(&RegistryPreparationPhase::ParseDocument));
            assert!(phases.contains(&RegistryPreparationPhase::CompileDocument));
            assert_eq!(synchronous.ids(), cooperative.ids());
            let rows = |registry: &PathRegistry| {
                registry
                    .rows()
                    .iter()
                    .map(|row| {
                        (
                            row.id.clone(),
                            row.label.clone(),
                            row.unavailable.clone(),
                            row.source,
                            row.status,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(rows(&synchronous), rows(cooperative));
            let diagnostics = |registry: &PathRegistry| {
                registry
                    .diagnostics()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            };
            assert_eq!(diagnostics(&synchronous), diagnostics(cooperative));
            assert_eq!(
                synchronous.report().map(|report| report.as_ref()),
                cooperative.report().map(|report| report.as_ref())
            );
            assert_eq!(
                synchronous.bytes("cook").unwrap().as_ref(),
                cooperative.bytes("cook").unwrap().as_ref()
            );
            let invalid = cooperative
                .rows()
                .iter()
                .find(|row| row.id == "invalid-draft")
                .unwrap();
            assert_eq!(invalid.source, PathSource::Draft);
            assert!(invalid.unavailable.as_deref().unwrap().contains("invalid-json"));
            let missing = cooperative
                .rows()
                .iter()
                .find(|row| row.id == "missing-draft")
                .unwrap();
            assert!(missing.unavailable.as_deref().unwrap().contains("read-file"));
            let offline = cooperative
                .rows()
                .iter()
                .find(|row| row.id == "offline-draft")
                .unwrap();
            assert_eq!(
                offline.unavailable.as_deref(),
                Some("server has not released it")
            );
            assert!(cooperative.bytes("offline-draft").is_none());
        });
    }

    #[test]
    fn cooperative_reload_fails_closed_at_retained_document_limit() {
        with_data(|selected, quests, _worker| {
            let folder = Folder::new();
            let mut preparation = RegistryPreparation::from_refs(
                folder.source(true),
                &selected,
                &quests,
            )
            .unwrap();
            preparation.index = Some(BUNDLED_INDEX.clone());
            preparation.retained_document_bytes = MAX_RETAINED_FOLDER_DOCUMENT_BYTES;
            preparation.pending = Some(PendingDocument {
                id: "bounded-draft".into(),
                path: folder.0.join("bounded-draft.json"),
                bundled: false,
                draft: ReleasePath {
                    id: "bounded-draft".into(),
                    file: Some("bounded-draft.json".into()),
                    name: None,
                    unavailable: None,
                    status: None,
                },
                authored_unavailable: None,
                reader: None,
                bytes: None,
                document: None,
                gathering: None,
            });

            let error = preparation
                .finish_document(Ok(FolderDocument {
                    bytes: Arc::new(vec![0]),
                    display: "Bounded draft".into(),
                }))
                .unwrap_err();
            assert_eq!(error.code.as_ref(), "registry-retention-limit");
        });
    }

    #[test]
    fn cooperative_scan_fails_closed_after_bounded_directory_entries() {
        with_data(|selected, quests, worker| {
            let folder = Folder::new();
            for number in 0..=MAX_FOLDER_ENTRIES {
                std::fs::write(folder.0.join(format!("ignored-{number:04}.txt")), b"").unwrap();
            }
            let mut preparation = RegistryPreparation::from_source(
                folder.source(true),
                Arc::clone(&selected),
                Arc::new(quests),
            )
            .unwrap();

            let error = loop {
                match preparation.step(worker) {
                    Ok(None) => {}
                    Ok(Some(_)) => panic!("over-limit folder unexpectedly prepared"),
                    Err(error) => break error,
                }
            };
            assert_eq!(error.code.as_ref(), "too-many-folder-entries");
        });
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
        for number in (0..MAX_REGISTRY_ROWS + 4).rev() {
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
    #[test]
    fn stale_or_cancelled_work_preparation_never_publishes_a_path_snapshot() {
        use api::identity::{Cancellation, Scope};
        use std::sync::Barrier;

        let folder = Folder::new();
        folder.cook("stale-publish-draft", "unpublished");
        set_source(folder.source(true));
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let input =
            crate::preparation::PreparationInput::new(1, 4096, 4 * 1024 * 1024, 32 * 1024 * 1024)
                .unwrap();
        let data = Arc::clone(&selected);
        let stale = crate::preparation::submit(
            crate::preparation::PreparationKind::Catalogue,
            Scope::Process,
            input,
            move |worker| prepare_current_registry(data, worker),
        )
        .unwrap();
        let completed = stale.identity();
        let prepared = stale.join().unwrap();
        let wrong_current = crate::work_admission::WorkService::shared()
            .unwrap()
            .lock()
            .mint_identity(Scope::Process, None)
            .unwrap();
        assert_ne!(completed, wrong_current);
        assert!(commit_snapshot(prepared, completed, wrong_current).is_err());
        assert!(snapshot()
            .rows()
            .iter()
            .all(|row| row.id != "stale-publish-draft"));

        let checkpoint = Arc::new(Barrier::new(2));
        let proceed = Arc::new(Barrier::new(2));
        let data = Arc::clone(&selected);
        let worker_checkpoint = Arc::clone(&checkpoint);
        let worker_proceed = Arc::clone(&proceed);
        let mut cancelled = crate::preparation::submit(
            crate::preparation::PreparationKind::Catalogue,
            Scope::Process,
            crate::preparation::PreparationInput::new(2, 4096, 4 * 1024 * 1024, 32 * 1024 * 1024)
                .unwrap(),
            move |worker| {
                let prepared = prepare_current_registry(data, worker);
                worker_checkpoint.wait();
                worker_proceed.wait();
                prepared
            },
        )
        .unwrap();
        checkpoint.wait();
        cancelled.cancel(Cancellation::Superseded);
        proceed.wait();
        assert!(matches!(
            cancelled.join(),
            Err(crate::preparation::PreparationError::Cancelled(
                Cancellation::Superseded
            ))
        ));
        assert!(snapshot()
            .rows()
            .iter()
            .all(|row| row.id != "stale-publish-draft"));
        set_source(FolderSource::default());
    }
}

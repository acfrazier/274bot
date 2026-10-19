//! Reload and catalog refresh as one transaction: admit bounded preparation
//! and validation work off the caller, then either apply it or, when running
//! or paused runs of that card would be replaced, publish a [`ReloadWarning`]
//! naming the exact slots and generations. A confirm re-checks the candidate
//! on a worker and the warned generations; Cancel and Stop discard in-flight
//! work and candidates without touching any execution. A replacement Start
//! is queued behind the reap of the run it replaces and settles like any Start.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{CardSource, ScriptChoice};
use super::{PendingCard, PendingStart, Scripts, StartKind};
use crate::session::{OperatorSession, ScriptStart};

/// Why a catalog fill or import request failed.
///
/// Returned at the `frontend-core` crate boundary by
/// [`Scripts::request_catalog_fill`], [`Scripts::request_catalog_import`],
/// [`Scripts::take_catalog_import_result`] and
/// [`Scripts::fill_catalog_blocking`]. The `Display` text of every failure
/// is the message those methods returned as `String` before, so
/// operator-visible wording is unchanged.
///
/// The failure kind stays private behind this opaque newtype, following the
/// existing `ImportError` newtype in `rs2b0t_import`: state crossing the
/// frontend-core seam goes through accessors, not public fields. The
/// bounded library snapshot and the catalog worker's own failures are
/// modeled in `script` ([`script::load::CatalogSnapshotError`],
/// [`script::load::CatalogWorkerError`]) and wrapped here through `From`:
/// they cross from `script` into `frontend-core`, so the crate-boundary
/// rule needs a `thiserror` kind on each side. WorkAdmission admission and
/// join failures stay on this side as their `Display` text, so no `script`
/// preparation type crosses the seam, even in a private field. A
/// submission failure is stored once as the async receipt, and the
/// requesting method returns it from that receipt, so one owner holds the
/// error and no `Clone` is needed.
pub struct CatalogError(CatalogFailure);

impl std::fmt::Debug for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl CatalogError {
    fn unexpected_catalog_commit(message: String) -> Self {
        Self(CatalogFailure::UnexpectedCatalogCommit(message))
    }
}

impl From<script::load::CatalogSnapshotError> for CatalogError {
    fn from(error: script::load::CatalogSnapshotError) -> Self {
        Self(CatalogFailure::Snapshot(error))
    }
}

impl From<api::work::SubmissionRefusal> for CatalogError {
    fn from(error: api::work::SubmissionRefusal) -> Self {
        Self(CatalogFailure::Charge(error))
    }
}

impl From<script::preparation::PreparationError> for CatalogError {
    fn from(error: script::preparation::PreparationError) -> Self {
        Self(CatalogFailure::Preparation(error.to_string()))
    }
}

impl From<script::load::CatalogWorkerError> for CatalogError {
    fn from(error: script::load::CatalogWorkerError) -> Self {
        Self(CatalogFailure::Worker(error))
    }
}

/// Why a catalog fill or import request failed. Kept private behind
/// [`CatalogError`]; the `Display` text of every variant is the message the
/// catalog methods returned as `String` before.
#[derive(Debug, thiserror::Error)]
enum CatalogFailure {
    /// Another root's catalog work is already queued or running.
    #[error("catalog preparation is already pending for another root")]
    BusyForAnotherRoot,
    /// A catalog fill or import is already queued or running.
    #[error("catalog preparation is already pending")]
    Busy,
    /// The previous import settled but its receipt was never collected.
    #[error("the previous catalog import result has not been collected")]
    UncollectedImportResult,
    /// The bounded pre-submission snapshot of the library did not fit.
    #[error(transparent)]
    Snapshot(script::load::CatalogSnapshotError),
    /// The admission charge for the catalog job was refused.
    #[error(transparent)]
    Charge(api::work::SubmissionRefusal),
    /// WorkAdmission refused, failed, or lost the catalog job. The payload
    /// is the preparation failure's `Display` text, so no `script` error
    /// type crosses the crate boundary. `Full` never reaches here: it is
    /// retained as pending work and retried.
    #[error("{0}")]
    Preparation(String),
    /// The catalog worker reported a failure (registry, source graph, diff).
    #[error(transparent)]
    Worker(script::load::CatalogWorkerError),
    /// A prepared catalog card could not be committed. Catalog cards never
    /// persist (`commit_prepared` only persists `ScriptSource::File`), so a
    /// commit failure here is an internal invariant violation, not a
    /// distinct catalog failure kind; the message is preserved for diagnosis.
    #[error("catalog commit invariant violated for a catalog card: {0}")]
    UnexpectedCatalogCommit(String),
}

#[derive(Debug)]
pub struct ReloadWarning {
    pub(crate) identity_key: String,
    pub(crate) source: script::ScriptSource,
    pub(crate) lookup: String,
    pub(crate) fingerprint: String,
    pub(crate) epoch: u64,
    pub(crate) running: Vec<String>,
    pub(crate) paused: Vec<String>,
    pub(crate) paused_during_prep: Vec<String>,
    pub(crate) affected_generations: Vec<(String, u64)>,
}

impl ReloadWarning {
    /// Card identity this warning is bound to.
    pub fn identity_key(&self) -> &str {
        &self.identity_key
    }

    /// Script source of the candidate.
    pub fn source(&self) -> CardSource {
        CardSource::from_script(self.source)
    }

    /// Library lookup name.
    pub fn lookup(&self) -> &str {
        &self.lookup
    }

    /// Candidate fingerprint.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Reload generation this warning belongs to.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Running slots that would be replaced.
    pub fn running(&self) -> &[String] {
        &self.running
    }

    /// Paused slots that would be replaced.
    pub fn paused(&self) -> &[String] {
        &self.paused
    }

    /// Slots that paused during prepare.
    pub fn paused_during_prep(&self) -> &[String] {
        &self.paused_during_prep
    }

    /// Affected (slot, generation) pairs.
    pub fn affected_generations(&self) -> &[(String, u64)] {
        &self.affected_generations
    }
}

pub(crate) struct CatalogPending {
    pub root: PathBuf,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<script::LoadFailure>,
    pub set_fingerprint: String,
}

enum PendingReloadKindInner {
    Manual,
    Catalog(CatalogPending),
}

#[derive(Debug)]
pub struct PendingReloadKind(PendingReloadKindInner);

impl std::fmt::Debug for PendingReloadKindInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => f.write_str("Manual"),
            Self::Catalog(pending) => f
                .debug_struct("Catalog")
                .field("root", &pending.root)
                .field("added", &pending.added)
                .field("removed", &pending.removed)
                .field("failed", &pending.failed)
                .field("set_fingerprint", &pending.set_fingerprint)
                .finish(),
        }
    }
}

#[allow(non_upper_case_globals)]
impl PendingReloadKind {
    pub const Manual: Self = Self(PendingReloadKindInner::Manual);

    pub(crate) fn catalog(pending: CatalogPending) -> Self {
        Self(PendingReloadKindInner::Catalog(pending))
    }

    pub fn is_manual(&self) -> bool {
        matches!(self.0, PendingReloadKindInner::Manual)
    }

    pub fn is_catalog(&self) -> bool {
        matches!(self.0, PendingReloadKindInner::Catalog(_))
    }

    pub(crate) fn as_catalog(&self) -> Option<&CatalogPending> {
        match &self.0 {
            PendingReloadKindInner::Catalog(pending) => Some(pending),
            PendingReloadKindInner::Manual => None,
        }
    }

    pub(crate) fn into_catalog(self) -> Option<CatalogPending> {
        match self.0 {
            PendingReloadKindInner::Catalog(pending) => Some(pending),
            PendingReloadKindInner::Manual => None,
        }
    }
}

#[derive(Debug)]
pub struct PendingReload {
    pub(crate) warning: ReloadWarning,
    pub(crate) prepared: Vec<script::PreparedCard>,
    warmed: Vec<script::PreparedCard>,
    pub(crate) kind: PendingReloadKind,
}

impl PendingReload {
    /// The warning published for this candidate.
    pub fn warning(&self) -> &ReloadWarning {
        &self.warning
    }

    /// Prepared cards waiting to apply.
    #[cfg(any(test, feature = "test-support"))]
    pub fn prepared(&self) -> &[script::PreparedCard] {
        &self.prepared
    }

    /// Manual vs catalog refresh.
    pub fn kind(&self) -> &PendingReloadKind {
        &self.kind
    }

    fn take_warmed(&mut self) -> Vec<script::PreparedCard> {
        std::mem::take(&mut self.warmed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadOutcomeKind {
    NothingChanged,
    NeedsConfirm,
    Applied,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReloadOutcome {
    kind: ReloadOutcomeKind,
    restarted: usize,
    stopped_paused: usize,
    failed: usize,
    error: Option<String>,
}

#[allow(non_upper_case_globals)]
impl ReloadOutcome {
    pub const NothingChanged: Self = Self {
        kind: ReloadOutcomeKind::NothingChanged,
        restarted: 0,
        stopped_paused: 0,
        failed: 0,
        error: None,
    };
    pub const NeedsConfirm: Self = Self {
        kind: ReloadOutcomeKind::NeedsConfirm,
        restarted: 0,
        stopped_paused: 0,
        failed: 0,
        error: None,
    };

    pub fn applied(restarted: usize, stopped_paused: usize, failed: usize) -> Self {
        Self {
            kind: ReloadOutcomeKind::Applied,
            restarted,
            stopped_paused,
            failed,
            error: None,
        }
    }

    /// Apply failed with `error`.
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            kind: ReloadOutcomeKind::Failed,
            restarted: 0,
            stopped_paused: 0,
            failed: 0,
            error: Some(error.into()),
        }
    }

    /// Whether nothing on disk changed.
    pub fn is_nothing_changed(&self) -> bool {
        self.kind == ReloadOutcomeKind::NothingChanged
    }

    /// Whether a confirm is required before apply.
    pub fn is_needs_confirm(&self) -> bool {
        self.kind == ReloadOutcomeKind::NeedsConfirm
    }

    /// Failure text, when this outcome failed.
    pub fn failure(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Applied counts: restarted, stopped-paused, failed.
    pub fn applied_counts(&self) -> Option<(usize, usize, usize)> {
        match self.kind {
            ReloadOutcomeKind::Applied => Some((self.restarted, self.stopped_paused, self.failed)),
            _ => None,
        }
    }
}

type ValidationBatch = Vec<(script::PreparedCard, Result<(), String>)>;

struct ValidationJob {
    epoch: u64,
    receiver: script::ValidationBatch,
    kind: ValidationKind,
}

struct ManualValidation {
    source: script::ScriptSource,
    lookup: String,
    identity_key: String,
    fingerprint: String,
    running_before: Vec<String>,
    paused_before: Vec<String>,
}

enum ValidationKind {
    Manual(ManualValidation),
    Catalog {
        root: PathBuf,
        diff: script::CatalogDiff,
        warmed: Vec<script::PreparedCard>,
        prepare_failed: Vec<script::LoadFailure>,
    },
}

enum PreparationRequest {
    Manual {
        snapshot: Arc<script::load::CardPreparationSnapshot>,
        manual: ManualValidation,
        confirming: bool,
    },
    Catalog {
        snapshot: Arc<script::load::CatalogPreparationSnapshot>,
        root: PathBuf,
    },
    CatalogConfirmation {
        snapshot: Arc<script::load::CatalogPreparationSnapshot>,
        root: PathBuf,
    },
}

enum PreparationJob {
    Pending(PreparationRequest),
    Manual {
        epoch: u64,
        task:
            script::preparation::PreparedTask<Result<script::load::CardPreparationOutcome, String>>,
        manual: ManualValidation,
        confirming: bool,
    },
    Catalog {
        epoch: u64,
        task: script::preparation::PreparedTask<
            Result<script::load::CatalogPreparationResult, script::load::CatalogWorkerError>,
        >,
        root: PathBuf,
    },
    CatalogConfirmation {
        epoch: u64,
        task: script::preparation::PreparedTask<
            Result<script::CatalogDiff, script::load::CatalogWorkerError>,
        >,
        root: PathBuf,
    },
}

const MAX_PENDING_STARTS: usize = 64;

/// Operator preparations that share the process WorkAdmission worker. Pending
/// variants retain one bounded snapshot for retry when admission is Full.
#[derive(Default)]
pub(super) struct ScriptWorkState {
    next_generation: u64,
    catalog_generation: u64,
    catalog: Option<CatalogJob>,
    catalog_fill_result: Option<Result<(), CatalogError>>,
    catalog_import_result: Option<(PathBuf, Result<usize, CatalogError>)>,
    starts: std::collections::HashMap<String, StartPreparation>,
    transpile_generation: u64,
    transpile: Option<TranspileJob>,
    transpile_result: Option<TranspileResult>,
    pending_marked_action: Option<crate::marked::PendingCatalogAssignment>,
}

enum CatalogJob {
    FillPending {
        generation: u64,
        root: PathBuf,
        path_file: PathBuf,
        snapshot: Arc<script::load::CatalogPreparationSnapshot>,
    },
    FillRunning {
        generation: u64,
        root: PathBuf,
        path_file: PathBuf,
        task: script::preparation::PreparedTask<
            Result<script::CatalogDiff, script::load::CatalogWorkerError>,
        >,
    },
    ImportPending {
        generation: u64,
        root: PathBuf,
        path_file: PathBuf,
        snapshot: Arc<script::load::CatalogPreparationSnapshot>,
    },
    ImportRunning {
        generation: u64,
        root: PathBuf,
        path_file: PathBuf,
        task: script::preparation::PreparedTask<
            Result<script::load::CatalogPreparationResult, script::load::CatalogWorkerError>,
        >,
    },
}

struct StartDemand {
    selection: script::ScriptSel,
    kind: StartKind,
    tallied: bool,
    catalog_root: Option<PathBuf>,
    assignment: Option<script::ScriptSel>,
    had_arm: bool,
    latched: bool,
}

struct StartPreparation {
    generation: u64,
    demand: StartDemand,
    stage: StartStage,
}

enum StartStage {
    WaitingCatalog,
    Pending {
        snapshot: Arc<script::load::CardPreparationSnapshot>,
    },
    Running {
        snapshot: Arc<script::load::CardPreparationSnapshot>,
        task:
            script::preparation::PreparedTask<Result<script::load::CardPreparationOutcome, String>>,
    },
    Ready {
        card: Box<script::JsCard>,
        siblings: Vec<(String, String)>,
        bag: Option<serde_json::Map<String, serde_json::Value>>,
        fingerprint: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranspileClient {
    PanelQueue,
    ExternalLoader,
    LiveSetup,
}

enum TranspileJob {
    Pending {
        generation: u64,
        client: TranspileClient,
        source: script::ScriptSource,
        lookup: String,
        snapshot: Arc<script::load::CardPreparationSnapshot>,
    },
    Running {
        generation: u64,
        client: TranspileClient,
        source: script::ScriptSource,
        lookup: String,
        snapshot: Arc<script::load::CardPreparationSnapshot>,
        task:
            script::preparation::PreparedTask<Result<script::load::CardPreparationOutcome, String>>,
    },
}

struct TranspileResult {
    generation: u64,
    client: TranspileClient,
    source: script::ScriptSource,
    lookup: String,
    result: Result<(), String>,
}

impl ScriptWorkState {
    fn next_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }
}

/// Reload transaction state. `generation` fences every prepared candidate
/// and validation result: Cancel and Stop all bump it.
#[derive(Default)]
pub(super) struct ReloadState {
    generation: u64,
    preparation: Option<PreparationJob>,
    validation: Option<ValidationJob>,
    pending: Option<PendingReload>,
    catalog_confirm: bool,
    catalog_report: Option<String>,
    outcome: Option<ReloadOutcome>,
}

impl ReloadState {
    fn clear_pending(&mut self) {
        self.pending = None;
        self.catalog_confirm = false;
    }

    fn install(&mut self, pending: PendingReload) {
        self.catalog_confirm = pending.kind.is_catalog();
        self.pending = Some(pending);
    }

    /// Discard a candidate in preparation or awaiting confirmation.
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.preparation = None;
        self.validation = None;
        self.clear_pending();
    }
}

impl Scripts {
    /// Whether a catalog fill or import has been submitted or retained for a
    /// bounded WorkAdmission retry.
    pub fn catalog_work_pending(&self) -> bool {
        self.async_work.catalog.is_some()
    }

    /// Queue the first source-only catalog scan. The caller may retry this
    /// method after a hard refusal; Full is retained internally.
    pub fn request_catalog_fill(
        &mut self,
        root: &Path,
        path_file: &Path,
    ) -> Result<(), CatalogError> {
        if self.catalog_root.as_deref() == Some(root) && self.catalog_filled {
            return Ok(());
        }
        let active_root = self.async_work.catalog.as_ref().map(|job| match job {
            CatalogJob::FillPending { root, .. }
            | CatalogJob::FillRunning { root, .. }
            | CatalogJob::ImportPending { root, .. }
            | CatalogJob::ImportRunning { root, .. } => root.as_path(),
        });
        match active_root {
            Some(active_root) if active_root == root => return Ok(()),
            Some(_) => return Err(CatalogError(CatalogFailure::BusyForAnotherRoot)),
            None => {}
        }
        if self.async_work.catalog_import_result.is_some() {
            return Err(CatalogError(CatalogFailure::UncollectedImportResult));
        }
        let snapshot = Arc::new(self.js.catalog_preparation_snapshot(root)?);
        let generation = self.async_work.next_generation();
        self.async_work.catalog_generation = generation;
        self.async_work.catalog_fill_result = None;
        self.async_work.catalog = Some(CatalogJob::FillPending {
            generation,
            root: root.to_path_buf(),
            path_file: path_file.to_path_buf(),
            snapshot,
        });
        self.submit_catalog_job();
        self.async_work.catalog_fill_result.take().unwrap_or(Ok(()))
    }

    /// Queue an explicit catalog import and its bounded preparation. Unlike
    /// the first Browse scan, the import also prepares changed cards.
    pub fn request_catalog_import(
        &mut self,
        root: &Path,
        path_file: &Path,
    ) -> Result<(), CatalogError> {
        if self.async_work.catalog.is_some() {
            return Err(CatalogError(CatalogFailure::Busy));
        }
        if self.async_work.catalog_import_result.is_some() {
            return Err(CatalogError(CatalogFailure::UncollectedImportResult));
        }
        let snapshot = Arc::new(self.js.catalog_preparation_snapshot(root)?);
        let generation = self.async_work.next_generation();
        self.async_work.catalog_generation = generation;
        self.async_work.catalog_fill_result = None;
        self.async_work.catalog = Some(CatalogJob::ImportPending {
            generation,
            root: root.to_path_buf(),
            path_file: path_file.to_path_buf(),
            snapshot,
        });
        self.submit_catalog_job();
        match self.async_work.catalog_import_result.take() {
            Some((_, result)) => result.map(|_| ()),
            None => Ok(()),
        }
    }

    /// Take the settled import receipt, if any: the root it was taken for
    /// and the prepared catalog's card count. A new fill or import is
    /// refused until this is collected.
    pub fn take_catalog_import_result(&mut self) -> Option<(PathBuf, Result<usize, CatalogError>)> {
        self.async_work.catalog_import_result.take()
    }

    /// Request one card's preparation through WorkAdmission for the panel
    /// warmup queue. Full is retained as one bounded demand.
    pub fn request_transpile(&mut self, source: CardSource, lookup: &str) -> Result<(), String> {
        let source = source.to_script();
        self.request_transpile_for(TranspileClient::PanelQueue, source, lookup)
            .map(|_| ())
    }

    /// Request one card's preparation for the panel's external file loader.
    /// `Ok(false)` means the single worker slot belongs to another client;
    /// the loader should retain its request and retry on a later dispatch.
    pub fn request_external_loader_transpile(
        &mut self,
        source: CardSource,
        lookup: &str,
    ) -> Result<bool, String> {
        let source = source.to_script();
        self.request_transpile_for(TranspileClient::ExternalLoader, source, lookup)
    }

    pub fn external_loader_transpile_pending(&self) -> bool {
        self.transpile_pending_for(TranspileClient::ExternalLoader)
    }

    pub fn take_external_loader_transpile_result(
        &mut self,
        source: CardSource,
        lookup: &str,
    ) -> Option<Result<(), String>> {
        let source = source.to_script();
        self.take_transpile_result_for(TranspileClient::ExternalLoader, source, lookup)
    }

    fn request_transpile_for(
        &mut self,
        client: TranspileClient,
        source: script::ScriptSource,
        lookup: &str,
    ) -> Result<bool, String> {
        if let Some(job) = self.async_work.transpile.as_ref() {
            let (active_client, active_source, active_lookup) = match job {
                TranspileJob::Pending {
                    client,
                    source,
                    lookup,
                    ..
                }
                | TranspileJob::Running {
                    client,
                    source,
                    lookup,
                    ..
                } => (*client, *source, lookup.as_str()),
            };
            return Ok(active_client == client
                && active_source == source
                && active_lookup == lookup);
        }
        if let Some(result) = self.async_work.transpile_result.as_ref() {
            return Ok(result.client == client
                && result.source == source
                && result.lookup == lookup);
        }
        if self.js.js_is_ready(source, lookup) {
            let generation = self.async_work.next_generation();
            self.async_work.transpile_generation = generation;
            self.async_work.transpile_result = Some(TranspileResult {
                generation,
                client,
                source,
                lookup: lookup.to_string(),
                result: Ok(()),
            });
            return Ok(true);
        }
        let snapshot = Arc::new(
            self.js
                .card_preparation_snapshot(source, lookup, None)?
                .force_prepare()
                .cache_even_if_unloadable(),
        );
        let generation = self.async_work.next_generation();
        self.async_work.transpile_generation = generation;
        self.async_work.transpile = Some(TranspileJob::Pending {
            generation,
            client,
            source,
            lookup: lookup.to_string(),
            snapshot,
        });
        self.submit_transpile_job();
        match self.async_work.transpile_result.take() {
            Some(result) => match result.result {
                Ok(()) => Ok(true),
                Err(error) => Err(error),
            },
            None => Ok(true),
        }
    }

    pub fn transpile_pending(&self) -> bool {
        self.transpile_pending_for(TranspileClient::PanelQueue)
    }

    fn transpile_pending_for(&self, client: TranspileClient) -> bool {
        self.async_work
            .transpile
            .as_ref()
            .is_some_and(|job| match job {
                TranspileJob::Pending { client: active, .. }
                | TranspileJob::Running { client: active, .. } => *active == client,
            })
    }

    pub fn take_transpile_result(&mut self) -> Option<(CardSource, String, Result<(), String>)> {
        let result = self.async_work.transpile_result.as_ref()?;
        if result.generation != self.async_work.transpile_generation
            || result.client != TranspileClient::PanelQueue
        {
            return None;
        }
        let result = self.async_work.transpile_result.take().unwrap();
        Some((
            CardSource::from_script(result.source),
            result.lookup,
            result.result,
        ))
    }

    fn take_transpile_result_for(
        &mut self,
        client: TranspileClient,
        source: script::ScriptSource,
        lookup: &str,
    ) -> Option<Result<(), String>> {
        let matches = self
            .async_work
            .transpile_result
            .as_ref()
            .is_some_and(|result| {
                result.generation == self.async_work.transpile_generation
                    && result.client == client
                    && result.source == source
                    && result.lookup == lookup
            });
        matches.then(|| self.async_work.transpile_result.take().unwrap().result)
    }

    /// Synchronous receipt for non-interactive live setup before its session
    /// begins pumping. Interactive callers use `request_catalog_fill` and
    /// advance it from `Scripts::poll`.
    pub fn fill_catalog_blocking(
        &mut self,
        root: &Path,
        path_file: &Path,
    ) -> Result<(), CatalogError> {
        self.request_catalog_fill(root, path_file)?;
        while self.catalog_work_pending() {
            self.poll_catalog_job();
            if self.catalog_work_pending() {
                std::thread::yield_now();
            }
        }
        self.async_work.catalog_fill_result.take().unwrap_or(Ok(()))
    }

    /// Synchronous receipt for non-interactive live setup before its session
    /// begins pumping. Interactive callers use `request_transpile`.
    pub fn transpile_blocking(&mut self, source: CardSource, lookup: &str) -> Result<(), String> {
        let source = source.to_script();
        loop {
            if self.request_transpile_for(TranspileClient::LiveSetup, source, lookup)? {
                break;
            }
            self.poll_transpile_job();
            if self.async_work.transpile_result.is_some()
                && !self.transpile_pending_for(TranspileClient::LiveSetup)
            {
                return Err("another client has an uncollected card preparation result".into());
            }
            std::thread::yield_now();
        }
        loop {
            self.poll_transpile_job();
            if let Some(result) =
                self.take_transpile_result_for(TranspileClient::LiveSetup, source, lookup)
            {
                return result;
            }
            if !self.transpile_pending_for(TranspileClient::LiveSetup) {
                return Err("card preparation finished without a result".into());
            }
            std::thread::yield_now();
        }
    }

    /// Submit the queued catalog job to WorkAdmission. A submission failure
    /// is stored once as the async receipt; the requesting method returns it
    /// from that receipt, so one owner holds the error.
    fn submit_catalog_job(&mut self) {
        let Some(job) = self.async_work.catalog.take() else {
            return;
        };
        match job {
            CatalogJob::FillPending {
                generation,
                root,
                path_file,
                snapshot,
            } => {
                let input = match script::preparation::PreparationInput::new(
                    generation,
                    snapshot.input_bytes(),
                    snapshot.scratch_bytes(),
                    snapshot.result_bytes(),
                ) {
                    Ok(input) => input,
                    Err(error) => {
                        let error = CatalogError::from(error);
                        self.show(format!("$RS2B0T registry: {error}"));
                        self.async_work.catalog_fill_result = Some(Err(error));
                        return;
                    }
                };
                let work = Arc::clone(&snapshot).cooperative_diff();
                match script::preparation::submit_cooperative(
                    script::preparation::PreparationKind::Catalogue,
                    api::identity::Scope::Process,
                    input,
                    work,
                ) {
                    Ok(task) => {
                        self.async_work.catalog = Some(CatalogJob::FillRunning {
                            generation,
                            root,
                            path_file,
                            task,
                        });
                    }
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {
                        self.async_work.catalog = Some(CatalogJob::FillPending {
                            generation,
                            root,
                            path_file,
                            snapshot,
                        });
                    }
                    Err(error) => {
                        let error = CatalogError::from(error);
                        self.show(format!("$RS2B0T registry: {error}"));
                        self.async_work.catalog_fill_result = Some(Err(error));
                    }
                }
            }
            CatalogJob::ImportPending {
                generation,
                root,
                path_file,
                snapshot,
            } => {
                let input = match script::preparation::PreparationInput::new(
                    generation,
                    snapshot.input_bytes(),
                    snapshot.scratch_bytes(),
                    snapshot.result_bytes(),
                ) {
                    Ok(input) => input,
                    Err(error) => {
                        let error = CatalogError::from(error);
                        self.show(format!("Import catalog: {error}"));
                        self.async_work.catalog_import_result = Some((root, Err(error)));
                        return;
                    }
                };
                let work = Arc::clone(&snapshot).cooperative_prepare();
                match script::preparation::submit_cooperative(
                    script::preparation::PreparationKind::Catalogue,
                    api::identity::Scope::Process,
                    input,
                    work,
                ) {
                    Ok(task) => {
                        self.async_work.catalog = Some(CatalogJob::ImportRunning {
                            generation,
                            root,
                            path_file,
                            task,
                        });
                    }
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {
                        self.async_work.catalog = Some(CatalogJob::ImportPending {
                            generation,
                            root,
                            path_file,
                            snapshot,
                        });
                    }
                    Err(error) => {
                        let error = CatalogError::from(error);
                        self.show(format!("Import catalog: {error}"));
                        self.async_work.catalog_import_result = Some((root, Err(error)));
                    }
                }
            }
            CatalogJob::FillRunning {
                generation,
                root,
                path_file,
                task,
            } => {
                self.async_work.catalog = Some(CatalogJob::FillRunning {
                    generation,
                    root,
                    path_file,
                    task,
                });
            }
            CatalogJob::ImportRunning {
                generation,
                root,
                path_file,
                task,
            } => {
                self.async_work.catalog = Some(CatalogJob::ImportRunning {
                    generation,
                    root,
                    path_file,
                    task,
                });
            }
        }
    }

    fn poll_catalog_job(&mut self) {
        let finished = match self.async_work.catalog.as_ref() {
            Some(CatalogJob::FillRunning { task, .. }) => task.is_finished(),
            Some(CatalogJob::ImportRunning { task, .. }) => task.is_finished(),
            Some(CatalogJob::FillPending { .. } | CatalogJob::ImportPending { .. }) => false,
            None => return,
        };
        if !finished {
            if matches!(
                self.async_work.catalog.as_ref(),
                Some(CatalogJob::FillPending { .. } | CatalogJob::ImportPending { .. })
            ) {
                self.submit_catalog_job();
            }
            return;
        }
        let Some(job) = self.async_work.catalog.take() else {
            return;
        };
        match job {
            CatalogJob::FillRunning {
                generation,
                root,
                path_file,
                task,
            } if generation == self.async_work.catalog_generation => match task.join() {
                Ok(Ok(diff)) => {
                    self.js.apply_catalog_diff(diff);
                    self.js.set_catalog_root(&root);
                    self.catalog_root = Some(root.clone());
                    self.catalog_filled = true;
                    let _ = script::persist_rs2b0t_root_at(&root, &path_file);
                    self.async_work.catalog_fill_result = Some(Ok(()));
                }
                Ok(Err(error)) => self.fail_catalog_fill(CatalogError::from(error)),
                Err(error) => self.fail_catalog_fill(CatalogError::from(error)),
            },
            CatalogJob::ImportRunning {
                generation,
                root,
                path_file,
                task,
            } if generation == self.async_work.catalog_generation => match task.join() {
                Ok(Ok(result)) => {
                    match self.apply_catalog_import(root.clone(), path_file, result) {
                        Ok(count) => {
                            self.async_work.catalog_import_result = Some((root, Ok(count)));
                            self.show(format!("Catalog imported: {count} cards"));
                        }
                        Err(error) => {
                            self.show(format!("Import catalog: {error}"));
                            self.async_work.catalog_import_result = Some((root, Err(error)));
                        }
                    }
                }
                Ok(Err(error)) => {
                    let error = CatalogError::from(error);
                    self.show(format!("Import catalog: {error}"));
                    self.async_work.catalog_import_result = Some((root, Err(error)));
                }
                Err(error) => {
                    let error = CatalogError::from(error);
                    self.show(format!("Import catalog: {error}"));
                    self.async_work.catalog_import_result = Some((root, Err(error)));
                }
            },
            CatalogJob::FillRunning { .. }
            | CatalogJob::ImportRunning { .. }
            | CatalogJob::FillPending { .. }
            | CatalogJob::ImportPending { .. } => {}
        }
    }

    fn fail_catalog_fill(&mut self, error: CatalogError) {
        self.show(format!("$RS2B0T registry: {error}"));
        self.async_work.catalog_fill_result = Some(Err(error));
    }

    fn apply_catalog_import(
        &mut self,
        root: PathBuf,
        path_file: PathBuf,
        result: script::load::CatalogPreparationResult,
    ) -> Result<usize, CatalogError> {
        let script::load::CatalogPreparationResult {
            diff,
            prepared,
            warmed,
            prepare_failed,
        } = result;
        for failure in prepare_failed {
            self.js.record_failure(failure);
        }
        self.js.apply_catalog_diff(diff);
        for item in prepared.into_iter().chain(warmed) {
            // Catalog cards never persist, so a commit failure here would be
            // an internal invariant violation, not a distinct failure kind.
            self.js
                .commit_prepared(item)
                .map_err(CatalogError::unexpected_catalog_commit)?;
        }
        self.js.set_catalog_root(&root);
        self.catalog_root = Some(root.clone());
        self.catalog_filled = true;
        let _ = script::persist_rs2b0t_root_at(&root, &path_file);
        Ok(self
            .js
            .cards()
            .iter()
            .filter(|card| card.source == script::ScriptSource::Catalog)
            .count())
    }

    /// Submit the queued transpile job to WorkAdmission. A submission
    /// failure is stored once as the async receipt; the requesting method
    /// returns it from that receipt, so one owner holds the error.
    fn submit_transpile_job(&mut self) {
        let Some(job) = self.async_work.transpile.take() else {
            return;
        };
        let TranspileJob::Pending {
            generation,
            client,
            source,
            lookup,
            snapshot,
        } = job
        else {
            self.async_work.transpile = Some(job);
            return;
        };
        let input = match script::preparation::PreparationInput::new(
            generation,
            snapshot.input_bytes(),
            snapshot.scratch_bytes(),
            snapshot.result_bytes(),
        ) {
            Ok(input) => input,
            Err(error) => {
                self.async_work.transpile_result = Some(TranspileResult {
                    generation,
                    client,
                    source,
                    lookup,
                    result: Err(error.to_string()),
                });
                return;
            }
        };
        let work = Arc::clone(&snapshot).cooperative_prepare();
        match script::preparation::submit_cooperative(
            script::preparation::PreparationKind::Card,
            api::identity::Scope::Process,
            input,
            work,
        ) {
            Ok(task) => {
                self.async_work.transpile = Some(TranspileJob::Running {
                    generation,
                    client,
                    source,
                    lookup,
                    snapshot,
                    task,
                });
            }
            Err(script::preparation::PreparationError::Refused(
                api::work::SubmissionRefusal::Full,
            )) => {
                self.async_work.transpile = Some(TranspileJob::Pending {
                    generation,
                    client,
                    source,
                    lookup,
                    snapshot,
                });
            }
            Err(error) => {
                self.async_work.transpile_result = Some(TranspileResult {
                    generation,
                    client,
                    source,
                    lookup,
                    result: Err(error.to_string()),
                });
            }
        }
    }

    fn poll_transpile_job(&mut self) {
        let finished = match self.async_work.transpile.as_ref() {
            Some(TranspileJob::Running { task, .. }) => task.is_finished(),
            Some(TranspileJob::Pending { .. }) => false,
            None => return,
        };
        if !finished {
            if matches!(
                self.async_work.transpile.as_ref(),
                Some(TranspileJob::Pending { .. })
            ) {
                self.submit_transpile_job();
                if let Some(result) = self.async_work.transpile_result.as_ref() {
                    if let Err(error) = &result.result {
                        self.show(format!("transpile {}: {error}", result.lookup));
                    }
                }
            }
            return;
        }
        let Some(job) = self.async_work.transpile.take() else {
            return;
        };
        let TranspileJob::Running {
            generation,
            client,
            source,
            lookup,
            snapshot,
            task,
        } = job
        else {
            self.async_work.transpile = Some(job);
            return;
        };
        if generation != self.async_work.transpile_generation {
            return;
        }
        let result = match task.join() {
            Ok(Ok(script::load::CardPreparationOutcome::Prepared(prepared))) => {
                if !self.card_snapshot_current(source, &lookup, &snapshot) {
                    Err("card changed while it was being prepared".into())
                } else {
                    self.js.commit_prepared(prepared).map(|_| ())
                }
            }
            Ok(Ok(script::load::CardPreparationOutcome::Unchanged)) => {
                if self.card_snapshot_current(source, &lookup, &snapshot)
                    && self.js.js_is_ready(source, &lookup)
                {
                    Ok(())
                } else {
                    Err("card preparation produced no JavaScript".into())
                }
            }
            Ok(Ok(script::load::CardPreparationOutcome::Failed { error, failure })) => {
                self.js.record_failure(failure);
                Err(error)
            }
            Ok(Ok(script::load::CardPreparationOutcome::Confirmed)) => {
                Err("unexpected confirmation result".into())
            }
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = &result {
            self.show(format!("transpile {lookup}: {error}"));
        }
        self.async_work.transpile_result = Some(TranspileResult {
            generation,
            client,
            source,
            lookup,
            result,
        });
    }
}

impl Scripts {
    pub(super) fn queue_loaded_start<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        profile: &str,
        selection: &script::ScriptSel,
        catalog_root: Option<&Path>,
        kind: StartKind,
        tallied: bool,
    ) -> Result<(), script::StartLoadError> {
        let script::ScriptSel::Loaded(source, lookup) = selection else {
            unreachable!("only loaded Starts use preparation work")
        };
        let source = *source;
        let lookup = lookup.as_str();
        if let Some(preparing) = self.async_work.starts.get(profile) {
            if preparing.demand.kind == kind
                && matches!(
                    &preparing.demand.selection,
                    script::ScriptSel::Loaded(existing_source, existing_lookup)
                        if *existing_source == source && existing_lookup == lookup
                )
            {
                return Ok(());
            }
            return Err(script::StartLoadError::Waiting(
                "script Start preparation already pending".into(),
            ));
        }
        if self.async_work.starts.len() >= MAX_PENDING_STARTS {
            return Err(script::StartLoadError::Refused(format!(
                "script Start preparation limit reached ({MAX_PENDING_STARTS})"
            )));
        }
        let wait_for_catalog = source == script::ScriptSource::Catalog
            && (self.async_work.catalog.is_some()
                || !self.catalog_filled
                || self.catalog_root.is_none());
        if wait_for_catalog && self.async_work.catalog.is_none() {
            let root = catalog_root.ok_or_else(|| {
                script::StartLoadError::Refused("catalog root is not available".into())
            })?;
            self.request_catalog_fill(root, &script::default_rs2b0t_path_file())
                .map_err(|error| script::StartLoadError::Refused(error.to_string()))?;
        }
        let stage = if wait_for_catalog {
            StartStage::WaitingCatalog
        } else {
            StartStage::Pending {
                snapshot: self
                    .start_snapshot(source, lookup)
                    .map_err(script::StartLoadError::Refused)?,
            }
        };
        let generation = self.async_work.next_generation();
        let (had_arm, latched) = super::arm_flags(core, profile);
        self.async_work.starts.insert(
            profile.to_string(),
            StartPreparation {
                generation,
                demand: StartDemand {
                    selection: selection.clone(),
                    kind,
                    tallied,
                    catalog_root: catalog_root.map(Path::to_path_buf),
                    assignment: Self::assignment(core, profile)
                        .and_then(|assignment| super::sel_from_assignment(&assignment)),
                    had_arm,
                    latched,
                },
                stage,
            },
        );
        if !wait_for_catalog {
            if let Err(error) = self.submit_start_preparation(profile) {
                self.async_work.starts.remove(profile);
                return Err(script::StartLoadError::Refused(error));
            }
        }
        Ok(())
    }

    fn start_snapshot(
        &self,
        source: script::ScriptSource,
        lookup: &str,
    ) -> Result<Arc<script::load::CardPreparationSnapshot>, String> {
        let card = self.startable_card(source, lookup)?;
        if let Some(reason) = &card.unloadable {
            return Err(format!("unloadable import: {reason}"));
        }
        Ok(Arc::new(
            self.js
                .card_preparation_snapshot(source, lookup, None)?
                .force_prepare(),
        ))
    }

    fn card_snapshot_current(
        &self,
        source: script::ScriptSource,
        lookup: &str,
        snapshot: &script::load::CardPreparationSnapshot,
    ) -> bool {
        self.js.get(source, lookup).is_some_and(|card| {
            snapshot.matches_card(card)
                && self.js.stored_fingerprint(&card.identity_key()) == snapshot.stored_fingerprint()
        })
    }

    fn submit_start_preparation(&mut self, profile: &str) -> Result<(), String> {
        let Some(preparation) = self.async_work.starts.get(profile) else {
            return Ok(());
        };
        let StartStage::Pending { snapshot } = &preparation.stage else {
            return Ok(());
        };
        let generation = preparation.generation;
        let snapshot = Arc::clone(snapshot);
        let input = script::preparation::PreparationInput::new(
            generation,
            snapshot.input_bytes(),
            snapshot.scratch_bytes(),
            snapshot.result_bytes(),
        )
        .map_err(|error| error.to_string())?;
        let work = Arc::clone(&snapshot).cooperative_prepare();
        match script::preparation::submit_cooperative(
            script::preparation::PreparationKind::Card,
            api::identity::Scope::Process,
            input,
            work,
        ) {
            Ok(task) => {
                if let Some(preparation) = self.async_work.starts.get_mut(profile) {
                    if preparation.generation == generation {
                        preparation.stage = StartStage::Running { snapshot, task };
                    }
                }
                Ok(())
            }
            Err(script::preparation::PreparationError::Refused(
                api::work::SubmissionRefusal::Full,
            )) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn poll_script_work<Io>(&mut self, core: &mut OperatorSession<Io>) {
        self.poll_catalog_job();
        crate::marked::poll_pending_catalog_assignment(core, self);
        self.poll_transpile_job();
        self.poll_start_preparation(core);
    }

    fn poll_start_preparation<Io>(&mut self, core: &mut OperatorSession<Io>) {
        let ready_profile = self
            .async_work
            .starts
            .iter()
            .find_map(|(profile, preparation)| {
                let ready = match &preparation.stage {
                    StartStage::WaitingCatalog => self.async_work.catalog.is_none(),
                    StartStage::Pending { .. } | StartStage::Ready { .. } => true,
                    StartStage::Running { task, .. } => task.is_finished(),
                };
                ready.then(|| profile.clone())
            });
        let Some(profile) = ready_profile else {
            return;
        };
        let Some(mut preparation) = self.async_work.starts.remove(&profile) else {
            return;
        };
        match preparation.stage {
            StartStage::WaitingCatalog => {
                if let Some(Err(error)) = self.async_work.catalog_fill_result.as_ref() {
                    self.report_start_failure(
                        &profile,
                        preparation.demand.kind,
                        preparation.demand.tallied,
                        error.to_string(),
                    );
                    return;
                }
                if !self.catalog_filled || self.catalog_root.is_none() {
                    let Some(root) = preparation.demand.catalog_root.as_deref() else {
                        self.report_start_failure(
                            &profile,
                            preparation.demand.kind,
                            preparation.demand.tallied,
                            "catalog root is not available".into(),
                        );
                        return;
                    };
                    if let Err(error) =
                        self.request_catalog_fill(root, &script::default_rs2b0t_path_file())
                    {
                        self.report_start_failure(
                            &profile,
                            preparation.demand.kind,
                            preparation.demand.tallied,
                            error.to_string(),
                        );
                        return;
                    }
                    self.async_work.starts.insert(profile, preparation);
                    return;
                }
                let script::ScriptSel::Loaded(source, lookup) =
                    preparation.demand.selection.clone()
                else {
                    unreachable!("only loaded Starts use preparation work")
                };
                match self.start_snapshot(source, &lookup) {
                    Ok(snapshot) => preparation.stage = StartStage::Pending { snapshot },
                    Err(error) => {
                        self.report_start_failure(
                            &profile,
                            preparation.demand.kind,
                            preparation.demand.tallied,
                            error,
                        );
                        return;
                    }
                }
            }
            StartStage::Pending { snapshot } => {
                preparation.stage = StartStage::Pending { snapshot };
                self.async_work.starts.insert(profile.clone(), preparation);
                if let Err(error) = self.submit_start_preparation(&profile) {
                    if let Some(preparation) = self.async_work.starts.remove(&profile) {
                        self.report_start_failure(
                            &profile,
                            preparation.demand.kind,
                            preparation.demand.tallied,
                            error,
                        );
                    }
                }
                return;
            }
            StartStage::Running { snapshot, task } => {
                let script::ScriptSel::Loaded(source, lookup) =
                    preparation.demand.selection.clone()
                else {
                    unreachable!("only loaded Starts use preparation work")
                };
                let result = match task.join() {
                    Ok(Ok(script::load::CardPreparationOutcome::Prepared(mut prepared))) => {
                        if !self.card_snapshot_current(source, &lookup, &snapshot) {
                            Err("card changed while it was being prepared".into())
                        } else {
                            let siblings = std::mem::take(&mut prepared.siblings);
                            self.js
                                .commit_prepared_for_start(prepared)
                                .map(|card| (card, siblings))
                        }
                    }
                    Ok(Ok(script::load::CardPreparationOutcome::Unchanged)) => {
                        Err("forced Start preparation returned unchanged".into())
                    }
                    Ok(Ok(script::load::CardPreparationOutcome::Failed { error, failure })) => {
                        self.js.record_failure(failure);
                        Err(error)
                    }
                    Ok(Ok(script::load::CardPreparationOutcome::Confirmed)) => {
                        Err("unexpected confirmation result".into())
                    }
                    Ok(Err(error)) => Err(error),
                    Err(error) => Err(error.to_string()),
                };
                let (card, siblings) = match result {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        self.report_start_failure(
                            &profile,
                            preparation.demand.kind,
                            preparation.demand.tallied,
                            error,
                        );
                        return;
                    }
                };
                let bag = self.merged_profile_bag(
                    core,
                    &profile,
                    CardSource::from_script(source),
                    &card.name,
                    &card.path,
                    super::SchemaRef::new(&card.settings_schema),
                );
                let bag = (!bag.is_empty()).then_some(bag);
                let fingerprint = self
                    .js
                    .stored_fingerprint(&card.identity_key())
                    .map(str::to_owned);
                preparation.stage = StartStage::Ready {
                    card: Box::new(card),
                    siblings,
                    bag,
                    fingerprint,
                };
            }
            StartStage::Ready {
                card,
                siblings,
                bag,
                fingerprint,
            } => {
                if !super::wall_member(core, &profile) || core.removal_pending(&profile) {
                    self.skip_preparing_start(&profile, &preparation.demand, "removed");
                    return;
                }
                let assigned = Self::assignment(core, &profile)
                    .and_then(|assignment| super::sel_from_assignment(&assignment));
                if assigned != preparation.demand.assignment {
                    self.skip_preparing_start(&profile, &preparation.demand, "reassigned");
                    return;
                }
                let (has_arm, latched_now) = super::arm_flags(core, &profile);
                if preparation.demand.had_arm && !has_arm {
                    self.skip_preparing_start(&profile, &preparation.demand, "disconnected");
                    return;
                }
                if !preparation.demand.latched && latched_now {
                    self.skip_preparing_start(&profile, &preparation.demand, "logged out");
                    return;
                }
                let script::ScriptSel::Loaded(source, lookup) = &preparation.demand.selection
                else {
                    unreachable!("only loaded Starts use preparation work")
                };
                let identity = card.identity_key();
                if self.js.get(*source, lookup).is_none_or(|current| {
                    current.identity_key() != identity
                        || self.js.stored_fingerprint(&identity) != fingerprint.as_deref()
                }) {
                    self.skip_preparing_start(&profile, &preparation.demand, "card changed");
                    return;
                }
                if super::script_active(core, &profile) {
                    self.skip_preparing_start(
                        &profile,
                        &preparation.demand,
                        "script already active",
                    );
                    return;
                }
                let start = ScriptStart::load(
                    card.js.clone(),
                    super::LoadShape::from_script(card.shape),
                    bag.clone(),
                    siblings.clone(),
                );
                match core.start_script(&profile, start, Some(card.identity_key())) {
                    Ok(_) => {
                        self.starts.insert(
                            profile,
                            PendingStart {
                                card: PendingCard::Loaded(card),
                                kind: preparation.demand.kind,
                                tallied: preparation.demand.tallied,
                            },
                        );
                    }
                    Err(script::StartLoadError::Waiting(_)) => {
                        preparation.stage = StartStage::Ready {
                            card,
                            siblings,
                            bag,
                            fingerprint,
                        };
                        self.async_work.starts.insert(profile, preparation);
                    }
                    Err(error) => {
                        let diagnostic = self
                            .js
                            .record_start_result(&card, Err(error))
                            .err()
                            .unwrap_or_else(|| "script Start refused".into());
                        self.report_start_failure(
                            &profile,
                            preparation.demand.kind,
                            preparation.demand.tallied,
                            diagnostic,
                        );
                    }
                }
                return;
            }
        }
        let stage = std::mem::replace(&mut preparation.stage, StartStage::WaitingCatalog);
        preparation.stage = stage;
        self.async_work.starts.insert(profile.clone(), preparation);
        if let Err(error) = self.submit_start_preparation(&profile) {
            if let Some(preparation) = self.async_work.starts.remove(&profile) {
                self.report_start_failure(
                    &profile,
                    preparation.demand.kind,
                    preparation.demand.tallied,
                    error,
                );
            }
        }
    }

    fn skip_preparing_start(&mut self, profile: &str, demand: &StartDemand, reason: &str) {
        if demand.tallied {
            self.skip_unstarted_tally(profile, reason);
        } else {
            match demand.kind {
                StartKind::Start => self.show(format!("script: {reason}")),
                StartKind::Reload => self.show(format!("reload: {profile}: {reason}")),
                StartKind::RestartSettings => {
                    self.show(format!("Restart skipped for {profile}: {reason}"))
                }
            }
        }
    }
    pub(super) fn has_start_preparation(&self, profile: &str) -> bool {
        self.async_work.starts.contains_key(profile)
    }

    pub(super) fn start_preparation_pending(&self) -> bool {
        !self.async_work.starts.is_empty()
            || self
                .async_work
                .pending_marked_action
                .as_ref()
                .is_some_and(|action| action.restart)
    }

    pub(super) fn tallied_start_preparation_pending(&self) -> bool {
        self.async_work
            .starts
            .values()
            .any(|preparation| preparation.demand.tallied)
            || self
                .async_work
                .pending_marked_action
                .as_ref()
                .is_some_and(|action| action.restart)
    }

    pub(super) fn cancel_start_preparation(&mut self, profile: &str) -> Option<(StartKind, bool)> {
        self.async_work
            .starts
            .remove(profile)
            .map(|preparation| (preparation.demand.kind, preparation.demand.tallied))
    }

    pub(super) fn cancel_all_start_preparations(&mut self) -> Vec<(String, StartKind, bool)> {
        self.async_work
            .starts
            .drain()
            .map(|(profile, preparation)| {
                (profile, preparation.demand.kind, preparation.demand.tallied)
            })
            .collect()
    }
    pub(crate) fn defer_pending_catalog_assignment(
        &mut self,
        action: crate::marked::PendingCatalogAssignment,
    ) -> Result<(), String> {
        if self.async_work.pending_marked_action.is_some() {
            return Err("an assignment is already waiting for catalog preparation".into());
        }
        self.async_work.pending_marked_action = Some(action);
        Ok(())
    }

    pub(crate) fn take_pending_catalog_assignment(
        &mut self,
    ) -> Option<crate::marked::PendingCatalogAssignment> {
        self.async_work.pending_marked_action.take()
    }
    pub(super) fn cancel_pending_catalog_assignment(
        &mut self,
        profile: Option<&str>,
    ) -> Option<Vec<String>> {
        let mut action = self.async_work.pending_marked_action.take()?;
        if !action.restart {
            self.async_work.pending_marked_action = Some(action);
            return None;
        }
        let cancelled = if let Some(profile) = profile {
            let Some(index) = action.profiles.iter().position(|name| name == profile) else {
                self.async_work.pending_marked_action = Some(action);
                return None;
            };
            vec![action.profiles.remove(index)]
        } else {
            std::mem::take(&mut action.profiles)
        };
        if !action.profiles.is_empty() {
            self.async_work.pending_marked_action = Some(action);
        }
        Some(cancelled)
    }

    pub(crate) fn catalog_scan_root(&self) -> Option<&Path> {
        self.catalog_root.as_deref()
    }
}

impl Scripts {
    /// Whether reload preparation or validation is still running or queued.
    pub fn reload_validation_pending(&self) -> bool {
        self.reload.preparation.is_some() || self.reload.validation.is_some()
    }

    /// The terminal result of the latest reload or catalog refresh.
    pub fn take_reload_outcome(&mut self) -> Option<ReloadOutcome> {
        self.reload.outcome.take()
    }

    /// The warning awaiting confirmation, if any.
    pub fn reload_warning(&self) -> Option<&ReloadWarning> {
        self.reload
            .pending
            .as_ref()
            .map(|pending| pending.warning())
    }

    pub fn pending_reload(&self) -> Option<&PendingReload> {
        self.reload.pending.as_ref()
    }

    /// A Reload warning awaits confirmation (a cheap check for button
    /// labels; the confirm itself re-checks the candidate against disk).
    pub fn reload_awaiting_confirm(&self) -> bool {
        self.reload
            .pending
            .as_ref()
            .is_some_and(|pending| pending.kind.is_manual())
    }

    /// The next catalog Refresh confirms the warning shown.
    pub fn catalog_refresh_confirm(&self) -> bool {
        self.reload.catalog_confirm
    }

    pub fn catalog_refresh_report(&self) -> Option<&str> {
        self.reload.catalog_report.as_deref()
    }

    fn finish_reload(&mut self, outcome: ReloadOutcome) {
        self.reload.outcome = Some(outcome);
    }

    fn fail_reload(&mut self, prefix: &str, error: String) {
        self.show(format!("{prefix}: {error}"));
        self.finish_reload(ReloadOutcome::failed(error));
    }

    /// The card a Reload of `profile` targets: its pending Browse
    /// selection, else `heading`, else its assignment. Compiled cards have
    /// nothing to reload.
    pub fn reload_target<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: Option<&str>,
        heading: Option<&ScriptChoice>,
    ) -> Option<(CardSource, String)> {
        let heading = heading.map(ScriptChoice::as_script);
        let profile = profile?;
        let sel = self
            .pending_browse
            .get(profile)
            .map(|sel| sel.as_script().clone())
            .or_else(|| heading.cloned())
            .or_else(|| {
                Self::assignment(core, profile).and_then(|a| super::sel_from_assignment(&a))
            })?;
        match sel {
            script::ScriptSel::Loaded(source, lookup) => {
                Some((CardSource::from_script(source), lookup))
            }
            script::ScriptSel::Compiled(_) => None,
        }
    }

    /// Whether the next Reload of `target` confirms a shown warning.
    pub fn reload_confirmation_pending(&self, target: Option<&(CardSource, String)>) -> bool {
        target.is_some_and(|(source, lookup)| self.manual_pending_binds(source.to_script(), lookup))
    }

    fn submit_preparation(&mut self, request: PreparationRequest) {
        let epoch = self.reload.generation;
        match request {
            PreparationRequest::Manual {
                snapshot,
                manual,
                confirming,
            } => {
                let input = match script::preparation::PreparationInput::new(
                    epoch,
                    snapshot.input_bytes(),
                    snapshot.scratch_bytes(),
                    snapshot.result_bytes(),
                ) {
                    Ok(input) => input,
                    Err(error) => {
                        self.fail_reload("reload", error.to_string());
                        return;
                    }
                };
                let work = Arc::clone(&snapshot).cooperative_prepare();
                match script::preparation::submit_cooperative(
                    script::preparation::PreparationKind::Card,
                    api::identity::Scope::Process,
                    input,
                    work,
                ) {
                    Ok(task) => {
                        self.reload.preparation = Some(PreparationJob::Manual {
                            epoch,
                            task,
                            manual,
                            confirming,
                        });
                    }
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {
                        self.reload.preparation =
                            Some(PreparationJob::Pending(PreparationRequest::Manual {
                                snapshot,
                                manual,
                                confirming,
                            }));
                    }
                    Err(error) => self.fail_reload("reload", error.to_string()),
                }
            }
            PreparationRequest::Catalog { snapshot, root } => {
                let input = match script::preparation::PreparationInput::new(
                    epoch,
                    snapshot.input_bytes(),
                    snapshot.scratch_bytes(),
                    snapshot.result_bytes(),
                ) {
                    Ok(input) => input,
                    Err(error) => {
                        self.fail_reload("Refresh catalog", error.to_string());
                        return;
                    }
                };
                let work = Arc::clone(&snapshot).cooperative_prepare();
                match script::preparation::submit_cooperative(
                    script::preparation::PreparationKind::Catalogue,
                    api::identity::Scope::Process,
                    input,
                    work,
                ) {
                    Ok(task) => {
                        self.reload.preparation =
                            Some(PreparationJob::Catalog { epoch, task, root });
                    }
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {
                        self.reload.preparation =
                            Some(PreparationJob::Pending(PreparationRequest::Catalog {
                                snapshot,
                                root,
                            }));
                    }
                    Err(error) => self.fail_reload("Refresh catalog", error.to_string()),
                }
            }
            PreparationRequest::CatalogConfirmation { snapshot, root } => {
                let input = match script::preparation::PreparationInput::new(
                    epoch,
                    snapshot.input_bytes(),
                    snapshot.scratch_bytes(),
                    snapshot.result_bytes(),
                ) {
                    Ok(input) => input,
                    Err(error) => {
                        self.fail_reload("Refresh catalog", error.to_string());
                        return;
                    }
                };
                let work = Arc::clone(&snapshot).cooperative_diff();
                match script::preparation::submit_cooperative(
                    script::preparation::PreparationKind::Catalogue,
                    api::identity::Scope::Process,
                    input,
                    work,
                ) {
                    Ok(task) => {
                        self.reload.preparation =
                            Some(PreparationJob::CatalogConfirmation { epoch, task, root });
                    }
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {
                        self.reload.preparation = Some(PreparationJob::Pending(
                            PreparationRequest::CatalogConfirmation { snapshot, root },
                        ));
                    }
                    Err(error) => self.fail_reload("Refresh catalog", error.to_string()),
                }
            }
        }
    }

    pub(super) fn poll_reload_preparation<Io>(&mut self, core: &mut OperatorSession<Io>) {
        if matches!(
            self.reload.preparation.as_ref(),
            Some(PreparationJob::Pending(_))
        ) {
            if let Some(PreparationJob::Pending(request)) = self.reload.preparation.take() {
                self.submit_preparation(request);
            }
        }
        let Some(job) = self.reload.preparation.as_ref() else {
            return;
        };
        let epoch = match job {
            PreparationJob::Pending(_) => return,
            PreparationJob::Manual { epoch, .. }
            | PreparationJob::Catalog { epoch, .. }
            | PreparationJob::CatalogConfirmation { epoch, .. } => *epoch,
        };
        if epoch != self.reload.generation {
            self.reload.preparation = None;
            return;
        }
        let finished = match job {
            PreparationJob::Pending(_) => return,
            PreparationJob::Manual { task, .. } => task.is_finished(),
            PreparationJob::Catalog { task, .. } => task.is_finished(),
            PreparationJob::CatalogConfirmation { task, .. } => task.is_finished(),
        };
        if !finished {
            return;
        }
        let Some(job) = self.reload.preparation.take() else {
            return;
        };
        match job {
            PreparationJob::Pending(_) => unreachable!(),
            PreparationJob::Manual {
                task,
                manual,
                confirming,
                ..
            } => match task.join() {
                Ok(Ok(outcome)) => {
                    self.complete_manual_preparation(core, manual, confirming, outcome)
                }
                Ok(Err(error)) => {
                    if confirming {
                        self.reload.clear_pending();
                    }
                    self.fail_reload("reload", error);
                }
                Err(error) => {
                    if confirming {
                        self.reload.clear_pending();
                    }
                    self.fail_reload("reload", error.to_string());
                }
            },
            PreparationJob::Catalog { task, root, .. } => match task.join() {
                Ok(Ok(result)) => self.complete_catalog_preparation(core, root, result),
                Ok(Err(error)) => self.fail_reload("Refresh catalog", error.to_string()),
                Err(error) => self.fail_reload("Refresh catalog", error.to_string()),
            },
            PreparationJob::CatalogConfirmation { task, root, .. } => match task.join() {
                Ok(Ok(diff)) => self.complete_catalog_confirmation(core, root, diff),
                Ok(Err(error)) => self.fail_reload("Refresh catalog", error.to_string()),
                Err(error) => self.fail_reload("Refresh catalog", error.to_string()),
            },
        }
    }

    fn complete_manual_preparation<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        mut manual: ManualValidation,
        confirming: bool,
        outcome: script::load::CardPreparationOutcome,
    ) {
        match outcome {
            script::load::CardPreparationOutcome::Confirmed if confirming => {
                let outcome = self.commit_manual_pending(core);
                self.finish_reload(outcome);
            }
            script::load::CardPreparationOutcome::Confirmed => {
                self.fail_reload("reload", "unexpected confirmation result".into());
            }
            script::load::CardPreparationOutcome::Unchanged => {
                if confirming {
                    self.reload.clear_pending();
                }
                self.show(script::NOTHING_CHANGED_RELOAD);
                self.finish_reload(ReloadOutcome::NothingChanged);
            }
            script::load::CardPreparationOutcome::Prepared(prepared) => {
                manual.fingerprint = prepared.fingerprint.clone();
                match self.spawn_validation(vec![prepared], ValidationKind::Manual(manual)) {
                    Ok(()) => self.clear_notice(),
                    Err(error) => self.fail_reload("reload", error),
                }
            }
            script::load::CardPreparationOutcome::Failed { error, failure } => {
                self.js.record_failure(failure);
                if confirming {
                    self.reload.clear_pending();
                }
                self.fail_reload("reload", error);
            }
        }
    }

    fn complete_catalog_preparation<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        root: PathBuf,
        result: script::load::CatalogPreparationResult,
    ) {
        let script::load::CatalogPreparationResult {
            diff,
            prepared,
            warmed,
            prepare_failed,
        } = result;
        if diff.is_noop() && prepared.is_empty() && warmed.is_empty() && prepare_failed.is_empty() {
            self.reload.catalog_report = Some(script::NOTHING_CHANGED_CATALOG.into());
            self.show(script::NOTHING_CHANGED_CATALOG);
            self.finish_reload(ReloadOutcome::NothingChanged);
            return;
        }
        for failure in &prepare_failed {
            self.js.record_failure(failure.clone());
        }
        if prepared.is_empty() {
            self.complete_catalog_validation(core, root, diff, warmed, prepare_failed, Vec::new());
            return;
        }
        let had_prepare_failure = !prepare_failed.is_empty();
        let kind = ValidationKind::Catalog {
            root,
            diff,
            warmed,
            prepare_failed,
        };
        match self.spawn_validation(prepared, kind) {
            Ok(()) if !had_prepare_failure => self.clear_notice(),
            Ok(()) => {}
            Err(error) => self.fail_reload("Refresh catalog", error),
        }
    }

    fn complete_catalog_confirmation<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        root: PathBuf,
        diff: script::CatalogDiff,
    ) {
        let confirmed = self.reload.pending.as_ref().is_some_and(|pending| {
            if pending.warning.epoch != self.reload.generation {
                return false;
            }
            let Some(catalog) = pending.kind.as_catalog() else {
                return false;
            };
            catalog.root == root
                && catalog.added == diff.added
                && catalog.removed == diff.removed
                && catalog.set_fingerprint == catalog_set_fingerprint(&diff)
        });
        if confirmed {
            let outcome = self.commit_catalog_pending(core, diff);
            self.finish_reload(outcome);
        } else {
            self.reload.clear_pending();
            self.begin_refresh_catalog(core, &root);
        }
    }

    fn spawn_validation(
        &mut self,
        prepared: Vec<script::PreparedCard>,
        kind: ValidationKind,
    ) -> Result<(), String> {
        if self.reload.validation.is_some() {
            return Err("script validation already running".into());
        }
        let epoch = self.reload.generation;
        let receiver = script::LoadIsolate::validate_prepared_batch(prepared)
            .map_err(|error| error.to_string())?;
        self.reload.validation = Some(ValidationJob {
            epoch,
            receiver,
            kind,
        });
        Ok(())
    }

    /// Fold a finished ValidationSeat batch, accepting only this reload's
    /// current generation.
    pub(super) fn poll_reload_validation<Io>(&mut self, core: &mut OperatorSession<Io>) {
        use std::sync::mpsc::TryRecvError;
        let batch = match self.reload.validation.as_ref() {
            None => return,
            Some(job) => match job.receiver.try_recv() {
                Ok(batch) => Some(batch),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => None,
            },
        };
        let Some(job) = self.reload.validation.take() else {
            return;
        };
        if job.epoch != self.reload.generation {
            return;
        }
        let Some(batch) = batch else {
            let error = "script validation worker exited without a result".to_string();
            self.show(error.clone());
            self.finish_reload(ReloadOutcome::failed(error));
            return;
        };
        match job.kind {
            ValidationKind::Manual(manual) => self.complete_manual_validation(core, manual, batch),
            ValidationKind::Catalog {
                root,
                diff,
                warmed,
                prepare_failed,
            } => self.complete_catalog_validation(core, root, diff, warmed, prepare_failed, batch),
        }
    }

    fn complete_manual_validation<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        manual: ManualValidation,
        mut batch: ValidationBatch,
    ) {
        let ManualValidation {
            source,
            lookup,
            identity_key,
            fingerprint,
            running_before,
            paused_before,
        } = manual;
        let Some((prepared, result)) = batch.pop().filter(|_| batch.is_empty()) else {
            self.fail_reload("reload", "validation returned no candidate".into());
            return;
        };
        if let Err(error) = result {
            self.js.record_prepared_failure(&prepared, &error);
            self.fail_reload("reload", error);
            return;
        }
        let (running, paused) = slots_with_identity(core, &identity_key);
        let paused_during_prep = paused
            .iter()
            .filter(|name| running_before.iter().any(|prior| prior == *name))
            .cloned()
            .collect();
        let affected: Vec<String> = running_before
            .iter()
            .chain(paused_before.iter())
            .cloned()
            .collect();
        let warning = ReloadWarning {
            identity_key,
            source,
            lookup,
            fingerprint,
            epoch: self.reload.generation,
            running,
            paused,
            paused_during_prep,
            affected_generations: generations_for(core, &affected),
        };
        if !warning.running.is_empty()
            || !warning.paused.is_empty()
            || !warning.paused_during_prep.is_empty()
        {
            self.show(reload_warning_text(&warning));
            self.reload.install(PendingReload {
                warning,
                prepared: vec![prepared],
                warmed: Vec::new(),
                kind: PendingReloadKind::Manual,
            });
            self.finish_reload(ReloadOutcome::NeedsConfirm);
        } else {
            let outcome = self.apply_prepared_reload(core, vec![prepared], warning);
            self.finish_reload(outcome);
        }
    }

    fn complete_catalog_validation<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        root: PathBuf,
        diff: script::CatalogDiff,
        warmed: Vec<script::PreparedCard>,
        mut prepare_failed: Vec<script::LoadFailure>,
        batch: ValidationBatch,
    ) {
        let mut prepared = Vec::new();
        for (candidate, result) in batch {
            match result {
                Ok(()) => prepared.push(candidate),
                Err(error) => {
                    self.js.record_prepared_failure(&candidate, &error);
                    if let Some(failure) = self
                        .js
                        .load_failure(&candidate.card.identity_key())
                        .cloned()
                    {
                        prepare_failed.push(failure);
                    }
                    self.show(format!("catalog {}: {error}", candidate.card.name));
                }
            }
        }
        let mut running = Vec::new();
        let mut paused = Vec::new();
        for candidate in &prepared {
            let (r, p) = slots_with_identity(core, &candidate.card.identity_key());
            running.extend(r);
            paused.extend(p);
        }
        let set_fingerprint = catalog_set_fingerprint(&diff);
        let affected: Vec<String> = running.iter().chain(paused.iter()).cloned().collect();
        let warning = ReloadWarning {
            identity_key: prepared
                .first()
                .map(|candidate| candidate.card.identity_key())
                .unwrap_or_default(),
            source: script::ScriptSource::Catalog,
            lookup: diff.changed.first().cloned().unwrap_or_default(),
            fingerprint: set_fingerprint.clone(),
            epoch: self.reload.generation,
            running,
            paused,
            paused_during_prep: Vec::new(),
            affected_generations: generations_for(core, &affected),
        };
        if !warning.running.is_empty() || !warning.paused.is_empty() {
            self.show(reload_warning_text(&warning));
            self.reload.install(PendingReload {
                warning,
                prepared,
                warmed,
                kind: PendingReloadKind::catalog(CatalogPending {
                    root,
                    added: diff.added,
                    removed: diff.removed,
                    failed: prepare_failed,
                    set_fingerprint,
                }),
            });
            self.finish_reload(ReloadOutcome::NeedsConfirm);
        } else {
            let outcome =
                self.apply_catalog_prepared(core, diff, prepared, warmed, prepare_failed, warning);
            self.finish_reload(outcome);
        }
    }

    /// Reload `target` (see [`Self::reload_target`]). Bounded preparation and
    /// throwaway V8 validation run off the caller, polled by [`Scripts::poll`].
    /// A second Reload submits a bounded confirmation scan for a shown warning.
    pub fn begin_reload<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        target: Option<(CardSource, String)>,
    ) {
        if self.reload.preparation.is_some() || self.reload.validation.is_some() {
            return;
        }
        self.reload.outcome = None;
        let Some((source, lookup)) = target else {
            self.fail_reload("reload", "no script to reload".into());
            return;
        };
        let source = source.to_script();
        let confirming = self.manual_pending_binds(source, &lookup);
        let confirmation_fingerprint = if confirming {
            self.reload
                .pending
                .as_ref()
                .map(|pending| pending.warning.fingerprint.clone())
        } else {
            None
        };
        let Some(card) = self.js.get(source, &lookup) else {
            self.fail_reload("reload", format!("no script ({source:?}, {lookup})"));
            return;
        };
        let identity_key = card.identity_key();
        let (running_before, paused_before) = slots_with_identity(core, &identity_key);
        let snapshot =
            match self
                .js
                .card_preparation_snapshot(source, &lookup, confirmation_fingerprint)
            {
                Ok(snapshot) => Arc::new(snapshot),
                Err(error) => return self.fail_reload("reload", error),
            };
        let manual = ManualValidation {
            source,
            lookup,
            identity_key,
            fingerprint: String::new(),
            running_before,
            paused_before,
        };
        self.submit_preparation(PreparationRequest::Manual {
            snapshot,
            manual,
            confirming,
        });
        if !confirming {
            self.clear_notice();
        }
    }

    /// Discard the prepared candidate without touching any execution.
    pub fn cancel_reload(&mut self) {
        self.reload.invalidate();
        self.reload.outcome = None;
        self.clear_notice();
    }

    fn manual_pending_binds(&self, source: script::ScriptSource, lookup: &str) -> bool {
        let Some(pending) = self.reload.pending.as_ref() else {
            return false;
        };
        if !pending.kind.is_manual() {
            return false;
        }
        let warning = &pending.warning;
        warning.epoch == self.reload.generation
            && warning.source == source
            && lookups_match(source, &warning.lookup, lookup)
    }

    fn commit_manual_pending<Io>(&mut self, core: &mut OperatorSession<Io>) -> ReloadOutcome {
        let Some(mut pending) = self.reload.pending.take() else {
            return ReloadOutcome::failed("no pending reload");
        };
        if pending.warning.epoch != self.reload.generation {
            self.reload.clear_pending();
            return ReloadOutcome::failed("reload cancelled");
        }
        if let Some(paused_during) = newly_paused_during(core, &pending.warning) {
            pending.warning.paused_during_prep = paused_during;
            let (running_now, paused_now) =
                slots_with_identity(core, &pending.warning.identity_key);
            pending.warning.running = running_now;
            pending.warning.paused = paused_now;
            self.show(reload_warning_text(&pending.warning));
            self.reload.install(pending);
            return ReloadOutcome::NeedsConfirm;
        }
        self.apply_prepared_reload(core, pending.prepared, pending.warning)
    }

    fn apply_prepared_reload<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        prepared: Vec<script::PreparedCard>,
        warning: ReloadWarning,
    ) -> ReloadOutcome {
        for item in &prepared {
            if let Err(e) = self.js.commit_prepared(item.clone()) {
                self.show(format!("reload: {e}"));
                self.reload.install(PendingReload {
                    warning,
                    prepared,
                    warmed: Vec::new(),
                    kind: PendingReloadKind::Manual,
                });
                return ReloadOutcome::failed(e);
            }
        }
        let (restarted, stopped_paused, failed, errors) =
            self.replace_prepared_slots(core, &prepared, &warning);
        self.reload.clear_pending();
        if failed > 0 {
            self.show(format_reload_failures(restarted, stopped_paused, &errors));
        } else {
            self.clear_notice();
        }
        ReloadOutcome::applied(restarted, stopped_paused, failed)
    }

    /// Restart one replaced slot on the prepared card. The caller has just
    /// stopped it, so the slot is usually still reaping (`Stopping`): the
    /// slot queues this Start behind the reap. `Ok` means the Start was
    /// accepted; its setup settles in [`Scripts::poll`].
    fn start_prepared<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        prepared: &script::PreparedCard,
    ) -> Result<(), String> {
        if core.play().is_none() {
            return Err("no play".into());
        }
        self.cancel_queued(profile);
        let card = &prepared.card;
        let bag = self.merged_profile_bag(
            core,
            profile,
            CardSource::from_script(card.source),
            &card.name,
            &card.path,
            super::SchemaRef::new(&card.settings_schema),
        );
        let bag = (!bag.is_empty()).then_some(bag);
        #[cfg(any(test, feature = "test-support"))]
        if self.fail_reload_start_for.as_deref() == Some(profile) {
            return Err("injected start failure".into());
        }
        let start = ScriptStart::load(
            card.js.clone(),
            super::LoadShape::from_script(card.shape),
            bag,
            prepared.siblings.clone(),
        );
        if let Err(e) = core.start_script(profile, start, Some(card.identity_key())) {
            return self.js.record_start_result(card, Err(e));
        }
        self.starts.insert(
            profile.to_string(),
            PendingStart {
                card: PendingCard::Loaded(Box::new(card.clone())),
                kind: StartKind::Reload,
                tallied: false,
            },
        );
        Ok(())
    }

    /// Replace every warned run still eligible: same identity, the warned
    /// generation, no logout pending, still assigned this card. Running
    /// runs restart; paused ones stop.
    fn replace_prepared_slots<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        prepared: &[script::PreparedCard],
        warning: &ReloadWarning,
    ) -> (usize, usize, usize, Vec<String>) {
        let mut restarted = 0usize;
        let mut stopped_paused = 0usize;
        let mut failed = 0usize;
        let mut errors = Vec::new();
        let prepared_by_key: Vec<(String, &script::PreparedCard)> = prepared
            .iter()
            .map(|p| (p.card.identity_key(), p))
            .collect();
        let mut targets: Vec<String> = warning
            .running
            .iter()
            .chain(warning.paused.iter())
            .cloned()
            .collect();
        targets.sort();
        targets.dedup();
        for slot_name in targets {
            if self.reload.generation != warning.epoch {
                errors.push("reload cancelled".into());
                failed += 1;
                break;
            }
            let Some(play) = core.play() else {
                errors.push(format!("{slot_name}: no play"));
                failed += 1;
                continue;
            };
            let Some(live_key) = play.script_source_identity(&slot_name) else {
                continue;
            };
            let Some(prepared_card) = prepared_by_key
                .iter()
                .find(|(k, _)| k == &live_key)
                .map(|(_, p)| *p)
            else {
                continue;
            };
            let Some(warned_gen) = warning
                .affected_generations
                .iter()
                .find(|(n, _)| n == &slot_name)
                .map(|(_, g)| *g)
            else {
                continue;
            };
            if play.script_runtime_generation(&slot_name) != Some(warned_gen) {
                continue;
            }
            if logout_pending(core, &slot_name) {
                continue;
            }
            let assignment_matches = Self::assignment(core, &slot_name)
                .is_some_and(|a| a.key() == prepared_card.card.identity_key());
            if !assignment_matches {
                continue;
            }
            match play.script_state(&slot_name) {
                script::RunState::Running | script::RunState::Starting => {
                    if !play.script_stop_if_identity_generation(&slot_name, &live_key, warned_gen) {
                        continue;
                    }
                    match self.start_prepared(core, &slot_name, prepared_card) {
                        Ok(()) => restarted += 1,
                        Err(e) => {
                            failed += 1;
                            errors.push(format!("{slot_name}: {e}"));
                        }
                    }
                }
                script::RunState::Paused
                    if play
                        .script_stop_if_identity_generation(&slot_name, &live_key, warned_gen) =>
                {
                    stopped_paused += 1;
                }
                _ => {}
            }
        }
        (restarted, stopped_paused, failed, errors)
    }

    /// Queue a bounded catalogue refresh. New cards are prepared internally,
    /// so callers receive no names to warm on the UI thread.
    pub fn begin_refresh_catalog<Io>(
        &mut self,
        _core: &mut OperatorSession<Io>,
        root: &Path,
    ) -> Vec<String> {
        if self.reload.preparation.is_some() || self.reload.validation.is_some() {
            return Vec::new();
        }
        self.reload.outcome = None;
        if self.catalog_pending_binds(root) {
            let snapshot = match self.js.catalog_preparation_snapshot(root) {
                Ok(snapshot) => Arc::new(snapshot),
                Err(error) => {
                    self.fail_reload("Refresh catalog", error.to_string());
                    return Vec::new();
                }
            };
            self.submit_preparation(PreparationRequest::CatalogConfirmation {
                snapshot,
                root: root.to_path_buf(),
            });
            return Vec::new();
        }
        if self
            .reload
            .pending
            .as_ref()
            .is_some_and(|pending| pending.kind.is_catalog())
        {
            self.reload.clear_pending();
        }
        let snapshot = match self.js.catalog_preparation_snapshot(root) {
            Ok(snapshot) => Arc::new(snapshot),
            Err(error) => {
                self.fail_reload("Refresh catalog", error.to_string());
                return Vec::new();
            }
        };
        self.submit_preparation(PreparationRequest::Catalog {
            snapshot,
            root: root.to_path_buf(),
        });
        self.clear_notice();
        Vec::new()
    }

    fn catalog_pending_binds(&self, root: &Path) -> bool {
        let Some(pending) = self.reload.pending.as_ref() else {
            return false;
        };
        if pending.warning.epoch != self.reload.generation {
            return false;
        }
        pending
            .kind
            .as_catalog()
            .is_some_and(|catalog| catalog.root == root)
    }

    fn commit_catalog_pending<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        diff: script::CatalogDiff,
    ) -> ReloadOutcome {
        let Some(mut pending) = self.reload.pending.take() else {
            return ReloadOutcome::failed("no pending catalog refresh");
        };
        if !pending.kind.is_catalog() {
            self.reload.pending = Some(pending);
            return ReloadOutcome::failed("pending reload is not catalog");
        }
        if pending.warning.epoch != self.reload.generation {
            self.reload.clear_pending();
            return ReloadOutcome::failed("reload cancelled");
        }
        if let Some(paused_during) = newly_paused_during(core, &pending.warning) {
            pending.warning.paused_during_prep = paused_during;
            let mut running = Vec::new();
            let mut paused = Vec::new();
            for prepared in &pending.prepared {
                let (r, p) = slots_with_identity(core, &prepared.card.identity_key());
                running.extend(r);
                paused.extend(p);
            }
            pending.warning.running = running;
            pending.warning.paused = paused;
            self.show(reload_warning_text(&pending.warning));
            self.reload.install(pending);
            return ReloadOutcome::NeedsConfirm;
        }
        let warmed = pending.take_warmed();
        let failed = pending
            .kind
            .into_catalog()
            .map(|catalog| catalog.failed)
            .unwrap_or_default();
        self.apply_catalog_prepared(
            core,
            diff,
            pending.prepared,
            warmed,
            failed,
            pending.warning,
        )
    }

    fn apply_catalog_prepared<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        mut diff: script::CatalogDiff,
        prepared: Vec<script::PreparedCard>,
        warmed: Vec<script::PreparedCard>,
        prepare_failed: Vec<script::LoadFailure>,
        warning: ReloadWarning,
    ) -> ReloadOutcome {
        let removed = diff.removed.clone();
        let mut changed_ok = 0usize;
        for item in &prepared {
            if let Err(e) = self.js.commit_prepared(item.clone()) {
                self.show(format!("catalog {}: {e}", item.card.name));
                continue;
            }
            changed_ok += 1;
        }
        diff.changed.clear();
        let mut report = self.js.apply_catalog_diff(diff);
        for item in &warmed {
            if let Err(error) = self.js.commit_prepared(item.clone()) {
                self.show(format!("catalog {}: {error}", item.card.name));
            }
        }
        report.changed = changed_ok;
        report.failed.extend(prepare_failed);
        for name in &removed {
            self.mark_removed_catalog_assignments(core, name);
        }
        let (restarted, stopped_paused, failed, errors) =
            self.replace_prepared_slots(core, &prepared, &warning);
        let mut summary = report.summary();
        if failed > 0 {
            summary = format!("{summary}, start failed {}: {}", failed, errors.join("; "));
        }
        let named = self.js.named_failure_output();
        if !named.is_empty() {
            summary = format!("{summary}\n{named}");
        }
        self.show(format!("Refresh catalog: {summary}"));
        self.reload.catalog_report = Some(summary);
        self.reload.clear_pending();
        ReloadOutcome::applied(restarted, stopped_paused, failed)
    }

    pub(super) fn mark_removed_catalog_assignments<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        card_name: &str,
    ) {
        let key =
            script::card_identity_key(script::ScriptSource::Catalog, Path::new(""), card_name);
        let names: Vec<String> = core
            .vault()
            .map(|v| {
                v.profiles()
                    .filter(|p| {
                        p.settings
                            .script_assignment
                            .as_ref()
                            .is_some_and(|a| a.key() == key)
                    })
                    .map(|p| p.username.clone())
                    .collect()
            })
            .unwrap_or_default();
        for name in names {
            self.mark_assignment_unavailable(core, &name, format!("catalog removed: {card_name}"));
        }
    }
}

fn newly_paused_during<Io>(
    core: &OperatorSession<Io>,
    warning: &ReloadWarning,
) -> Option<Vec<String>> {
    let found: Vec<String> = warning
        .running
        .iter()
        .filter(|name| {
            core.play()
                .is_some_and(|p| p.script_state(name) == script::RunState::Paused)
                && !warning.paused_during_prep.iter().any(|n| n == *name)
        })
        .cloned()
        .collect();
    (!found.is_empty()).then_some(found)
}

fn logout_pending<Io>(core: &OperatorSession<Io>, name: &str) -> bool {
    core.fleet().latched(name)
        || core
            .play()
            .and_then(|p| p.arm(name))
            .is_some_and(|arm| arm.wants_logout())
}

/// Slots whose current run is card `key`: (running or starting, paused).
fn slots_with_identity<Io>(core: &OperatorSession<Io>, key: &str) -> (Vec<String>, Vec<String>) {
    let mut running = Vec::new();
    let mut paused = Vec::new();
    let Some(play) = core.play() else {
        return (running, paused);
    };
    for name in super::slot_names(core) {
        if play.script_source_identity(&name).as_deref() != Some(key) {
            continue;
        }
        match play.script_state(&name) {
            script::RunState::Running | script::RunState::Starting => running.push(name),
            script::RunState::Paused => paused.push(name),
            _ => {}
        }
    }
    (running, paused)
}

fn generations_for<Io>(core: &OperatorSession<Io>, names: &[String]) -> Vec<(String, u64)> {
    let Some(play) = core.play() else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|n| play.script_runtime_generation(n).map(|g| (n.clone(), g)))
        .collect()
}

fn lookups_match(source: script::ScriptSource, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    source == script::ScriptSource::File
        && (script::paths_match(a, Path::new(b)) || script::paths_match(b, Path::new(a)))
}

fn catalog_set_fingerprint(diff: &script::CatalogDiff) -> String {
    let fps: Vec<(String, String)> = diff
        .added
        .iter()
        .chain(&diff.changed)
        .filter_map(|name| {
            diff.current_fingerprint(name)
                .map(|fingerprint| (name.clone(), fingerprint.to_string()))
        })
        .collect();
    catalog_set_fingerprint_from_diff(&diff.added, &diff.changed, &diff.removed, &fps)
}

fn catalog_set_fingerprint_from_diff(
    added: &[String],
    changed: &[String],
    removed: &[String],
    fps: &[(String, String)],
) -> String {
    let mut fps = fps.to_vec();
    fps.sort();
    let mut parts = vec![
        format!("a:{}", added.join(",")),
        format!("c:{}", changed.join(",")),
        format!("r:{}", removed.join(",")),
    ];
    for (name, fp) in fps {
        parts.push(format!("{name}={fp}"));
    }
    parts.join("|")
}

fn format_reload_failures(restarted: usize, stopped_paused: usize, errors: &[String]) -> String {
    format!(
        "reload: restarted {restarted}, stopped paused {stopped_paused}, failed {}: {}",
        errors.len(),
        errors.join("; ")
    )
}

/// The confirmation text for a warning: exactly which runs are replaced.
fn reload_warning_text(w: &ReloadWarning) -> String {
    // The affected runs first: a one-row front end may cut the card path.
    let mut parts = vec!["reload will replace running bots".to_string()];
    if !w.running.is_empty() {
        parts.push(format!("running: {}", w.running.join(", ")));
    }
    if !w.paused.is_empty() {
        parts.push(format!(
            "paused bots will be stopped: {}",
            w.paused.join(", ")
        ));
    }
    if !w.paused_during_prep.is_empty() {
        parts.push(format!(
            "paused during prepare: {}",
            w.paused_during_prep.join(", ")
        ));
    }
    parts.push(format!("card {}", w.identity_key));
    parts.join(" — ")
}

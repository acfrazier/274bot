use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::identity::Scope;
use script::preparation::{PreparationError, PreparationInput, PreparationKind, PreparedTask};

use script::manifest::ScriptManifest;
use serde_json::Value;

pub(super) const IMPORTS_KEY: &str = "luau_imports";

#[cfg(test)]
thread_local! {
    static SOURCE_READ_COUNT: std::cell::RefCell<Arc<std::sync::atomic::AtomicUsize>> =
        std::cell::RefCell::new(Arc::new(std::sync::atomic::AtomicUsize::new(0)));
}

use super::{
    LoadStage, PendingCard, PendingStart, ScriptStart, ScriptStartError, ScriptStartErrorKind,
    Scripts, StartKind,
};
use crate::session::OperatorSession;

#[derive(Debug, Clone)]
pub(super) struct LuauCard {
    path: String,
    name: String,
    manifest: ScriptManifest,
}

impl LuauCard {
    pub(super) fn path(&self) -> &Path {
        Path::new(&self.path)
    }
    pub(super) fn identity(&self) -> &str {
        &self.path
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn display_name(&self) -> &str {
        self.manifest.name()
    }

    pub(super) fn description(&self) -> &str {
        self.manifest.description()
    }

    pub(super) fn category(&self) -> &str {
        self.manifest.category()
    }

    pub(super) fn tags(&self) -> &[String] {
        self.manifest.tags()
    }

    pub(super) fn manifest(&self) -> &ScriptManifest {
        &self.manifest
    }

    pub(super) fn identity_id(&self) -> String {
        self.path.clone()
    }

    pub(super) fn identity_matches(&self, identity: &str) -> bool {
        self.path == identity
    }

    pub(super) fn identity_key(&self) -> String {
        script::card_identity_key(script::ScriptSource::File, self.path(), &self.name)
    }

    pub(super) fn assignment(&self) -> vault::ScriptAssignment {
        script::card_assignment(script::ScriptSource::File, self.path(), self.display_name())
    }
}

#[derive(Debug, Clone)]
pub(super) struct LuauLoadFailure {
    identity_key: String,
    name: String,
    path: PathBuf,
    stage: LoadStage,
    diagnostic: String,
    line: Option<u32>,
    column: Option<u32>,
}

impl LuauLoadFailure {
    fn read(path: &Path, error: impl std::fmt::Display) -> Self {
        Self::new(
            path,
            LoadStage::Read,
            format!("luau-load: {error}"),
            None,
            None,
        )
    }

    fn manifest(path: &Path, error: script_luau::ManifestError) -> Self {
        Self::new(
            path,
            LoadStage::ParseTranspile,
            format!("luau-manifest: {error}"),
            u32::try_from(error.line()).ok(),
            u32::try_from(error.column()).ok(),
        )
    }

    fn too_large(path: &Path, bytes: u64) -> Self {
        Self::new(
            path,
            LoadStage::ParseTranspile,
            format!(
                "luau-manifest-file-too-large: file is {bytes} bytes, limit is {} bytes",
                script_luau::manifest::SOURCE_LIMIT_BYTES
            ),
            None,
            None,
        )
    }

    fn new(
        path: &Path,
        stage: LoadStage,
        diagnostic: String,
        line: Option<u32>,
        column: Option<u32>,
    ) -> Self {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Luau script")
            .to_owned();
        Self {
            identity_key: script::card_identity_key(script::ScriptSource::File, path, &name),
            name,
            path: path.to_path_buf(),
            stage,
            diagnostic,
            line,
            column,
        }
    }

    pub(super) fn identity_key(&self) -> &str {
        &self.identity_key
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn stage(&self) -> LoadStage {
        self.stage
    }

    pub(super) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    pub(super) fn position(&self) -> (Option<u32>, Option<u32>) {
        (self.line, self.column)
    }

    pub(super) fn named_line(&self) -> String {
        format!("{}: {}", self.name, self.diagnostic)
    }

    pub(super) fn copyable_text(&self) -> String {
        let location = match (self.line, self.column) {
            (Some(line), Some(column)) => {
                format!("{}:{line}:{column}", self.path.display())
            }
            _ => self.path.display().to_string(),
        };
        format!("{}\n{}\n{}", self.name, location, self.diagnostic)
    }
}

pub(super) struct LuauFile {
    path: PathBuf,
    source: String,
    manifest: script_luau::LuauManifest,
}

impl LuauFile {
    pub(super) fn read(path: &Path) -> Result<Self, LuauLoadFailure> {
        #[cfg(test)]
        let counter = SOURCE_READ_COUNT.with(|count| Arc::clone(&count.borrow()));
        Self::read_inner(
            path,
            #[cfg(test)]
            &counter,
        )
    }

    fn read_inner(
        path: &Path,
        #[cfg(test)] counter: &std::sync::atomic::AtomicUsize,
    ) -> Result<Self, LuauLoadFailure> {
        let canonical = path
            .canonicalize()
            .map_err(|error| LuauLoadFailure::read(path, error))?;
        let metadata =
            fs::metadata(&canonical).map_err(|error| LuauLoadFailure::read(&canonical, error))?;
        if metadata.len() > script_luau::manifest::SOURCE_LIMIT_BYTES as u64 {
            return Err(LuauLoadFailure::too_large(&canonical, metadata.len()));
        }
        #[cfg(test)]
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let bytes =
            fs::read(&canonical).map_err(|error| LuauLoadFailure::read(&canonical, error))?;
        if bytes.len() > script_luau::manifest::SOURCE_LIMIT_BYTES {
            return Err(LuauLoadFailure::too_large(&canonical, bytes.len() as u64));
        }
        let source =
            String::from_utf8(bytes).map_err(|error| LuauLoadFailure::read(&canonical, error))?;
        let manifest = script_luau::read_manifest(&source)
            .map_err(|error| LuauLoadFailure::manifest(&canonical, error))?;
        Ok(Self {
            path: canonical,
            source,
            manifest,
        })
    }

    pub(super) fn card(&self) -> LuauCard {
        let name = self
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("Luau script")
            .to_owned();
        LuauCard {
            path: self.path.to_string_lossy().into_owned(),
            name,
            manifest: self.manifest.manifest().clone(),
        }
    }

    pub(super) fn erased_source(&self) -> String {
        self.manifest.erased_source(&self.source)
    }
}

struct PreparedLuauFile {
    card: LuauCard,
    source: String,
}

#[cfg(test)]
struct PreparationStall {
    entered: std::sync::mpsc::Sender<std::thread::ThreadId>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

#[cfg(test)]
impl PreparationStall {
    fn wait(&self) {
        let _ = self.entered.send(std::thread::current().id());
        let _ = self.release.lock().unwrap().recv();
    }
}

/// The exact task owns its result. Dropping it on disable or cancellation
/// prevents an old completion from being consumed by a later enable.
struct FilePreparation {
    path: Arc<PathBuf>,
    for_start: bool,
    task: Option<PreparedTask<Result<PreparedLuauFile, LuauLoadFailure>>>,
    #[cfg(test)]
    counter: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    stall: Option<Arc<PreparationStall>>,
}

impl FilePreparation {
    fn submit(&mut self) -> Result<(), LuauLoadFailure> {
        if self.task.is_some() {
            return Ok(());
        }
        let input_bytes = self
            .path
            .as_os_str()
            .len()
            .checked_add(128)
            .ok_or_else(|| LuauLoadFailure::read(&self.path, "preparation input is too large"))?;
        // The source is capped before parsing. Reserve for the parser's
        // nested table vectors (including spare capacity), projected settings
        // and erased source, without statting the file on the frame thread.
        let limit = script_luau::manifest::SOURCE_LIMIT_BYTES;
        let input = PreparationInput::new(1, input_bytes, 256 * limit, 64 * limit)
            .map_err(|error| LuauLoadFailure::read(&self.path, error))?;
        let path = Arc::clone(&self.path);
        let for_start = self.for_start;
        #[cfg(test)]
        let counter = Arc::clone(&self.counter);
        #[cfg(test)]
        let stall = self.stall.clone();
        match script::preparation::submit(PreparationKind::Card, Scope::Process, input, move |_| {
            let file = LuauFile::read_inner(
                &path,
                #[cfg(test)]
                &counter,
            )?;
            let prepared = PreparedLuauFile {
                card: file.card(),
                source: if for_start {
                    file.erased_source()
                } else {
                    String::new()
                },
            };
            #[cfg(test)]
            if let Some(stall) = stall {
                stall.wait();
            }
            Ok(prepared)
        }) {
            Ok(task) => self.task = Some(task),
            Err(PreparationError::Refused(api::work::SubmissionRefusal::Full)) => {}
            Err(error) => return Err(LuauLoadFailure::read(&self.path, error)),
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<Result<PreparedLuauFile, LuauLoadFailure>> {
        if self.task.is_none() {
            return self.submit().err().map(Err);
        }
        if !self.task.as_ref().is_some_and(PreparedTask::is_finished) {
            return None;
        }
        Some(
            self.task
                .take()
                .expect("finished Luau task")
                .join()
                .map_err(|error| LuauLoadFailure::read(&self.path, error))
                .and_then(std::convert::identity),
        )
    }
}

struct StartDemand {
    profile: String,
    kind: StartKind,
    tallied: bool,
    assignment: Option<script::ScriptSel>,
    had_arm: bool,
    latched: bool,
}

pub(super) struct StartPreparation {
    lookup: String,
    file: FilePreparation,
    ready: Option<PreparedLuauFile>,
    published: bool,
    demands: VecDeque<StartDemand>,
}
#[derive(Default)]
pub(super) struct LuauLibrary {
    enabled: bool,
    cards: Vec<LuauCard>,
    last_refusal: Option<LuauLoadFailure>,
    restore: VecDeque<FilePreparation>,
    #[cfg(test)]
    prepare_stall: Option<Arc<PreparationStall>>,
}

impl LuauLibrary {
    #[cfg(test)]
    pub(super) fn reset_source_read_count_for_test() {
        SOURCE_READ_COUNT.with(|count| {
            *count.borrow_mut() = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        });
    }

    #[cfg(test)]
    pub(super) fn source_read_count_for_test() -> usize {
        SOURCE_READ_COUNT.with(|count| count.borrow().load(std::sync::atomic::Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(super) fn stall_preparation_for_test(
        &mut self,
    ) -> (
        std::sync::mpsc::Receiver<std::thread::ThreadId>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered, arrival) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        self.prepare_stall = Some(Arc::new(PreparationStall {
            entered,
            release: std::sync::Mutex::new(gate),
        }));
        (arrival, release)
    }

    #[cfg(test)]
    pub(super) fn clear_prepare_stall_for_test(&mut self) {
        self.prepare_stall = None;
    }

    fn preparation(&self, path: &Path, for_start: bool) -> FilePreparation {
        FilePreparation {
            path: Arc::new(path.to_path_buf()),
            for_start,
            task: None,
            #[cfg(test)]
            counter: SOURCE_READ_COUNT.with(|count| Arc::clone(&count.borrow())),
            #[cfg(test)]
            stall: self.prepare_stall.clone(),
        }
    }

    pub(super) fn restore_pending(&self) -> bool {
        !self.restore.is_empty()
    }

    fn poll_restore(&mut self) {
        let Some(result) = self.restore.front_mut().and_then(FilePreparation::poll) else {
            return;
        };
        self.restore.pop_front();
        match result {
            Ok(prepared) => self.register(prepared.card),
            Err(error) => self.last_refusal = Some(error),
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn visible_cards(&self) -> &[LuauCard] {
        if self.enabled {
            &self.cards
        } else {
            &[]
        }
    }

    pub(super) fn card(&self, identity: &str) -> Option<&LuauCard> {
        self.cards
            .iter()
            .find(|card| card.identity_matches(identity))
    }

    pub(super) fn last_refusal(&self) -> Option<&LuauLoadFailure> {
        self.last_refusal.as_ref()
    }
    #[cfg(feature = "js")]
    pub(super) fn clear_refusal(&mut self) {
        self.last_refusal = None;
    }

    pub(super) fn set_disabled(&mut self) {
        self.enabled = false;
        self.restore.clear();
    }

    pub(super) fn restore(&mut self, references: Option<&Value>) {
        self.enabled = true;
        self.cards.clear();
        self.last_refusal = None;
        self.restore.clear();
        let mut seen = HashSet::new();
        let Some(paths) = references.and_then(Value::as_array) else {
            return;
        };
        for path in paths.iter().filter_map(Value::as_str) {
            if seen.insert(path) {
                self.restore
                    .push_back(self.preparation(Path::new(path), false));
            }
        }
    }

    pub(super) fn read_file(&mut self, path: &Path) -> Result<LuauFile, LuauLoadFailure> {
        match LuauFile::read(path) {
            Ok(file) => {
                self.last_refusal = None;
                Ok(file)
            }
            Err(error) => {
                self.last_refusal = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(super) fn register(&mut self, card: LuauCard) {
        if let Some(existing) = self
            .cards
            .iter_mut()
            .find(|existing| existing.path == card.path)
        {
            *existing = card;
        } else {
            self.cards.push(card);
        }
        self.last_refusal = None;
    }
}

impl Scripts {
    pub(super) fn luau_start_preparation_pending(&self) -> bool {
        self.luau_start_work
            .iter()
            .any(|work| !work.demands.is_empty())
    }

    pub(super) fn has_luau_start_preparation(&self, profile: &str) -> bool {
        self.luau_start_work
            .iter()
            .any(|work| work.demands.iter().any(|demand| demand.profile == profile))
    }

    pub(super) fn tallied_luau_start_preparation_pending(&self) -> bool {
        self.luau_start_work
            .iter()
            .any(|work| work.demands.iter().any(|demand| demand.tallied))
    }

    pub(super) fn cancel_luau_start_preparation(
        &mut self,
        profile: &str,
    ) -> Option<(StartKind, bool)> {
        let index = self
            .luau_start_work
            .iter()
            .position(|work| work.demands.iter().any(|demand| demand.profile == profile))?;
        let work = &mut self.luau_start_work[index];
        let place = work
            .demands
            .iter()
            .position(|demand| demand.profile == profile)?;
        let demand = work.demands.remove(place)?;
        if work.demands.is_empty() {
            self.luau_start_work.swap_remove(index);
        }
        Some((demand.kind, demand.tallied))
    }

    pub(super) fn cancel_all_luau_start_preparations(&mut self) -> Vec<(String, StartKind, bool)> {
        std::mem::take(&mut self.luau_start_work)
            .into_iter()
            .flat_map(|work| work.demands)
            .map(|demand| (demand.profile, demand.kind, demand.tallied))
            .collect()
    }

    pub(super) fn refuse_disabled_luau_starts(&mut self) {
        for (profile, kind, tallied) in self.cancel_all_luau_start_preparations() {
            self.report_start_failure(
                &profile,
                kind,
                tallied,
                ScriptStartError::luau_disabled().to_string(),
            );
        }
    }

    pub(super) fn retire_luau_start_cache(&mut self) {
        if self.admit.is_empty() {
            self.luau_start_work.retain(|work| !work.demands.is_empty());
        }
    }

    pub(super) fn start_luau_file<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        lookup: &str,
        kind: StartKind,
        tallied: bool,
    ) -> Result<Option<crate::operations::OperationId>, ScriptStartError> {
        if !super::luau_runtime_enabled(core) {
            return Err(ScriptStartError::luau_disabled());
        }
        if !self.luau.enabled() {
            self.restore_luau_from_preferences(core);
        }
        if self.luau.card(lookup).is_none() && !self.luau.restore_pending() {
            if let Some(failure) = self
                .luau
                .last_refusal()
                .filter(|failure| failure.path() == Path::new(lookup))
            {
                return Err(script::StartLoadError::RuntimeLoad(failure.named_line()).into());
            }
            return Err(ScriptStartError::refused(format!(
                "Luau script is not imported: {lookup}"
            )));
        }
        if let Some(reason) = core
            .play()
            .and_then(host_play::Play::script_start_wait_reason)
        {
            return Err(ScriptStartError::waiting(format!(
                "{profile}: blocked, {reason}"
            )));
        }
        for work in &self.luau_start_work {
            if let Some(demand) = work.demands.iter().find(|demand| demand.profile == profile) {
                return if work.lookup == lookup && demand.kind == kind {
                    Ok(None)
                } else {
                    Err(ScriptStartError::waiting(
                        "script Start preparation already pending",
                    ))
                };
            }
        }
        const MAX_PENDING_STARTS: usize = 64;
        if self
            .luau_start_work
            .iter()
            .map(|work| work.demands.len())
            .sum::<usize>()
            >= MAX_PENDING_STARTS
        {
            return Err(ScriptStartError::refused(format!(
                "script Start preparation limit reached ({MAX_PENDING_STARTS})"
            )));
        }
        let (had_arm, latched) = super::arm_flags(core, profile);
        let demand = StartDemand {
            profile: profile.to_owned(),
            kind,
            tallied,
            assignment: Self::assignment(core, profile)
                .and_then(|assignment| super::sel_from_assignment(&assignment)),
            had_arm,
            latched,
        };
        if let Some(work) = self
            .luau_start_work
            .iter_mut()
            .find(|work| work.lookup == lookup)
        {
            work.demands.push_back(demand);
        } else {
            let mut file = self.luau.preparation(Path::new(lookup), true);
            file.submit()
                .map_err(|failure| script::StartLoadError::RuntimeLoad(failure.named_line()))?;
            self.luau_start_work.push(StartPreparation {
                lookup: lookup.to_owned(),
                file,
                ready: None,
                published: false,
                demands: VecDeque::from([demand]),
            });
        }
        Ok(None)
    }

    pub(super) fn poll_luau_work<Io>(&mut self, core: &mut OperatorSession<Io>) {
        if !super::luau_runtime_enabled(core) {
            self.luau.set_disabled();
            self.refuse_disabled_luau_starts();
            return;
        }
        self.luau.poll_restore();
        let Some(index) = self
            .luau_start_work
            .iter()
            .position(|work| !work.demands.is_empty())
        else {
            return;
        };
        let mut work = self.luau_start_work.swap_remove(index);
        if work.ready.is_none() {
            match work.file.poll() {
                None => {
                    self.luau_start_work.push(work);
                    return;
                }
                Some(Ok(prepared)) => work.ready = Some(prepared),
                Some(Err(error)) => {
                    self.luau.last_refusal = Some(error.clone());
                    for demand in work.demands {
                        self.report_start_failure(
                            &demand.profile,
                            demand.kind,
                            demand.tallied,
                            error.named_line(),
                        );
                    }
                    return;
                }
            }
        }
        if self.luau.card(&work.lookup).is_none() && self.luau.restore_pending() {
            self.luau_start_work.push(work);
            return;
        }
        let demand = work.demands.pop_front().expect("pending Luau Start");
        let assigned = Self::assignment(core, &demand.profile)
            .and_then(|assignment| super::sel_from_assignment(&assignment));
        let (has_arm, latched_now) = super::arm_flags(core, &demand.profile);
        let skip = if !super::wall_member(core, &demand.profile)
            || core.removal_pending(&demand.profile)
        {
            Some("removed")
        } else if assigned != demand.assignment {
            Some("reassigned")
        } else if demand.had_arm && !has_arm {
            Some("disconnected")
        } else if !demand.latched && latched_now {
            Some("logged out")
        } else if super::script_active(core, &demand.profile) {
            Some("script already active")
        } else {
            None
        };
        if let Some(reason) = skip {
            if demand.tallied {
                self.skip_unstarted_tally(&demand.profile, reason);
            } else {
                self.show(format!("script: {reason}"));
                self.start_command_skips
                    .push((demand.profile, reason.into()));
            }
        } else {
            let prepared = work.ready.as_ref().expect("prepared Luau file");
            match self.start_prepared_luau(core, &demand, &work.lookup, prepared) {
                Ok(()) => {
                    if !work.published {
                        self.luau.register(prepared.card.clone());
                        work.published = true;
                    }
                }
                Err(error) if error.kind() == ScriptStartErrorKind::Waiting => {
                    work.demands.push_front(demand);
                }
                Err(error) => self.report_start_failure(
                    &demand.profile,
                    demand.kind,
                    demand.tallied,
                    error.to_string(),
                ),
            }
        }
        if !work.demands.is_empty() || !self.admit.is_empty() {
            self.luau_start_work.push(work);
        }
    }

    fn start_prepared_luau<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        demand: &StartDemand,
        lookup: &str,
        prepared: &PreparedLuauFile,
    ) -> Result<(), ScriptStartError> {
        if !super::luau_runtime_enabled(core) {
            return Err(ScriptStartError::luau_disabled());
        }
        if self.luau.card(lookup).is_none() {
            return Err(ScriptStartError::refused(format!(
                "Luau script is not imported: {lookup}"
            )));
        }
        if !prepared.card.identity_matches(lookup) {
            return Err(ScriptStartError::refused(format!(
                "Luau file identity changed: {lookup}"
            )));
        }
        if self.parameter_edit_blocks_start(&demand.profile) {
            return Err(ScriptStartError::waiting(
                "Script settings are uncommitted or saving.",
            ));
        }
        if let Some(reason) = core
            .play()
            .and_then(host_play::Play::script_start_wait_reason)
        {
            return Err(ScriptStartError::waiting(format!(
                "{}: blocked, {reason}",
                demand.profile
            )));
        }
        let card = &prepared.card;
        let identity = card.identity_id();
        let schema = super::SchemaRef::new(card.manifest().settings_schema());
        let overrides = self
            .profile_overrides(core, &demand.profile, &card.identity_key(), &identity)
            .map_err(ScriptStartError::refused)?;
        super::parameter_edit::validate_manifest_profile_settings_for_schema(
            core,
            schema.defs(),
            &overrides,
        )?;
        let bag = self.run_bag(core, &demand.profile, schema.defs(), &overrides);
        core.start_script(
            &demand.profile,
            ScriptStart::luau(prepared.source.clone(), bag),
            Some(identity),
        )?;
        #[cfg(feature = "js")]
        {
            self.last_refused_load = None;
        }
        self.starts.insert(
            demand.profile.clone(),
            PendingStart {
                card: PendingCard::Luau(Box::new(card.clone())),
                kind: demand.kind,
                tallied: demand.tallied,
            },
        );
        Ok(())
    }
}

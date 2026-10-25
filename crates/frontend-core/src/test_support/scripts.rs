//! Script-card fixtures for front-end tests (see the parent module).
//!
//! Panel and TUI tests build assignments, card snapshots, schemas, stores,
//! native statuses and admission holders here, so they never name the crates
//! below the operator seam. Everything here panics on failure: a fixture is
//! a test's given, not its fallible production path.

use std::path::Path;
#[cfg(feature = "js")]
use std::path::PathBuf;

use serde_json::{Map, Value};

use super::VaultFixture;
use crate::scripts::{CompiledCardId, SchemaRef, Scripts, StatusView};
#[cfg(feature = "js")]
use crate::scripts::{CardSource, LoadShape, ScriptChoice};
use crate::session::OperatorSession;
use crate::views::SlotDetail;

/// A script assignment a test saves with
/// [`Scripts::persist_assignment`]: the catalog card, file, compiled card or
/// missing file a profile last ran. Panel and TUI tests never name the vault
/// crate; the assignment they compare and save is this newtype.
#[derive(Debug, PartialEq, Eq)]
pub struct Assignment {
    assignment: vault::ScriptAssignment,
}

impl Assignment {
    /// A catalog assignment for `name`.
    pub fn catalog(name: &str) -> Self {
        Self {
            assignment: vault::ScriptAssignment {
                source_kind: "catalog".into(),
                identity: name.into(),
                display_name: name.into(),
                unavailable: None,
            },
        }
    }

    /// A file assignment with an explicit stored identity.
    pub fn file(identity: &str, display_name: &str) -> Self {
        Self {
            assignment: vault::ScriptAssignment {
                source_kind: "file".into(),
                identity: identity.into(),
                display_name: display_name.into(),
                unavailable: None,
            },
        }
    }

    /// A compiled-card assignment.
    pub fn compiled(id: CompiledCardId) -> Self {
        Self {
            assignment: script::compiled_assignment(id.to_script()),
        }
    }

    /// A missing-file assignment that stays present with `reason`.
    pub fn missing_file(path: &Path, display_name: &str, reason: &str) -> Self {
        Self {
            assignment: script::missing_file_assignment(path, display_name, reason),
        }
    }

    /// Wrap a seam assignment (frontend-core-internal plumbing).
    pub(crate) fn from_assignment(assignment: vault::ScriptAssignment) -> Self {
        Self { assignment }
    }

    /// The seam assignment (frontend-core-internal plumbing).
    pub(crate) fn into_assignment(self) -> vault::ScriptAssignment {
        self.assignment
    }

    /// The stored kind (`catalog`, `file`, `builtin` or `compiled`).
    pub fn source_kind(&self) -> &str {
        &self.assignment.source_kind
    }

    /// The canonical identity: register name, stored file path or id.
    pub fn identity(&self) -> &str {
        &self.assignment.identity
    }

    /// The picker label (never used to look a card up).
    pub fn display_name(&self) -> &str {
        &self.assignment.display_name
    }

    /// Why the stored identity cannot currently resolve, if any.
    pub fn unavailable(&self) -> Option<&str> {
        self.assignment.unavailable.as_deref()
    }

    /// The persistence key.
    pub fn key(&self) -> String {
        self.assignment.key()
    }
}

#[cfg(feature = "js")]
/// A loaded card's owned facts: picker name, origin file, source, loader
/// shape, texts and identities. Unlike the borrowed `LoadedCard` view, the
/// snapshot stays usable across polls, Starts and reloads.
#[derive(Debug)]
pub struct LoadedCardSnapshot {
    name: String,
    description: String,
    schema: Vec<script::SettingDef>,
    path: PathBuf,
    source: CardSource,
    shape: LoadShape,
    origin: String,
    js: String,
    sha256: String,
    identity_id: String,
    identity_key: String,
    assignment: vault::ScriptAssignment,
}

#[cfg(feature = "js")]
impl LoadedCardSnapshot {
    fn snapshot(card: &script_js::JsCard) -> Self {
        Self {
            name: card.name.clone(),
            description: card.description.clone(),
            schema: card.settings_schema.clone(),
            path: card.path.clone(),
            source: CardSource::from_script(card.source),
            shape: LoadShape::from_script(card.shape),
            origin: card.origin.clone(),
            js: card.js.clone(),
            sha256: card.sha256.clone(),
            identity_id: card.identity_id(),
            identity_key: card.identity_key(),
            assignment: card.assignment(),
        }
    }

    /// The picker name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Its settings schema as the panes read it.
    pub fn schema_ref(&self) -> SchemaRef<'_> {
        SchemaRef::from_script(&self.schema)
    }

    /// The origin file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the card came from.
    pub fn source(&self) -> CardSource {
        self.source
    }

    /// Its loader shape.
    pub fn shape(&self) -> LoadShape {
        self.shape
    }

    /// The raw origin text.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The prepared JavaScript a Start runs.
    pub fn js(&self) -> &str {
        &self.js
    }

    /// The origin text's SHA-256.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The picker identity (a file card's path, a catalog card's name).
    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }

    /// The persistence identity key.
    pub fn identity_key(&self) -> &str {
        &self.identity_key
    }

    /// This card as a picker choice.
    pub fn choice(&self) -> ScriptChoice {
        ScriptChoice::loaded(self.source, &self.identity_id)
    }

    /// This card as a savable assignment.
    pub fn assignment(&self) -> Assignment {
        Assignment::from_assignment(self.assignment.clone())
    }
}

impl Scripts {
    #[cfg(feature = "js")]
    /// Load `path` and snapshot the owned card facts (tests).
    pub fn load_card(&mut self, path: &Path) -> Result<LoadedCardSnapshot, String> {
        self.js_mut()
            .load(path)
            .map(|card| LoadedCardSnapshot::snapshot(&card))
    }

    #[cfg(feature = "js")]
    /// Snapshot the library card `lookup` (tests).
    pub fn card_snapshot(&self, source: CardSource, lookup: &str) -> Option<LoadedCardSnapshot> {
        self.js()
            .get(source.to_script(), lookup)
            .map(LoadedCardSnapshot::snapshot)
    }

    #[cfg(feature = "js")]
    /// Ensure the catalog or file card is prepared (tests).
    pub fn ensure_card(&mut self, source: CardSource, name: &str) -> Result<(), String> {
        self.js_mut().ensure_js(source.to_script(), name)
    }

    #[cfg(feature = "js")]
    /// Whether the raw source changed since the card loaded (tests).
    pub fn raw_source_changed(&self, source: CardSource, name: &str) -> Result<bool, String> {
        self.js().raw_source_changed(source.to_script(), name)
    }

    #[cfg(feature = "js")]
    /// Replace the legacy global per-card store with one at `path` (tests).
    pub fn set_legacy_store(&mut self, path: PathBuf) {
        self.set_legacy(script::ScriptSettingsStore::at(path));
    }

    #[cfg(feature = "js")]
    /// Set one legacy override without saving, as an unsaved legacy edit
    /// (tests).
    pub fn set_legacy_str(&mut self, source: CardSource, name: &str, id: &str, value: &str) {
        self.legacy_mut()
            .set_str(source.to_script(), name, id, value);
    }

    #[cfg(feature = "js")]
    /// The persisted legacy overrides for one card (tests).
    pub fn legacy_overrides(&self, source: CardSource, name: &str) -> Map<String, Value> {
        self.legacy().overrides(source.to_script(), name)
    }

    #[cfg(feature = "js")]
    /// A recorded load failure's content fingerprint (tests).
    pub fn load_failure_fingerprint(&self, identity_key: &str) -> Option<String> {
        self.js()
            .load_failure(identity_key)
            .map(|failure| failure.fingerprint.clone())
    }

    #[cfg(feature = "js")]
    /// A recorded load failure's API family (`unversioned`, `v1`, `v2`).
    pub fn load_failure_api_family(&self, identity_key: &str) -> Option<String> {
        self.js()
            .load_failure(identity_key)
            .and_then(|failure| failure.api_family.map(|family| family.as_str().to_owned()))
    }

    #[cfg(feature = "js")]
    /// Register the rs2b0t catalog at `root` (tests). Synchronous: the
    /// transpile tests must not settle async work.
    pub fn register_catalog(&mut self, root: &Path, path_file: &Path) -> Result<usize, String> {
        self.js_mut().register_rs2b0t(root, path_file)
    }

    #[cfg(feature = "js")]
    /// Whether the transpile cache already holds `origin` (tests).
    pub fn is_transpile_cached(&self, origin: &str) -> bool {
        self.js().cache().is_cached(origin.as_bytes())
    }

    /// Save `profile`'s last successful assignment from a test fixture
    /// (tests). The vault conversion stays inside frontend-core.
    pub fn persist_test_assignment<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        assignment: Assignment,
    ) -> bool {
        self.persist_assignment(core, profile, assignment.into_assignment())
    }

    /// `profile`'s last successful assignment as a test fixture (tests).
    pub fn test_assignment<Io>(core: &OperatorSession<Io>, profile: &str) -> Option<Assignment> {
        Self::profile_assignment(core, profile).map(Assignment::from_assignment)
    }
}

#[cfg(feature = "js")]
/// A standalone card library for tests that never open a session: same-stem
/// files in different directories stay distinct and content changes are
/// observable per file.
pub struct TestLibrary {
    library: script_js::JsLibrary,
}

#[cfg(feature = "js")]
impl TestLibrary {
    /// An empty library stored at `store` with its transpile cache below
    /// `cache_root` (tests).
    pub fn with_cache(store: PathBuf, cache_root: PathBuf) -> Self {
        Self {
            library: script_js::JsLibrary::with_cache(store, cache_root),
        }
    }

    /// Load `path` and snapshot the owned card facts (tests).
    pub fn load(&mut self, path: &Path) -> Result<LoadedCardSnapshot, String> {
        self.library
            .load(path)
            .map(|card| LoadedCardSnapshot::snapshot(&card))
    }

    /// Register the rs2b0t catalog at `root` (tests). Synchronous: the
    /// transpile tests must not settle async work.
    pub fn register_catalog(&mut self, root: &Path, path_file: &Path) -> Result<usize, String> {
        self.library.register_rs2b0t(root, path_file)
    }

    /// Ensure the catalog or file card is prepared (tests).
    pub fn ensure(&mut self, source: CardSource, name: &str) -> Result<(), String> {
        self.library.ensure_js(source.to_script(), name)
    }

    /// Whether the raw source changed since the card loaded (tests).
    pub fn raw_source_changed(&self, source: CardSource, name: &str) -> Result<bool, String> {
        self.library.raw_source_changed(source.to_script(), name)
    }
}

/// One setting field a test declares, then a whole card schema: the
/// `Vec<SettingDef>` a params pane reads through [`SchemaRef`], plus the
/// built-in card schemas. Tests never name the script crate.
pub struct TestSchema {
    defs: Vec<script::SettingDef>,
}

fn setting_def(
    id: &str,
    ty: &str,
    default: Option<&str>,
    label: Option<&str>,
    options: &[&str],
) -> script::SettingDef {
    script::SettingDef {
        id: id.to_string(),
        ty: ty.to_string(),
        default: default.map(str::to_owned),
        label: label.map(str::to_owned),
        min: None,
        max: None,
        step: None,
        options: options.iter().map(|option| (*option).to_string()).collect(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: None,
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }
}

impl TestSchema {
    /// An empty schema (tests).
    pub fn empty() -> Self {
        Self { defs: Vec::new() }
    }

    /// A one-setting schema (mirrors the params-pane test helper).
    pub fn setting(
        id: &str,
        ty: &str,
        default: Option<&str>,
        label: Option<&str>,
        options: &[&str],
    ) -> Self {
        Self {
            defs: vec![setting_def(id, ty, default, label, options)],
        }
    }

    /// Append one more setting (tests).
    pub fn add_setting(
        &mut self,
        id: &str,
        ty: &str,
        default: Option<&str>,
        label: Option<&str>,
        options: &[&str],
    ) -> &mut Self {
        self.defs.push(setting_def(id, ty, default, label, options));
        self
    }

    /// The last setting, which the `set_*` modifiers below edit.
    fn last_mut(&mut self) -> &mut script::SettingDef {
        self.defs.last_mut().expect("a schema setting to modify")
    }

    /// The last setting's group (tests).
    pub fn set_group(&mut self, group: &str) -> &mut Self {
        self.last_mut().group = Some(group.to_string());
        self
    }

    /// The last setting's label (tests).
    pub fn set_label(&mut self, label: &str) -> &mut Self {
        self.last_mut().label = Some(label.to_string());
        self
    }

    /// The last setting's default (tests).
    pub fn set_default(&mut self, default: &str) -> &mut Self {
        self.last_mut().default = Some(default.to_string());
        self
    }

    /// The last setting's minimum bound (tests).
    pub fn set_min(&mut self, min: &str) -> &mut Self {
        self.last_mut().min = Some(min.to_string());
        self
    }

    /// The last setting's maximum bound (tests).
    pub fn set_max(&mut self, max: &str) -> &mut Self {
        self.last_mut().max = Some(max.to_string());
        self
    }

    /// The last setting's step (tests).
    pub fn set_step(&mut self, step: &str) -> &mut Self {
        self.last_mut().step = Some(step.to_string());
        self
    }

    /// The last setting's option labels, parallel to its options (tests).
    pub fn set_option_labels(&mut self, labels: &[&str]) -> &mut Self {
        self.last_mut().option_labels = labels.iter().map(|label| (*label).to_string()).collect();
        self
    }

    /// The last setting's visibility condition (tests).
    pub fn set_show_if(&mut self, show_if: &str) -> &mut Self {
        self.last_mut().show_if = Some(show_if.to_string());
        self
    }

    /// The last setting's dynamic option source (tests).
    pub fn set_options_from(&mut self, options_from: &str) -> &mut Self {
        self.last_mut().options_from = Some(options_from.to_string());
        self
    }

    /// The last setting's CSV toggle (tests).
    pub fn set_csv_toggle(&mut self, csv_toggle: &str) -> &mut Self {
        self.last_mut().csv_toggle = Some(csv_toggle.to_string());
        self
    }

    /// The last setting's help text (tests).
    pub fn set_help(&mut self, help: &str) -> &mut Self {
        self.last_mut().help = Some(help.to_string());
        self
    }

    /// The last setting's selected-item choice descriptor (tests).
    pub fn set_item_options(
        &mut self,
        prefix: &[&str],
        candidates: &[(&str, Option<&str>)],
        sort_keys_by_label: bool,
    ) -> &mut Self {
        self.last_mut().item_option_spec = Some(script::ItemOptionSpec {
            prefix: prefix.iter().map(|entry| (*entry).to_string()).collect(),
            candidates: candidates
                .iter()
                .map(|(key, label)| script::ItemOptionCandidate {
                    key: (*key).to_string(),
                    label: label.map(str::to_owned),
                })
                .collect(),
            sort_keys_by_label,
        });
        self
    }

    /// This schema as the panes read it (tests).
    pub fn schema_ref(&self) -> SchemaRef<'_> {
        SchemaRef::from_script(&self.defs)
    }

    /// Schema defaults over `overrides`, with the optional scenario inject
    /// (tests). Pure merge: no store involved.
    pub fn merged_bag(
        &self,
        overrides: &Map<String, Value>,
        inject: Option<&Map<String, Value>>,
    ) -> Map<String, Value> {
        script::merge_bag(&self.defs, overrides, inject)
    }

    /// The built-in gatherer card's schema (tests).
    pub fn gatherer() -> Self {
        Self {
            defs: script::gatherer::settings::schema().to_vec(),
        }
    }

    /// The built-in Quester card's schema (tests).
    pub fn quester_card() -> Self {
        Self {
            defs: (script::quester::card::CARD.schema)().to_vec(),
        }
    }
}

/// A quest the built-in Quester card refuses, if it names one (tests).
pub fn unavailable_quest(id: &str) -> Option<&'static str> {
    script::quester::card::unavailable_quest(id)
}

#[cfg(feature = "js")]
/// A legacy per-card settings store: the schema defaults merged over the
/// persisted overrides a params pane edits. Tests never name the script
/// crate.
pub struct TestStore {
    store: script::ScriptSettingsStore,
}

#[cfg(feature = "js")]
impl TestStore {
    /// The store at `path` (tests).
    pub fn at(path: PathBuf) -> Self {
        Self {
            store: script::ScriptSettingsStore::at(path),
        }
    }

    /// Schema defaults over this store's persisted overrides (tests).
    pub fn merged_bag(
        &self,
        source: CardSource,
        name: &str,
        schema: &TestSchema,
        inject: Option<&Map<String, Value>>,
    ) -> Map<String, Value> {
        self.store
            .merged_bag(source.to_script(), name, &schema.defs, inject)
    }

    /// This store's persisted overrides for one card (tests).
    pub fn overrides(&self, source: CardSource, name: &str) -> Map<String, Value> {
        self.store.overrides(source.to_script(), name)
    }

    /// Write one override without saving (tests).
    pub fn set_value(&mut self, source: CardSource, name: &str, id: &str, value: Value) {
        self.store.set_value(source.to_script(), name, id, value);
    }

    /// Persist the store (tests).
    pub fn save(&mut self) -> Result<(), String> {
        self.store.save()
    }
}

/// A native card's published status: run identity, compiled card, phase,
/// settings revisions, status fields and terminal failure. Tests build it
/// here and read it through [`StatusView`] or attach it to a slot detail;
/// they never name the script crate.
pub struct NativeStatus {
    status: script::native::ScriptStatus,
}

impl NativeStatus {
    fn base(card: CompiledCardId) -> script::native::ScriptStatus {
        script::native::ScriptStatus {
            run: api::selected::RunKey {
                slot: 1,
                run: 2,
                session: 3,
            },
            card: card.to_script(),
            phase: script::native::NativePhase::Working,
            active_settings: 1,
            pending_settings: None,
            fields: std::sync::Arc::from([]),
            failure: None,
        }
    }

    /// A working status for `card` (tests).
    pub fn working(card: CompiledCardId) -> Self {
        Self {
            status: Self::base(card),
        }
    }

    /// A blocked status for `card` with a terminal failure (tests).
    pub fn blocked(card: CompiledCardId, code: &str, message: &str) -> Self {
        let mut status = Self::base(card);
        status.phase = script::native::NativePhase::Blocked;
        status.failure = Some(script::native::ScriptFailure {
            code: std::sync::Arc::from(code),
            message: std::sync::Arc::from(message),
        });
        Self { status }
    }

    /// The run identity (tests).
    pub fn with_run(mut self, slot: u64, run: u64, session: u64) -> Self {
        self.status.run = api::selected::RunKey { slot, run, session };
        self
    }

    /// The settings revision the run applies (tests).
    pub fn with_active_settings(mut self, active: u64) -> Self {
        self.status.active_settings = active;
        self
    }

    /// A settings revision the run has not applied yet (tests).
    pub fn with_pending_settings(mut self, pending: u64) -> Self {
        self.status.pending_settings = Some(pending);
        self
    }

    /// Append one text field (tests).
    pub fn with_text_field(mut self, key: &'static str, label: &'static str, value: &str) -> Self {
        let mut fields = self.status.fields.to_vec();
        fields.push(script::native::StatusField {
            key,
            label,
            value: script::native::StatusValue::Text(std::sync::Arc::from(value)),
        });
        self.status.fields = std::sync::Arc::from(fields);
        self
    }

    /// Keep only the fields whose key is in `keys` (tests).
    pub fn keep_fields(&mut self, keys: &[&str]) {
        let kept: Vec<script::native::StatusField> = self
            .status
            .fields
            .iter()
            .filter(|field| keys.contains(&field.key))
            .cloned()
            .collect();
        self.status.fields = std::sync::Arc::from(kept);
    }

    /// This status as the panes read it (tests).
    pub fn view(&self) -> StatusView<'_> {
        StatusView::from_script(&self.status)
    }

    /// Attach this status to a slot detail (tests).
    pub fn attach_to(&self, detail: SlotDetail) -> SlotDetail {
        detail.with_native_status(Some(std::sync::Arc::new(self.status.clone())))
    }
}

/// WorkAdmission fully occupied: every later preparation refuses until one
/// of these tasks settles. The holder keeps the tasks alive; dropping it
/// releases them.
pub struct SaturatedAdmission {
    tasks: Vec<script::preparation::PreparedTask<()>>,
}

impl SaturatedAdmission {
    /// The number of occupying tasks.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether any task occupies admission.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

/// Occupy every WorkAdmission slot (tests).
pub fn saturate_work_admission() -> SaturatedAdmission {
    let input =
        script::preparation::PreparationInput::new(1, 0, 0, 0).expect("admission test input");
    let mut tasks = Vec::new();
    loop {
        match script::preparation::submit(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            |_| (),
        ) {
            Ok(task) => {
                tasks.push(task);
                assert!(
                    tasks.len() <= 64,
                    "WorkAdmission accepted more jobs than its fixed bound"
                );
            }
            Err(script::preparation::PreparationError::Refused(
                api::work::SubmissionRefusal::Full,
            )) => break,
            Err(error) => panic!("could not occupy WorkAdmission: {error}"),
        }
    }
    for task in &tasks {
        wait_for_preparation(task);
    }
    SaturatedAdmission { tasks }
}

/// Pump one preparation task until it settles (frontend-core-internal; the
/// occupying helper above is the tests' entry point).
pub(crate) fn wait_for_preparation(task: &script::preparation::PreparedTask<()>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !task.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "WorkAdmission preparation did not settle"
        );
        std::thread::yield_now();
    }
}

/// Reset the quester folder registry to its default (disabled) source, so
/// folder-source tests never leak into each other (tests).
pub fn reset_quester_registry() {
    script::quester::registry::set_source(script::quester::registry::FolderSource::default());
}

/// The stored file identity of `path` (tests).
pub fn file_identity(path: &Path) -> String {
    script::file_identity(path)
}

#[cfg(feature = "js")]
/// The in-tree live example `file_name`, if one is built in (tests).
pub fn live_example_path(file_name: &str) -> Option<PathBuf> {
    script_js::live_example_path(file_name)
}

#[cfg(feature = "js")]
/// The SHA-256 of raw origin bytes, as load failures record it (tests).
pub fn raw_content_fingerprint(card_path: &Path, origin: &str) -> String {
    script_js::raw_content_fingerprint(card_path, origin)
}

#[cfg(feature = "js")]
/// The banner a reload shows when nothing changed (tests).
pub fn nothing_changed_reload_message() -> &'static str {
    script::NOTHING_CHANGED_RELOAD
}

/// A compiled card's persistence identity key (tests).
pub fn compiled_identity_key(id: CompiledCardId) -> String {
    script::compiled_identity_key(id.to_script())
}

#[cfg(feature = "js")]
/// Save the library card's assignment on `profile`: false when the card is
/// missing or the write refuses (tests).
pub fn assign_loaded<Io>(
    scripts: &mut Scripts,
    core: &mut OperatorSession<Io>,
    profile: &str,
    source: CardSource,
    lookup: &str,
) -> bool {
    let Some(card) = scripts.card_snapshot(source, lookup) else {
        return false;
    };
    scripts.persist_assignment(core, profile, card.assignment().into_assignment())
}

/// Save a compiled card's assignment on `profile`: false when the write
/// refuses (tests).
pub fn assign_compiled<Io>(
    scripts: &mut Scripts,
    core: &mut OperatorSession<Io>,
    profile: &str,
    id: CompiledCardId,
) -> bool {
    scripts.persist_assignment(core, profile, Assignment::compiled(id).into_assignment())
}

#[cfg(feature = "js")]
/// Start `source` JS on `name`'s slot with a frontend loader shape (tests):
/// the direct Play load the live walk tests drive.
pub fn play_start_load<Io>(
    core: &OperatorSession<Io>,
    name: &str,
    source: String,
    shape: LoadShape,
    bag: Option<Map<String, Value>>,
    siblings: Vec<(String, String)>,
) -> Result<(), String> {
    core.play_for_test()
        .ok_or_else(|| "Play disappeared".to_string())?
        .script_start_load(name, source, shape.to_script(), bag, siblings)
}

/// A native card's published status as owned frontend values: the
/// point-in-time snapshot the live tests hold across polls instead of the
/// native status value. Tests never name the script crate.
pub struct NativeSnapshot {
    card: CompiledCardId,
    active_settings: u64,
    pending_settings: Option<u64>,
    fields: Vec<(String, String)>,
}

impl NativeSnapshot {
    /// The current published status of `name`'s run, if any (tests).
    pub fn capture<Io>(core: &OperatorSession<Io>, name: &str) -> Option<Self> {
        let status = core.play()?.script_native_status(name)?;
        Some(Self {
            card: CompiledCardId::from_script(status.card),
            active_settings: status.active_settings,
            pending_settings: status.pending_settings,
            fields: status
                .fields
                .iter()
                .filter_map(|field| match &field.value {
                    script::native::StatusValue::Text(text) => {
                        Some((field.key.to_string(), text.to_string()))
                    }
                    _ => None,
                })
                .collect(),
        })
    }

    /// The compiled card that published it.
    pub fn card(&self) -> CompiledCardId {
        self.card
    }

    /// The settings revision the run applies.
    pub fn active_settings(&self) -> u64 {
        self.active_settings
    }

    /// A settings revision the run has not applied yet.
    pub fn pending_settings(&self) -> Option<u64> {
        self.pending_settings
    }

    /// One text status field by key.
    pub fn text_field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(name, _)| name.as_str() == key)
            .map(|(_, value)| value.as_str())
    }
}

/// `profile`'s saved per-card settings bag `key` from the live session
/// (tests). Reads the session vault inside frontend-core and returns owned
/// values, so panel tests never hold vault values.
pub fn session_script_bag<Io>(
    core: &OperatorSession<Io>,
    profile: &str,
    key: &str,
) -> Option<Map<String, Value>> {
    core.vault()?
        .get(profile)?
        .settings
        .script_settings
        .get(key)
        .cloned()
}

impl VaultFixture {
    /// `username`'s saved assignment, as the heading shows it (tests).
    pub fn assignment_of(&self, username: &str) -> Option<Assignment> {
        self.vault
            .get(username)
            .and_then(|profile| profile.settings.script_assignment.clone())
            .map(Assignment::from_assignment)
    }

    /// `username`'s saved per-card settings bag `key` (tests).
    pub fn profile_bag(&self, username: &str, key: &str) -> Option<Map<String, Value>> {
        self.vault
            .get(username)
            .and_then(|profile| profile.settings.script_settings.get(key).cloned())
    }
}

//! Script coordination shared by the panel and the TUI: the card library
//! and catalog, per-profile assignment and pending Browse selection,
//! per-profile parameter bags (legacy migration, typed edits, live push),
//! Start / Start all / Stop all with their settlement, the reload and
//! catalog-refresh transaction ([`reload`]) and bulk parameter sync
//! ([`sync`]).
//!
//! [`Scripts`] holds coordination state only; every operation takes the
//! [`OperatorSession`] that owns the vault, the play and the profile
//! writer. The host still owns isolate lifecycle and execution identity.
//! Operator-facing text is published as one [`Notice`] the front end shows
//! on its message line, in call order (the last one wins).

mod native;
pub use native::{NativeCommand, NativeDetail, NativeTarget, SchemaView};

mod reload;
mod sync;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};
use vault::ScriptAssignment;

use crate::operations::OperationId;
use crate::session::{ArmMirror, OperatorSession, ScriptStart, StartSettled};

pub use reload::{PendingReload, PendingReloadKind, ReloadOutcome, ReloadWarning};
pub use sync::{SyncReport, SyncScope};

/// A card's parameters bound to the run they were edited for.
#[derive(Debug, Clone)]
pub struct LiveSettings {
    /// Card identity and run generation the push is fenced to.
    pub identity: String,
    pub generation: u64,
    /// Native controls also bind the worker incarnation and session.
    pub run: Option<api::selected::RunKey>,
    /// The run's whole bag: the card's merged parameters and its profile's
    /// global settings.
    pub bag: Arc<Map<String, Value>>,
    /// Present only after the native preparer has validated the complete bag.
    pub prepared: Option<Arc<script::native::PreparedConfig>>,
}

impl PartialEq for LiveSettings {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.generation == other.generation
            && self.run == other.run
            && self.bag == other.bag
            && match (&self.prepared, &other.prepared) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

/// What a durable parameter write did to the run captured at edit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveDelivery {
    /// No run of that card was captured, or it has ended.
    NotRunning,
    Delivered,
    Applied,
    PendingBoundary,
    RestartRequired,
    Rejected(String),
    /// The run already had this bag.
    Unchanged,
    /// The captured run was replaced (identity or generation changed)
    /// before the write became durable; it was not posted to.
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsResult {
    Saved(LiveDelivery),
    /// This edit never settled on its own: a newer edit of the same card's
    /// parameters in the same commit replaced it, or the commit failed and
    /// a newer write of the profile reports the failure.
    Superseded,
    Failed(String),
}

/// One settled script-parameter write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWrite {
    pub op: OperationId,
    pub profile: String,
    pub result: SettingsResult,
}

/// Operator-facing message from the last script operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Show(String),
    Clear,
    /// Replace the banner with `with` only while it still shows `shown`:
    /// an asynchronous update (a Start settling) never erases a newer
    /// message the front end or another command put there.
    Retract {
        shown: String,
        with: Option<String>,
    },
}

impl Notice {
    /// Apply this notice to a front end's banner (the one both the panel
    /// and the TUI use).
    pub fn apply(self, banner: &mut Option<String>) {
        match self {
            Self::Show(text) => *banner = Some(text),
            Self::Clear => *banner = None,
            Self::Retract { shown, with } => {
                if banner.as_deref() == Some(shown.as_str()) {
                    *banner = with;
                }
            }
        }
    }
}

/// Which operator action a pending Start came from: it decides where a
/// setup failure is reported once it is observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartKind {
    Start,
    StartAll,
    Reload,
}

/// A Start whose worker has not settled. Assignment changes only on Ready.
#[derive(Debug, Clone)]
enum PendingCard {
    Compiled(script::CompiledId),
    Loaded(Box<script::JsCard>),
}

#[derive(Debug, Clone)]
struct PendingStart {
    card: PendingCard,
    kind: StartKind,
}

/// The last Start all tally. A member whose setup fails after the click
/// moves from started to failed, so the report lists it.
#[derive(Debug, Clone, Default)]
struct BulkStart {
    started: usize,
    skipped: usize,
    failures: Vec<String>,
}

/// One shared Browse card: a compiled registry card or a loaded JS card.
/// Loaded cards remain borrowed from the library, so a Browse render does not
/// clone source bytes or the whole card.
#[derive(Debug, Clone, Copy)]
pub enum BrowseCard<'a> {
    Compiled(&'static script::native::CompiledCard),
    Loaded(&'a script::JsCard),
}

impl<'a> BrowseCard<'a> {
    pub fn name(self) -> &'a str {
        match self {
            Self::Compiled(card) => card.name,
            Self::Loaded(card) => card.name.as_str(),
        }
    }

    pub fn description(self) -> &'a str {
        match self {
            Self::Compiled(card) => card.description,
            Self::Loaded(card) => card.description.as_str(),
        }
    }

    pub fn category(self) -> &'a str {
        match self {
            Self::Compiled(card) => card.category,
            Self::Loaded(card) => card.category.as_str(),
        }
    }

    pub fn tags(self) -> Option<&'a [String]> {
        match self {
            Self::Compiled(_) => None,
            Self::Loaded(card) => Some(&card.tags),
        }
    }

    pub fn kind(self) -> script::ScriptKind {
        match self {
            Self::Compiled(_) => script::ScriptKind::Compiled,
            Self::Loaded(card) => card.kind,
        }
    }

    pub fn source(self) -> script::ScriptSource {
        match self {
            Self::Compiled(_) => script::ScriptSource::Builtin,
            Self::Loaded(card) => card.source,
        }
    }

    pub fn unloadable(self) -> Option<&'a str> {
        match self {
            Self::Compiled(_) => None,
            Self::Loaded(card) => card.unloadable.as_deref(),
        }
    }

    pub fn loaded(self) -> Option<&'a script::JsCard> {
        match self {
            Self::Compiled(_) => None,
            Self::Loaded(card) => Some(card),
        }
    }

    pub fn selection(self) -> script::ScriptSel {
        match self {
            Self::Compiled(card) => script::ScriptSel::Compiled(card.id),
            Self::Loaded(card) => script::ScriptSel::Loaded(card.source, card.identity_id()),
        }
    }
}

pub struct Scripts {
    /// The card library: catalog, loaded files, transpile cache.
    pub js: script::JsLibrary,
    /// Legacy global per-card overrides (`script-settings.json`), claimed
    /// into a profile's bag on its first use of a card.
    pub legacy: script::ScriptSettingsStore,
    /// Scenario/live inject merged last on Start.
    pub inject: Option<Map<String, Value>>,
    /// Per-profile Browse selection, never treated as a successful Start.
    pending_browse: HashMap<String, script::ScriptSel>,
    starts: HashMap<String, PendingStart>,
    bulk_start: Option<BulkStart>,
    last_bulk_report: Option<String>,
    catalog_filled: bool,
    reload: reload::ReloadState,
    sync: sync::SyncState,
    notice: Option<Notice>,
    /// The text of the last notice shown (`None` once cleared): a settled
    /// Start only replaces the banner when it still shows the load-failure
    /// list it changes.
    shown: Option<String>,
    start_failures: Vec<(String, String)>,
    /// Test-only: fail the replacement Start for this profile after it
    /// passed eligibility and was stopped.
    #[cfg(any(test, feature = "test-support"))]
    pub fail_reload_start_for: Option<String>,
}

impl Scripts {
    pub fn new(js: script::JsLibrary, legacy: script::ScriptSettingsStore) -> Self {
        Self {
            js,
            legacy,
            inject: None,
            pending_browse: HashMap::new(),
            starts: HashMap::new(),
            bulk_start: None,
            last_bulk_report: None,
            catalog_filled: false,
            reload: reload::ReloadState::default(),
            sync: sync::SyncState::default(),
            notice: None,
            shown: None,
            start_failures: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            fail_reload_start_for: None,
        }
    }

    /// The message to show since the last take.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }

    fn show(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.shown = Some(text.clone());
        self.notice = Some(Notice::Show(text));
    }

    fn clear_notice(&mut self) {
        self.shown = None;
        self.notice = Some(Notice::Clear);
    }

    /// Show the library's load-failure list (or clear the banner when there
    /// is none): what an operator Start reports once it is accepted.
    pub fn show_load_failures(&mut self) {
        if self.js.load_failures().is_empty() {
            self.clear_notice();
        } else {
            self.show(self.js.named_failure_output());
        }
    }

    /// Operator Start setup failures (profile, `script: …` text) since the
    /// last take, for callers that attribute them further.
    pub fn take_start_failures(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.start_failures)
    }

    // ---- catalog -------------------------------------------------------

    /// Whether the `$RS2B0T` catalog has been registered (or its first-run
    /// prompt handled) this session.
    pub fn catalog_filled(&self) -> bool {
        self.catalog_filled
    }

    pub fn mark_catalog_filled(&mut self) {
        self.catalog_filled = true;
    }

    /// Register the catalog at `root` once per session. Without a root the
    /// catalog stays unfilled so a later Browse can still prompt.
    pub fn fill_catalog_once(&mut self, root: Option<&Path>) {
        if self.catalog_filled {
            return;
        }
        let Some(root) = root else {
            return;
        };
        self.catalog_filled = true;
        if let Err(error) = self
            .js
            .register_rs2b0t(root, &script::default_rs2b0t_path_file())
        {
            if host::debug_enabled() {
                eprintln!("[scripts] $RS2B0T registry: {error}");
            }
        }
    }

    /// The shared Browse card sequence: compiled registry cards first, then
    /// loaded JS/catalog cards in library order.
    pub fn browse_cards(&self) -> impl Iterator<Item = BrowseCard<'_>> {
        script::compiled_cards()
            .iter()
            .map(BrowseCard::Compiled)
            .chain(self.js.cards().iter().map(BrowseCard::Loaded))
    }

    // ---- assignment and Browse selection -------------------------------

    /// `profile`'s last successful assignment (as saved, staged edits
    /// included).
    pub fn assignment<Io>(core: &OperatorSession<Io>, profile: &str) -> Option<ScriptAssignment> {
        core.vault()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_assignment.clone())
    }

    /// The card a profile's script heading names: its pending Browse
    /// selection, else its last successful assignment.
    pub fn heading<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
    ) -> Option<script::ScriptSel> {
        if let Some(sel) = self.pending_browse.get(profile) {
            return Some(sel.clone());
        }
        Self::assignment(core, profile).and_then(|a| sel_from_assignment(&a))
    }

    /// Whether `profile`'s Browse selection differs from its assignment.
    pub fn heading_is_pending<Io>(&self, core: &OperatorSession<Io>, profile: &str) -> bool {
        let Some(pending) = self.pending_browse.get(profile) else {
            return false;
        };
        match Self::assignment(core, profile) {
            None => true,
            Some(asg) => sel_from_assignment(&asg).as_ref() != Some(pending),
        }
    }

    pub fn set_pending_browse(&mut self, profile: &str, sel: script::ScriptSel) {
        self.pending_browse.insert(profile.to_string(), sel);
    }

    pub fn pending_browse(&self, profile: &str) -> Option<&script::ScriptSel> {
        self.pending_browse.get(profile)
    }

    /// Stage an edit of `profile`'s settings and queue its durable write.
    fn upsert_profile_settings<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        mirror: ArmMirror,
        edit: impl FnOnce(&mut vault::ProfileSettings),
    ) -> Result<OperationId, String> {
        let result = core.vault().map(|vault| {
            if matches!(mirror, ArmMirror::NativeSettings { .. }) {
                core.profile_for_edit(profile).cloned()
            } else {
                vault.get(profile).cloned()
            }
        });
        let result = match result {
            None => Err("script: vault locked".to_string()),
            Some(None) => Err(format!("script: no profile {profile}")),
            Some(Some(mut row)) => {
                edit(&mut row.settings);
                core.save_profile(row, mirror, "script")
            }
        };
        if let Err(error) = &result {
            self.show(error.clone());
        }
        result
    }

    /// Save `profile`'s last successful assignment. Re-starting the same
    /// card writes nothing.
    pub fn persist_assignment<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        assignment: ScriptAssignment,
    ) -> bool {
        if Self::assignment(core, profile).as_ref() == Some(&assignment) {
            return true;
        }
        self.upsert_profile_settings(core, profile, ArmMirror::None, |settings| {
            settings.script_assignment = Some(assignment);
        })
        .is_ok()
    }

    pub fn mark_assignment_unavailable<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        let _ = self.upsert_profile_settings(core, profile, ArmMirror::None, |settings| {
            if let Some(asg) = settings.script_assignment.as_mut() {
                asg.unavailable = Some(reason);
            }
        });
    }

    // ---- per-profile parameters ----------------------------------------

    /// Copy the legacy global overrides for `card_name` into `profile`'s
    /// bag on its first use of the card.
    fn claim_legacy_for<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) {
        let already = core
            .vault()
            .and_then(|v| v.get(profile))
            .is_some_and(|p| p.settings.script_settings.contains_key(key));
        if already {
            return;
        }
        let legacy = self.legacy.overrides(
            script::parse_source_kind(key.split(':').next().unwrap_or("catalog"))
                .unwrap_or(script::ScriptSource::Catalog),
            card_name,
        );
        let _ = self.upsert_profile_settings(core, profile, ArmMirror::None, |settings| {
            script::claim_legacy_overrides(settings, key, card_name, &legacy);
        });
    }

    /// `profile`'s overrides bag for the card `key`.
    pub fn profile_overrides<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        self.claim_legacy_for(core, profile, key, card_name);
        core.vault()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_settings.get(key).cloned())
            .unwrap_or_default()
    }

    /// What a Start of this card on `profile` receives: schema defaults,
    /// the profile's overrides, then the inject.
    pub fn merged_profile_bag<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        schema: &[script::SettingDef],
    ) -> Map<String, Value> {
        let key = script::card_identity_key(source, path, name);
        let overrides = self.profile_overrides(core, profile, &key, name);
        self.run_bag(core, profile, schema, &overrides)
    }

    /// The bag a run on `profile` receives, on every path (Start, a
    /// parameter edit, Apply to all): the card's merged parameters, then
    /// the profile-global settings unless the inject names its own. The
    /// isolate replaces both bags on each post, so a card-only bag would
    /// wipe the account's clue duel partner from a running script.
    fn run_bag<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        schema: &[script::SettingDef],
        overrides: &Map<String, Value>,
    ) -> Map<String, Value> {
        let partner = core
            .settings_for_edit(profile)
            .map_or("", |settings| settings.clue_duel_partner.as_str());
        self.run_bag_with(schema, overrides, partner)
    }

    /// [`Self::run_bag`] with the profile's global partner given.
    fn run_bag_with(
        &self,
        schema: &[script::SettingDef],
        overrides: &Map<String, Value>,
        partner: &str,
    ) -> Map<String, Value> {
        let mut bag = script::merge_bag(schema, overrides, self.inject.as_ref());
        let injected = self
            .inject
            .as_ref()
            .is_some_and(|inject| inject.contains_key(script::CLUE_DUEL_PARTNER));
        let partner = partner.trim();
        if !injected && !partner.is_empty() {
            bag.insert(
                script::CLUE_DUEL_PARTNER.into(),
                Value::String(partner.into()),
            );
        }
        bag
    }

    /// The bag the running assignment of `profile` holds once `settings`
    /// are saved, for a profile save that changes a global setting: frozen
    /// scripts read the clue duel partner live, at each crossing. `None`
    /// when no run of the profile's assigned card is live.
    pub fn profile_save_live<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        settings: &vault::ProfileSettings,
    ) -> Option<LiveSettings> {
        let assignment = settings.script_assignment.as_ref()?;
        let key = assignment.key();
        let (identity, generation) = live_fence(core, profile, &key)?;
        let (schema, overrides) = match sel_from_assignment(assignment)? {
            script::ScriptSel::Loaded(source, lookup) => (
                self.js
                    .get(source, &lookup)
                    .map(|card| card.settings_schema.as_slice())
                    .unwrap_or_default(),
                settings
                    .script_settings
                    .get(&key)
                    .cloned()
                    .unwrap_or_default(),
            ),
            script::ScriptSel::Compiled(id) => {
                let card = script::compiled_card(id)?;
                let values = match settings.script_settings.get(&key) {
                    Some(entry) => {
                        let (version, values) = vault::CompiledSettingsRecord::view(entry).ok()?;
                        if version != card.schema_version {
                            return None;
                        }
                        values.clone()
                    }
                    None => Map::new(),
                };
                ((card.schema)(), values)
            }
        };
        Some(LiveSettings {
            identity,
            generation,
            run: core.play().and_then(|play| play.script_native_run(profile)),
            bag: Arc::new(self.run_bag_with(schema, &overrides, &settings.clue_duel_partner)),
            prepared: None,
        })
    }

    /// The legacy global bag for a card (no profile focused).
    pub fn legacy_bag(
        &self,
        source: script::ScriptSource,
        name: &str,
        schema: &[script::SettingDef],
    ) -> Map<String, Value> {
        self.legacy
            .merged_bag(source, name, schema, self.inject.as_ref())
    }

    /// Set one typed parameter on `profile`'s bag for a card. The write is
    /// queued at once; the matching run (same card identity, generation
    /// captured now) receives the merged bag only once it is durable, and
    /// the result arrives as a [`SettingsWrite`]. A prepared bulk sync from
    /// this profile is dropped: its snapshot no longer matches.
    #[allow(clippy::too_many_arguments)]
    pub fn set_profile_setting<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        id: &str,
        value: Value,
    ) -> Result<OperationId, String> {
        let key = script::card_identity_key(source, path, name);
        self.claim_legacy_for(core, profile, &key, name);
        let mut bag = core
            .vault()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_settings.get(&key).cloned())
            .unwrap_or_default();
        let migrated = script::migrate_legacy_setting_value(name, id, &value);
        bag.insert(id.to_string(), migrated);
        let live = self.live_settings(core, profile, &key, source, name, path, &bag);
        let result = self.upsert_profile_settings(
            core,
            profile,
            ArmMirror::ScriptSettings {
                card: key.clone(),
                live,
            },
            |settings| {
                settings.script_settings.insert(key.clone(), bag);
            },
        );
        if result.is_ok() {
            self.sync.source_edited(profile, &key);
            self.clear_notice();
        }
        result
    }

    /// The run of card `key` on `profile` a parameter write is for, with
    /// the bag it would receive; `None` when no run of that card exists.
    #[allow(clippy::too_many_arguments)]
    fn live_settings<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        key: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        overrides: &Map<String, Value>,
    ) -> Option<LiveSettings> {
        let (identity, generation) = live_fence(core, profile, key)?;
        let schema = self
            .js
            .get(source, &lookup_name(source, name, path))
            .map(|c| c.settings_schema.as_slice())
            .unwrap_or_default();
        let bag = self.run_bag(core, profile, schema, overrides);
        Some(LiveSettings {
            identity,
            generation,
            run: None,
            bag: Arc::new(bag),
            prepared: None,
        })
    }

    // ---- Start / Start all / Stop all ----------------------------------

    /// Whether any Load Start has not settled.
    pub fn starts_pending(&self) -> bool {
        !self.starts.is_empty()
    }

    /// Start `profile` on its last successful assignment.
    pub fn start_profile<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        catalog_root: Option<&Path>,
    ) -> Result<(), String> {
        if script_active(core, profile) {
            return Ok(());
        }
        let sel = Self::assignment(core, profile)
            .and_then(|a| sel_from_assignment(&a))
            .ok_or_else(|| "no assignment".to_string())?;
        self.start_sel(core, profile, sel, catalog_root, StartKind::Start)
    }

    /// Operator Start on `profile`: its pending Browse selection, else the
    /// front end's `heading`, else its assignment. Another profile's draft
    /// never reaches this Start. Refused while the profile's script is
    /// active.
    pub fn start_selected<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        heading: Option<&script::ScriptSel>,
        catalog_root: Option<&Path>,
    ) -> Result<(), String> {
        if script_active(core, profile) {
            return Err("script already active: stop it first".into());
        }
        let sel = self
            .pending_browse
            .get(profile)
            .cloned()
            .or_else(|| heading.cloned())
            .or_else(|| Self::assignment(core, profile).and_then(|a| sel_from_assignment(&a)))
            .ok_or_else(|| "no assignment".to_string())?;
        self.start_sel(core, profile, sel, catalog_root, StartKind::Start)
    }

    fn start_sel<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        sel: script::ScriptSel,
        catalog_root: Option<&Path>,
        kind: StartKind,
    ) -> Result<(), String> {
        if core.play().is_none() {
            return Err("no play".into());
        }
        match sel {
            script::ScriptSel::Compiled(id) => {
                let bag = self.compiled_bag(core, profile, id)?;
                core.start_script(profile, ScriptStart::Compiled { id, bag }, None)
                    .map_err(|e| e.to_string())?;
                self.starts.insert(
                    profile.to_string(),
                    PendingStart {
                        card: PendingCard::Compiled(id),
                        kind,
                    },
                );
                Ok(())
            }
            script::ScriptSel::Loaded(source, lookup) => {
                // A saved catalog assignment restored at launch names a card
                // before any Browse/Load has filled the catalog.
                if source == script::ScriptSource::Catalog {
                    self.fill_catalog_once(catalog_root);
                }
                let card = self.js.get(source, &lookup).ok_or_else(|| match source {
                    script::ScriptSource::File => format!("missing file: {lookup}"),
                    _ => format!("unavailable: {lookup}"),
                })?;
                if let Some(reason) = &card.unloadable {
                    return Err(format!("unloadable import: {reason}"));
                }
                self.js.ensure_js(source, &lookup)?;
                let card = self
                    .js
                    .get(source, &lookup)
                    .cloned()
                    .ok_or_else(|| format!("no loaded script: {lookup}"))?;
                let bag = self.merged_profile_bag(
                    core,
                    profile,
                    source,
                    &card.name,
                    &card.path,
                    &card.settings_schema,
                );
                let bag = (!bag.is_empty()).then_some(bag);
                let siblings = script::resolve_sibling_modules(
                    &card.path,
                    &card.origin,
                    self.js.cache(),
                    script::CacheMeta {
                        kind: card.kind,
                        source: card.source,
                        shape: None,
                        api_family: Some(card.api_family.as_str().into()),
                    },
                )?;
                let start = ScriptStart::Load {
                    js: card.js.clone(),
                    shape: card.shape,
                    bag,
                    siblings,
                };
                if let Err(e) = core.start_script(profile, start, Some(card.identity_key())) {
                    return self.js.record_start_result(&card, Err(e));
                }
                self.starts.insert(
                    profile.to_string(),
                    PendingStart {
                        card: PendingCard::Loaded(Box::new(card)),
                        kind,
                    },
                );
                Ok(())
            }
        }
    }

    /// Start every wall member idle on its last successful assignment.
    /// Active members are skipped; the tally is shown and a member whose
    /// setup fails later moves from started to failed.
    pub fn start_all<Io>(&mut self, core: &mut OperatorSession<Io>, catalog_root: Option<&Path>) {
        let members = core.members().to_vec();
        if members.is_empty() {
            self.show("Start all: no wall members");
            return;
        }
        let mut failures = Vec::new();
        let mut started = 0usize;
        let mut skipped = 0usize;
        for name in members {
            if script_active(core, &name) {
                skipped += 1;
                continue;
            }
            let sel = Self::assignment(core, &name).and_then(|a| sel_from_assignment(&a));
            let result = match sel {
                Some(sel) => self.start_sel(core, &name, sel, catalog_root, StartKind::StartAll),
                None => Err("no assignment".to_string()),
            };
            match result {
                Ok(()) => started += 1,
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        let report = format_bulk("Start all", started, skipped, &failures);
        self.bulk_start = Some(BulkStart {
            started,
            skipped,
            failures,
        });
        if self.last_bulk_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_report = Some(report.clone());
        self.show(report);
    }

    /// Stop every member's script (and any other running slot), including a
    /// slot still reaping a reload whose replacement Start is queued: that
    /// Start is dropped. Any reload in preparation or awaiting confirmation
    /// is cancelled. Returns how many were stopped.
    pub fn stop_all<Io>(&mut self, core: &mut OperatorSession<Io>) -> usize {
        if core.play().is_none() {
            return 0;
        }
        let names = slot_names(core);
        let (_, stopped) = core.stop_scripts(&names);
        self.reload.invalidate();
        let report = format!("Stop all: stopped {stopped}");
        if self.last_bulk_report.as_deref() == Some(report.as_str()) {
            return stopped;
        }
        self.last_bulk_report = Some(report);
        if stopped > 0 {
            self.clear_notice();
        }
        stopped
    }

    /// The last Start all / Stop all report.
    pub fn last_bulk_report(&self) -> Option<&str> {
        self.last_bulk_report.as_deref()
    }

    // ---- per-frame settlement ------------------------------------------

    /// Fold everything that settled since the last frame: reload
    /// validation, Load Starts (Ready commits the assignment and clears the
    /// card's load diagnostic; a failure records it) and parameter writes.
    /// Call once per UI frame, after [`OperatorSession::poll`].
    pub fn poll<Io>(&mut self, core: &mut OperatorSession<Io>) {
        self.poll_reload_validation(core);
        self.settle_starts(core);
        for write in core.take_settings_writes() {
            if let Some((op, outcome)) = self.sync.record(&write) {
                core.set_outcome(op, &write.profile, outcome);
            }
        }
        if let Some(summary) = self.sync.take_settled_summary() {
            self.show(summary);
        }
    }

    fn settle_starts<Io>(&mut self, core: &mut OperatorSession<Io>) {
        for StartSettled {
            slot: name,
            outcome,
            ..
        } in core.take_settled_starts()
        {
            let Some(pending) = self.starts.remove(&name) else {
                continue;
            };
            match outcome {
                Some(script::StartOutcome::Ready) => match pending.card {
                    PendingCard::Compiled(id) => {
                        let key = script::compiled_identity_key(id);
                        let _ = self.upsert_profile_settings(
                            core,
                            &name,
                            ArmMirror::None,
                            |settings| {
                                settings.script_assignment = Some(script::compiled_assignment(id));
                                if settings.script_settings.get(&key).is_none_or(Map::is_empty) {
                                    let schema_version =
                                        if settings.script_settings.contains_key(&key) {
                                            1 // Empty legacy envelopes are schema 1.
                                        } else {
                                            script::compiled_card(id)
                                                .expect("registered card")
                                                .schema_version
                                        };
                                    settings.script_settings.insert(
                                        key,
                                        vault::CompiledSettingsRecord {
                                            schema_version,
                                            values: Map::new(),
                                        }
                                        .into_entry(),
                                    );
                                }
                            },
                        );
                    }
                    PendingCard::Loaded(card) => {
                        let key = card.identity_key();
                        let had_failure = self.js.load_failure(&key).is_some();
                        let listing = self.shown.take_if(|shown| {
                            had_failure && *shown == self.js.named_failure_output()
                        });
                        let _ = self.js.record_start_result(&card, Ok(()));
                        if let Some(shown) = listing {
                            let with = (!self.js.load_failures().is_empty())
                                .then(|| self.js.named_failure_output());
                            self.shown.clone_from(&with);
                            self.notice = Some(Notice::Retract { shown, with });
                        }
                        self.persist_assignment(core, &name, card.assignment());
                    }
                },
                Some(script::StartOutcome::Failed(error)) => {
                    let diagnostic = match pending.card {
                        PendingCard::Loaded(card) => self
                            .js
                            .record_start_result(
                                &card,
                                Err(script::StartLoadError::RuntimeLoad(error.clone())),
                            )
                            .err()
                            .unwrap_or(error),
                        PendingCard::Compiled(_) => error,
                    };
                    self.report_start_failure(&name, pending.kind, diagnostic);
                }
                Some(script::StartOutcome::Rejected(error)) => {
                    self.report_start_failure(&name, pending.kind, error.to_string());
                }
                Some(script::StartOutcome::Cancelled) | None => {}
            }
        }
    }

    fn report_start_failure(&mut self, name: &str, kind: StartKind, diagnostic: String) {
        match kind {
            StartKind::Start => {
                let message = format!("script: {diagnostic}");
                self.start_failures
                    .push((name.to_string(), message.clone()));
                self.show(message);
            }
            StartKind::StartAll => {
                let bulk = self.bulk_start.get_or_insert_with(BulkStart::default);
                bulk.started = bulk.started.saturating_sub(1);
                bulk.failures.push(format!("{name}: {diagnostic}"));
                let report = format_bulk("Start all", bulk.started, bulk.skipped, &bulk.failures);
                self.last_bulk_report = Some(report.clone());
                self.show(report);
            }
            StartKind::Reload => self.show(format!("reload: {name}: {diagnostic}")),
        }
    }
}

/// The run of card `key` on `profile`: its identity and generation.
fn live_fence<Io>(core: &OperatorSession<Io>, profile: &str, key: &str) -> Option<(String, u64)> {
    let play = core.play()?;
    let identity = play.script_source_identity(profile)?;
    if identity != key {
        return None;
    }
    Some((identity, play.script_runtime_generation(profile)?))
}

/// Wall members plus any other slot the session holds.
fn slot_names<Io>(core: &OperatorSession<Io>) -> Vec<String> {
    let mut names: Vec<String> = core.members().to_vec();
    for name in core.slots().keys() {
        if !names.iter().any(|n| n == name) {
            names.push(name.clone());
        }
    }
    names
}

fn script_active<Io>(core: &OperatorSession<Io>, name: &str) -> bool {
    matches!(
        core.play()
            .map_or(script::RunState::Idle, |p| p.script_state(name)),
        script::RunState::Starting
            | script::RunState::Running
            | script::RunState::Paused
            | script::RunState::Stopping
    )
}

/// The Browse selection a saved assignment names.
pub fn sel_from_assignment(asg: &ScriptAssignment) -> Option<script::ScriptSel> {
    if asg.source_kind == "compiled" {
        return script::compiled_id(&asg.identity).map(script::ScriptSel::Compiled);
    }
    let source = script::parse_source_kind(&asg.source_kind)?;
    Some(script::ScriptSel::Loaded(source, asg.identity.clone()))
}

/// The library lookup for a card: a File card by path, others by name.
fn lookup_name(source: script::ScriptSource, name: &str, path: &Path) -> String {
    match source {
        script::ScriptSource::File => path.to_string_lossy().into_owned(),
        _ => name.to_string(),
    }
}

fn format_bulk(op: &str, started: usize, skipped: usize, failures: &[String]) -> String {
    if failures.is_empty() {
        return format!("{op}: started {started}, skipped {skipped}");
    }
    let shown: Vec<&str> = failures.iter().take(6).map(String::as_str).collect();
    format!(
        "{op}: started {started}, skipped {skipped}, failed {}: {}",
        failures.len(),
        shown.join("; ")
    )
}

#[cfg(test)]
mod tests;

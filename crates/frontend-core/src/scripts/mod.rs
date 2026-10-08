//! Script coordination shared by the panel and the TUI: the card library
//! and catalog, per-profile assignment and pending Browse selection,
//! per-profile parameter bags (legacy migration, typed edits, live push),
//! Start / Start all (paced through a login-FIFO-style permit) / Stop all with their settlement, the reload and
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

mod marked;
mod parameter_edit;
mod reload;
mod start_admit;
mod start_tally;
use parameter_edit::ParameterEdit;
pub use parameter_edit::{
    parse_parameter_text, resolve_parameter_options, ParameterCommit, ParameterEditKey,
    ParameterOptions,
};

mod sync;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use start_admit::START_ADMIT_PER_FRAME;
use start_admit::{QueuedStart, StartAdmit, StartPermit};
use start_tally::{Outcome, StartTally};

use api::hostlog::{Level, Source};
use serde_json::{Map, Value};
use vault::ScriptAssignment;

use crate::operations::OperationId;
use crate::session::{ArmMirror, OperatorSession, ScriptStart, StartSettled};
use crate::views::QueuePlace;

pub use reload::{PendingReload, PendingReloadKind, ReloadOutcome, ReloadWarning};
pub use sync::{RestartPrompt, SyncReport, SyncScope};

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
/// setup failure is reported and whether the saved assignment may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartKind {
    Start,
    Reload,
    RestartSettings,
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
    /// Counted as started in the running Start tally: a setup failure
    /// moves it to failed there.
    tallied: bool,
}

#[derive(Debug, Clone)]
struct SettingsCard {
    selection: script::ScriptSel,
    name: String,
}

/// Where a new skip or failure in the running tally is logged.
#[derive(Debug, Clone, Copy)]
enum LogTo {
    /// The bot's own log.
    Slot,
    /// The process log (a marked row with no profile).
    Process,
    /// Already logged (a settled Start's operation line) or not a skip.
    None,
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
    /// Reused editor keys for the currently displayed profile and card.
    parameter_key_cache: Vec<ParameterEditKey>,
    parameter_key_scope: Option<(Option<String>, script::ScriptSel)>,
    parameter_edits: HashMap<ParameterEditKey, ParameterEdit>,
    settings_cards: HashMap<OperationId, SettingsCard>,
    starts: HashMap<String, PendingStart>,
    /// Paced Start-all / marked-Start places. Empty in the idle path.
    admit: StartAdmit,
    /// Catalog root for grants that settle after the click frame.
    admit_catalog: Option<PathBuf>,
    /// The running Start all / marked-Start report (see [`start_tally`]).
    tally: Option<StartTally>,
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
            parameter_key_cache: Vec::new(),
            parameter_key_scope: None,
            parameter_edits: HashMap::new(),
            settings_cards: HashMap::new(),
            starts: HashMap::new(),
            admit: StartAdmit::default(),
            admit_catalog: None,
            tally: None,
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

    fn track_settings_card(&mut self, op: OperationId, selection: script::ScriptSel, name: &str) {
        self.settings_cards.insert(
            op,
            SettingsCard {
                selection,
                name: name.to_string(),
            },
        );
    }

    /// The message to show since the last take.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }

    pub fn pending_restart_prompt(&self) -> Option<&RestartPrompt> {
        self.sync.pending_restart_prompt()
    }

    pub fn dismiss_restart_prompt(&mut self) {
        self.sync.dismiss_restart_prompt();
    }

    pub fn restart_required_for(&self, profile: &str, selection: &script::ScriptSel) -> bool {
        self.sync
            .restart_card(profile)
            .is_some_and(|required| required == selection)
    }

    pub fn has_restart_badge(&self, profile: &str) -> bool {
        self.sync.has_restart_badge(profile)
    }

    pub fn restart_generation(&self) -> u64 {
        self.sync.restart_generation()
    }

    pub fn restart_pending_settings<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
    ) -> Result<(), String> {
        self.restart_pending_settings_with_after_check(core, catalog_root, |_, _| {})
    }

    /// Keep the check-to-stop boundary deterministic in tests.
    fn restart_pending_settings_with_after_check<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
        mut after_check: impl FnMut(&mut OperatorSession<Io>, &sync::RestartTarget),
    ) -> Result<(), String> {
        let prompt = self
            .sync
            .take_restart_prompt()
            .ok_or_else(|| "no restart prompt".to_string())?;
        let mut skipped = Vec::new();
        for target in prompt.targets() {
            let (identity, generation) = match self.restart_target_skip(core, target) {
                Ok(run) => run,
                Err(reason) => {
                    self.sync.clear_restart_badge(&target.profile);
                    skipped.push((target.profile.clone(), reason.to_string()));
                    continue;
                }
            };
            if self.admit.contains(&target.profile) {
                skipped.push((target.profile.clone(), "start already queued".into()));
                continue;
            }
            after_check(core, target);
            let stopped = core.play().is_some_and(|play| {
                play.script_stop_if_identity_generation(&target.profile, &identity, generation)
            });
            if !stopped {
                self.sync.clear_restart_badge(&target.profile);
                skipped.push((target.profile.clone(), "not running".into()));
                continue;
            }
            match self.queue_pending_restart_settings(
                core,
                &target.profile,
                target.selection.clone(),
            ) {
                Ok(()) => {}
                Err(reason) => skipped.push((target.profile.clone(), reason)),
            }
        }
        self.admit_starts(core, catalog_root);
        if !skipped.is_empty() {
            self.show(
                skipped
                    .iter()
                    .map(|(profile, reason)| format!("Skipped {profile}: {reason}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        Ok(())
    }

    fn restart_target_skip<Io>(
        &self,
        core: &OperatorSession<Io>,
        target: &sync::RestartTarget,
    ) -> Result<(String, u64), &'static str> {
        let profile = &target.profile;
        if !wall_member(core, profile) {
            return Err("not loaded");
        }
        let Some(play) = core.play() else {
            return Err("not running");
        };
        let Some(generation) = play.script_runtime_generation(profile) else {
            return Err("not running");
        };
        if !matches!(
            play.script_state(profile),
            script::RunState::Running | script::RunState::Paused
        ) {
            return Err("not running");
        }
        let Some(run_identity) = play.script_source_identity(profile) else {
            return Err("different script");
        };
        let (has_arm, logged_out) = arm_flags(core, profile);
        if !has_arm || logged_out {
            return Err("logged out");
        }
        let Some(assignment) = Self::assignment(core, profile) else {
            return Err("reassigned");
        };
        if sel_from_assignment(&assignment).as_ref() != Some(&target.selection) {
            return Err("reassigned");
        }
        let identity = vault::assignment_key(&assignment.source_kind, &assignment.identity);
        if run_identity != identity {
            return Err("different script");
        }
        let status = play.script_native_status(profile);
        if let Some(status) = status.as_deref() {
            if play.script_native_run(profile) != Some(status.run)
                || !matches!(target.selection, script::ScriptSel::Compiled(id) if id == status.card)
            {
                return Err("different script");
            }
            if status
                .pending_settings
                .is_none_or(|pending| pending <= status.active_settings)
            {
                return Err("settings no longer pending");
            }
        }
        if !self.sync.restart_target_is_current(
            profile,
            &target.selection,
            target.generation,
            status.as_deref().map(|status| status.active_settings),
            status.as_deref().and_then(|status| status.pending_settings),
        ) {
            return Err("settings no longer pending");
        }
        Ok((run_identity, generation))
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
        let legacy = self.legacy_overrides_for(key, card_name);
        let _ = self.upsert_profile_settings(core, profile, ArmMirror::None, |settings| {
            script::claim_legacy_overrides(settings, key, card_name, &legacy);
        });
    }

    /// `profile`'s overrides bag for the card `key`, claiming the legacy
    /// global overrides into the profile on its first use of the card. That
    /// claim queues a durable write; a read that must not write (a
    /// confirmation still awaiting the operator) uses
    /// [`Self::peek_profile_overrides`].
    pub fn profile_overrides<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        self.claim_legacy_for(core, profile, key, card_name);
        self.peek_profile_overrides(core, profile, key, card_name)
    }

    /// The bag [`Self::profile_overrides`] returns, without the claim: the
    /// profile's own overrides for the card, or, before its first use of the
    /// card, the migrated legacy overrides the claim would store. Writes
    /// nothing and queues nothing.
    pub fn peek_profile_overrides<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        let Some(row) = core.vault().and_then(|v| v.get(profile)) else {
            return Map::new();
        };
        match row.settings.script_settings.get(key) {
            Some(own) => own.clone(),
            None => {
                script::migrate_overrides(card_name, &self.legacy_overrides_for(key, card_name))
            }
        }
    }

    /// The legacy global overrides for the card `key` names.
    fn legacy_overrides_for(&self, key: &str, card_name: &str) -> Map<String, Value> {
        self.legacy.overrides(
            script::parse_source_kind(key.split(':').next().unwrap_or("catalog"))
                .unwrap_or(script::ScriptSource::Catalog),
            card_name,
        )
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
        let lookup = lookup_name(source, name, path);
        let migrated = self
            .js
            .get(source, &lookup)
            .and_then(|card| card.settings_schema.iter().find(|setting| setting.id == id))
            .map(|setting| script::coerce_setting_value(&setting.ty, &migrated))
            .unwrap_or(migrated);
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
        if let Ok(op) = result.as_ref() {
            self.track_settings_card(*op, script::ScriptSel::Loaded(source, lookup), name);
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

    /// Whether any Load Start has not settled, including paced Starts still
    /// waiting for a permit.
    pub fn starts_pending(&self) -> bool {
        !self.starts.is_empty() || !self.admit.is_empty()
    }

    /// k-of-n place while `profile` waits for a Start-all (or marked) permit.
    pub fn start_queue_place(&self, profile: &str) -> Option<QueuePlace> {
        self.admit.status(profile)
    }

    /// Drop a queued-but-not-yet-admitted Start. Returns whether a place
    /// was removed. Already-dispatched Starts are stopped through the host.
    pub fn cancel_queued(&mut self, profile: &str) -> bool {
        self.cancel_queued_as(profile, "cancelled")
    }

    /// Drop a waiting Start and record `reason` in the running Start tally.
    pub fn cancel_queued_as(&mut self, profile: &str, reason: &str) -> bool {
        if self.admit.leave(profile).is_none() {
            return false;
        }
        self.credit_skipped(profile, reason);
        true
    }

    /// Open the running tally for a Start click. While a bot it holds still
    /// waits for its permit or its setup, the click folds into it (and
    /// names it); otherwise the finished tally is replaced.
    pub(crate) fn open_tally(&mut self, label: &'static str) {
        let open = self.tally.as_ref().is_some_and(StartTally::waiting)
            || self.starts.values().any(|pending| pending.tallied);
        match self.tally.as_mut() {
            Some(tally) if open => tally.relabel(label),
            _ => self.tally = Some(StartTally::new(label)),
        }
    }

    /// A click-time skip. A bot this tally already started stays started.
    pub(crate) fn tally_skip(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Skipped(reason.to_string()), LogTo::Slot);
    }

    /// A click-time skip of a marked row with no profile (no slot to log on).
    pub(crate) fn tally_skip_unavailable(&mut self, row: &str, reason: &str) {
        self.tally_record(row, Outcome::Skipped(reason.to_string()), LogTo::Process);
    }

    pub(crate) fn tally_fail(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Failed(reason.to_string()), LogTo::Slot);
    }

    /// Record `profile`'s outcome in the running tally. A new skip or
    /// failure also goes to the log (`log`), so it survives a later banner.
    fn tally_record(&mut self, profile: &str, outcome: Outcome, log: LogTo) {
        let Some(tally) = self.tally.as_mut() else {
            return;
        };
        if !tally.set(profile, outcome) {
            return;
        }
        let (level, verb, reason) = match tally.outcome(profile) {
            Some(Outcome::Skipped(reason)) => (Level::Info, "skipped", reason),
            Some(Outcome::Failed(reason)) => (Level::Error, "failed", reason),
            Some(Outcome::Held(reason)) => (Level::Info, "queued", reason),
            _ => return,
        };
        let label = tally.label();
        match log {
            LogTo::Slot => crate::log::global().slot_line(
                profile,
                Source::Host,
                level,
                &format!("{label} {verb}: {reason}"),
            ),
            LogTo::Process => crate::log::global().process_line(
                Source::Host,
                level,
                &format!("{label} {verb} {profile}: {reason}"),
            ),
            LogTo::None => {}
        }
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
        self.start_sel(core, profile, &sel, catalog_root, StartKind::Start, false)
            .map_err(|error| error.to_string())
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
        self.start_sel(core, profile, &sel, catalog_root, StartKind::Start, false)
            .map_err(|error| error.to_string())
    }

    /// The loaded card `(source, lookup)` names, if it can start. A file
    /// card whose file is gone is refused as missing, the same on every
    /// platform: Start never runs a deleted script from the copy loaded
    /// earlier, whether or not the card's stored path still resolves.
    pub(crate) fn startable_card(
        &self,
        source: script::ScriptSource,
        lookup: &str,
    ) -> Result<&script::JsCard, String> {
        self.js
            .get(source, lookup)
            .filter(|card| source != script::ScriptSource::File || card.path.is_file())
            .ok_or_else(|| match source {
                script::ScriptSource::File => format!("missing file: {lookup}"),
                _ => format!("unavailable: {lookup}"),
            })
    }

    fn start_sel<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        sel: &script::ScriptSel,
        catalog_root: Option<&Path>,
        kind: StartKind,
        tallied: bool,
    ) -> Result<(), script::StartLoadError> {
        if self.parameter_edit_blocks_start(profile) {
            return Err(script::StartLoadError::Waiting(
                "settings edit is uncommitted or its save has not settled".into(),
            ));
        }
        if core.play().is_none() {
            return Err(script::StartLoadError::Refused("no play".into()));
        }
        let tallied = tallied || self.admit.contains(profile);
        let result = self.dispatch_start(core, profile, sel, catalog_root, kind, tallied);
        if matches!(&result, Err(script::StartLoadError::Waiting(_))) {
            if let Err(error) = &result {
                self.show(error.to_string());
            }
            return result;
        }
        let from_queue = self.admit.leave(profile).is_some();
        match &result {
            Ok(()) => self.credit_queue_outcome(from_queue, profile, Ok(())),
            Err(error) => self.credit_queue_outcome(from_queue, profile, Err(&error.to_string())),
        }
        result
    }

    fn dispatch_start<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        profile: &str,
        sel: &script::ScriptSel,
        catalog_root: Option<&Path>,
        kind: StartKind,
        tallied: bool,
    ) -> Result<(), script::StartLoadError> {
        match sel {
            script::ScriptSel::Compiled(id) => {
                let bag = self
                    .compiled_bag(core, profile, *id)
                    .map_err(script::StartLoadError::Refused)?;
                core.start_script(profile, ScriptStart::Compiled { id: *id, bag }, None)?;
                self.starts.insert(
                    profile.to_string(),
                    PendingStart {
                        card: PendingCard::Compiled(*id),
                        kind,
                        tallied,
                    },
                );
                Ok(())
            }
            script::ScriptSel::Loaded(source, lookup) => {
                // A saved catalog assignment restored at launch names a card
                // before any Browse/Load has filled the catalog.
                if *source == script::ScriptSource::Catalog {
                    self.fill_catalog_once(catalog_root);
                }
                let card = self
                    .startable_card(*source, lookup)
                    .map_err(script::StartLoadError::Refused)?;
                if let Some(reason) = &card.unloadable {
                    return Err(script::StartLoadError::Refused(format!(
                        "unloadable import: {reason}"
                    )));
                }
                self.js
                    .ensure_js(*source, lookup)
                    .map_err(script::StartLoadError::Refused)?;
                let card = self.js.get(*source, lookup).cloned().ok_or_else(|| {
                    script::StartLoadError::Refused(format!("no loaded script: {lookup}"))
                })?;
                let bag = self.merged_profile_bag(
                    core,
                    profile,
                    *source,
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
                )
                .map_err(script::StartLoadError::Refused)?;
                let start = ScriptStart::Load {
                    js: card.js.clone(),
                    shape: card.shape,
                    bag,
                    siblings,
                };
                if let Err(e) = core.start_script(profile, start, Some(card.identity_key())) {
                    if matches!(e, script::StartLoadError::Waiting(_)) {
                        return Err(e);
                    }
                    return self
                        .js
                        .record_start_result(&card, Err(e))
                        .map_err(script::StartLoadError::Refused);
                }
                self.starts.insert(
                    profile.to_string(),
                    PendingStart {
                        card: PendingCard::Loaded(Box::new(card)),
                        kind,
                        tallied,
                    },
                );
                Ok(())
            }
        }
    }

    /// Start every wall member idle on its last successful assignment.
    /// Eligible Starts are enqueued and released through the Start-all
    /// permit (at most [`START_ADMIT_PER_FRAME`] in this click). Active
    /// members are skipped. The running tally is shown; a member whose setup
    /// fails later moves from started to failed.
    pub fn start_all<Io>(&mut self, core: &mut OperatorSession<Io>, catalog_root: Option<&Path>) {
        let members = core.members().to_vec();
        if members.is_empty() {
            self.show("Start all: no wall members");
            return;
        }
        if let Some(root) = catalog_root {
            self.admit_catalog = Some(root.to_path_buf());
        }
        self.open_tally("Start all");
        for name in members {
            if self.admit.contains(&name) {
                continue;
            }
            if script_active(core, &name) {
                self.tally_skip(&name, "already active");
                continue;
            }
            let sel = Self::assignment(core, &name).and_then(|a| sel_from_assignment(&a));
            let Some(sel) = sel else {
                self.tally_fail(&name, "no assignment");
                continue;
            };
            if let Err(error) = self.queue_start(core, &name, sel.clone(), Some(sel)) {
                self.tally_fail(&name, &error);
            }
        }
        self.admit_starts(core, catalog_root);
        self.publish_bulk();
    }

    /// Enqueue `profile` for a paced Start in the running tally (opened by
    /// the click). Immediate Start paths call [`Self::start_sel`] instead.
    /// Idempotent per profile.
    pub(crate) fn queue_start<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        profile: &str,
        sel: script::ScriptSel,
        assigned: Option<script::ScriptSel>,
    ) -> Result<(), String> {
        if core.play().is_none() {
            return Err("no play".into());
        }
        if script_active(core, profile) {
            return Err("script already active: stop it first".into());
        }
        if self.admit.contains(profile) {
            return Ok(());
        }
        let (had_arm, latched) = arm_flags(core, profile);
        self.admit.enqueue(QueuedStart {
            profile: profile.to_string(),
            sel,
            assigned,
            latched,
            had_arm,
            kind: StartKind::Start,
        });
        self.tally_record(profile, Outcome::Queued, LogTo::None);
        Ok(())
    }

    /// Grant up to [`START_ADMIT_PER_FRAME`] waiting Starts.
    pub(crate) fn admit_starts<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        catalog_root: Option<&Path>,
    ) {
        if let Some(root) = catalog_root {
            self.admit_catalog = Some(root.to_path_buf());
        }
        self.admit_ready(core);
        self.publish_start_places(core);
    }

    /// Stop every member's script (and any other running slot), including a
    /// slot still reaping a reload whose replacement Start is queued: that
    /// Start is dropped. Queued-but-not-yet-admitted Starts are cancelled.
    /// Any reload in preparation or awaiting confirmation is cancelled.
    /// Returns how many were stopped.
    pub fn stop_all<Io>(&mut self, core: &mut OperatorSession<Io>) -> usize {
        let cancelled = self.admit.drain();
        for entry in &cancelled {
            self.tally_record(
                &entry.profile,
                Outcome::Skipped("cancelled".into()),
                LogTo::Slot,
            );
        }
        if !cancelled.is_empty() {
            self.publish_bulk();
        }
        self.publish_start_places(core);
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
        if stopped > 0 || !cancelled.is_empty() {
            self.clear_notice();
        }
        stopped
    }

    /// The running Start all / marked-Start report, or the last Stop all
    /// report when that came later.
    pub fn last_bulk_report(&self) -> Option<&str> {
        self.last_bulk_report.as_deref()
    }

    // ---- per-frame settlement ------------------------------------------

    /// Fold everything that settled since the last frame: reload
    /// validation, Load Starts (Ready commits the assignment and clears the
    /// card's load diagnostic; a failure records it) and parameter writes.
    /// Call once per UI frame, after [`OperatorSession::poll`].
    pub fn poll<Io>(&mut self, core: &mut OperatorSession<Io>) {
        self.settle_parameter_edits(core);
        self.poll_reload_validation(core);
        self.release_stopped_restarts(core);
        if !self.admit.is_empty() {
            self.admit_ready(core);
        }
        self.publish_start_places(core);
        self.settle_starts(core);
        for write in core.take_settings_writes() {
            let is_sync_write = self.sync.tracks_write(write.op);
            if matches!(
                &write.result,
                SettingsResult::Saved(LiveDelivery::RestartRequired)
            ) {
                let card = self.settings_cards.remove(&write.op).or_else(|| {
                    let assignment = Self::assignment(core, &write.profile)?;
                    Some(SettingsCard {
                        selection: sel_from_assignment(&assignment)?,
                        name: assignment.display_name,
                    })
                });
                if let Some(card) = card {
                    let status = core
                        .play()
                        .and_then(|play| play.script_native_status(&write.profile));
                    self.sync.note_restart_required(
                        &write.profile,
                        &card.selection,
                        status.as_deref().map(|status| status.active_settings),
                    );
                    if !is_sync_write {
                        self.sync
                            .queue_single_restart_prompt(write.op, card, &write.profile);
                    }
                }
            } else {
                self.settings_cards.remove(&write.op);
            }
            let sync_outcome = self.sync.record(&write);
            if let Some((op, outcome)) = sync_outcome {
                core.set_outcome(op, &write.profile, outcome);
            }
        }
        self.sync.observe_restart_activation(core);
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
            if matches!(&outcome, Some(script::StartOutcome::Ready)) {
                self.sync.clear_restart_badge(&name);
            }
            match outcome {
                Some(script::StartOutcome::Ready) if pending.kind == StartKind::RestartSettings => {
                }
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
                    self.report_start_failure(&name, pending.kind, pending.tallied, diagnostic);
                }
                Some(script::StartOutcome::Rejected(error)) => {
                    self.report_start_failure(
                        &name,
                        pending.kind,
                        pending.tallied,
                        error.to_string(),
                    );
                }
                Some(script::StartOutcome::Cancelled) | None => {}
            }
        }
    }

    /// A Start that failed during setup. Its operation already logged the
    /// failure on the bot.
    fn report_start_failure(
        &mut self,
        name: &str,
        kind: StartKind,
        tallied: bool,
        diagnostic: String,
    ) {
        if kind == StartKind::Reload {
            self.show(format!("reload: {name}: {diagnostic}"));
            return;
        }
        let message = format!("script: {diagnostic}");
        if tallied
            && self
                .tally
                .as_ref()
                .is_some_and(|t| t.outcome(name).is_some())
        {
            self.tally_record(name, Outcome::Failed(diagnostic), LogTo::None);
            self.publish_bulk();
        } else {
            self.show(message.clone());
        }
        self.start_failures.push((name.to_string(), message));
    }

    fn admit_ready<Io>(&mut self, core: &mut OperatorSession<Io>) {
        let mut granted = 0;
        while granted < START_ADMIT_PER_FRAME {
            if self.admit.poll() != StartPermit::Grant {
                break;
            }
            let Some(entry) = self.admit.take_head() else {
                break;
            };
            if self.dispatch_queued(core, entry) {
                granted += 1;
            }
        }
    }

    /// `true` when `start_sel` ran (counts against the frame budget).
    fn dispatch_queued<Io>(&mut self, core: &mut OperatorSession<Io>, entry: QueuedStart) -> bool {
        if !wall_member(core, &entry.profile) || core.removal_pending(&entry.profile) {
            self.drop_queued(&entry, "removed");
            return false;
        }
        if script_active(core, &entry.profile) {
            self.drop_queued(&entry, "already active");
            return false;
        }
        if let Some(required) = &entry.assigned {
            let current =
                Self::assignment(core, &entry.profile).and_then(|a| sel_from_assignment(&a));
            if current.as_ref() != Some(required) {
                self.drop_queued(&entry, "reassigned");
                return false;
            }
        }
        let (has_arm, latched_now) = arm_flags(core, &entry.profile);
        if entry.had_arm && !has_arm {
            self.drop_queued(&entry, "disconnected");
            return false;
        }
        if !entry.latched && latched_now {
            self.drop_queued(&entry, "logged out");
            return false;
        }
        let catalog = self.admit_catalog.clone();
        let tallied = entry.kind != StartKind::RestartSettings;
        match self.start_sel(
            core,
            &entry.profile,
            &entry.sel,
            catalog.as_deref(),
            entry.kind,
            tallied,
        ) {
            Ok(()) if tallied => self.credit_started(&entry.profile),
            Ok(()) => {}
            Err(script::StartLoadError::Waiting(reason)) => {
                if tallied {
                    self.tally_record(&entry.profile, Outcome::Held(reason), LogTo::Slot);
                    self.publish_bulk();
                }
                self.admit.return_head(entry);
                return true;
            }
            Err(error) if tallied => self.credit_failed(&entry.profile, &error.to_string()),
            Err(error) => self.show(format!("Restart failed for {}: {error}", entry.profile)),
        }
        true
    }

    fn drop_queued(&mut self, entry: &QueuedStart, reason: &str) {
        if entry.kind == StartKind::RestartSettings {
            self.show(format!("Skipped {}: {reason}", entry.profile));
        } else {
            self.credit_skipped(&entry.profile, reason);
        }
    }

    /// Credit a Start that left the queue through [`Self::start_sel`].
    fn credit_queue_outcome(&mut self, from_queue: bool, profile: &str, outcome: Result<(), &str>) {
        if !from_queue {
            return;
        }
        match outcome {
            Ok(()) => self.credit_started(profile),
            Err(error) => self.credit_failed(profile, error),
        }
    }

    fn credit_started(&mut self, profile: &str) {
        self.tally_record(profile, Outcome::Started, LogTo::None);
        self.publish_bulk();
    }

    fn credit_failed(&mut self, profile: &str, error: &str) {
        self.tally_record(profile, Outcome::Failed(error.to_string()), LogTo::Slot);
        self.publish_bulk();
    }

    fn credit_skipped(&mut self, profile: &str, reason: &str) {
        self.tally_record(profile, Outcome::Skipped(reason.to_string()), LogTo::Slot);
        self.publish_bulk();
    }

    /// Show the running tally when its text changed.
    pub(crate) fn publish_bulk(&mut self) {
        let Some(tally) = self.tally.as_ref() else {
            return;
        };
        let report = tally.report();
        if self.last_bulk_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_report = Some(report.clone());
        self.show(report);
    }

    pub fn publish_start_places<Io>(&mut self, core: &mut OperatorSession<Io>) {
        if !self.admit.take_publish() {
            return;
        }
        core.publish_start_queue(|name| self.admit.status(name));
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

fn wall_member<Io>(core: &OperatorSession<Io>, name: &str) -> bool {
    core.members().iter().any(|member| member == name)
}

fn arm_flags<Io>(core: &OperatorSession<Io>, name: &str) -> (bool, bool) {
    match core.play().and_then(|play| play.arm(name)) {
        Some(arm) => (true, arm.login_latched() || arm.wants_logout()),
        None => (false, false),
    }
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

#[cfg(test)]
mod tests;

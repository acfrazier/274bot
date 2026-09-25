//! Per-profile script assignment, bulk Start/Stop, live settings, reload
//! and catalog refresh. Session is the integration owner.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use vault::ScriptAssignment;

use crate::session::Session;

#[derive(Debug, Clone)]
pub struct ReloadWarning {
    pub identity_key: String,
    pub source: script::ScriptSource,
    pub lookup: String,
    pub fingerprint: String,
    pub epoch: u64,
    pub running: Vec<String>,
    pub paused: Vec<String>,
    pub paused_during_prep: Vec<String>,
    pub affected_generations: Vec<(String, u64)>,
}

impl Default for ReloadWarning {
    fn default() -> Self {
        Self {
            identity_key: String::new(),
            source: script::ScriptSource::Catalog,
            lookup: String::new(),
            fingerprint: String::new(),
            epoch: 0,
            running: Vec::new(),
            paused: Vec::new(),
            paused_during_prep: Vec::new(),
            affected_generations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PendingReloadKind {
    Manual,
    Catalog {
        root: PathBuf,
        added: Vec<String>,
        removed: Vec<String>,
        failed: Vec<script::LoadFailure>,
        set_fingerprint: String,
    },
}

#[derive(Debug, Clone)]
pub struct PendingReload {
    pub warning: ReloadWarning,
    pub prepared: Vec<script::PreparedCard>,
    pub kind: PendingReloadKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    NothingChanged,
    NeedsConfirm,
    Applied {
        restarted: usize,
        stopped_paused: usize,
        failed: usize,
    },
    Failed(String),
}

/// Which operator action a pending Start came from: it decides where a
/// setup failure is reported once it is observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingStartKind {
    Start,
    StartAll,
    Reload,
}

/// A Load Start whose isolate setup has not settled. Start returns before
/// V8 setup, so the card's assignment is persisted and its load diagnostic
/// cleared only on Ready; a failure records the diagnostic instead.
#[derive(Debug, Clone)]
pub(crate) struct PendingStart {
    card: script::JsCard,
    kind: PendingStartKind,
}

/// The last Start all tally. A member whose setup fails after the click
/// moves from started to failed, so the report lists it.
#[derive(Debug, Clone, Default)]
pub(crate) struct BulkStart {
    started: usize,
    skipped: usize,
    failures: Vec<String>,
}

impl Session {
    pub fn restore_script_heading(&mut self, profile: &str) {
        if let Some(sel) = self.pending_browse.get(profile).cloned() {
            self.script_sel = Some(sel);
            return;
        }
        if let Some(asg) = self.profile_assignment(profile) {
            self.script_sel = sel_from_assignment(&asg);
            return;
        }
        self.script_sel = None;
    }

    pub fn profile_assignment(&self, profile: &str) -> Option<ScriptAssignment> {
        self.vault
            .as_ref()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_assignment.clone())
    }

    pub fn heading_is_pending(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        let Some(pending) = self.pending_browse.get(&name) else {
            return false;
        };
        match (pending, self.profile_assignment(&name)) {
            (_, None) => true,
            (sel, Some(asg)) => sel_from_assignment(&asg).as_ref() != Some(sel),
        }
    }

    pub fn set_pending_browse(&mut self, profile: &str, sel: script::ScriptSel) {
        self.pending_browse.insert(profile.to_string(), sel);
    }

    fn upsert_profile_settings(
        &mut self,
        profile: &str,
        edit: impl FnOnce(&mut vault::ProfileSettings),
    ) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("script: vault locked".into());
            return false;
        };
        let Some(mut row) = vault.get(profile).cloned() else {
            self.error = Some(format!("script: no profile {profile}"));
            return false;
        };
        edit(&mut row.settings);
        match vault.upsert(row) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("script: {e}"));
                false
            }
        }
    }

    pub fn persist_successful_assignment(&mut self, profile: &str, assignment: ScriptAssignment) {
        self.upsert_profile_settings(profile, |settings| {
            settings.script_assignment = Some(assignment);
        });
    }

    pub fn mark_assignment_unavailable(&mut self, profile: &str, reason: impl Into<String>) {
        let reason = reason.into();
        self.upsert_profile_settings(profile, |settings| {
            if let Some(asg) = settings.script_assignment.as_mut() {
                asg.unavailable = Some(reason);
            }
        });
    }

    fn claim_legacy_for(&mut self, profile: &str, key: &str, card_name: &str) {
        let already = self
            .vault
            .as_ref()
            .and_then(|v| v.get(profile))
            .is_some_and(|p| p.settings.script_settings.contains_key(key));
        if already {
            return;
        }
        let legacy = self.script_settings.overrides(
            script::parse_source_kind(key.split(':').next().unwrap_or("catalog"))
                .unwrap_or(script::ScriptSource::Catalog),
            card_name,
        );
        self.upsert_profile_settings(profile, |settings| {
            script::claim_legacy_overrides(settings, key, card_name, &legacy);
        });
    }

    pub fn profile_overrides(
        &mut self,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        self.claim_legacy_for(profile, key, card_name);
        self.vault
            .as_ref()
            .and_then(|v| v.get(profile))
            .and_then(|p| p.settings.script_settings.get(key).cloned())
            .unwrap_or_default()
    }

    pub fn merged_profile_bag(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        schema: &[script::SettingDef],
    ) -> Map<String, Value> {
        let key = script::card_identity_key(source, path, name);
        let overrides = self.profile_overrides(profile, &key, name);
        script::merge_bag(schema, &overrides, self.script_settings_inject.as_ref())
    }

    pub fn set_profile_setting(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        id: &str,
        value: Value,
    ) -> bool {
        let key = script::card_identity_key(source, path, name);
        self.claim_legacy_for(profile, &key, name);
        let ok = self.upsert_profile_settings(profile, |settings| {
            let mut bag = settings
                .script_settings
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let migrated = script::migrate_legacy_setting_value(name, id, &value);
            bag.insert(id.to_string(), migrated);
            settings.script_settings.insert(key.clone(), bag);
        });
        if ok {
            self.push_live_settings(profile, source, name, path);
        }
        ok
    }

    pub fn push_live_settings(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
    ) {
        let (identity, generation) = {
            let Some(play) = self.play.as_ref() else {
                return;
            };
            let Some(identity) = play.script_source_identity(profile) else {
                return;
            };
            let key = script::card_identity_key(source, path, name);
            if identity != key {
                return;
            }
            let Some(generation) = play.script_runtime_generation(profile) else {
                return;
            };
            (identity, generation)
        };
        let schema = self
            .js
            .get(source, &lookup_name(source, name, path))
            .map(|c| c.settings_schema.clone())
            .unwrap_or_default();
        let bag = self.merged_profile_bag(profile, source, name, path, &schema);
        if let Some(play) = self.play.as_ref() {
            let _ = play.script_post_settings_fenced(profile, &bag, &identity, generation);
        }
    }

    /// Start the given profile using last successful assignment only.
    pub fn script_start_profile(&mut self, profile: &str) -> Result<(), String> {
        self.script_start_profile_with(profile, false)
    }

    pub(crate) fn script_start_profile_with(
        &mut self,
        profile: &str,
        prefer_pending: bool,
    ) -> Result<(), String> {
        if script_active_name(self, profile) {
            return Ok(());
        }
        let sel = if prefer_pending {
            self.pending_browse
                .get(profile)
                .cloned()
                .or_else(|| self.script_sel.clone())
                .or_else(|| {
                    self.profile_assignment(profile)
                        .and_then(|a| sel_from_assignment(&a))
                })
        } else {
            self.profile_assignment(profile)
                .and_then(|a| sel_from_assignment(&a))
        }
        .ok_or_else(|| "no assignment".to_string())?;
        self.script_start_sel(profile, sel)
    }

    fn script_start_sel(&mut self, profile: &str, sel: script::ScriptSel) -> Result<(), String> {
        if self.play.is_none() {
            return Err("no play".into());
        }
        match sel {
            script::ScriptSel::Compiled(id) => {
                {
                    let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
                    play.script_start(profile, id)?;
                    play.script_attach_identity(profile, script::compiled_identity_key(id));
                }
                self.persist_successful_assignment(profile, script::compiled_assignment(id));
                Ok(())
            }
            script::ScriptSel::Loaded(source, lookup) => {
                let card = self
                    .js
                    .get(source, &lookup)
                    .cloned()
                    .ok_or_else(|| match source {
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
                    profile,
                    source,
                    &card.name,
                    &card.path,
                    &card.settings_schema,
                );
                let bag = if bag.is_empty() { None } else { Some(bag) };
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
                let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
                if let Err(e) = play.script_start_load_typed(
                    profile,
                    card.js.clone(),
                    card.shape,
                    bag,
                    siblings,
                ) {
                    return self.js.record_start_result(&card, Err(e));
                }
                play.script_attach_identity(profile, card.identity_key());
                self.pending_starts.insert(
                    profile.to_string(),
                    PendingStart {
                        card,
                        kind: PendingStartKind::Start,
                    },
                );
                Ok(())
            }
        }
    }

    pub fn script_start_all(&mut self) {
        let members = self.wall.members.clone();
        if members.is_empty() {
            self.error = Some("Start all: no wall members".into());
            return;
        }
        let mut failures = Vec::new();
        let mut started = 0usize;
        let mut skipped = 0usize;
        for name in members {
            let state = self
                .play
                .as_ref()
                .map(|p| p.script_state(&name))
                .unwrap_or(script::RunState::Idle);
            match state {
                script::RunState::Running
                | script::RunState::Paused
                | script::RunState::Stopping
                | script::RunState::Starting => {
                    skipped += 1;
                    continue;
                }
                script::RunState::Idle | script::RunState::Error => {}
            }
            match self.script_start_profile(&name) {
                Ok(()) => {
                    started += 1;
                    if let Some(pending) = self.pending_starts.get_mut(&name) {
                        pending.kind = PendingStartKind::StartAll;
                    }
                }
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        let report = format_bulk("Start all", started, skipped, &failures);
        self.bulk_start = Some(BulkStart {
            started,
            skipped,
            failures,
        });
        if self.last_bulk_script_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_script_report = Some(report.clone());
        self.error = Some(report);
    }

    /// Commit or report every Load Start whose setup has settled since the
    /// last frame: Ready persists the assignment and clears the card's
    /// load diagnostic; a failure records it and reports it where the
    /// Start was made. Called once per UI frame.
    pub fn settle_script_starts(&mut self) {
        if self.pending_starts.is_empty() {
            return;
        }
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let mut settled = Vec::new();
        for name in self.pending_starts.keys() {
            match play.script_poll_start(name) {
                script::StartPoll::Pending => {}
                script::StartPoll::Settled(outcome) => settled.push((name.clone(), Some(outcome))),
                // The slot was removed (its Stop cancelled the Start).
                script::StartPoll::NotOwed => settled.push((name.clone(), None)),
            }
        }
        for (name, outcome) in settled {
            let Some(pending) = self.pending_starts.remove(&name) else {
                continue;
            };
            match outcome {
                Some(script::StartOutcome::Ready) => {
                    let _ = self.js.record_start_result(&pending.card, Ok(()));
                    // Persisting is bookkeeping, not an operator action: it
                    // must not clear a message shown since the click.
                    let shown = self.error.take();
                    self.persist_successful_assignment(&name, pending.card.assignment());
                    if self.error.is_none() {
                        self.error = shown;
                    }
                }
                Some(script::StartOutcome::Failed(e)) => {
                    let diagnostic = self
                        .js
                        .record_start_result(
                            &pending.card,
                            Err(script::StartLoadError::RuntimeLoad(e.clone())),
                        )
                        .err()
                        .unwrap_or(e);
                    self.report_start_failure(&name, pending.kind, diagnostic);
                }
                Some(script::StartOutcome::Cancelled) | None => {}
            }
        }
    }

    fn report_start_failure(&mut self, name: &str, kind: PendingStartKind, diagnostic: String) {
        match kind {
            PendingStartKind::Start => {
                let message = format!("script: {diagnostic}");
                if let Some(watch) = self.external_core_watch() {
                    if watch.configured() && watch.account() == name {
                        watch.fail_start(message.clone());
                    }
                }
                self.error = Some(message);
            }
            PendingStartKind::StartAll => {
                let bulk = self.bulk_start.get_or_insert_with(BulkStart::default);
                bulk.started = bulk.started.saturating_sub(1);
                bulk.failures.push(format!("{name}: {diagnostic}"));
                let report = format_bulk("Start all", bulk.started, bulk.skipped, &bulk.failures);
                self.last_bulk_script_report = Some(report.clone());
                self.error = Some(report);
            }
            PendingStartKind::Reload => {
                self.error = Some(format!("reload: {name}: {diagnostic}"));
            }
        }
    }

    pub fn script_stop_all(&mut self) {
        let mut names: Vec<String> = self.wall.members.clone();
        for name in self.slots.keys() {
            if !names.iter().any(|n| n == name) {
                names.push(name.clone());
            }
        }
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let mut stopped = 0usize;
        for name in names {
            let state = play.script_state(&name);
            if matches!(
                state,
                script::RunState::Running | script::RunState::Paused | script::RunState::Starting
            ) {
                play.script_stop(&name);
                stopped += 1;
            }
        }
        self.reload_generation = self.reload_generation.wrapping_add(1);
        self.clear_pending_reload();
        let report = format!("Stop all: stopped {stopped}");
        if self.last_bulk_script_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_script_report = Some(report.clone());
        if stopped > 0 {
            self.error = None;
        }
    }

    fn clear_pending_reload(&mut self) {
        self.reload_warning = None;
        self.pending_reload = None;
        self.catalog_refresh_confirm = false;
    }

    fn install_pending(&mut self, pending: PendingReload) {
        self.reload_warning = Some(pending.warning.clone());
        self.catalog_refresh_confirm = matches!(pending.kind, PendingReloadKind::Catalog { .. });
        self.pending_reload = Some(pending);
    }

    /// Product Reload click: preview first, confirm only a bound warning.
    pub fn script_reload_clicked(&mut self) -> ReloadOutcome {
        self.script_reload(self.manual_pending_binds_current())
    }

    pub fn script_reload_confirmation_pending(&self) -> bool {
        self.manual_pending_binds_current()
    }

    /// Discard the prepared candidate without touching any execution.
    pub fn cancel_reload(&mut self) {
        self.clear_pending_reload();
        self.error = None;
    }

    pub fn script_reload(&mut self, commit: bool) -> ReloadOutcome {
        let Some((source, lookup)) = self.current_reload_target() else {
            return ReloadOutcome::Failed("no script to reload".into());
        };
        if commit && self.manual_pending_binds(source, &lookup) {
            return self.commit_manual_pending();
        }
        self.preview_manual_reload(source, lookup)
    }

    fn current_reload_target(&self) -> Option<(script::ScriptSource, String)> {
        let name = self.focused_name()?;
        match self
            .pending_browse
            .get(&name)
            .cloned()
            .or_else(|| self.script_sel.clone())
            .or_else(|| {
                self.profile_assignment(&name)
                    .and_then(|a| sel_from_assignment(&a))
            })? {
            script::ScriptSel::Loaded(source, lookup) => Some((source, lookup)),
            script::ScriptSel::Compiled(_) => None,
        }
    }

    fn manual_pending_binds_current(&self) -> bool {
        self.current_reload_target()
            .is_some_and(|(source, lookup)| self.manual_pending_binds(source, &lookup))
    }

    fn manual_pending_binds(&self, source: script::ScriptSource, lookup: &str) -> bool {
        let Some(pending) = self.pending_reload.as_ref() else {
            return false;
        };
        if !matches!(pending.kind, PendingReloadKind::Manual) {
            return false;
        }
        let w = &pending.warning;
        if w.epoch != self.reload_generation || w.source != source {
            return false;
        }
        if !lookups_match(source, &w.lookup, lookup) {
            return false;
        }
        let Ok(fingerprint) = self.js.disk_fingerprint(source, lookup) else {
            return false;
        };
        w.fingerprint == fingerprint
    }

    fn preview_manual_reload(
        &mut self,
        source: script::ScriptSource,
        lookup: String,
    ) -> ReloadOutcome {
        match self.js.raw_source_changed(source, &lookup) {
            Ok(false) => {
                self.error = Some(script::NOTHING_CHANGED_RELOAD.into());
                if matches!(
                    self.pending_reload.as_ref().map(|p| &p.kind),
                    Some(PendingReloadKind::Manual)
                ) {
                    self.clear_pending_reload();
                }
                return ReloadOutcome::NothingChanged;
            }
            Ok(true) => {}
            Err(e) => {
                self.error = Some(format!("reload: {e}"));
                return ReloadOutcome::Failed(e);
            }
        }
        let key = self
            .js
            .get(source, &lookup)
            .map(|c| c.identity_key())
            .unwrap_or_default();
        let fingerprint = match self.js.disk_fingerprint(source, &lookup) {
            Ok(fp) => fp,
            Err(e) => {
                self.error = Some(format!("reload: {e}"));
                return ReloadOutcome::Failed(e);
            }
        };
        let (running, paused) = self.slots_with_identity(&key);
        let epoch = self.reload_generation;
        let prepared = match self.js.prepare_card(source, &lookup) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(format!("reload: {e}"));
                return ReloadOutcome::Failed(e);
            }
        };
        if self.reload_generation != epoch {
            return ReloadOutcome::Failed("reload cancelled".into());
        }
        let (running_now, paused_now) = self.slots_with_identity(&key);
        let paused_during: Vec<String> = paused_now
            .iter()
            .filter(|n| running.iter().any(|r| r == *n))
            .cloned()
            .collect();
        let warning = ReloadWarning {
            identity_key: key,
            source,
            lookup,
            fingerprint,
            epoch,
            running: running_now,
            paused: paused_now,
            paused_during_prep: paused_during,
            affected_generations: self.generations_for(
                &running
                    .iter()
                    .chain(paused.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        };
        let needs_warn = !warning.running.is_empty()
            || !warning.paused.is_empty()
            || !warning.paused_during_prep.is_empty();
        if needs_warn {
            self.error = Some(reload_warning_text(&warning));
            self.install_pending(PendingReload {
                warning,
                prepared: vec![prepared],
                kind: PendingReloadKind::Manual,
            });
            return ReloadOutcome::NeedsConfirm;
        }
        self.apply_prepared_reload(vec![prepared], warning)
    }

    fn commit_manual_pending(&mut self) -> ReloadOutcome {
        let Some(mut pending) = self.pending_reload.take() else {
            return ReloadOutcome::Failed("no pending reload".into());
        };
        if pending.warning.epoch != self.reload_generation {
            self.clear_pending_reload();
            return ReloadOutcome::Failed("reload cancelled".into());
        }
        if let Some(paused_during) = self.newly_paused_during(&pending.warning) {
            pending.warning.paused_during_prep = paused_during;
            let (running_now, paused_now) = self.slots_with_identity(&pending.warning.identity_key);
            pending.warning.running = running_now;
            pending.warning.paused = paused_now;
            self.error = Some(reload_warning_text(&pending.warning));
            self.install_pending(pending);
            return ReloadOutcome::NeedsConfirm;
        }
        self.apply_prepared_reload(pending.prepared, pending.warning)
    }

    fn newly_paused_during(&self, warning: &ReloadWarning) -> Option<Vec<String>> {
        let mut found = Vec::new();
        for name in &warning.running {
            let paused_now = self
                .play
                .as_ref()
                .is_some_and(|p| p.script_state(name) == script::RunState::Paused);
            if paused_now && !warning.paused_during_prep.iter().any(|n| n == name) {
                found.push(name.clone());
            }
        }
        if found.is_empty() {
            None
        } else {
            Some(found)
        }
    }

    fn apply_prepared_reload(
        &mut self,
        prepared: Vec<script::PreparedCard>,
        warning: ReloadWarning,
    ) -> ReloadOutcome {
        for item in &prepared {
            if let Err(e) = self.js.commit_prepared(item.clone()) {
                self.error = Some(format!("reload: {e}"));
                self.install_pending(PendingReload {
                    warning,
                    prepared,
                    kind: PendingReloadKind::Manual,
                });
                return ReloadOutcome::Failed(e);
            }
        }
        let (restarted, stopped_paused, failed, errors) =
            self.replace_prepared_slots(&prepared, &warning);
        self.clear_pending_reload();
        if failed > 0 {
            self.error = Some(format_reload_failures(restarted, stopped_paused, &errors));
        } else {
            self.error = None;
        }
        ReloadOutcome::Applied {
            restarted,
            stopped_paused,
            failed,
        }
    }

    /// Restart one replaced slot on the prepared card. The caller has just
    /// stopped it, so the slot is usually still reaping (`Stopping`): the
    /// slot queues this Start behind the reap. `Ok` means the Start was
    /// accepted; its setup settles in [`Self::settle_script_starts`].
    fn script_start_prepared(
        &mut self,
        profile: &str,
        prepared: &script::PreparedCard,
    ) -> Result<(), String> {
        if self.play.is_none() {
            return Err("no play".into());
        }
        let card = &prepared.card;
        let bag = self.merged_profile_bag(
            profile,
            card.source,
            &card.name,
            &card.path,
            &card.settings_schema,
        );
        let bag = if bag.is_empty() { None } else { Some(bag) };
        #[cfg(test)]
        if self.fail_reload_start_for.as_deref() == Some(profile) {
            return Err("injected start failure".into());
        }
        let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
        if let Err(e) = play.script_start_load_typed(
            profile,
            card.js.clone(),
            card.shape,
            bag,
            prepared.siblings.clone(),
        ) {
            return self.js.record_start_result(card, Err(e));
        }
        play.script_attach_identity(profile, card.identity_key());
        self.pending_starts.insert(
            profile.to_string(),
            PendingStart {
                card: card.clone(),
                kind: PendingStartKind::Reload,
            },
        );
        Ok(())
    }

    fn replace_prepared_slots(
        &mut self,
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
            if self.reload_generation != warning.epoch {
                errors.push("reload cancelled".into());
                failed += 1;
                break;
            }
            let Some(play) = self.play.as_ref() else {
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
            let Some(now_gen) = play.script_runtime_generation(&slot_name) else {
                continue;
            };
            if now_gen != warned_gen {
                continue;
            }
            if self.reload_logout_pending(&slot_name) {
                continue;
            }
            let assignment_matches = self
                .profile_assignment(&slot_name)
                .is_some_and(|a| a.key() == prepared_card.card.identity_key());
            if !assignment_matches {
                continue;
            }
            let state = play.script_state(&slot_name);
            match state {
                script::RunState::Running | script::RunState::Starting => {
                    if !play.script_stop_if_identity_generation(&slot_name, &live_key, warned_gen) {
                        continue;
                    }
                    match self.script_start_prepared(&slot_name, prepared_card) {
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
                script::RunState::Paused => {}
                _ => {}
            }
        }
        (restarted, stopped_paused, failed, errors)
    }

    fn reload_logout_pending(&self, name: &str) -> bool {
        if self.wall.latch.contains(name) {
            return true;
        }
        self.play
            .as_ref()
            .and_then(|p| p.arm(name))
            .is_some_and(|arm| arm.wants_logout())
    }

    fn slots_with_identity(&self, key: &str) -> (Vec<String>, Vec<String>) {
        let mut running = Vec::new();
        let mut paused = Vec::new();
        let Some(play) = self.play.as_ref() else {
            return (running, paused);
        };
        for name in self.slot_names() {
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

    fn slot_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.wall.members.clone();
        for name in self.slots.keys() {
            if !names.iter().any(|n| n == name) {
                names.push(name.clone());
            }
        }
        names
    }

    fn generations_for(&self, names: &[String]) -> Vec<(String, u64)> {
        let Some(play) = self.play.as_ref() else {
            return Vec::new();
        };
        names
            .iter()
            .filter_map(|n| play.script_runtime_generation(n).map(|g| (n.clone(), g)))
            .collect()
    }

    /// Re-read the configured catalog. Does not rewrite the persisted path.
    pub fn refresh_catalog(&mut self) {
        let root = match self.catalog_root() {
            Ok(Some(root)) => root,
            Ok(None) => {
                self.error = Some("Refresh catalog: no catalog configured".into());
                return;
            }
            Err(e) => {
                self.error = Some(format!("Refresh catalog: {e}"));
                return;
            }
        };
        self.refresh_catalog_at(&root);
    }

    pub(crate) fn refresh_catalog_at(&mut self, root: &Path) {
        if self.catalog_pending_binds(root) {
            match self.commit_catalog_pending() {
                ReloadOutcome::NeedsConfirm => return,
                ReloadOutcome::Failed(e) => {
                    self.error = Some(format!("Refresh catalog: {e}"));
                    return;
                }
                ReloadOutcome::Applied { .. } | ReloadOutcome::NothingChanged => return,
            }
        }
        let diff = match self.js.diff_catalog(root) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(format!("Refresh catalog: {e}"));
                return;
            }
        };
        if diff.is_noop() {
            self.catalog_refresh_report = Some(script::NOTHING_CHANGED_CATALOG.into());
            self.error = Some(script::NOTHING_CHANGED_CATALOG.into());
            return;
        }
        let changed = diff.changed.clone();
        let removed = diff.removed.clone();
        let added = diff.added.clone();
        for name in &added {
            self.enqueue_transpile(script::ScriptSource::Catalog, name.clone(), false);
        }
        let mut prepared_ok = Vec::new();
        let mut prepare_failed: Vec<script::LoadFailure> = Vec::new();
        for name in &changed {
            match self.js.prepare_card(script::ScriptSource::Catalog, name) {
                Ok(prepared) => prepared_ok.push(prepared),
                Err(e) => {
                    if let Some(card) = self.js.get(script::ScriptSource::Catalog, name) {
                        if let Some(failure) = self.js.load_failure(&card.identity_key()).cloned() {
                            prepare_failed.push(failure);
                        }
                    }
                    self.error = Some(format!("catalog {name}: {e}"));
                }
            }
        }
        let mut running = Vec::new();
        let mut paused = Vec::new();
        for prepared in &prepared_ok {
            let (r, p) = self.slots_with_identity(&prepared.card.identity_key());
            running.extend(r);
            paused.extend(p);
        }
        let set_fingerprint = catalog_set_fingerprint(&added, &changed, &removed, &prepared_ok);
        let epoch = self.reload_generation;
        let warning = ReloadWarning {
            identity_key: prepared_ok
                .first()
                .map(|p| p.card.identity_key())
                .unwrap_or_default(),
            source: script::ScriptSource::Catalog,
            lookup: changed.first().cloned().unwrap_or_default(),
            fingerprint: set_fingerprint.clone(),
            epoch,
            running: running.clone(),
            paused: paused.clone(),
            paused_during_prep: Vec::new(),
            affected_generations: self.generations_for(
                &running
                    .iter()
                    .chain(paused.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        };
        if !running.is_empty() || !paused.is_empty() {
            self.error = Some(reload_warning_text(&warning));
            self.install_pending(PendingReload {
                warning,
                prepared: prepared_ok,
                kind: PendingReloadKind::Catalog {
                    root: root.to_path_buf(),
                    added,
                    removed,
                    failed: prepare_failed,
                    set_fingerprint,
                },
            });
            return;
        }
        self.apply_catalog_prepared(root, diff, prepared_ok, prepare_failed, warning);
    }

    fn catalog_pending_binds(&self, root: &Path) -> bool {
        let Some(pending) = self.pending_reload.as_ref() else {
            return false;
        };
        let PendingReloadKind::Catalog {
            root: pending_root,
            added,
            removed,
            set_fingerprint,
            ..
        } = &pending.kind
        else {
            return false;
        };
        if pending.warning.epoch != self.reload_generation || pending_root != root {
            return false;
        }
        let Ok(diff) = self.js.diff_catalog(root) else {
            return false;
        };
        let mut fps = Vec::new();
        for prepared in &pending.prepared {
            match self
                .js
                .disk_fingerprint(script::ScriptSource::Catalog, &prepared.card.name)
            {
                Ok(fp) => fps.push((prepared.card.name.clone(), fp)),
                Err(_) => return false,
            }
        }
        let current =
            catalog_set_fingerprint_from_diff(&diff.added, &diff.changed, &diff.removed, &fps);
        current == *set_fingerprint && diff.added == *added && diff.removed == *removed
    }

    fn commit_catalog_pending(&mut self) -> ReloadOutcome {
        let Some(mut pending) = self.pending_reload.take() else {
            return ReloadOutcome::Failed("no pending catalog refresh".into());
        };
        let PendingReloadKind::Catalog { root, .. } = pending.kind.clone() else {
            self.pending_reload = Some(pending);
            return ReloadOutcome::Failed("pending reload is not catalog".into());
        };
        if pending.warning.epoch != self.reload_generation {
            self.clear_pending_reload();
            return ReloadOutcome::Failed("reload cancelled".into());
        }
        if let Some(paused_during) = self.newly_paused_during(&pending.warning) {
            pending.warning.paused_during_prep = paused_during;
            let mut running = Vec::new();
            let mut paused = Vec::new();
            for prepared in &pending.prepared {
                let (r, p) = self.slots_with_identity(&prepared.card.identity_key());
                running.extend(r);
                paused.extend(p);
            }
            pending.warning.running = running;
            pending.warning.paused = paused;
            self.error = Some(reload_warning_text(&pending.warning));
            self.install_pending(pending);
            return ReloadOutcome::NeedsConfirm;
        }
        let diff = match self.js.diff_catalog(&root) {
            Ok(d) => d,
            Err(e) => return ReloadOutcome::Failed(e),
        };
        let prepared = pending.prepared;
        let failed = match pending.kind {
            PendingReloadKind::Catalog { failed, .. } => failed,
            PendingReloadKind::Manual => Vec::new(),
        };
        self.apply_catalog_prepared(&root, diff, prepared, failed, pending.warning);
        ReloadOutcome::Applied {
            restarted: 0,
            stopped_paused: 0,
            failed: 0,
        }
    }

    fn apply_catalog_prepared(
        &mut self,
        root: &Path,
        mut diff: script::CatalogDiff,
        prepared: Vec<script::PreparedCard>,
        prepare_failed: Vec<script::LoadFailure>,
        warning: ReloadWarning,
    ) {
        let removed = diff.removed.clone();
        let mut changed_ok = 0usize;
        for item in &prepared {
            let name = item.card.name.clone();
            if let Err(e) = self.js.commit_prepared(item.clone()) {
                self.error = Some(format!("catalog {name}: {e}"));
                continue;
            }
            changed_ok += 1;
        }
        diff.changed.clear();
        let mut report = self.js.apply_catalog_diff(diff);
        report.changed = changed_ok;
        report.failed.extend(prepare_failed);
        for name in &removed {
            self.mark_removed_catalog_assignments(name);
        }
        let (_restarted, _stopped_paused, failed, errors) =
            self.replace_prepared_slots(&prepared, &warning);
        let mut summary = report.summary();
        if failed > 0 {
            summary = format!("{summary}, start failed {}: {}", failed, errors.join("; "));
        }
        let named = self.js.named_failure_output();
        if !named.is_empty() {
            summary = format!("{summary}\n{named}");
        }
        self.catalog_refresh_report = Some(summary.clone());
        self.error = Some(format!("Refresh catalog: {summary}"));
        self.clear_pending_reload();
        let _ = root;
    }

    fn mark_removed_catalog_assignments(&mut self, card_name: &str) {
        let key =
            script::card_identity_key(script::ScriptSource::Catalog, Path::new(""), card_name);
        let names: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        for name in names {
            let matches = self
                .profile_assignment(&name)
                .is_some_and(|a| a.key() == key);
            if matches {
                self.mark_assignment_unavailable(&name, format!("catalog removed: {card_name}"));
            }
        }
    }
}

fn lookups_match(source: script::ScriptSource, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if source == script::ScriptSource::File {
        script::paths_match(a, Path::new(b)) || script::paths_match(b, Path::new(a))
    } else {
        false
    }
}

fn catalog_set_fingerprint(
    added: &[String],
    changed: &[String],
    removed: &[String],
    prepared: &[script::PreparedCard],
) -> String {
    let fps: Vec<(String, String)> = prepared
        .iter()
        .map(|p| (p.card.name.clone(), p.fingerprint.clone()))
        .collect();
    catalog_set_fingerprint_from_diff(added, changed, removed, &fps)
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

fn sel_from_assignment(asg: &ScriptAssignment) -> Option<script::ScriptSel> {
    if asg.source_kind == "compiled" {
        return script::compiled_ids()
            .iter()
            .copied()
            .find(|id| id.0 == asg.identity)
            .map(script::ScriptSel::Compiled);
    }
    let source = script::parse_source_kind(&asg.source_kind)?;
    Some(script::ScriptSel::Loaded(source, asg.identity.clone()))
}

fn lookup_name(source: script::ScriptSource, name: &str, path: &Path) -> String {
    match source {
        script::ScriptSource::File => path.to_string_lossy().into_owned(),
        _ => name.to_string(),
    }
}

fn script_active_name(session: &Session, name: &str) -> bool {
    matches!(
        session
            .play
            .as_ref()
            .map(|p| p.script_state(name))
            .unwrap_or(script::RunState::Idle),
        script::RunState::Starting
            | script::RunState::Running
            | script::RunState::Paused
            | script::RunState::Stopping
    )
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

fn reload_warning_text(w: &ReloadWarning) -> String {
    let mut parts = vec![format!(
        "reload {} will replace running bots",
        w.identity_key
    )];
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
    parts.join(" — ")
}

#[cfg(test)]
#[path = "profile_script_tests.rs"]
mod tests;

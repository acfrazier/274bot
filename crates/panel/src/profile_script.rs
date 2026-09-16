//! Per-profile script assignment, bulk Start/Stop, live settings, reload
//! and catalog refresh. Session is the integration owner.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

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
                {
                    let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
                    play.script_start_load(profile, card.js.clone(), card.shape, bag, siblings)?;
                    play.script_attach_identity(profile, card.identity_key());
                }
                self.persist_successful_assignment(profile, card.assignment());
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
                | script::RunState::Stopping => {
                    skipped += 1;
                    continue;
                }
                script::RunState::Idle | script::RunState::Error => {}
            }
            match self.script_start_profile(&name) {
                Ok(()) => started += 1,
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        let report = format_bulk("Start all", started, skipped, &failures);
        if self.last_bulk_script_report.as_deref() == Some(report.as_str()) {
            return;
        }
        self.last_bulk_script_report = Some(report.clone());
        self.error = Some(report);
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
            if matches!(state, script::RunState::Running | script::RunState::Paused) {
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

    fn script_start_prepared(
        &mut self,
        profile: &str,
        prepared: &script::PreparedCard,
    ) -> Result<(), String> {
        if script_active_name(self, profile) {
            return Ok(());
        }
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
        {
            let play = self.play.as_ref().ok_or_else(|| "no play".to_string())?;
            play.script_start_load(
                profile,
                card.js.clone(),
                card.shape,
                bag,
                prepared.siblings.clone(),
            )?;
            play.script_attach_identity(profile, card.identity_key());
        }
        self.persist_successful_assignment(profile, card.assignment());
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
                script::RunState::Running => {
                    play.script_stop(&slot_name);
                    match self.script_start_prepared(&slot_name, prepared_card) {
                        Ok(()) => restarted += 1,
                        Err(e) => {
                            failed += 1;
                            errors.push(format!("{slot_name}: {e}"));
                        }
                    }
                }
                script::RunState::Paused => {
                    play.script_stop(&slot_name);
                    stopped_paused += 1;
                }
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
            .is_some_and(|arm| arm.want_logout.load(Ordering::Relaxed))
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
                script::RunState::Running => running.push(name),
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
        script::RunState::Running | script::RunState::Paused | script::RunState::Stopping
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
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};
    use vault::{Profile, ProfileSettings, Vault};

    fn tmp(name: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "274bot-p2-{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session_with_profiles(names: &[&str]) -> (Session, std::path::PathBuf) {
        script::IsolatedEnv::ensure_thread();
        let dir = tmp("vault");
        let path = dir.join("v.vault");
        let mut vault = Vault::create(&path, "bot").unwrap();
        for (i, name) in names.iter().enumerate() {
            vault
                .upsert(Profile {
                    username: (*name).into(),
                    password: "pw".into(),
                    uid: 1 + i as i32,
                    settings: ProfileSettings::default(),
                })
                .unwrap();
        }
        let mut s = Session::new();
        s.vault = Some(vault);
        s.persist_ui = false;
        (s, dir)
    }

    #[test]
    fn focus_restores_assignment_not_other_profile_pending() {
        let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
        s.set_pending_browse(
            "alice",
            script::ScriptSel::Loaded(script::ScriptSource::Catalog, "ChickenKiller".into()),
        );
        s.persist_successful_assignment(
            "bob",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "Alcher".into(),
                display_name: "Alcher".into(),
                unavailable: None,
            },
        );
        s.restore_script_heading("alice");
        assert_eq!(
            s.script_sel,
            Some(script::ScriptSel::Loaded(
                script::ScriptSource::Catalog,
                "ChickenKiller".into()
            ))
        );
        s.restore_script_heading("bob");
        assert_eq!(
            s.script_sel,
            Some(script::ScriptSel::Loaded(
                script::ScriptSource::Catalog,
                "Alcher".into()
            ))
        );
        s.focus.lock().unwrap().focused = Some("bob".into());
        assert!(!s.heading_is_pending());
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.restore_script_heading("alice");
        assert!(s.heading_is_pending());
    }

    #[test]
    fn two_profiles_keep_isolated_settings_after_legacy_claim() {
        let (mut s, dir) = session_with_profiles(&["alice", "bob"]);
        let store = dir.join("script-settings.json");
        fs::write(&store, r#"{"catalog:SmithingBot":{"bar":"Adamant"}}"#).unwrap();
        s.script_settings = script::ScriptSettingsStore::at(store);
        let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
        assert_eq!(a.get("bar"), Some(&Value::String("Adamantite".into())));
        s.set_profile_setting(
            "alice",
            script::ScriptSource::Catalog,
            "SmithingBot",
            Path::new(""),
            "bar",
            Value::String("Iron".into()),
        );
        let b = s.profile_overrides("bob", "catalog:SmithingBot", "SmithingBot");
        assert_eq!(b.get("bar"), Some(&Value::String("Adamantite".into())));
        let a = s.profile_overrides("alice", "catalog:SmithingBot", "SmithingBot");
        assert_eq!(a.get("bar"), Some(&Value::String("Iron".into())));
    }

    #[test]
    fn missing_file_assignment_is_kept_not_substituted() {
        let (mut s, _dir) = session_with_profiles(&["alice"]);
        s.persist_successful_assignment(
            "alice",
            script::missing_file_assignment(
                Path::new("/tmp/missing-bot.ts"),
                "bot",
                "missing file: /tmp/missing-bot.ts",
            ),
        );
        let asg = s.profile_assignment("alice").unwrap();
        assert_eq!(asg.identity, "/tmp/missing-bot.ts");
        assert!(asg.unavailable.is_some());
        assert_ne!(asg.display_name, asg.identity);
        s.restore_script_heading("alice");
        match &s.script_sel {
            Some(script::ScriptSel::Loaded(script::ScriptSource::File, id)) => {
                assert_eq!(id, "/tmp/missing-bot.ts");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn start_all_without_play_reports_per_profile_and_is_idempotent() {
        let (mut s, _dir) = session_with_profiles(&["alice", "bob"]);
        s.wall.load("alice");
        s.wall.load("bob");
        s.script_start_all();
        let first = s.error.clone().unwrap();
        assert!(first.contains("Start all"));
        assert!(first.contains("failed"));
        s.script_start_all();
        assert_eq!(s.error.as_deref(), Some(first.as_str()));
    }

    #[test]
    fn same_stem_files_are_distinct_and_hash_whitespace() {
        script::IsolatedEnv::ensure_thread();
        let dir = tmp("files");
        let a = dir.join("one");
        let b = dir.join("two");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let fa = a.join("bot.ts");
        let fb = b.join("bot.ts");
        let src = "export default class T extends LoopingBot { override loop() {} }\n";
        fs::write(&fa, src).unwrap();
        fs::write(&fb, src).unwrap();
        let mut lib = script::JsLibrary::with_cache(dir.join("store.json"), dir.join("cache"));
        let ca = lib.load(&fa).unwrap();
        let cb = lib.load(&fb).unwrap();
        assert_ne!(ca.identity_key(), cb.identity_key());
        assert_eq!(ca.name, cb.name);
        assert!(!lib
            .raw_source_changed(script::ScriptSource::File, &fa.to_string_lossy())
            .unwrap());
        fs::write(&fa, format!("{src} ")).unwrap();
        assert!(lib
            .raw_source_changed(script::ScriptSource::File, &fa.to_string_lossy())
            .unwrap());
        assert!(!lib
            .raw_source_changed(script::ScriptSource::File, &fb.to_string_lossy())
            .unwrap());
    }

    #[test]
    fn catalog_diff_noop_and_remove_keep_assignment() {
        let (mut s, _dir) = session_with_profiles(&["alice"]);
        s.persist_successful_assignment(
            "alice",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "GoneBot".into(),
                display_name: "GoneBot".into(),
                unavailable: None,
            },
        );
        s.mark_removed_catalog_assignments("GoneBot");
        let asg = s.profile_assignment("alice").unwrap();
        assert_eq!(asg.identity, "GoneBot");
        assert!(asg.unavailable.unwrap().contains("catalog removed"));
    }

    const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

    fn empty_play() -> host_play::Play {
        host_play::run_with_io(
            &host_play::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _, _| {},
        )
    }

    fn session_with_play(names: &[&str]) -> (Session, std::path::PathBuf) {
        let (mut s, dir) = session_with_profiles(names);
        s.js = script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
        let mut play = empty_play();
        for name in names {
            play.attach_arm(name, host_play::SlotArm::new(42, false));
            s.wall.load(name);
        }
        s.play = Some(play);
        if let Some(first) = names.first() {
            s.focus.lock().unwrap().focused = Some((*first).into());
        }
        (s, dir)
    }

    fn write_bot(dir: &std::path::Path, file: &str, src: &str) -> std::path::PathBuf {
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, src).unwrap();
        path
    }

    fn fake_catalog(root: &std::path::Path, bots: &[(&str, &str)]) {
        let scripts = root.join("src/bot/scripts");
        fs::create_dir_all(&scripts).unwrap();
        let mut index =
            String::from("import { ScriptRegistry } from '../runtime/ScriptRegistry.js';\n");
        for (name, src) in bots {
            let folder = name.replace(' ', "");
            index.push_str(&format!("import {folder} from './{folder}/{folder}.js';\n"));
            let d = scripts.join(&folder);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join(format!("{folder}.ts")), src).unwrap();
        }
        for (name, _) in bots {
            let folder = name.replace(' ', "");
            index.push_str(&format!(
                "ScriptRegistry.register({{ name: '{name}', description: '{name}', category: 'Test', create: () => new {folder}() }});\n"
            ));
        }
        fs::write(scripts.join("index.ts"), index).unwrap();
    }

    #[test]
    fn claim_legacy_read_path_writes_vault_once() {
        let (mut s, dir) = session_with_profiles(&["alice"]);
        let vault_path = dir.join("v.vault");
        let before = fs::read(&vault_path).unwrap();
        let _ = s.profile_overrides("alice", "catalog:Alcher", "Alcher");
        let after_claim = fs::read(&vault_path).unwrap();
        assert_ne!(
            before, after_claim,
            "first claim may persist the migrated key"
        );
        let _ = s.merged_profile_bag(
            "alice",
            script::ScriptSource::Catalog,
            "Alcher",
            Path::new(""),
            &[],
        );
        let _ = s.profile_overrides("alice", "catalog:Alcher", "Alcher");
        let after_reads = fs::read(&vault_path).unwrap();
        assert_eq!(
            after_claim, after_reads,
            "already-migrated read path must not rewrite the vault"
        );
    }

    #[test]
    fn external_ts_load_start_stop_persists_assignment_only_on_success() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "ext.ts", BOT_TS);
        assert!(s.profile_assignment("alice").is_none());
        s.load_js(&path);
        assert_eq!(s.error, None, "{:?}", s.error);
        assert!(s.heading_is_pending());
        assert!(s.profile_assignment("alice").is_none());
        s.enqueue_transpile(
            script::ScriptSource::File,
            path.to_string_lossy().into_owned(),
            true,
        );
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        let play = s.play.as_ref().unwrap();
        assert_eq!(play.script_state("alice"), script::RunState::Running);
        let asg = s
            .profile_assignment("alice")
            .expect("success-only assignment");
        assert!(asg.identity.contains("ext.ts"), "{}", asg.identity);
        assert!(asg.unavailable.is_none());
        play.script_stop("alice");
        assert_eq!(play.script_state("alice"), script::RunState::Idle);
        assert_eq!(
            s.profile_assignment("alice").unwrap().identity,
            asg.identity,
            "stop keeps last successful assignment"
        );
    }

    #[test]
    fn load_js_selects_without_auto_start_and_same_path_does_not_duplicate() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "once.ts", BOT_TS);
        s.load_js(&path);
        s.enqueue_transpile(
            script::ScriptSource::File,
            path.to_string_lossy().into_owned(),
            true,
        );
        assert_eq!(s.error, None, "{:?}", s.error);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Idle,
            "load must not Start"
        );
        let first =
            s.js.cards()
                .iter()
                .filter(|c| c.source == script::ScriptSource::File)
                .count();
        assert_eq!(first, 1);
        s.load_js(&path);
        let again =
            s.js.cards()
                .iter()
                .filter(|c| c.source == script::ScriptSource::File)
                .count();
        assert_eq!(again, 1, "same path must replace, not duplicate");
        s.script_start_selected();
        s.play.as_ref().unwrap().script_stop("alice");
        assert_eq!(s.script_reload_clicked(), ReloadOutcome::NothingChanged);
    }

    #[test]
    fn reload_unchanged_reports_exact_string() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "same.ts", BOT_TS);
        s.load_js(&path);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        let out = s.script_reload_clicked();
        assert_eq!(out, ReloadOutcome::NothingChanged);
        assert_eq!(s.error.as_deref(), Some(script::NOTHING_CHANGED_RELOAD));
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_clicked_warns_before_replacing_running() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "run.ts", BOT_TS);
        s.load_js(&path);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        let old_js =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .js
                .clone();
        fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
        let first = s.script_reload_clicked();
        assert_eq!(first, ReloadOutcome::NeedsConfirm);
        assert!(
            s.error.as_deref().unwrap_or("").contains("running"),
            "{:?}",
            s.error
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        let now_js =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .js
                .clone();
        assert_eq!(old_js, now_js, "preview must not replace registration");
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_commit_gates_pause_during_prep() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "pause.ts", BOT_TS);
        s.load_js(&path);
        s.script_start_selected();
        fs::write(&path, format!("{BOT_TS}// changed\n")).unwrap();
        assert_eq!(s.script_reload(false), ReloadOutcome::NeedsConfirm);
        s.play.as_ref().unwrap().script_pause("alice");
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Paused
        );
        let old_js =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .js
                .clone();
        let commit = s.script_reload(true);
        assert_eq!(commit, ReloadOutcome::NeedsConfirm);
        assert!(
            s.error
                .as_deref()
                .unwrap_or("")
                .contains("paused during prepare"),
            "{:?}",
            s.error
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Paused
        );
        let now_js =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .js
                .clone();
        assert_eq!(old_js, now_js);
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn prepare_failure_preserves_old_instance() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "keep.ts", BOT_TS);
        s.load_js(&path);
        s.script_start_selected();
        let old =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .clone();
        fs::write(&path, "const x = 1;\n").unwrap();
        let out = s.script_reload(true);
        match out {
            ReloadOutcome::Failed(e) => assert!(
                e.contains("shape") || e.contains("unloadable") || e.contains("prepare"),
                "{e}"
            ),
            other => panic!("expected Failed, got {other:?}"),
        }
        let now =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap();
        assert_eq!(now.origin, old.origin);
        assert_eq!(now.js, old.js);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn live_settings_reach_running_and_paused() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let src = "export default class T extends LoopingBot { override loop() {} }\n";
        let path = write_bot(&dir, "live.ts", src);
        s.load_js(&path);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        assert!(s.set_profile_setting(
            "alice",
            script::ScriptSource::File,
            "live",
            &path,
            "n",
            Value::String("2".into()),
        ));
        let play = s.play.as_ref().unwrap();
        let identity = play.script_source_identity("alice").unwrap();
        let gen = play.script_runtime_generation("alice").unwrap();
        let mut bag = serde_json::Map::new();
        bag.insert("n".into(), Value::String("2".into()));
        assert!(
            !play.script_post_settings_fenced("alice", &bag, &identity, gen),
            "unchanged bag after live Running delivery is not reposted"
        );
        play.script_pause("alice");
        assert!(s.set_profile_setting(
            "alice",
            script::ScriptSource::File,
            "live",
            &path,
            "n",
            Value::String("3".into()),
        ));
        bag.insert("n".into(), Value::String("3".into()));
        let play = s.play.as_ref().unwrap();
        assert!(
            !play.script_post_settings_fenced("alice", &bag, &identity, gen),
            "unchanged bag after live Paused delivery is not reposted"
        );
        play.script_stop("alice");
        assert!(!play.script_post_settings_fenced("alice", &bag, &identity, gen));
    }

    #[test]
    fn catalog_prepare_failure_does_not_mutate_or_block_valid() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let root = dir.join("catalog");
        fake_catalog(&root, &[("GoodBot", BOT_TS), ("BadBot", BOT_TS)]);
        s.js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "GoodBot")
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "BadBot")
            .unwrap();
        let good_js =
            s.js.get(script::ScriptSource::Catalog, "GoodBot")
                .unwrap()
                .js
                .clone();
        let bad =
            s.js.get(script::ScriptSource::Catalog, "BadBot")
                .unwrap()
                .clone();
        fs::write(
            root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
            "export default class T extends LoopingBot { override loop() { return; } }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/bot/scripts/BadBot/BadBot.ts"),
            "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { override loop() {} }\n",
        )
        .unwrap();
        s.refresh_catalog_at(&root);
        let good = s.js.get(script::ScriptSource::Catalog, "GoodBot").unwrap();
        assert!(!good.js.is_empty(), "successful prepare must keep js");
        assert_ne!(good.js, good_js, "changed good card commits prepared js");
        let bad_now = s.js.get(script::ScriptSource::Catalog, "BadBot").unwrap();
        assert_eq!(bad_now.origin, bad.origin);
        assert_eq!(bad_now.js, bad.js);
        assert!(
            s.error.as_deref().unwrap_or("").contains("BadBot")
                || s.catalog_refresh_report
                    .as_deref()
                    .unwrap_or("")
                    .contains("failed"),
            "independent failure is reported: {:?} {:?}",
            s.error,
            s.catalog_refresh_report
        );
        assert!(
            s.js.load_failures().iter().any(|f| f.name == "BadBot"),
            "failed catalog card stays inspectable: {:?}",
            s.js.load_failures()
        );
    }

    #[test]
    fn catalog_mixed_batch_keeps_two_failures_after_success() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let root = dir.join("catalog-mixed");
        fake_catalog(
            &root,
            &[
                ("GoodBot", BOT_TS),
                ("BadParse", BOT_TS),
                ("BadImport", BOT_TS),
            ],
        );
        s.js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "GoodBot")
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "BadParse")
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "BadImport")
            .unwrap();
        fs::write(
            root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
            "export default class T extends LoopingBot { override loop() { return; } }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/bot/scripts/BadParse/BadParse.ts"),
            "export default class T extends LoopingBot { override loop() { const x = \"unterminated } }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/bot/scripts/BadImport/BadImport.ts"),
            "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { override loop() {} }\n",
        )
        .unwrap();
        s.refresh_catalog_at(&root);
        let names: Vec<_> =
            s.js.load_failures()
                .iter()
                .map(|f| f.name.as_str())
                .collect();
        assert!(
            names.contains(&"BadParse") && names.contains(&"BadImport"),
            "both failures stay inspectable: {names:?} err={:?} report={:?}",
            s.error,
            s.catalog_refresh_report
        );
        assert!(!names.contains(&"GoodBot"));
        assert!(s.js.named_failure_output().contains("BadParse"));
        assert!(s
            .catalog_refresh_report
            .as_deref()
            .unwrap_or("")
            .contains("BadParse"));
    }

    #[test]
    fn catalog_refresh_warns_before_stopping_running() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let root = dir.join("catalog-run");
        fake_catalog(&root, &[("RunBot", BOT_TS)]);
        s.js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "RunBot")
            .unwrap();
        s.persist_successful_assignment(
            "alice",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "RunBot".into(),
                display_name: "RunBot".into(),
                unavailable: None,
            },
        );
        s.script_sel = Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "RunBot".into(),
        ));
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        fs::write(
            root.join("src/bot/scripts/RunBot/RunBot.ts"),
            format!("{BOT_TS}// changed\n"),
        )
        .unwrap();
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running,
            "catalog refresh must warn before stopping"
        );
        assert!(
            s.reload_warning.is_some() || s.error.as_deref().unwrap_or("").contains("running"),
            "{:?} {:?}",
            s.error,
            s.reload_warning.as_ref().map(|w| &w.running)
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    fn focus_profile(s: &mut Session, name: &str) {
        s.focus.lock().unwrap().focused = Some(name.into());
        s.restore_script_heading(name);
    }

    fn start_file_on(s: &mut Session, profile: &str, path: &std::path::Path) {
        focus_profile(s, profile);
        let sel = script::ScriptSel::Loaded(
            script::ScriptSource::File,
            path.to_string_lossy().into_owned(),
        );
        s.script_sel = Some(sel.clone());
        s.set_pending_browse(profile, sel);
        s.script_start_selected();
        assert_eq!(s.error, None, "{profile} start: {:?}", s.error);
    }

    fn warn_shared_reload(s: &mut Session, path: &std::path::Path) {
        fs::write(path, format!("{BOT_TS}// changed\n")).unwrap();
        focus_profile(s, "alice");
        assert_eq!(s.script_reload_clicked(), ReloadOutcome::NeedsConfirm);
    }

    fn assert_applied(out: ReloadOutcome, restarted: usize, failed: usize) {
        match out {
            ReloadOutcome::Applied {
                restarted: got_r,
                failed: got_f,
                ..
            } => {
                assert_eq!(got_r, restarted, "restarted");
                assert_eq!(got_f, failed, "failed");
            }
            other => panic!(
                "expected Applied {{ restarted: {restarted}, failed: {failed} }}, got {other:?}"
            ),
        }
    }

    #[test]
    fn cancel_reload_preserves_running_and_paused_executions() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        s.play.as_ref().unwrap().script_pause("bob");
        let generations = s.generations_for(&["alice".into(), "bob".into()]);
        let old_js =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .js
                .clone();
        warn_shared_reload(&mut s, &path);
        assert!(s.script_reload_confirmation_pending());
        s.cancel_reload();
        assert!(!s.script_reload_confirmation_pending());
        assert_eq!(
            s.generations_for(&["alice".into(), "bob".into()]),
            generations
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Paused
        );
        assert_eq!(
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .js,
            old_js
        );
        // A later click must prepare and warn again, never reuse cancelled consent.
        assert_eq!(s.script_reload_clicked(), ReloadOutcome::NeedsConfirm);
        s.play.as_ref().unwrap().script_stop("alice");
        s.play.as_ref().unwrap().script_stop("bob");
    }

    #[test]
    fn reload_confirm_does_not_authorize_switched_selection() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path_a = write_bot(&dir, "a.ts", BOT_TS);
        let path_b = write_bot(&dir, "b.ts", BOT_TS);
        s.load_js(&path_a);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        focus_profile(&mut s, "bob");
        s.load_js(&path_b);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        s.play.as_ref().unwrap().script_pause("bob");
        let old_b =
            s.js.get(script::ScriptSource::File, &path_b.to_string_lossy())
                .unwrap()
                .js
                .clone();
        fs::write(&path_a, format!("{BOT_TS}// a changed\n")).unwrap();
        fs::write(&path_b, format!("{BOT_TS}// b changed\n")).unwrap();
        focus_profile(&mut s, "alice");
        assert_eq!(s.script_reload_clicked(), ReloadOutcome::NeedsConfirm);
        assert!(
            s.reload_warning
                .as_ref()
                .is_some_and(|w| w.running.iter().any(|n| n == "alice")),
            "{:?}",
            s.reload_warning.as_ref().map(|w| &w.running)
        );
        focus_profile(&mut s, "bob");
        let second = s.script_reload_clicked();
        assert_eq!(
            second,
            ReloadOutcome::NeedsConfirm,
            "A's warning must not confirm B"
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Paused,
            "B must not be replaced without its own warning"
        );
        let now_b =
            s.js.get(script::ScriptSource::File, &path_b.to_string_lossy())
                .unwrap()
                .js
                .clone();
        assert_eq!(old_b, now_b, "B registration stays until B is confirmed");
        s.play.as_ref().unwrap().script_stop("alice");
        s.play.as_ref().unwrap().script_stop("bob");
    }

    #[test]
    fn reload_warn_survives_focus_only() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path_a = write_bot(&dir, "keep-a.ts", BOT_TS);
        let path_b = write_bot(&dir, "keep-b.ts", BOT_TS);
        s.load_js(&path_a);
        s.script_start_selected();
        focus_profile(&mut s, "bob");
        s.load_js(&path_b);
        fs::write(&path_a, format!("{BOT_TS}// changed\n")).unwrap();
        focus_profile(&mut s, "alice");
        assert_eq!(s.script_reload_clicked(), ReloadOutcome::NeedsConfirm);
        let warned = s.reload_warning.as_ref().unwrap().lookup.clone();
        focus_profile(&mut s, "bob");
        assert!(
            s.reload_warning.is_some(),
            "focus alone must not cancel a bound warning"
        );
        assert_eq!(s.reload_warning.as_ref().unwrap().lookup, warned);
        focus_profile(&mut s, "alice");
        let out = s.script_reload_clicked();
        match out {
            ReloadOutcome::Applied { restarted, .. } => assert_eq!(restarted, 1),
            other => panic!("focus-only must leave A's confirm valid, got {other:?}"),
        }
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn catalog_confirm_gates_newly_paused() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let root = dir.join("catalog-pause");
        fake_catalog(&root, &[("PauseBot", BOT_TS)]);
        s.js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "PauseBot")
            .unwrap();
        s.persist_successful_assignment(
            "alice",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "PauseBot".into(),
                display_name: "PauseBot".into(),
                unavailable: None,
            },
        );
        s.script_sel = Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "PauseBot".into(),
        ));
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        let old_js =
            s.js.get(script::ScriptSource::Catalog, "PauseBot")
                .unwrap()
                .js
                .clone();
        fs::write(
            root.join("src/bot/scripts/PauseBot/PauseBot.ts"),
            format!("{BOT_TS}// changed\n"),
        )
        .unwrap();
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_pause("alice");
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Paused,
            "newly paused catalog bot needs its own warning"
        );
        assert!(
            s.error
                .as_deref()
                .unwrap_or("")
                .contains("paused during prepare")
                || s.reload_warning
                    .as_ref()
                    .is_some_and(|w| !w.paused_during_prep.is_empty()),
            "{:?} {:?}",
            s.error,
            s.reload_warning.as_ref().map(|w| &w.paused_during_prep)
        );
        let now_js =
            s.js.get(script::ScriptSource::Catalog, "PauseBot")
                .unwrap()
                .js
                .clone();
        assert_eq!(old_js, now_js);
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_toplevel_throw_fails_before_replacement() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let path = write_bot(&dir, "throw.ts", BOT_TS);
        s.load_js(&path);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        let old =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .clone();
        fs::write(
            &path,
            "throw new Error('prep boom');\nexport default class T extends LoopingBot { override loop() {} }\n",
        )
        .unwrap();
        let out = s.script_reload_clicked();
        match out {
            ReloadOutcome::Failed(e) => assert!(
                e.contains("prep boom") || e.contains("load:") || e.contains("prepare"),
                "{e}"
            ),
            other => panic!("top-level throw must fail before replacement, got {other:?}"),
        }
        let now =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap();
        assert_eq!(now.origin, old.origin);
        assert_eq!(now.js, old.js);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_missing_named_export_fails_before_replacement() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let sib = write_bot(&dir, "helper.ts", "export function helper() {}\n");
        let src = "import { missingFn } from './helper.js';\nexport default class T extends LoopingBot { override loop() { missingFn(); } }\n";
        let good = src.replace("missingFn", "helper");
        let path = write_bot(&dir, "named.ts", &good);
        let _ = sib;
        s.load_js(&path);
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        let old =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap()
                .clone();
        fs::write(&path, src).unwrap();
        let out = s.script_reload_clicked();
        match out {
            ReloadOutcome::Failed(e) => assert!(
                e.contains("missingFn") || e.contains("load:") || e.contains("prepare"),
                "{e}"
            ),
            other => panic!("missing named export must fail before replacement, got {other:?}"),
        }
        let now =
            s.js.get(script::ScriptSource::File, &path.to_string_lossy())
                .unwrap();
        assert_eq!(now.origin, old.origin);
        assert_eq!(now.js, old.js);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn catalog_disk_change_after_warn_does_not_start_stale_prepared() {
        let (mut s, dir) = session_with_play(&["alice"]);
        let root = dir.join("catalog-stale");
        fake_catalog(&root, &[("StaleBot", BOT_TS)]);
        s.js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "StaleBot")
            .unwrap();
        s.persist_successful_assignment(
            "alice",
            ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "StaleBot".into(),
                display_name: "StaleBot".into(),
                unavailable: None,
            },
        );
        s.script_sel = Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "StaleBot".into(),
        ));
        s.script_start_selected();
        let old_js =
            s.js.get(script::ScriptSource::Catalog, "StaleBot")
                .unwrap()
                .js
                .clone();
        let bot = root.join("src/bot/scripts/StaleBot/StaleBot.ts");
        fs::write(&bot, format!("{BOT_TS}// first\n")).unwrap();
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        fs::write(&bot, format!("{BOT_TS}// second\n")).unwrap();
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running,
            "content change after warn must not authorize the previous prepared set"
        );
        let now = s.js.get(script::ScriptSource::Catalog, "StaleBot").unwrap();
        assert_eq!(now.js, old_js);
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_removal_skips_target_and_reloads_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        s.play.as_mut().unwrap().stop_slot("bob");
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 0);
        assert!(
            !s.error.as_deref().unwrap_or("").contains("bob"),
            "removal is cancellation, not a start failure: {:?}",
            s.error
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_reports_true_startup_failure_without_aborting_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        s.fail_reload_start_for = Some("bob".into());
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 1);
        assert!(
            s.error.as_deref().unwrap_or("").contains("bob"),
            "true post-stop start failure must stay visible: {:?}",
            s.error
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Idle,
            "failed replacement start leaves the stopped target stopped"
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_native_stop_skips_target_and_reloads_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        s.play.as_ref().unwrap().script_stop("bob");
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 0);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Idle
        );
        assert!(s
            .play
            .as_ref()
            .unwrap()
            .script_source_identity("bob")
            .is_none());
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_session_stop_skips_target_and_reloads_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        focus_profile(&mut s, "bob");
        s.script_stop();
        focus_profile(&mut s, "alice");
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 0);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Idle
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }

    #[test]
    fn reload_stop_all_clears_pending_without_restart() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        s.script_stop_all();
        assert!(s.pending_reload.is_none());
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Idle
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Idle
        );
    }

    #[test]
    fn reload_logout_skips_target_and_reloads_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        let bob_gen = s.play.as_ref().unwrap().script_runtime_generation("bob");
        s.logout("bob");
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 0);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Running,
            "logout skips replacement; it does not stop the isolate"
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_runtime_generation("bob"),
            bob_gen
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
        s.play.as_ref().unwrap().script_stop("bob");
    }

    #[test]
    fn reload_logout_all_skips_replacement() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        let alice_gen = s.play.as_ref().unwrap().script_runtime_generation("alice");
        let bob_gen = s.play.as_ref().unwrap().script_runtime_generation("bob");
        s.logout_all();
        let out = s.script_reload_clicked();
        assert_applied(out, 0, 0);
        assert_eq!(
            s.play.as_ref().unwrap().script_runtime_generation("alice"),
            alice_gen
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_runtime_generation("bob"),
            bob_gen
        );
        s.play.as_ref().unwrap().script_stop("alice");
        s.play.as_ref().unwrap().script_stop("bob");
    }

    #[test]
    fn reload_reassignment_skips_target_and_reloads_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        let other = write_bot(&dir, "other.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        let bob_gen = s.play.as_ref().unwrap().script_runtime_generation("bob");
        s.persist_successful_assignment(
            "bob",
            ScriptAssignment {
                source_kind: "file".into(),
                identity: other.to_string_lossy().into_owned(),
                display_name: "other".into(),
                unavailable: None,
            },
        );
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 0);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Running,
            "reassignment wins; do not stop the old isolate"
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_runtime_generation("bob"),
            bob_gen
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
        s.play.as_ref().unwrap().script_stop("bob");
    }

    #[test]
    fn reload_new_start_after_stop_is_not_consumed() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let path = write_bot(&dir, "shared.ts", BOT_TS);
        s.load_js(&path);
        start_file_on(&mut s, "alice", &path);
        start_file_on(&mut s, "bob", &path);
        warn_shared_reload(&mut s, &path);
        s.play.as_ref().unwrap().script_stop("bob");
        start_file_on(&mut s, "bob", &path);
        let bob_gen = s.play.as_ref().unwrap().script_runtime_generation("bob");
        focus_profile(&mut s, "alice");
        let out = s.script_reload_clicked();
        assert_applied(out, 1, 0);
        assert_eq!(
            s.play.as_ref().unwrap().script_runtime_generation("bob"),
            bob_gen,
            "a new Start after cancellation is a new generation"
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
        s.play.as_ref().unwrap().script_stop("bob");
    }

    #[test]
    fn catalog_native_stop_skips_target_and_reloads_peer() {
        let (mut s, dir) = session_with_play(&["alice", "bob"]);
        let root = dir.join("catalog-stop");
        fake_catalog(&root, &[("StopBot", BOT_TS)]);
        s.js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        s.js.ensure_js(script::ScriptSource::Catalog, "StopBot")
            .unwrap();
        let asg = ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "StopBot".into(),
            display_name: "StopBot".into(),
            unavailable: None,
        };
        s.persist_successful_assignment("alice", asg.clone());
        s.persist_successful_assignment("bob", asg);
        s.script_sel = Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "StopBot".into(),
        ));
        focus_profile(&mut s, "alice");
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        focus_profile(&mut s, "bob");
        s.script_sel = Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "StopBot".into(),
        ));
        s.script_start_selected();
        assert_eq!(s.error, None, "{:?}", s.error);
        fs::write(
            root.join("src/bot/scripts/StopBot/StopBot.ts"),
            format!("{BOT_TS}// changed\n"),
        )
        .unwrap();
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("bob");
        s.refresh_catalog_at(&root);
        assert_eq!(
            s.play.as_ref().unwrap().script_state("bob"),
            script::RunState::Idle
        );
        assert_eq!(
            s.play.as_ref().unwrap().script_state("alice"),
            script::RunState::Running
        );
        s.play.as_ref().unwrap().script_stop("alice");
    }
}

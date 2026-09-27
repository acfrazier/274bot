//! Reload and catalog refresh as one transaction: prepare the candidate on
//! the caller (finite), validate it in throwaway isolates on a worker,
//! then either apply it or, when running or paused runs of that card would
//! be replaced, publish a [`ReloadWarning`] naming the exact slots and
//! generations. A confirm re-checks the candidate against disk and the
//! warned generations; Cancel and Stop all discard it without touching any
//! execution. A replacement Start is queued behind the reap of the run it
//! replaces and settles like any Start.

use std::path::{Path, PathBuf};

use super::{PendingStart, Scripts, StartKind};
use crate::session::{OperatorSession, ScriptStart};

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

type ValidationBatch = Vec<(script::PreparedCard, Result<(), String>)>;

struct ValidationJob {
    epoch: u64,
    receiver: std::sync::mpsc::Receiver<ValidationBatch>,
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
        prepare_failed: Vec<script::LoadFailure>,
    },
}

/// Reload transaction state. `generation` fences every prepared candidate
/// and validation result: Cancel and Stop all bump it.
#[derive(Default)]
pub(super) struct ReloadState {
    generation: u64,
    validation: Option<ValidationJob>,
    pending: Option<PendingReload>,
    warning: Option<ReloadWarning>,
    catalog_confirm: bool,
    catalog_report: Option<String>,
    outcome: Option<ReloadOutcome>,
}

impl ReloadState {
    fn clear_pending(&mut self) {
        self.warning = None;
        self.pending = None;
        self.catalog_confirm = false;
    }

    fn install(&mut self, pending: PendingReload) {
        self.warning = Some(pending.warning.clone());
        self.catalog_confirm = matches!(pending.kind, PendingReloadKind::Catalog { .. });
        self.pending = Some(pending);
    }

    /// Discard a candidate in preparation or awaiting confirmation.
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.validation = None;
        self.clear_pending();
    }
}

impl Scripts {
    pub fn reload_validation_pending(&self) -> bool {
        self.reload.validation.is_some()
    }

    /// The terminal result of the latest reload or catalog refresh.
    pub fn take_reload_outcome(&mut self) -> Option<ReloadOutcome> {
        self.reload.outcome.take()
    }

    /// The warning awaiting confirmation, if any.
    pub fn reload_warning(&self) -> Option<&ReloadWarning> {
        self.reload.warning.as_ref()
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
            .is_some_and(|pending| matches!(pending.kind, PendingReloadKind::Manual))
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
        self.finish_reload(ReloadOutcome::Failed(error));
    }

    /// The card a Reload of `profile` targets: its pending Browse
    /// selection, else `heading`, else its assignment. Compiled cards have
    /// nothing to reload.
    pub fn reload_target<Io>(
        &self,
        core: &OperatorSession<Io>,
        profile: Option<&str>,
        heading: Option<&script::ScriptSel>,
    ) -> Option<(script::ScriptSource, String)> {
        let profile = profile?;
        let sel = self
            .pending_browse
            .get(profile)
            .cloned()
            .or_else(|| heading.cloned())
            .or_else(|| {
                Self::assignment(core, profile).and_then(|a| super::sel_from_assignment(&a))
            })?;
        match sel {
            script::ScriptSel::Loaded(source, lookup) => Some((source, lookup)),
            script::ScriptSel::Compiled(_) => None,
        }
    }

    /// Whether the next Reload of `target` confirms a shown warning.
    pub fn reload_confirmation_pending(
        &self,
        target: Option<&(script::ScriptSource, String)>,
    ) -> bool {
        target.is_some_and(|(source, lookup)| self.manual_pending_binds(*source, lookup))
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
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("script-reload-validate".into())
            .spawn(move || {
                let results = prepared
                    .into_iter()
                    .map(|prepared| {
                        let result = prepared.validate();
                        (prepared, result)
                    })
                    .collect();
                let _ = sender.send(results);
            })
            .map_err(|error| format!("script validation worker: {error}"))?;
        self.reload.validation = Some(ValidationJob {
            epoch,
            receiver,
            kind,
        });
        Ok(())
    }

    /// Fold a finished validation worker. The worker owns every throwaway
    /// V8 runtime; this only commits or reports its bounded result.
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
            self.finish_reload(ReloadOutcome::Failed(error));
            return;
        };
        match job.kind {
            ValidationKind::Manual(manual) => self.complete_manual_validation(core, manual, batch),
            ValidationKind::Catalog {
                root,
                diff,
                prepare_failed,
            } => self.complete_catalog_validation(core, root, diff, prepare_failed, batch),
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
        let set_fingerprint =
            catalog_set_fingerprint(&diff.added, &diff.changed, &diff.removed, &prepared);
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
                kind: PendingReloadKind::Catalog {
                    root,
                    added: diff.added,
                    removed: diff.removed,
                    failed: prepare_failed,
                    set_fingerprint,
                },
            });
            self.finish_reload(ReloadOutcome::NeedsConfirm);
        } else {
            let outcome =
                self.apply_catalog_prepared(core, diff, prepared, prepare_failed, warning);
            self.finish_reload(outcome);
        }
    }

    /// Reload `target` (see [`Self::reload_target`]). Preparation is finite
    /// on the caller; throwaway V8 validation runs on a worker polled by
    /// [`Scripts::poll`]. A second Reload of the same unchanged candidate
    /// confirms a shown warning.
    pub fn begin_reload<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        target: Option<(script::ScriptSource, String)>,
    ) {
        if self.reload.validation.is_some() {
            return;
        }
        self.reload.outcome = None;
        let Some((source, lookup)) = target else {
            self.fail_reload("reload", "no script to reload".into());
            return;
        };
        if self.manual_pending_binds(source, &lookup) {
            let outcome = self.commit_manual_pending(core);
            self.finish_reload(outcome);
            return;
        }
        match self.js.raw_source_changed(source, &lookup) {
            Ok(false) => {
                self.show(script::NOTHING_CHANGED_RELOAD);
                if matches!(
                    self.reload.pending.as_ref().map(|pending| &pending.kind),
                    Some(PendingReloadKind::Manual)
                ) {
                    self.reload.clear_pending();
                }
                self.finish_reload(ReloadOutcome::NothingChanged);
                return;
            }
            Ok(true) => {}
            Err(error) => return self.fail_reload("reload", error),
        }
        let identity_key = self
            .js
            .get(source, &lookup)
            .map(|card| card.identity_key())
            .unwrap_or_default();
        let fingerprint = match self.js.disk_fingerprint(source, &lookup) {
            Ok(fingerprint) => fingerprint,
            Err(error) => return self.fail_reload("reload", error),
        };
        let (running_before, paused_before) = slots_with_identity(core, &identity_key);
        let prepared = match self.js.prepare_card_unvalidated(source, &lookup) {
            Ok(prepared) => prepared,
            Err(error) => return self.fail_reload("reload", error),
        };
        let kind = ValidationKind::Manual(ManualValidation {
            source,
            lookup,
            identity_key,
            fingerprint,
            running_before,
            paused_before,
        });
        match self.spawn_validation(vec![prepared], kind) {
            Ok(()) => self.clear_notice(),
            Err(error) => self.fail_reload("reload", error),
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
        if !matches!(pending.kind, PendingReloadKind::Manual) {
            return false;
        }
        let w = &pending.warning;
        if w.epoch != self.reload.generation || w.source != source {
            return false;
        }
        if !lookups_match(source, &w.lookup, lookup) {
            return false;
        }
        self.js
            .disk_fingerprint(source, lookup)
            .is_ok_and(|fingerprint| w.fingerprint == fingerprint)
    }

    fn commit_manual_pending<Io>(&mut self, core: &mut OperatorSession<Io>) -> ReloadOutcome {
        let Some(mut pending) = self.reload.pending.take() else {
            return ReloadOutcome::Failed("no pending reload".into());
        };
        if pending.warning.epoch != self.reload.generation {
            self.reload.clear_pending();
            return ReloadOutcome::Failed("reload cancelled".into());
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
                    kind: PendingReloadKind::Manual,
                });
                return ReloadOutcome::Failed(e);
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
        ReloadOutcome::Applied {
            restarted,
            stopped_paused,
            failed,
        }
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
        let card = &prepared.card;
        let bag = self.merged_profile_bag(
            core,
            profile,
            card.source,
            &card.name,
            &card.path,
            &card.settings_schema,
        );
        let bag = (!bag.is_empty()).then_some(bag);
        #[cfg(any(test, feature = "test-support"))]
        if self.fail_reload_start_for.as_deref() == Some(profile) {
            return Err("injected start failure".into());
        }
        let start = ScriptStart::Load {
            js: card.js.clone(),
            shape: card.shape,
            bag,
            siblings: prepared.siblings.clone(),
        };
        if let Err(e) = core.start_script(profile, start, Some(card.identity_key())) {
            return self.js.record_start_result(card, Err(e));
        }
        self.starts.insert(
            profile.to_string(),
            PendingStart {
                card: card.clone(),
                kind: StartKind::Reload,
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

    /// Catalog Refresh from `root`. Returns the newly added card names a
    /// front end may warm (transpile) in the background.
    pub fn begin_refresh_catalog<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        root: &Path,
    ) -> Vec<String> {
        if self.reload.validation.is_some() {
            return Vec::new();
        }
        self.reload.outcome = None;
        if self.catalog_pending_binds(root) {
            let outcome = self.commit_catalog_pending(core);
            if let ReloadOutcome::Failed(error) = &outcome {
                self.show(format!("Refresh catalog: {error}"));
            }
            self.finish_reload(outcome);
            return Vec::new();
        }
        let diff = match self.js.diff_catalog(root) {
            Ok(diff) => diff,
            Err(error) => {
                self.fail_reload("Refresh catalog", error);
                return Vec::new();
            }
        };
        if diff.is_noop() {
            self.reload.catalog_report = Some(script::NOTHING_CHANGED_CATALOG.into());
            self.show(script::NOTHING_CHANGED_CATALOG);
            self.finish_reload(ReloadOutcome::NothingChanged);
            return Vec::new();
        }
        let added = diff.added.clone();
        let mut prepared = Vec::new();
        let mut prepare_failed = Vec::new();
        for name in &diff.changed {
            match self
                .js
                .prepare_card_unvalidated(script::ScriptSource::Catalog, name)
            {
                Ok(candidate) => prepared.push(candidate),
                Err(error) => {
                    if let Some(card) = self.js.get(script::ScriptSource::Catalog, name) {
                        if let Some(failure) = self.js.load_failure(&card.identity_key()).cloned() {
                            prepare_failed.push(failure);
                        }
                    }
                    self.show(format!("catalog {name}: {error}"));
                }
            }
        }
        let had_prepare_failure = !prepare_failed.is_empty();
        let kind = ValidationKind::Catalog {
            root: root.to_path_buf(),
            diff,
            prepare_failed,
        };
        match self.spawn_validation(prepared, kind) {
            Ok(()) if !had_prepare_failure => self.clear_notice(),
            Ok(()) => {}
            Err(error) => self.fail_reload("Refresh catalog", error),
        }
        added
    }

    fn catalog_pending_binds(&self, root: &Path) -> bool {
        let Some(pending) = self.reload.pending.as_ref() else {
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
        if pending.warning.epoch != self.reload.generation || pending_root != root {
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

    fn commit_catalog_pending<Io>(&mut self, core: &mut OperatorSession<Io>) -> ReloadOutcome {
        let Some(mut pending) = self.reload.pending.take() else {
            return ReloadOutcome::Failed("no pending catalog refresh".into());
        };
        let PendingReloadKind::Catalog { root, .. } = pending.kind.clone() else {
            self.reload.pending = Some(pending);
            return ReloadOutcome::Failed("pending reload is not catalog".into());
        };
        if pending.warning.epoch != self.reload.generation {
            self.reload.clear_pending();
            return ReloadOutcome::Failed("reload cancelled".into());
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
        let diff = match self.js.diff_catalog(&root) {
            Ok(d) => d,
            Err(e) => return ReloadOutcome::Failed(e),
        };
        let failed = match pending.kind {
            PendingReloadKind::Catalog { failed, .. } => failed,
            PendingReloadKind::Manual => Vec::new(),
        };
        self.apply_catalog_prepared(core, diff, pending.prepared, failed, pending.warning)
    }

    fn apply_catalog_prepared<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        mut diff: script::CatalogDiff,
        prepared: Vec<script::PreparedCard>,
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
        ReloadOutcome::Applied {
            restarted,
            stopped_paused,
            failed,
        }
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

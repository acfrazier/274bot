//! Native settings preparation is a persistence prerequisite, never a pump callback.
//! Same-card replacements inherit accepted copy holds through their durable commit.
use super::*;
use script::native::{PreparedConfig, SettingsBag, StartError};
use script::preparation::PreparedTask;
use std::sync::atomic::AtomicBool;

/// Cancellation is shared by the sync report and its existing preparations/
/// writes. Equality follows the owning sync operation, not its mutable flag.
#[derive(Debug, Clone)]
pub(crate) struct NativeCopyCancellation {
    op: OperationId,
    cancelled: Arc<AtomicBool>,
}

impl PartialEq for NativeCopyCancellation {
    fn eq(&self, other: &Self) -> bool {
        self.op == other.op
    }
}

impl Eq for NativeCopyCancellation {}

impl NativeCopyCancellation {
    pub(crate) fn new(op: OperationId) -> Self {
        Self {
            op,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Accepted native copies own the confirmation token through preparation and
/// the profile commit. A stalled copy must not strand the Start queue.
pub(super) struct NativeCopyHold {
    _token: Arc<String>,
    deadline: Instant,
    cancellation: NativeCopyCancellation,
    target: crate::selection::ProfileIdentity,
    member_name: String,
}

impl NativeCopyHold {
    const TIMEOUT: Duration = Duration::from_secs(30);

    fn new(
        target: crate::selection::ProfileIdentity,
        member_name: String,
        token: Arc<String>,
        cancellation: NativeCopyCancellation,
    ) -> Self {
        Self {
            _token: token,
            deadline: Instant::now() + Self::TIMEOUT,
            cancellation,
            target,
            member_name,
        }
    }

    fn target_available(
        &self,
        play_available: bool,
        vault: Option<&vault::Vault>,
        members: &[String],
    ) -> bool {
        if !play_available {
            return false;
        }
        let Some(profile) = vault.and_then(|vault| {
            vault.profiles().find(|profile| {
                crate::selection::ProfileIdentity::uid(profile.uid) == self.target
            })
        }) else {
            return false;
        };
        members.contains(&self.member_name) || members.contains(&profile.username)
    }
}

#[derive(Clone)]
struct AssignmentIntent {
    previous: Option<vault::ScriptAssignment>,
    next: Option<vault::ScriptAssignment>,
}

/// At most this many settings receipts may wait for the shared worker.
/// This bounds retained drafts when an operator repeatedly saves while full.
const MAX_PENDING_NATIVE_SETTINGS: usize = 64;

/// A fleet settings copy keeps at most this many native saves and live
/// deliveries in flight, so a single edit made during a large copy still
/// finds room; the copy queues the rest ([`OperatorSession::native_copy_room`]).
const COPY_IN_FLIGHT: usize = MAX_PENDING_NATIVE_SETTINGS - MAX_PENDING_NATIVE_SETTINGS / 4;

type NativePreparation = PreparedTask<Result<host_play::PreparedScriptConfig, StartError>>;

pub(super) struct PendingPreparation {
    profile: Profile,
    /// Explicit or inherited intent from a pending same-card draft. New copies
    /// supersede older intents; rebase checks the same staged-vault baseline.
    assignment_intent: Option<AssignmentIntent>,
    renamed_from: Option<String>,
    mirror: ArmMirror,
    label: &'static str,
    worker: Option<NativePreparation>,
    waiting: Option<(script::CompiledId, u64, Arc<SettingsBag>)>,
    failure: Option<String>,
    #[cfg(test)]
    publication_gate: Option<Arc<AtomicBool>>,
    copy_start_hold: Option<NativeCopyHold>,
}

impl PendingPreparation {
    fn is_finished(&self) -> bool {
        #[cfg(test)]
        if self
            .publication_gate
            .as_ref()
            .is_some_and(|gate| !gate.load(Ordering::Acquire))
        {
            return false;
        }
        self.failure.is_some() || self.worker.as_ref().is_some_and(PreparedTask::is_finished)
    }
}

pub(super) struct PendingDelivery {
    op: OperationId,
    name: String,
    live: LiveSettings,
    worker: Option<NativePreparation>,
    admission: Option<(script::CompiledId, u64)>,
}

impl ArmMirror {
    pub(super) fn native_identity(&self) -> Option<String> {
        match &self.0 {
            ArmMirrorKind::NativeSettings { id, .. } => Some(script::compiled_identity_key(*id)),
            ArmMirrorKind::ScriptSettings { card, .. } => Some(card.clone()),
            ArmMirrorKind::Remember(Some(live)) if live.run.is_some() => {
                Some(live.identity.clone())
            }
            _ => None,
        }
    }

    pub(super) fn native_draft(&self) -> Option<(script::CompiledId, Arc<SettingsBag>)> {
        match &self.0 {
            ArmMirrorKind::NativeSettings { id, bag, .. } => Some((*id, Arc::clone(bag))),
            _ => None,
        }
    }

    fn prepared(self, config: Arc<PreparedConfig>) -> Self {
        match self.0 {
            ArmMirrorKind::NativeSettings { id, live, .. } => Self(ArmMirrorKind::ScriptSettings {
                card: script::compiled_identity_key(id),
                live: live.map(|mut live| {
                    live.prepared = Some(config);
                    live
                }),
            }),
            other => Self(other),
        }
    }
}

impl<Io> OperatorSession<Io> {
    /// How many more native saves a fleet settings copy may start now.
    pub(crate) fn native_copy_room(&self) -> usize {
        COPY_IN_FLIGHT.saturating_sub(self.preparations.len() + self.deliveries.len())
    }

    pub(super) fn queue_native_delivery(
        &mut self,
        op: OperationId,
        name: String,
        live: LiveSettings,
    ) {
        if self
            .play
            .as_ref()
            .and_then(|play| play.script_native_run(&name))
            != live.run
        {
            self.settings_writes.push(SettingsWrite {
                op,
                profile: name,
                result: SettingsResult::saved(LiveDelivery::Stale),
            });
            return;
        }
        if self.deliveries.len() >= MAX_PENDING_NATIVE_SETTINGS {
            self.settings_writes.push(SettingsWrite {
                op,
                profile: name,
                result: SettingsResult::saved(LiveDelivery::rejected(
                    "too many pending native settings deliveries",
                )),
            });
            return;
        }
        let result = self
            .play
            .as_ref()
            .ok_or_else(|| "no play".to_string())
            .and_then(|play| {
                let id = script::compiled_id(live.identity.strip_prefix("compiled:").unwrap_or(""))
                    .ok_or_else(|| "native card unavailable".to_string())?;
                let revision =
                    op.0.max(play.script_native_settings_revision(&name).unwrap_or(1))
                        .checked_add(1)
                        .ok_or_else(|| "settings revision exhausted".to_string())?;
                Ok((id, revision))
            });
        match result {
            Ok((id, revision)) => {
                let play = self.play.as_ref().expect("checked play");
                match play.script_prepare_config(id, revision, Arc::clone(&live.bag)) {
                    Ok(worker) => self.deliveries.push(PendingDelivery {
                        op,
                        name,
                        live,
                        worker: Some(worker),
                        admission: None,
                    }),
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {
                        self.deliveries.push(PendingDelivery {
                            op,
                            name,
                            live,
                            worker: None,
                            admission: Some((id, revision)),
                        });
                    }
                    Err(error) => self.settings_writes.push(SettingsWrite {
                        op,
                        profile: name,
                        result: SettingsResult::saved(LiveDelivery::rejected(error.to_string())),
                    }),
                }
            }
            Err(error) => self.settings_writes.push(SettingsWrite {
                op,
                profile: name,
                result: SettingsResult::saved(LiveDelivery::rejected(error)),
            }),
        }
    }

    pub(super) fn take_native_deliveries(&mut self, wait: bool) {
        loop {
            let mut index = 0;
            let mut blocked_on_admission = false;
            while index < self.deliveries.len() {
                if let Some((id, revision)) = self.deliveries[index].admission {
                    let result = self
                        .play
                        .as_ref()
                        .ok_or_else(|| {
                            script::preparation::PreparationError::Unavailable("no play".into())
                        })
                        .and_then(|play| {
                            play.script_prepare_config(
                                id,
                                revision,
                                Arc::clone(&self.deliveries[index].live.bag),
                            )
                        });
                    match result {
                        Ok(worker) => {
                            self.deliveries[index].worker = Some(worker);
                            self.deliveries[index].admission = None;
                        }
                        Err(script::preparation::PreparationError::Refused(
                            api::work::SubmissionRefusal::Full,
                        )) => {
                            if wait {
                                blocked_on_admission = true;
                            }
                            // Keep later receipts moving: joining one releases
                            // its retained WorkAdmission charge.
                            index += 1;
                            continue;
                        }
                        Err(error) => {
                            let pending = self.deliveries.remove(index);
                            self.settings_writes.push(SettingsWrite {
                                op: pending.op,
                                profile: pending.name,
                                result: SettingsResult::saved(LiveDelivery::rejected(
                                    error.to_string(),
                                )),
                            });
                            continue;
                        }
                    }
                }
                if !wait
                    && !self.deliveries[index]
                        .worker
                        .as_ref()
                        .is_some_and(PreparedTask::is_finished)
                {
                    index += 1;
                    continue;
                }
                let mut pending = self.deliveries.remove(index);
                let worker = pending
                    .worker
                    .take()
                    .expect("admitted delivery has a worker");
                let identity = worker.identity();
                let prepared = worker
                    .join()
                    .map_err(|error| error.to_string())
                    .and_then(|result| result.map_err(|error| error.to_string()));
                let delivery =
                    if !self.native_edit_current(&pending.name, &pending.live.identity, pending.op)
                    {
                        LiveDelivery::Stale
                    } else if let Some(play) = &self.play {
                        if play.script_native_run(&pending.name) != pending.live.run {
                            LiveDelivery::Stale
                        } else {
                            match prepared.and_then(|config| {
                                config.commit(identity).map_err(|error| error.to_string())
                            }) {
                                Ok(config) => {
                                    pending.live.prepared = Some(config);
                                    deliver_settings(play, &pending.name, Some(pending.live))
                                }
                                Err(error) => LiveDelivery::rejected(error),
                            }
                        }
                    } else {
                        LiveDelivery::Stale
                    };
                self.settings_writes.push(SettingsWrite {
                    op: pending.op,
                    profile: pending.name,
                    result: SettingsResult::saved(delivery),
                });
            }
            if !wait || !blocked_on_admission {
                return;
            }
            // Retry admissions only after later receipts have been consumed.
            std::thread::park_timeout(Duration::from_millis(5));
        }
    }
    /// A parameter editor can compose a newer edit over its still-preparing
    /// draft. Starts and spawns use the durable profile, not this projection.
    pub fn settings_for_edit(&self, name: &str) -> Option<&ProfileSettings> {
        self.profile_for_edit(name).map(|profile| &profile.settings)
    }

    pub(crate) fn profile_for_edit(&self, name: &str) -> Option<&Profile> {
        self.preparations
            .iter()
            .filter(|(op, pending)| {
                pending.profile.username == name
                    && pending
                        .mirror
                        .native_identity()
                        .is_some_and(|card| self.native_edit_current(name, &card, **op))
            })
            .max_by_key(|(op, _)| *op)
            .map(|(_, pending)| &pending.profile)
            .or_else(|| self.vault.as_ref()?.get(name))
    }

    fn native_edit_current(&self, name: &str, card: &str, op: OperationId) -> bool {
        self.native_edits.get(&(name.to_owned(), card.to_owned())) == Some(&op)
            && self
                .profile_edits
                .get(name)
                .is_none_or(|removed| *removed < op)
    }

    /// Attach the Apply token to the existing preparation, not a second
    /// completion tracker. JS copies stage their bag synchronously.
    pub(crate) fn hold_native_copy_start(
        &mut self,
        op: OperationId,
        target: crate::selection::ProfileIdentity,
        member_name: String,
        token: Arc<String>,
        cancellation: NativeCopyCancellation,
    ) {
        if let Some(pending) = self.preparations.get_mut(&op) {
            pending.copy_start_hold = Some(NativeCopyHold::new(
                target,
                member_name,
                token,
                cancellation,
            ));
        }
    }

    pub(super) fn queue_native_settings(
        &mut self,
        profile: Profile,
        mirror: ArmMirror,
        label: &'static str,
        id: script::CompiledId,
        bag: Arc<SettingsBag>,
        renamed_from: Option<&str>,
    ) -> Result<OperationId, String> {
        let source = renamed_from.unwrap_or(&profile.username);
        if self
            .vault
            .as_ref()
            .and_then(|vault| vault.get(source))
            .is_none()
        {
            return Err(format!("{label}: profile unavailable: {source}"));
        }
        let card = script::compiled_identity_key(id);
        let assign = matches!(
            &mirror.0,
            ArmMirrorKind::NativeSettings { assign: true, .. }
        );
        let assignment_intent = if assign {
            self.vault
                .as_ref()
                .and_then(|vault| vault.get(source))
                .filter(|base| {
                    base.settings.script_assignment != profile.settings.script_assignment
                })
                .map(|base| AssignmentIntent {
                    previous: base.settings.script_assignment.clone(),
                    next: profile.settings.script_assignment.clone(),
                })
        } else {
            // Settings-only edits carry the same-card copy's original baseline,
            // never the assignment inherited from another card's projection.
            self.native_edits
                .get(&(source.to_owned(), card.clone()))
                .and_then(|prior_op| self.preparations.get(prior_op))
                .and_then(|pending| pending.assignment_intent.clone())
        };
        if self.preparations.len() >= MAX_PENDING_NATIVE_SETTINGS {
            return Err(format!("{label}: too many pending native settings saves"));
        }
        let play = self
            .play
            .as_ref()
            .ok_or_else(|| format!("{label}: no play"))?;
        let op = self.operations.open(ActionKind::SaveProfile);
        let revision =
            op.0.max(play.script_native_settings_revision(source).unwrap_or(1))
                .checked_add(1)
                .ok_or("settings revision exhausted")?;
        let (worker, waiting) = match play.script_prepare_config(id, revision, Arc::clone(&bag)) {
            Ok(worker) => (Some(worker), None),
            Err(script::preparation::PreparationError::Refused(
                api::work::SubmissionRefusal::Full,
            )) => (None, Some((id, revision, bag))),
            Err(error) => {
                let error = error.to_string();
                self.operations
                    .set(op, &profile.username, Outcome::failed(error.clone()));
                return Err(error);
            }
        };
        if assign && assignment_intent.is_some() {
            // Last accepted copy wins even when it finishes before an older
            // card's preparation. Older settings may still be saved.
            for pending in self.preparations.values_mut() {
                if pending
                    .renamed_from
                    .as_deref()
                    .unwrap_or(&pending.profile.username)
                    == source
                {
                    pending.assignment_intent = None;
                }
            }
        }
        self.operations.set(op, &profile.username, Outcome::Pending);
        // Supersession must not admit a queued Start on the pre-copy bag.
        // Move the accepted copy's token, deadline and cancellation together;
        // replacing its draft does not restart the bounded wait.
        let copy_start_hold = self
            .native_edits
            .insert((source.to_owned(), card.clone()), op)
            .and_then(|prior_op| self.preparations.get_mut(&prior_op))
            .and_then(|prior| prior.copy_start_hold.take());
        self.native_edits
            .insert((profile.username.clone(), card), op);
        self.preparations.insert(
            op,
            PendingPreparation {
                profile,
                assignment_intent,
                renamed_from: renamed_from.map(str::to_owned),
                mirror,
                label,
                worker,
                waiting,
                failure: None,
                #[cfg(test)]
                publication_gate: None,
                copy_start_hold,
            },
        );
        Ok(op)
    }

    pub(super) fn take_preparations(&mut self, wait: bool) {
        if self.preparations.is_empty() {
            return;
        }
        loop {
            if self.preparations.is_empty() {
                return;
            }
            for pending in self.preparations.values_mut() {
                let Some((id, revision, bag)) = pending.waiting.as_ref() else {
                    continue;
                };
                let Some(play) = self.play.as_ref() else {
                    pending.failure = Some("no play".into());
                    pending.waiting = None;
                    continue;
                };
                match play.script_prepare_config(*id, *revision, Arc::clone(bag)) {
                    Ok(worker) => {
                        pending.worker = Some(worker);
                        pending.waiting = None;
                    }
                    Err(script::preparation::PreparationError::Refused(
                        api::work::SubmissionRefusal::Full,
                    )) => {}
                    Err(error) => {
                        pending.failure = Some(error.to_string());
                        pending.waiting = None;
                    }
                }
            }
            let now = Instant::now();
            let mut ready: Vec<_> = self
                .preparations
                .iter()
                .filter(|(_, pending)| {
                    #[cfg(test)]
                    let gated = pending
                        .publication_gate
                        .as_ref()
                        .is_some_and(|gate| !gate.load(Ordering::Acquire));
                    #[cfg(not(test))]
                    let gated = false;
                    if !gated && (pending.is_finished() || (wait && pending.worker.is_some())) {
                        return true;
                    }
                    pending.copy_start_hold.as_ref().is_some_and(|hold| {
                        now >= hold.deadline
                            || hold.cancellation.is_cancelled()
                            || !hold.target_available(
                                self.play.is_some(),
                                self.vault.as_ref(),
                                self.members(),
                            )
                    })
                })
                .map(|(op, _)| *op)
                .collect();
            ready.sort_unstable();
            let nothing_ready = ready.is_empty();
            for op in ready {
                let mut pending = self.preparations.remove(&op).expect("ready preparation");
                let name = pending.profile.username.clone();
                let worker = pending.worker.take();
                let identity = worker.as_ref().map(PreparedTask::identity);
                let source = pending.renamed_from.as_deref().unwrap_or(&name);
                let result = if pending
                    .copy_start_hold
                    .as_ref()
                    .is_some_and(|hold| now >= hold.deadline)
                    && !worker.as_ref().is_some_and(PreparedTask::is_finished)
                {
                    // Abandon the exact job; its worker and charge remain in
                    // custody until the shared WorkAdmission worker returns.
                    // Its late result cannot be persisted after this failed copy.
                    Err("settings copy timed out during native preparation".to_string())
                } else if pending
                    .copy_start_hold
                    .as_ref()
                    .is_some_and(|hold| hold.cancellation.is_cancelled())
                {
                    Err("settings copy cancelled".to_string())
                } else if pending.copy_start_hold.as_ref().is_some_and(|hold| {
                    !hold.target_available(
                        self.play.is_some(),
                        self.vault.as_ref(),
                        self.members(),
                    )
                }) {
                    Err("settings copy cancelled: bot or vault unavailable".to_string())
                } else if let Some(error) = pending.failure.take() {
                    Err(error)
                } else {
                    worker
                        .expect("ready native preparation has a worker")
                        .join()
                        .map_err(|error| error.to_string())
                        .and_then(|result| result.map_err(|error| error.to_string()))
                };
                let card = pending
                    .mirror
                    .native_identity()
                    .expect("native draft identity");
                let current = self.native_edit_current(&name, &card, op)
                    && self.native_edit_current(source, &card, op)
                    && self.vault.as_ref().is_some_and(|vault| {
                        vault.get(source).is_some()
                            && (pending.renamed_from.is_none() || vault.get(&name).is_none())
                    });
                // Invalid drafts remain failed even if an unrelated durable write
                // overtook them. They were never eligible for persistence.
                if !current && result.is_ok() {
                    self.operations.set(op, &name, Outcome::Cancelled);
                    if pending.mirror.is_native_settings() {
                        self.settings_writes.push(SettingsWrite {
                            op,
                            profile: name.clone(),
                            result: SettingsResult::Superseded,
                        });
                    }
                    continue;
                }
                let config = match result.and_then(|config| {
                    config
                        .commit(identity.expect("successful native preparation had an identity"))
                        .map_err(|error| error.to_string())
                }) {
                    Ok(config) => config,
                    Err(error) => {
                        self.operations
                            .set(op, &name, Outcome::failed(error.clone()));
                        self.write_failures
                            .push(crate::profile_saves::WriteFailure::new(
                                op,
                                name.clone(),
                                pending.label,
                                error.clone(),
                            ));
                        if pending.mirror.is_native_settings() {
                            self.settings_writes.push(SettingsWrite {
                                op,
                                profile: name.clone(),
                                result: SettingsResult::failed(error),
                            });
                        }
                        continue;
                    }
                };
                let assignment_action = if pending.assignment_intent.is_some() {
                    "native sync assignment"
                } else {
                    "native parameters"
                };
                // Preparation owns this card's entry, not a snapshot of the whole
                // profile. Preserve unrelated writes that landed while it ran.
                if let Some(mut latest) = self
                    .vault
                    .as_ref()
                    .and_then(|vault| vault.get(source))
                    .cloned()
                {
                    if let Some(entry) = pending.profile.settings.script_settings.remove(&card) {
                        latest.settings.script_settings.insert(card, entry);
                    } else {
                        latest.settings.script_settings.remove(&card);
                    }
                    if let Some(assignment) = pending.assignment_intent {
                        if latest.settings.script_assignment == assignment.previous {
                            latest.settings.script_assignment = assignment.next;
                        }
                    }
                    latest.username.clone_from(&name);
                    pending.profile = latest;
                }
                self.ensure_writer();
                self.hold_durable(&name);
                api::hostlog::register_secret(&pending.profile.password);
                let mut changes = vec![VaultChange::Upsert(pending.profile.clone())];
                if let Some(old) = pending.renamed_from {
                    self.hold_durable(&old);
                    self.cancel_profile_start(&old);
                    if let Some(vault) = self.vault_for_write() {
                        vault.stage_remove(&old);
                    }
                    changes.push(VaultChange::Remove(old));
                }
                if let Some(vault) = self.vault_for_write() {
                    vault.stage_upsert(pending.profile);
                }
                self.submit_write_at(
                    op,
                    changes,
                    pending.mirror.prepared(config),
                    pending.label,
                    assignment_action,
                );
                if let Some(write) = self.writes.get_mut(&op) {
                    write.copy_start_hold = pending.copy_start_hold;
                }
            }
            if !wait {
                return;
            }
            // Synchronous flushes wait on the receipt, never fail on temporary
            // WorkAdmission fullness. Frame polling above never parks the UI.
            if nothing_ready {
                std::thread::park_timeout(Duration::from_millis(5));
            }
        }
    }

    /// A disk write already submitted cannot be recalled. On a bounded-wait
    /// failure, release Starts onto the durable bag and never push this copy
    /// into the newly admitted run, even if the writer eventually completes.
    pub(super) fn release_abandoned_copy_writes(&mut self) {
        let now = Instant::now();
        for (op, pending) in &mut self.writes {
            let Some(hold) = pending.copy_start_hold.as_ref() else {
                continue;
            };
            let unavailable = !hold.target_available(
                self.play.is_some(),
                self.vault.as_ref(),
                self.fleet.members(),
            );
            let cancelled = hold.cancellation.is_cancelled();
            if now < hold.deadline && !unavailable && !cancelled {
                continue;
            }
            let error = if cancelled {
                "settings copy cancelled; the write may still complete"
            } else if unavailable {
                "settings copy cancelled: bot or vault unavailable"
            } else {
                "settings copy timed out waiting for persistence; the write may still complete"
            }
            .to_string();
            pending.copy_start_hold = None;
            pending.copy_abandoned = true;
            self.operations
                .set(*op, &pending.member, Outcome::failed(error.clone()));
            self.write_failures.push(WriteFailure::new(
                *op,
                pending.member.clone(),
                pending.label,
                error.clone(),
            ));
            self.settings_writes.push(SettingsWrite {
                op: *op,
                profile: pending.member.clone(),
                result: SettingsResult::failed(error),
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::marked::prepare_apply_settings_marked;
    use crate::scripts::{CompiledCardId, ScriptChoice, Scripts};
    use crate::selection::{MarkedSelection, ProfileIdentity};
    use crate::surface::HeadlessSurface;
    use host_play::{InstancePermit, Play, PlayOptions};
    use serde_json::{json, Map, Value};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use vault::{Profile, ProfileSettings, Vault};

    const PASSPHRASE: &str = "test-passphrase-01";

    /// Hang guard for every wait in this module, never a pass condition and
    /// never a latency target. Each wait ends on the product's own settlement
    /// signal (the slot's run state, the preparation leaving
    /// `core.preparations`): every accepted preparation reaches a terminal.
    /// Preparation runs on one process-wide background worker shared with
    /// every other test in this binary, so on a loaded host a wait queues
    /// behind all of them. The guard only stops a deadlocked test and is
    /// minutes, not the expected duration.
    const HANG_GUARD: Duration = Duration::from_secs(600);

    struct Fixture {
        core: OperatorSession<()>,
        scripts: Scripts,
        dir: PathBuf,
        _iso: script::IsolatedEnv,
    }

    fn empty_play() -> Play {
        host_play::run_with_io(
            &PlayOptions {
                host: "127.0.0.1".into(),
                transport: host_play::Transport::Tcp,
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

    fn native_fixture(test: &str) -> Fixture {
        #[cfg(feature = "js")]
        crate::test_support::scripts::install_catalog_cards();
        let iso = script::IsolatedEnv::enter(&format!("frontend-core-native-settings-{test}"));
        let dir = iso.dir.clone();
        let mut vault = Vault::create(&dir.join("vault"), PASSPHRASE).unwrap();
        for (i, name) in ["alice", "bob"].into_iter().enumerate() {
            vault
                .upsert(Profile {
                    username: name.into(),
                    password: "pw".into(),
                    uid: 1 + i as i32,
                    settings: ProfileSettings::default(),
                })
                .unwrap();
        }

        let mut core = OperatorSession::new(InstancePermit::SkipLock);
        core.set_spawn_workers(false);
        core.start(vault, empty_play());
        let mut surface = HeadlessSurface::new();
        for name in ["alice", "bob"] {
            core.load(name, &mut surface);
        }
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        core.play_mut().unwrap().bind_script_test_data(data);
        #[cfg(feature = "js")]
        let scripts = Scripts::new(
            script_js::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
            script::ScriptSettingsStore::at(dir.join("script-settings.json")),
        );
        #[cfg(not(feature = "js"))]
        let scripts = Scripts::new();
        Fixture {
            core,
            scripts,
            dir,
            _iso: iso,
        }
    }

    impl Fixture {
        fn configure_expected_bag(
            &self,
            name: &str,
            id: script::CompiledId,
            bag: Map<String, Value>,
        ) -> script::CompiledDelivery {
            let play = self.core.play().unwrap();
            let run = play.script_native_run(name).expect("running native script");
            let revision = play
                .script_native_settings_revision(name)
                .expect("native settings revision")
                .checked_add(1)
                .expect("settings revision");
            let worker = play
                .script_prepare_config(id, revision, Arc::new(bag))
                .unwrap();
            let identity = worker.identity();
            let prepared = worker.join().unwrap().unwrap().commit(identity).unwrap();
            play.script_configure_compiled(name, prepared, run)
        }
    }

    fn preparing_copy(test: &str) -> Fixture {
        let mut f = native_fixture(test);
        let id = script::CompiledId("Gatherer");
        for name in ["alice", "bob"] {
            assert!(f.scripts.persist_assignment(
                &mut f.core,
                name,
                script::compiled_assignment(id)
            ));
        }
        f.core.flush_writes();
        f.scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                CompiledCardId::from_script(id),
                json!({"targetPreference": "Nearest"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        f.core.flush_writes();
        f.scripts
            .prepare_compiled_settings_sync(&f.core, "alice", CompiledCardId::from_script(id), None)
            .unwrap();
        f.scripts.apply_settings_sync(&mut f.core).unwrap();
        f
    }

    /// Pause only publication of a real WorkAdmission result. The worker can
    /// finish and release its thread while the owner holds the exact terminal.
    fn block_preparation(f: &mut Fixture) -> Arc<AtomicBool> {
        let op = *f.core.preparations.keys().max().unwrap();
        let gate = Arc::new(AtomicBool::new(false));
        let pending = f.core.preparations.get_mut(&op).unwrap();
        pending.publication_gate = Some(Arc::clone(&gate));
        gate
    }

    /// Poll until the slot is Running; a slot that ends in `Error` has failed
    /// its Start, which fails the test now.
    fn wait_running(f: &mut Fixture, name: &str) {
        let guard = Instant::now() + HANG_GUARD;
        loop {
            let state = f.core.play().unwrap().script_state(name);
            if state == script::RunState::Running {
                return;
            }
            assert!(
                state != script::RunState::Error,
                "{name} ended {state:?} instead of Running"
            );
            assert!(
                Instant::now() < guard,
                "{name} did not start: hang guard expired"
            );
            f.core.poll();
            f.scripts.poll(&mut f.core);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_pre_copy_start(f: &mut Fixture) {
        let id = script::CompiledId("Gatherer");
        let before = f
            .scripts
            .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
            .unwrap();
        assert_ne!(before["targetPreference"], json!("Nearest"));
        f.scripts.start_profile(&mut f.core, "bob", None).unwrap();
        wait_running(f, "bob");
        assert_eq!(
            f.configure_expected_bag("bob", id, before),
            script::CompiledDelivery::Unchanged,
        );
        let report = f.scripts.last_settings_sync().unwrap();
        assert_eq!(
            (report.saved, report.failed.len(), report.pending()),
            (0, 1, 0)
        );
    }

    #[test]
    fn fleet_copy_hold_survives_native_supersession_until_replacement_commit() {
        let mut f = preparing_copy("copy-superseded-hold");
        let id = script::CompiledId("Gatherer");
        let original = *f.core.preparations.keys().next().unwrap();
        f.scripts.start_all(&mut f.core, None);
        let place = f.scripts.start_queue_place("bob").unwrap();
        f.scripts
            .set_compiled_setting(
                &mut f.core,
                "bob",
                CompiledCardId::from_script(id),
                "radius",
                json!(24),
            )
            .unwrap();
        let release = block_preparation(&mut f);

        // Settle only the superseded copy, leaving the real replacement
        // result gated. The queued Start must not consume the pre-copy bag.
        let guard = Instant::now() + HANG_GUARD;
        while f.core.preparations.contains_key(&original) {
            assert!(
                Instant::now() < guard,
                "original copy did not settle: hang guard expired"
            );
            f.core.poll();
            std::thread::yield_now();
        }
        for _ in 0..3 {
            f.core.poll();
            f.scripts.poll(&mut f.core);
        }
        let state = f.core.play().unwrap().script_state("bob");
        let held_place = f.scripts.start_queue_place("bob");
        release.store(true, Ordering::Release);
        assert_eq!(state, script::RunState::Idle, "replacement still preparing");
        assert_eq!(held_place, Some(place));

        // The transferred hold also survives the replacement's writer boundary.
        let gate = f.core.write_gate();
        let held = gate.lock().unwrap();
        f.core.take_preparations(true);
        f.scripts.poll(&mut f.core);
        assert_eq!(
            f.core.play().unwrap().script_state("bob"),
            script::RunState::Idle
        );
        assert_eq!(f.scripts.start_queue_place("bob"), Some(place));
        drop(held);
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);
        wait_running(&mut f, "bob");
        let bag = f
            .scripts
            .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
            .unwrap();
        assert_eq!(bag["targetPreference"], json!("Nearest"));
        assert_eq!(bag["radius"], json!(24));
        assert_eq!(
            f.configure_expected_bag("bob", id, bag),
            script::CompiledDelivery::Unchanged,
            "the queued run starts with the durable replacement, not a late push"
        );
        assert_eq!(f.scripts.last_settings_sync().unwrap().superseded, 1);
    }

    #[test]
    fn failed_native_preparation_releases_copy_hold() {
        let mut f = preparing_copy("copy-preparation-failure");
        let op = *f.core.preparations.keys().next().unwrap();
        let mut pending = f.core.preparations.remove(&op).unwrap();
        pending.worker.take().unwrap().join().unwrap().unwrap();
        let id = script::CompiledId("Gatherer");
        let mut invalid = f
            .scripts
            .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
            .unwrap();
        invalid.insert("radius".into(), json!("not a radius"));
        pending.worker = Some(
            f.core
                .play()
                .unwrap()
                .script_prepare_config(id, op.0, Arc::new(invalid))
                .unwrap(),
        );
        f.core.preparations.insert(op, pending);
        f.core.take_preparations(true);
        f.scripts.poll(&mut f.core);
        assert_pre_copy_start(&mut f);
    }

    #[test]
    fn cancelled_native_preparation_releases_copy_hold_without_a_late_write() {
        let mut f = preparing_copy("copy-cancel");
        let release = block_preparation(&mut f);
        assert!(f.scripts.start_profile(&mut f.core, "bob", None).is_err());
        f.scripts.cancel_settings_sync();
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert_pre_copy_start(&mut f);
        release.store(true, Ordering::Release);
        f.core.flush_writes();
        let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
        let entry = disk
            .get("bob")
            .unwrap()
            .settings
            .script_settings
            .get(&script::compiled_identity_key(script::CompiledId(
                "Gatherer",
            )))
            .cloned();
        assert!(entry
            .as_ref()
            .and_then(|entry| vault::CompiledSettingsRecord::view(entry).ok())
            .is_none_or(|(_, values)| values.get("targetPreference") != Some(&json!("Nearest"))));
    }

    #[test]
    fn withheld_native_preparation_times_out_and_releases_copy_hold() {
        let mut f = preparing_copy("copy-preparation-timeout");
        let release = block_preparation(&mut f);
        f.core
            .preparations
            .values_mut()
            .next()
            .unwrap()
            .copy_start_hold
            .as_mut()
            .unwrap()
            .deadline = Instant::now();
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert_pre_copy_start(&mut f);
        assert!(f.scripts.last_settings_sync().unwrap().failed[0]
            .1
            .contains("timed out"));
        release.store(true, Ordering::Release);
        f.core.flush_writes();
    }

    #[test]
    fn removed_slot_releases_its_native_preparation_hold() {
        let mut f = preparing_copy("copy-removed");
        let release = block_preparation(&mut f);
        f.core
            .remove("bob", Instant::now(), &mut HeadlessSurface::new());
        f.core.poll();
        f.scripts.poll(&mut f.core);
        f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
        wait_running(&mut f, "alice");
        assert_eq!(f.scripts.last_settings_sync().unwrap().failed.len(), 1);
        release.store(true, Ordering::Release);
        f.core.flush_writes();
    }

    #[test]
    fn deleted_native_copy_target_releases_its_hold_with_unavailable_error() {
        let mut f = preparing_copy("copy-deleted-target");
        let release = block_preparation(&mut f);
        f.core
            .vault_remove("bob")
            .unwrap()
            .expect("delete bob profile");
        f.core.poll();
        f.scripts.poll(&mut f.core);

        let report = f.scripts.last_settings_sync().unwrap();
        assert_eq!(
            (report.saved, report.failed.len(), report.pending()),
            (0, 1, 0),
            "{report:?}"
        );
        assert_eq!(
            report.failed[0].1,
            "settings copy cancelled: bot or vault unavailable"
        );
        release.store(true, Ordering::Release);
        f.core.flush_writes();
    }

    #[test]
    fn locked_vault_releases_its_native_preparation_hold() {
        let mut f = preparing_copy("copy-locked");
        let id = script::CompiledId("Gatherer");
        let before = f
            .scripts
            .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
            .unwrap();
        let release = block_preparation(&mut f);
        f.core.vault = None;
        f.core.poll();
        f.scripts.poll(&mut f.core);
        f.core
            .play()
            .unwrap()
            .script_start_typed("bob", id, before.clone())
            .unwrap();
        wait_running(&mut f, "bob");
        assert_eq!(
            f.configure_expected_bag("bob", id, before),
            script::CompiledDelivery::Unchanged
        );
        let report = f.scripts.last_settings_sync().unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            report.failed[0].1,
            "settings copy cancelled: bot or vault unavailable"
        );
        release.store(true, Ordering::Release);
    }

    #[test]
    fn stalled_copy_writer_releases_hold_and_does_not_push_a_late_commit() {
        let mut f = preparing_copy("copy-writer-timeout");
        let id = script::CompiledId("Gatherer");
        let before = f
            .scripts
            .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
            .unwrap();
        let gate = f.core.write_gate();
        let held = gate.lock().unwrap();
        f.core.take_preparations(true);
        let write = f
            .core
            .writes
            .values_mut()
            .find(|write| write.member == "bob")
            .unwrap();
        write.copy_start_hold.as_mut().unwrap().deadline = Instant::now();
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert_pre_copy_start(&mut f);
        drop(held);
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);
        assert_eq!(
            f.configure_expected_bag("bob", id, before),
            script::CompiledDelivery::Unchanged
        );
        let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
        let key = script::compiled_identity_key(id);
        let entry = &disk.get("bob").unwrap().settings.script_settings[&key];
        assert_eq!(
            vault::CompiledSettingsRecord::view(entry).unwrap().1["targetPreference"],
            json!("Nearest")
        );
        assert_eq!(f.scripts.last_settings_sync().unwrap().saved, 0);
    }

    #[test]
    fn fleet_copy_follows_a_renamed_native_target_before_confirm() {
        let mut f = native_fixture("copy-rename-before-confirm");
        let id = script::CompiledId("Gatherer");
        for name in ["alice", "bob"] {
            assert!(f.scripts.persist_assignment(
                &mut f.core,
                name,
                script::compiled_assignment(id)
            ));
        }
        f.core.flush_writes();
        f.scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                CompiledCardId::from_script(id),
                json!({"targetPreference": "Nearest"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        f.core.flush_writes();

        let target = ProfileIdentity::uid(2);
        let mut marked = MarkedSelection::default();
        marked.set(target, true);
        prepare_apply_settings_marked(
            &marked,
            &f.core,
            &mut f.scripts,
            "alice",
            &ScriptChoice::from_script(script::ScriptSel::Compiled(id)),
        )
        .unwrap();
        f.scripts
            .freeze_settings_sync_targets(vec![(target, "bob".into())]);

        let mut renamed = f.core.vault().unwrap().get("bob").unwrap().clone();
        renamed.username = "bobby".into();
        f.core
            .rename_profile("bob", renamed, ArmMirror::none(), "profile")
            .unwrap();
        f.core.flush_writes();

        f.scripts.apply_settings_sync(&mut f.core).unwrap();
        let release = block_preparation(&mut f);
        f.core.poll();
        f.scripts.poll(&mut f.core);
        release.store(true, Ordering::Release);
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);

        let report = f.scripts.last_settings_sync().unwrap();
        assert_eq!(
            (report.saved, report.failed.len(), report.pending()),
            (1, 0, 0),
            "{report:?}"
        );
        let profile = f.core.vault().unwrap().get("bobby").unwrap();
        let entry = &profile.settings.script_settings[&script::compiled_identity_key(id)];
        assert_eq!(
            vault::CompiledSettingsRecord::view(entry).unwrap().1["targetPreference"],
            json!("Nearest")
        );
    }

    #[test]
    fn accepted_marked_native_copy_commits_assignment_and_settings_before_start() {
        let mut f = native_fixture("accepted-copy");
        let id = script::CompiledId("Gatherer");
        let assignment = script::compiled_assignment(id);
        assert!(f
            .scripts
            .persist_assignment(&mut f.core, "alice", assignment.clone()));
        f.core.flush_writes();
        assert_eq!(Scripts::assignment(&f.core, "alice"), Some(assignment));

        let mut source_values = Map::new();
        source_values.insert("targetPreference".into(), json!("Nearest"));
        f.scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                CompiledCardId::from_script(id),
                source_values,
            )
            .unwrap();
        f.core.flush_writes();

        let mut marked = MarkedSelection::default();
        marked.mark_all([ProfileIdentity::uid(2)]);
        prepare_apply_settings_marked(
            &marked,
            &f.core,
            &mut f.scripts,
            "alice",
            &ScriptChoice::from_script(script::ScriptSel::Compiled(id)),
        )
        .unwrap();
        f.scripts.apply_settings_sync(&mut f.core).unwrap();
        assert!(
            Scripts::assignment(&f.core, "bob").is_none(),
            "the assignment stays in the unvalidated preparation draft"
        );

        // This profile write lands while Bob's native settings are preparing.
        // Settlement must merge it without replacing the accepted card entry
        // or losing the explicit assignment intent.
        let mut unrelated = f.core.vault().unwrap().get("bob").unwrap().clone();
        unrelated.settings.random_events = false;
        f.core
            .save_profile(unrelated, crate::ArmMirror::none(), "unrelated")
            .unwrap();
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);
        let report = f.scripts.last_settings_sync().unwrap();
        assert_eq!(report.saved, 1, "{report:?}");
        assert!(report.failed.is_empty(), "{report:?}");

        let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
        let saved = disk.get("bob").unwrap();
        assert_eq!(
            saved.settings.script_assignment.as_ref(),
            Some(&script::compiled_assignment(id))
        );
        assert!(
            !saved.settings.random_events,
            "the unrelated write survived"
        );
        let key = script::compiled_identity_key(id);
        let entry = saved.settings.script_settings.get(&key).unwrap();
        let (schema_version, values) = vault::CompiledSettingsRecord::view(entry).unwrap();
        assert_eq!(
            schema_version,
            script::compiled_card(id).unwrap().schema_version
        );
        assert_eq!(values.get("targetPreference"), Some(&json!("Nearest")));

        let copied_bag = f
            .scripts
            .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
            .unwrap();
        assert_eq!(copied_bag["targetPreference"], "Nearest");
        f.scripts.start_profile(&mut f.core, "bob", None).unwrap();
        wait_running(&mut f, "bob");
        assert_eq!(
            f.configure_expected_bag("bob", id, copied_bag),
            script::CompiledDelivery::Unchanged,
            "Bob's running Gatherer has the copied non-default setting"
        );
    }

    #[test]
    fn accepted_native_copy_cannot_overwrite_a_later_explicit_assignment() {
        for supersede in [false, true] {
            let mut f = native_fixture(if supersede {
                "copy-reassigned-superseded"
            } else {
                "copy-reassigned"
            });
            let id = script::CompiledId("Gatherer");
            assert!(f.scripts.persist_assignment(
                &mut f.core,
                "alice",
                script::compiled_assignment(id)
            ));
            f.scripts
                .set_compiled_setting(
                    &mut f.core,
                    "alice",
                    CompiledCardId::from_script(id),
                    "radius",
                    json!(24),
                )
                .unwrap();
            f.core.flush_writes();

            // Apply to marked accepts Bob while he has no assignment. Native
            // preparation has not been folded into the vault yet.
            let mut marked = MarkedSelection::default();
            marked.mark_all([ProfileIdentity::uid(2)]);
            prepare_apply_settings_marked(
                &marked,
                &f.core,
                &mut f.scripts,
                "alice",
                &ScriptChoice::from_script(script::ScriptSel::Compiled(id)),
            )
            .unwrap();
            f.scripts.apply_settings_sync(&mut f.core).unwrap();
            assert!(Scripts::assignment(&f.core, "bob").is_none());

            // An explicit Assign wins over that older accepted copy. No
            // subsequent Start, Assign or Browse-pick targets Bob.
            let thiever = vault::ScriptAssignment {
                source_kind: "catalog".into(),
                identity: "Thiever".into(),
                display_name: "Thiever".into(),
                unavailable: None,
            };
            assert!(f
                .scripts
                .persist_assignment(&mut f.core, "bob", thiever.clone()));
            // Settle only the writer, not preparations: Thiever must actually
            // be on disk before the older native intent is allowed to finish.
            let written = f.core.writer.as_mut().unwrap().wait_take().unwrap();
            f.core.settle_write(written);
            let before = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
            assert_eq!(
                before
                    .get("bob")
                    .unwrap()
                    .settings
                    .script_assignment
                    .as_ref(),
                Some(&thiever)
            );
            if supersede {
                // A settings-only replacement must not inherit a stale
                // assignment intent from the accepted same-card copy.
                f.scripts
                    .set_compiled_setting(
                        &mut f.core,
                        "bob",
                        CompiledCardId::from_script(id),
                        "radius",
                        json!(32),
                    )
                    .unwrap();
            }
            f.core.select("alice");
            f.scripts
                .set_compiled_setting(
                    &mut f.core,
                    "alice",
                    CompiledCardId::from_script(id),
                    "radius",
                    json!(40),
                )
                .unwrap();
            assert_eq!(Scripts::assignment(&f.core, "bob"), Some(thiever.clone()));
            f.core.flush_writes();
            f.scripts.poll(&mut f.core);

            let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
            let bob = disk.get("bob").unwrap();
            assert_eq!(
                bob.settings.script_assignment.as_ref(),
                Some(&thiever),
                "supersede={supersede}: the old copy must not reassign Bob"
            );
            let entry = &bob.settings.script_settings[&script::compiled_identity_key(id)];
            let (_, values) = vault::CompiledSettingsRecord::view(entry).unwrap();
            assert_eq!(values["radius"], json!(if supersede { 32 } else { 24 }));
            assert_eq!(Scripts::assignment(&f.core, "bob"), Some(thiever));
        }
    }

    fn assert_second_copy_wins(second_finishes_first: bool) {
        let mut f = native_fixture(if second_finishes_first {
            "second-copy-finishes-first"
        } else {
            "second-copy-finishes-last"
        });
        let gatherer = script::CompiledId("Gatherer");
        #[cfg(feature = "js")]
        let sherlock = script::CompiledId("Sherlock");
        #[cfg(not(feature = "js"))]
        let sherlock = script::CompiledId("Grinch");
        f.scripts
            .set_compiled_overrides_assigning(
                &mut f.core,
                "bob",
                gatherer,
                Map::new(),
                Some(script::compiled_assignment(gatherer)),
            )
            .unwrap();
        let gate = second_finishes_first.then(|| block_preparation(&mut f));
        assert!(Scripts::assignment(&f.core, "bob").is_none());
        let second = f
            .scripts
            .set_compiled_overrides_assigning(
                &mut f.core,
                "bob",
                sherlock,
                Map::new(),
                Some(script::compiled_assignment(sherlock)),
            )
            .unwrap();
        assert!(Scripts::assignment(&f.core, "bob").is_none());

        let intermediate = if let Some(release) = gate {
            let guard = Instant::now() + HANG_GUARD;
            while f.core.preparations.contains_key(&second) {
                assert!(
                    Instant::now() < guard,
                    "second copy did not settle: hang guard expired"
                );
                f.core.poll();
                std::thread::yield_now();
            }
            while let Some(written) = f.core.writer.as_mut().unwrap().wait_take() {
                f.core.settle_write(written);
            }
            let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
            let assignment = disk.get("bob").unwrap().settings.script_assignment.clone();
            // Release before asserting so a negative control cannot strand
            // the older real preparation during fixture cleanup.
            release.store(true, Ordering::Release);
            Some(assignment)
        } else {
            None
        };
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);

        let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
        let saved = disk.get("bob").unwrap();
        assert_eq!(
            saved.settings.script_assignment,
            Some(script::compiled_assignment(sherlock)),
            "the later accepted copy onto a vault-unassigned bot must win"
        );
        if let Some(assignment) = intermediate {
            assert_eq!(
                assignment,
                Some(script::compiled_assignment(sherlock)),
                "the later copy must be durable before the older one finishes"
            );
        }
        for id in [gatherer, sherlock] {
            assert!(
                saved
                    .settings
                    .script_settings
                    .contains_key(&script::compiled_identity_key(id)),
                "both accepted copies must save their settings"
            );
        }
    }

    #[test]
    fn zz_review_second_copy_onto_unassigned_bot() {
        for second_finishes_first in [false, true] {
            assert_second_copy_wins(second_finishes_first);
        }
    }

    #[test]
    fn settings_only_native_preparation_preserves_concurrent_assignment() {
        let mut f = native_fixture("settings-only-assignment");
        let id = script::CompiledId("Gatherer");
        assert!(f
            .scripts
            .persist_assignment(&mut f.core, "bob", script::compiled_assignment(id)));
        f.core.flush_writes();

        let mut values = Map::new();
        values.insert("targetPreference".into(), json!("Nearest"));
        f.scripts
            .set_compiled_overrides(&mut f.core, "bob", CompiledCardId::from_script(id), values)
            .unwrap();

        let concurrent_assignment = vault::ScriptAssignment {
            source_kind: "catalog".into(),
            identity: "Other".into(),
            display_name: "Other".into(),
            unavailable: None,
        };
        assert!(f
            .scripts
            .persist_assignment(&mut f.core, "bob", concurrent_assignment.clone()));
        f.core.flush_writes();

        let disk = Vault::unlock(&f.dir.join("vault"), PASSPHRASE).unwrap();
        let saved = disk.get("bob").unwrap();
        assert_eq!(
            saved.settings.script_assignment.as_ref(),
            Some(&concurrent_assignment),
            "a settings-only draft must not restore its stale assignment"
        );
        let key = script::compiled_identity_key(id);
        let entry = saved.settings.script_settings.get(&key).unwrap();
        let (_, values) = vault::CompiledSettingsRecord::view(entry).unwrap();
        assert_eq!(values.get("targetPreference"), Some(&json!("Nearest")));
    }
}

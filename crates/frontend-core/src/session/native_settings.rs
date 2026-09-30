//! Native settings preparation is a persistence prerequisite, never a pump callback.
use super::*;
use script::native::{PreparedConfig, SettingsBag, StartError};
use std::thread::JoinHandle;

pub(super) struct PendingPreparation {
    profile: Profile,
    renamed_from: Option<String>,
    mirror: ArmMirror,
    label: &'static str,
    worker: JoinHandle<Result<Arc<PreparedConfig>, StartError>>,
}

pub(super) struct PendingDelivery {
    op: OperationId,
    name: String,
    live: LiveSettings,
    worker: JoinHandle<Result<Arc<PreparedConfig>, StartError>>,
}

impl ArmMirror {
    pub(super) fn native_identity(&self) -> Option<String> {
        match self {
            Self::NativeSettings { id, .. } => Some(script::compiled_identity_key(*id)),
            Self::ScriptSettings { card, .. } => Some(card.clone()),
            Self::Remember(Some(live)) if live.run.is_some() => Some(live.identity.clone()),
            _ => None,
        }
    }

    pub(super) fn native_draft(&self) -> Option<(script::CompiledId, Arc<SettingsBag>)> {
        match self {
            Self::NativeSettings { id, bag, .. } => Some((*id, Arc::clone(bag))),
            _ => None,
        }
    }

    fn prepared(self, config: Arc<PreparedConfig>) -> Self {
        match self {
            Self::NativeSettings { id, live, .. } => Self::ScriptSettings {
                card: script::compiled_identity_key(id),
                live: live.map(|mut live| {
                    live.prepared = Some(config);
                    live
                }),
            },
            other => other,
        }
    }
}

impl<Io> OperatorSession<Io> {
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
                result: SettingsResult::Saved(LiveDelivery::Stale),
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
                play.script_prepare_config(id, revision, Arc::clone(&live.bag))
            });
        match result {
            Ok(worker) => self.deliveries.push(PendingDelivery {
                op,
                name,
                live,
                worker,
            }),
            Err(error) => self.settings_writes.push(SettingsWrite {
                op,
                profile: name,
                result: SettingsResult::Saved(LiveDelivery::Rejected(error)),
            }),
        }
    }

    pub(super) fn take_native_deliveries(&mut self, wait: bool) {
        let mut index = 0;
        while index < self.deliveries.len() {
            if !wait && !self.deliveries[index].worker.is_finished() {
                index += 1;
                continue;
            }
            let mut pending = self.deliveries.remove(index);
            let prepared = pending
                .worker
                .join()
                .map_err(|_| "native settings preparation worker panicked".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let delivery =
                if !self.native_edit_current(&pending.name, &pending.live.identity, pending.op) {
                    LiveDelivery::Stale
                } else if let Some(play) = &self.play {
                    if play.script_native_run(&pending.name) != pending.live.run {
                        LiveDelivery::Stale
                    } else {
                        match prepared {
                            Ok(config) => {
                                pending.live.prepared = Some(config);
                                deliver_settings(play, &pending.name, Some(pending.live))
                            }
                            Err(error) => LiveDelivery::Rejected(error),
                        }
                    }
                } else {
                    LiveDelivery::Stale
                };
            self.settings_writes.push(SettingsWrite {
                op: pending.op,
                profile: pending.name,
                result: SettingsResult::Saved(delivery),
            });
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
        let play = self
            .play
            .as_ref()
            .ok_or_else(|| format!("{label}: no play"))?;
        let op = self.operations.open(ActionKind::SaveProfile);
        let revision =
            op.0.max(play.script_native_settings_revision(source).unwrap_or(1))
                .checked_add(1)
                .ok_or("settings revision exhausted")?;
        let worker = match play.script_prepare_config(id, revision, bag) {
            Ok(worker) => worker,
            Err(error) => {
                self.operations
                    .set(op, &profile.username, Outcome::Failed(error.clone()));
                return Err(error);
            }
        };
        self.operations.set(op, &profile.username, Outcome::Pending);
        let card = script::compiled_identity_key(id);
        self.native_edits
            .insert((source.to_owned(), card.clone()), op);
        self.native_edits
            .insert((profile.username.clone(), card), op);
        self.preparations.insert(
            op,
            PendingPreparation {
                profile,
                renamed_from: renamed_from.map(str::to_owned),
                mirror,
                label,
                worker,
            },
        );
        Ok(op)
    }

    pub(super) fn take_preparations(&mut self, wait: bool) {
        if self.preparations.is_empty() {
            return;
        }
        let mut ready: Vec<_> = self
            .preparations
            .iter()
            .filter(|(_, pending)| wait || pending.worker.is_finished())
            .map(|(op, _)| *op)
            .collect();
        ready.sort_unstable();
        for op in ready {
            let mut pending = self.preparations.remove(&op).expect("ready preparation");
            let name = pending.profile.username.clone();
            let source = pending.renamed_from.as_deref().unwrap_or(&name);
            let result = pending
                .worker
                .join()
                .map_err(|_| "native settings preparation worker panicked".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
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
            let config = match result {
                Ok(config) => config,
                Err(error) => {
                    self.operations
                        .set(op, &name, Outcome::Failed(error.clone()));
                    self.write_failures
                        .push(crate::profile_saves::WriteFailure {
                            op,
                            target: name.clone(),
                            label: pending.label,
                            error: error.clone(),
                        });
                    if matches!(pending.mirror, ArmMirror::NativeSettings { .. }) {
                        self.settings_writes.push(SettingsWrite {
                            op,
                            profile: name.clone(),
                            result: SettingsResult::Failed(error),
                        });
                    }
                    continue;
                }
            };
            if !current {
                self.operations.set(op, &name, Outcome::Cancelled);
                if matches!(pending.mirror, ArmMirror::NativeSettings { .. }) {
                    self.settings_writes.push(SettingsWrite {
                        op,
                        profile: name.clone(),
                        result: SettingsResult::Superseded,
                    });
                }
                continue;
            }
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
                if let Some(vault) = self.vault.as_mut() {
                    vault.stage_remove(&old);
                }
                changes.push(VaultChange::Remove(old));
            }
            if let Some(vault) = self.vault.as_mut() {
                vault.stage_upsert(pending.profile);
            }
            self.submit_write_at(op, changes, pending.mirror.prepared(config), pending.label);
        }
    }
}

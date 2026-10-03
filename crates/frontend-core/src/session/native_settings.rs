//! Native settings preparation is a persistence prerequisite, never a pump callback.
//! Same-card replacements inherit accepted copy holds through their durable commit.
use super::*;
use script::native::{PreparedConfig, SettingsBag, StartError};
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;

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

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Accepted native copies own the confirmation token through preparation and
/// the profile commit. A stalled copy must not strand the Start queue.
pub(super) struct NativeCopyHold {
    _token: Arc<String>,
    deadline: Instant,
    cancellation: NativeCopyCancellation,
}

impl NativeCopyHold {
    const TIMEOUT: Duration = Duration::from_secs(30);

    fn new(token: Arc<String>, cancellation: NativeCopyCancellation) -> Self {
        Self {
            _token: token,
            deadline: Instant::now() + Self::TIMEOUT,
            cancellation,
        }
    }
}

pub(super) struct PendingPreparation {
    profile: Profile,
    /// Explicit or inherited intent from a pending same-card draft; absent
    /// intent preserves the latest assignment during rebase.
    assignment_intent: Option<Option<vault::ScriptAssignment>>,
    renamed_from: Option<String>,
    mirror: ArmMirror,
    label: &'static str,
    worker: JoinHandle<Result<Arc<PreparedConfig>, StartError>>,
    copy_start_hold: Option<NativeCopyHold>,
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

    /// Attach the Apply token to the existing preparation, not a second
    /// completion tracker. JS copies stage their bag synchronously.
    pub(crate) fn hold_native_copy_start(
        &mut self,
        op: OperationId,
        token: Arc<String>,
        cancellation: NativeCopyCancellation,
    ) {
        if let Some(pending) = self.preparations.get_mut(&op) {
            pending.copy_start_hold = Some(NativeCopyHold::new(token, cancellation));
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
        let assignment_intent = self.profile_for_edit(source).and_then(|base| {
            if base.settings.script_assignment != profile.settings.script_assignment {
                Some(profile.settings.script_assignment.clone())
            } else {
                // Same-card edits compose over their current draft. If it
                // owns an assignment intent, carry it through supersession.
                self.native_edits
                    .get(&(source.to_owned(), card.clone()))
                    .and_then(|prior_op| self.preparations.get(prior_op))
                    .and_then(|pending| pending.assignment_intent.clone())
            }
        });
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
                copy_start_hold,
            },
        );
        Ok(op)
    }

    pub(super) fn take_preparations(&mut self, wait: bool) {
        if self.preparations.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut ready: Vec<_> = self
            .preparations
            .iter()
            .filter(|(_, pending)| {
                wait || pending.worker.is_finished()
                    || pending.copy_start_hold.as_ref().is_some_and(|hold| {
                        now >= hold.deadline
                            || hold.cancellation.is_cancelled()
                            || self.play.is_none()
                            || self.vault.is_none()
                            || !self.members().contains(&pending.profile.username)
                    })
            })
            .map(|(op, _)| *op)
            .collect();
        ready.sort_unstable();
        for op in ready {
            let mut pending = self.preparations.remove(&op).expect("ready preparation");
            let name = pending.profile.username.clone();
            let source = pending.renamed_from.as_deref().unwrap_or(&name);
            let result = if pending
                .copy_start_hold
                .as_ref()
                .is_some_and(|hold| now >= hold.deadline)
                && !pending.worker.is_finished()
            {
                // Dropping the JoinHandle detaches a stuck preparer. Its late
                // result cannot be persisted after this failed copy.
                Err("settings copy timed out during native preparation".to_string())
            } else if pending
                .copy_start_hold
                .as_ref()
                .is_some_and(|hold| hold.cancellation.is_cancelled())
            {
                Err("settings copy cancelled".to_string())
            } else if pending.copy_start_hold.is_some()
                && (self.play.is_none() || self.vault.is_none() || !self.members().contains(&name))
            {
                Err("settings copy cancelled: bot or vault unavailable".to_string())
            } else {
                pending
                    .worker
                    .join()
                    .map_err(|_| "native settings preparation worker panicked".to_string())
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
                if let Some(assignment) = pending.assignment_intent {
                    latest.settings.script_assignment = assignment;
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
            if let Some(write) = self.writes.get_mut(&op) {
                write.copy_start_hold = pending.copy_start_hold;
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
            let unavailable = self.play.is_none()
                || self.vault.is_none()
                || !self.fleet.members().contains(&pending.member);
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
                .set(*op, &pending.member, Outcome::Failed(error.clone()));
            self.write_failures.push(WriteFailure {
                op: *op,
                target: pending.member.clone(),
                label: pending.label,
                error: error.clone(),
            });
            self.settings_writes.push(SettingsWrite {
                op: *op,
                profile: pending.member.clone(),
                result: SettingsResult::Failed(error),
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::marked::prepare_apply_settings_marked;
    use crate::scripts::Scripts;
    use crate::selection::{MarkedSelection, ProfileIdentity};
    use crate::surface::HeadlessSurface;
    use host_play::{InstancePermit, Play, PlayOptions};
    use serde_json::{json, Map, Value};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use vault::{Profile, ProfileSettings, Vault};

    const PASSPHRASE: &str = "test-passphrase-01";

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
        let scripts = Scripts::new(
            script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
            script::ScriptSettingsStore::at(dir.join("script-settings.json")),
        );
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
            let prepared = play
                .script_prepare_config(id, revision, Arc::new(bag))
                .unwrap()
                .join()
                .unwrap()
                .unwrap();
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
                id,
                json!({"targetPreference": "Nearest"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        f.core.flush_writes();
        f.scripts
            .prepare_compiled_settings_sync(&f.core, "alice", id, None)
            .unwrap();
        f.scripts.apply_settings_sync(&mut f.core).unwrap();
        f
    }

    /// Gate a real preparation result without relying on worker scheduling.
    fn block_preparation(
        f: &mut Fixture,
    ) -> (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>) {
        let op = *f.core.preparations.keys().max().unwrap();
        let mut pending = f.core.preparations.remove(&op).unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        // Keep the real worker in the gated worker; a late result has no
        // access to the session or writer after its handle is discarded.
        let original = pending.worker;
        pending.worker = std::thread::spawn(move || {
            let result = original.join().unwrap();
            wait.recv().unwrap();
            finished.send(()).unwrap();
            result
        });
        f.core.preparations.insert(op, pending);
        (release, done)
    }

    fn wait_running(f: &mut Fixture, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while f.core.play().unwrap().script_state(name) != script::RunState::Running {
            assert!(Instant::now() < deadline, "{name} did not start");
            f.core.poll();
            f.scripts.poll(&mut f.core);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_pre_copy_start(f: &mut Fixture) {
        let id = script::CompiledId("Gatherer");
        let before = f.scripts.compiled_bag(&f.core, "bob", id).unwrap();
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
            .set_compiled_setting(&mut f.core, "bob", id, "radius", json!(24))
            .unwrap();
        let (release, done) = block_preparation(&mut f);

        // Settle only the superseded copy, leaving the real replacement
        // result gated. The queued Start must not consume the pre-copy bag.
        let deadline = Instant::now() + Duration::from_secs(10);
        while f.core.preparations.contains_key(&original) {
            assert!(Instant::now() < deadline, "original copy did not settle");
            f.core.poll();
            std::thread::yield_now();
        }
        for _ in 0..3 {
            f.core.poll();
            f.scripts.poll(&mut f.core);
        }
        let state = f.core.play().unwrap().script_state("bob");
        let held_place = f.scripts.start_queue_place("bob");
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
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
        let bag = f.scripts.compiled_bag(&f.core, "bob", id).unwrap();
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
        pending.worker.join().unwrap().unwrap();
        pending.worker = std::thread::spawn(|| {
            Err(StartError::Unavailable(
                "injected preparation failure".into(),
            ))
        });
        f.core.preparations.insert(op, pending);
        f.core.take_preparations(true);
        f.scripts.poll(&mut f.core);
        assert_pre_copy_start(&mut f);
    }

    #[test]
    fn cancelled_native_preparation_releases_copy_hold_without_a_late_write() {
        let mut f = preparing_copy("copy-cancel");
        let (release, done) = block_preparation(&mut f);
        assert!(f.scripts.start_profile(&mut f.core, "bob", None).is_err());
        f.scripts.cancel_settings_sync();
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert_pre_copy_start(&mut f);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
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
    fn stuck_native_preparation_times_out_and_releases_copy_hold() {
        let mut f = preparing_copy("copy-preparation-timeout");
        let (release, done) = block_preparation(&mut f);
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
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
        f.core.flush_writes();
    }

    #[test]
    fn removed_slot_releases_its_native_preparation_hold() {
        let mut f = preparing_copy("copy-removed");
        let (release, done) = block_preparation(&mut f);
        f.core
            .remove("bob", Instant::now(), &mut HeadlessSurface::new());
        f.core.poll();
        f.scripts.poll(&mut f.core);
        f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
        wait_running(&mut f, "alice");
        assert_eq!(f.scripts.last_settings_sync().unwrap().failed.len(), 1);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
        f.core.flush_writes();
    }

    #[test]
    fn locked_vault_releases_its_native_preparation_hold() {
        let mut f = preparing_copy("copy-locked");
        let id = script::CompiledId("Gatherer");
        let before = f.scripts.compiled_bag(&f.core, "bob", id).unwrap();
        let (release, done) = block_preparation(&mut f);
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
        assert_eq!(f.scripts.last_settings_sync().unwrap().failed.len(), 1);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn stalled_copy_writer_releases_hold_and_does_not_push_a_late_commit() {
        let mut f = preparing_copy("copy-writer-timeout");
        let id = script::CompiledId("Gatherer");
        let before = f.scripts.compiled_bag(&f.core, "bob", id).unwrap();
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
            .set_compiled_overrides(&mut f.core, "alice", id, source_values)
            .unwrap();
        f.core.flush_writes();

        let mut marked = MarkedSelection::default();
        marked.mark_all([ProfileIdentity::uid(2)]);
        prepare_apply_settings_marked(
            &marked,
            &f.core,
            &mut f.scripts,
            "alice",
            &script::ScriptSel::Compiled(id),
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
            .save_profile(unrelated, crate::ArmMirror::None, "unrelated")
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

        let copied_bag = f.scripts.compiled_bag(&f.core, "bob", id).unwrap();
        assert_eq!(copied_bag["targetPreference"], "Nearest");
        f.scripts.start_profile(&mut f.core, "bob", None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while f.core.play().unwrap().script_state("bob") != script::RunState::Running {
            assert!(
                Instant::now() < deadline,
                "Bob's profile Start did not settle"
            );
            f.core.poll();
            f.scripts.poll(&mut f.core);
            std::thread::yield_now();
        }
        assert_eq!(
            f.configure_expected_bag("bob", id, copied_bag),
            script::CompiledDelivery::Unchanged,
            "Bob's running Gatherer has the copied non-default setting"
        );
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
            .set_compiled_overrides(&mut f.core, "bob", id, values)
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

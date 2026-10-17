use super::*;
use crate::ctx::test_support::NullDriver;
use std::sync::atomic::{AtomicU64, Ordering};

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()
}

#[test]
fn work_scope_requires_the_bound_slot_lifetime() {
    let mut slot = SlotScript::new();
    assert_eq!(slot.work_scope(), None);
    slot.bind_incarnation(42);
    assert_eq!(
        slot.work_scope(),
        Some(api::identity::Scope::Slot(
            api::identity::SlotKey::from_incarnation(42).unwrap(),
        )),
    );
}
#[cfg(feature = "load")]
fn start(slot: &mut SlotScript, bag: SettingsBag) {
    slot.start_compiled(
        "account",
        crate::CompiledId("Sherlock"),
        Arc::new(bag),
        selected(),
        Arc::default(),
    )
    .unwrap();
}

fn settle(slot: &mut SlotScript) -> StartOutcome {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            StartPoll::Settled(outcome) => return outcome,
            StartPoll::Pending => {
                assert!(Instant::now() < deadline, "native preparation stalled");
                std::thread::yield_now();
            }
            StartPoll::NotOwed => panic!("no pending native start"),
        }
    }
}

fn tick(slot: &mut SlotScript) {
    slot.on_game_tick(&mut ScriptCtx {
        driver: &mut NullDriver::default(),
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::CompiledTick::default(),
    });
}

#[cfg(feature = "load")]
#[test]
fn large_catalogue_job_and_operator_start_both_complete() {
    use crate::preparation::{self, PreparationInput, PreparationKind};
    use api::identity::Scope;

    const SOURCE_BYTES: usize = 64 * 1024 * 1024;
    let source = vec![0_u8; SOURCE_BYTES];
    let (entered, started) = std::sync::mpsc::sync_channel(0);
    let (release, released) = std::sync::mpsc::sync_channel(0);
    let large = preparation::submit(
        PreparationKind::Catalogue,
        Scope::Process,
        PreparationInput::new(1, source.len(), 32 * 1024 * 1024, 32 * 1024 * 1024).unwrap(),
        move |_| {
            entered.send(()).unwrap();
            released.recv().unwrap();
            source.len()
        },
    )
    .unwrap();
    started
        .recv_timeout(Duration::from_secs(5))
        .expect("large catalogue entered the worker");
    let mut slot = SlotScript::new();
    slot.bind_incarnation(102_409);
    start(&mut slot, SettingsBag::new());
    assert_eq!(slot.state(), RunState::Starting);
    release.send(()).unwrap();
    assert_eq!(large.join().unwrap(), SOURCE_BYTES);
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    assert_eq!(slot.state(), RunState::Running);
}

#[cfg(feature = "load")]
#[test]
fn oversized_native_settings_have_a_typed_visible_refusal() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(102_411);
    let bag = SettingsBag::from_iter([(
        "oversized".into(),
        serde_json::Value::String("x".repeat(8 * 1024 * 1024)),
    )]);
    let error = slot
        .start_compiled(
            "account",
            crate::CompiledId("Sherlock"),
            Arc::new(bag),
            selected(),
            Arc::default(),
        )
        .expect_err("oversized settings must not enter the work ledger");
    assert_eq!(
        error,
        StartError::WorkRefused(api::work::SubmissionRefusal::InvalidArgument),
    );
    assert!(error.to_string().contains("preparation refused"));
    assert_eq!(slot.state(), RunState::Idle);
}

#[cfg(feature = "load")]
#[test]
fn native_card_policy_timing() {
    let _selected = selected();
    let mut slot = SlotScript::new();
    slot.bind_incarnation(102_410);
    let start_at = Instant::now();
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    eprintln!("NATIVE_START_POLICY elapsed={:?}", start_at.elapsed());
    let restart_at = Instant::now();
    restart_and_settle(&mut slot, restart_at);
    eprintln!("NATIVE_RESTART_POLICY elapsed={:?}", restart_at.elapsed());
}

#[test]
fn settings_apply_only_publishes_a_nonzero_prepared_revision() {
    let mut output = Output::default();
    output.settings_applied(0);
    assert_eq!(output.applied, None);
    output.settings_applied(7);
    assert_eq!(output.applied.map(std::num::NonZeroU64::get), Some(7));
}

#[cfg(feature = "load")]
#[test]
fn invalid_settings_and_factory_type_leave_no_runnable_instance() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(10);
    slot.attach_source_identity("file:previous.ts");
    start(
        &mut slot,
        serde_json::from_value(serde_json::json!({"clueDuelPartner": 7})).unwrap(),
    );
    assert!(matches!(
        settle(&mut slot),
        StartOutcome::Rejected(StartError::Config(_))
    ));
    assert_eq!(slot.state(), RunState::Idle);
    assert_eq!(slot.source_identity(), Some("file:previous.ts"));
    assert!(slot.native_run().is_none());
    let wrong_type = PreparedConfig::new(crate::CompiledId("Sherlock"), 1, 1, Arc::default(), ());
    assert!(matches!(
        (crate::sherlock::CARD.create)(
            RunKey {
                slot: 10,
                run: 2,
                session: 0
            },
            wrong_type,
            &mut RetainedMemory::default()
        ),
        Err(StartError::Config(_))
    ));
    // A valid retry installs only its own fresh generation and uses no isolate.
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    assert_eq!(slot.state(), RunState::Running);
    assert!(slot.load.is_none());
    assert!(slot
        .start_load(
            "export function tick() {}".into(),
            LoadShape::NativeTick,
            vec![]
        )
        .is_err());
}

#[cfg(feature = "load")]
#[test]
fn a_late_factory_reply_cannot_replace_a_new_run_after_stop() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(11);
    start(&mut slot, SettingsBag::new());
    // Keep the real worker handle so this test can force drain of a completed
    // old reply. In production Stop discards it, making resurrection impossible.
    let stale = slot.preparing.take().unwrap();
    slot.stop();
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    let replacement = slot.native_run().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stale.worker.is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    slot.preparing = Some(stale);
    slot.observe_lifecycle();
    assert_eq!(slot.native_run(), Some(replacement));
    assert_eq!(slot.state(), RunState::Running);
    assert!(slot.preparing.is_none());
}

#[cfg(feature = "load")]
#[test]
fn retained_memory_contention_refuses_without_blocking_shared_preparation() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(12);
    let retained = Arc::new(Mutex::new(RetainedMemory::default()));
    slot.retained = Some(Arc::clone(&retained));
    let held = retained.lock().unwrap();
    start(&mut slot, SettingsBag::new());
    assert!(matches!(
        settle(&mut slot),
        StartOutcome::Rejected(StartError::Unavailable(message))
            if message.contains("retained memory")
    ));
    assert!(slot.native_run().is_none());
    drop(held);
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
}

struct Receiver {
    apply: SettingsApply,
    acknowledge: Arc<AtomicU64>,
}
impl Script for Receiver {
    fn tick(&mut self, cx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        cx.output.status(ScriptStatus {
            run: cx.cx.run(),
            card: crate::CompiledId("test"),
            phase: NativePhase::Working,
            active_settings: 1,
            pending_settings: None,
            fields: Arc::from([]),
            failure: None,
        });
        cx.output
            .settings_applied(self.acknowledge.load(Ordering::Relaxed));
        Ok(ScriptFlow::Continue)
    }
    fn configure(&mut self, next: Arc<PreparedConfig>) -> Result<SettingsApply, ConfigError> {
        next.get::<u64>()
            .ok_or_else(|| ConfigError::new("value", "type", "expected typed value"))?;
        Ok(self.apply)
    }
}

fn receiver(incarnation: u64, apply: SettingsApply) -> (SlotScript, Arc<AtomicU64>) {
    let acknowledge = Arc::new(AtomicU64::new(0));
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_test_script(
        Box::new(Receiver {
            apply,
            acknowledge: Arc::clone(&acknowledge),
        }),
        Some(selected()),
    )
    .unwrap();
    tick(&mut slot);
    (slot, acknowledge)
}

fn config(revision: u64, value: u64) -> Arc<PreparedConfig> {
    PreparedConfig::new(
        crate::CompiledId("test"),
        1,
        revision,
        Arc::new(SettingsBag::from_iter([("value".into(), value.into())])),
        value,
    )
}

#[test]
fn boundary_acknowledgement_never_applies_an_obsolete_or_restart_required_edit() {
    let (mut slot, acknowledge) = receiver(12, SettingsApply::PendingBoundary);
    let run = slot.native_run().unwrap();
    assert_eq!(
        slot.configure_compiled(config(2, 20), run),
        CompiledDelivery::PendingBoundary
    );
    let status = slot.native_status().unwrap();
    assert_eq!(
        (status.active_settings, status.pending_settings),
        (1, Some(2))
    );
    assert_eq!(
        slot.configure_compiled(config(3, 30), run),
        CompiledDelivery::PendingBoundary
    );
    acknowledge.store(2, Ordering::Relaxed);
    tick(&mut slot);
    let status = slot.native_status().unwrap();
    assert_eq!(
        (status.active_settings, status.pending_settings),
        (1, Some(3))
    );
    acknowledge.store(3, Ordering::Relaxed);
    tick(&mut slot);
    let status = slot.native_status().unwrap();
    assert_eq!((status.active_settings, status.pending_settings), (3, None));
    assert_eq!(
        slot.compiled.as_ref().unwrap().config.get::<u64>(),
        Some(&30)
    );

    let (mut restart, acknowledge) = receiver(13, SettingsApply::RestartRequired);
    let run = restart.native_run().unwrap();
    assert_eq!(
        restart.configure_compiled(config(2, 40), run),
        CompiledDelivery::RestartRequired
    );
    acknowledge.store(2, Ordering::Relaxed);
    tick(&mut restart);
    let status = restart.native_status().unwrap();
    assert_eq!(
        (status.active_settings, status.pending_settings),
        (1, Some(2))
    );
    restart.stop();
    assert!(restart.native_status().is_none());
    assert_eq!(
        restart.configure_compiled(config(3, 50), run),
        CompiledDelivery::Stale
    );
}

#[test]
fn configuration_fences_account_incarnation_revision_and_type_without_load_success() {
    let (mut alice, _) = receiver(20, SettingsApply::Applied);
    let (mut bob, _) = receiver(21, SettingsApply::Applied);
    let alice_run = alice.native_run().unwrap();
    let bob_run = bob.native_run().unwrap();
    assert_eq!(alice_run.run, bob_run.run);
    assert_eq!(
        bob.configure_compiled(config(2, 10), alice_run),
        CompiledDelivery::Stale
    );
    assert_eq!(
        alice.configure_compiled(config(2, 10), alice_run),
        CompiledDelivery::Applied
    );
    assert_eq!(
        bob.configure_compiled(config(2, 20), bob_run),
        CompiledDelivery::Applied
    );
    assert_eq!(
        alice.compiled.as_ref().unwrap().config.get::<u64>(),
        Some(&10)
    );
    assert_eq!(
        bob.compiled.as_ref().unwrap().config.get::<u64>(),
        Some(&20)
    );
    assert_eq!(
        alice.configure_compiled(config(2, 99), alice_run),
        CompiledDelivery::Stale
    );
    let invalid = PreparedConfig::new(
        crate::CompiledId("test"),
        1,
        3,
        Arc::new(SettingsBag::from_iter([("value".into(), 99.into())])),
        (),
    );
    assert!(matches!(
        alice.configure_compiled(invalid, alice_run),
        CompiledDelivery::Rejected(_)
    ));
    assert_eq!(
        alice.compiled.as_ref().unwrap().config.get::<u64>(),
        Some(&10)
    );
    #[cfg(feature = "load")]
    {
        assert!(!alice.post_settings_bag_fenced(
            &SettingsBag::new(),
            "compiled:test",
            alice_run.run
        ));
    }
}

#[test]
fn stale_factory_cleanup_contains_panics_and_runs_stop_once() {
    struct Panics(Arc<AtomicU64>);
    impl Script for Panics {
        fn tick(&mut self, _: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            unreachable!()
        }
        fn on_stop(&mut self, _: StopReason) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("teardown panic");
        }
    }
    impl Drop for Panics {
        fn drop(&mut self) {
            panic!("destructor panic");
        }
    }
    let calls = Arc::new(AtomicU64::new(0));
    let owner = ScriptOwner(Some(Box::new(Panics(Arc::clone(&calls)))));
    std::thread::spawn(move || drop(owner)).join().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn discarded_preparation_contains_configuration_destructor_panics() {
    struct PanickingConfig(std::sync::mpsc::Sender<()>);
    impl Drop for PanickingConfig {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
            panic!("configuration destructor panic");
        }
    }
    for finished in [false, true] {
        let (mut slot, _) = receiver(30, SettingsApply::Applied);
        let mut run = slot.compiled.take().unwrap();
        let (dropped, observed) = std::sync::mpsc::channel();
        run.config = PreparedConfig::new(
            crate::CompiledId("test"),
            1,
            1,
            Arc::default(),
            PanickingConfig(dropped),
        );
        let worker = preparation::submit(
            PreparationKind::Profile,
            api::identity::Scope::Slot(api::identity::SlotKey::from_incarnation(30).unwrap()),
            PreparationInput::new(0, 0, 1024, 1024).unwrap(),
            move |_| PreparationResult(Some(Ok(*run))),
        )
        .unwrap();
        if finished {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !worker.is_finished() {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
        slot.preparing = Some(Box::new(Preparation {
            generation: 0,
            worker,
        }));
        slot.stop();
        observed.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(slot.state(), RunState::Idle);
    }
}

#[test]
fn removal_notifies_the_native_script_once_with_removed_reason() {
    struct Removed(std::sync::mpsc::Sender<StopReason>);
    impl Script for Removed {
        fn tick(&mut self, _: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            Ok(ScriptFlow::Continue)
        }
        fn on_stop(&mut self, reason: StopReason) {
            self.0.send(reason).unwrap();
        }
    }
    let (send, receive) = std::sync::mpsc::channel();
    let mut slot = SlotScript::new();
    slot.bind_incarnation(31);
    slot.start_test_script(Box::new(Removed(send)), Some(selected()))
        .unwrap();
    slot.stop_removed();
    assert_eq!(receive.recv().unwrap(), StopReason::Removed);
    drop(slot);
    assert!(receive.try_recv().is_err());
}

/// A Sherlock revision prepared exactly as a settings edit prepares it.
#[cfg(feature = "load")]
fn sherlock_config(revision: u64) -> Arc<PreparedConfig> {
    FamilyPreparation::run(move |worker| {
        prepare_config(
            worker,
            crate::CompiledId("Sherlock"),
            revision,
            Arc::new(SettingsBag::new()),
            selected(),
            Arc::default(),
            None,
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap()
}

#[cfg(feature = "load")]
fn permission_config(revision: u64, allow: bool) -> Arc<PreparedConfig> {
    FamilyPreparation::run(move |worker| {
        prepare_config(
            worker,
            crate::CompiledId("Sherlock"),
            revision,
            Arc::new(
                serde_json::from_value(serde_json::json!({
                    "allow_teleports": allow,
                    "allow_wilderness": allow,
                    "allow_danger_zones": allow,
                }))
                .unwrap(),
            ),
            selected(),
            Arc::default(),
            None,
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap()
}

#[cfg(feature = "load")]
#[test]
fn walk_permissions_capture_start_apply_restart_and_account_isolation() {
    let mut alice = SlotScript::new();
    let mut bob = SlotScript::new();
    alice.bind_incarnation(81);
    bob.bind_incarnation(82);
    let initial = permission_config(1, true);
    start(&mut alice, initial.bag().clone());
    start(&mut bob, SettingsBag::new());
    assert_eq!(settle(&mut alice), StartOutcome::Ready);
    assert_eq!(settle(&mut bob), StartOutcome::Ready);
    assert_eq!(
        alice.native_walk_permissions(),
        Some(initial.walk_permissions())
    );
    assert_eq!(bob.native_walk_permissions(), Some(Default::default()));
    let revision = permission_config(2, false);
    let run = alice.native_run().unwrap();
    assert_eq!(
        alice.configure_compiled(revision.clone(), run),
        CompiledDelivery::Applied
    );
    assert_eq!(alice.native_walk_permissions(), Some(Default::default()));
    assert_eq!(
        bob.configure_compiled(permission_config(2, true), run),
        CompiledDelivery::Stale
    );
    assert_eq!(bob.native_walk_permissions(), Some(Default::default()));
    alice.compiled.as_mut().unwrap().pending = Some(PendingConfig {
        config: permission_config(3, true),
        boundary: false,
    });
    restart_and_settle(&mut alice, Instant::now());
    assert_eq!(
        alice.native_walk_permissions(),
        Some(revision.walk_permissions())
    );
    // An accepted boundary revision is re-offered through the new instance.
    alice.compiled.as_mut().unwrap().pending = Some(PendingConfig {
        config: permission_config(4, true),
        boundary: true,
    });
    restart_and_settle(&mut alice, Instant::now());
    assert_eq!(
        alice.native_walk_permissions(),
        Some(initial.walk_permissions())
    );
    alice.stop();
    assert_eq!(alice.native_walk_permissions(), None);
}

#[test]
fn walk_permissions_do_not_apply_before_matching_boundary_acknowledgement() {
    let (mut slot, acknowledge) = receiver(83, SettingsApply::PendingBoundary);
    let run = slot.native_run().unwrap();
    let bag = Arc::new(
        serde_json::from_value(serde_json::json!({
            "value": 20,
            "allow_teleports": true,
            "allow_wilderness": true,
            "allow_danger_zones": true,
        }))
        .unwrap(),
    );
    let config = PreparedConfig::new(crate::CompiledId("test"), 1, 2, bag, 20_u64);
    let granted = config.walk_permissions();
    assert_eq!(
        slot.configure_compiled(config, run),
        CompiledDelivery::PendingBoundary
    );
    assert_eq!(slot.native_walk_permissions(), Some(Default::default()));
    acknowledge.store(1, Ordering::Relaxed);
    tick(&mut slot);
    assert_eq!(slot.native_walk_permissions(), Some(Default::default()));
    acknowledge.store(2, Ordering::Relaxed);
    tick(&mut slot);
    assert_eq!(slot.native_walk_permissions(), Some(granted));
    assert!(granted.allow_teleports && granted.allow_wilderness && granted.allow_danger_zones);
}

fn restart_and_settle(slot: &mut SlotScript, now: Instant) {
    slot.restart_from_identity(now).unwrap();
    let deadline = now + Duration::from_secs(10);
    while slot.state() == RunState::Starting {
        assert!(Instant::now() < deadline, "compiled restart stalled");
        slot.observe_lifecycle();
        std::thread::yield_now();
    }
    assert_eq!(slot.state(), RunState::Running);
}

#[cfg(feature = "load")]
#[test]
fn watchdog_restarts_compiled_with_effective_not_pending_configuration() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(40);
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    let old_run = slot.native_run().unwrap();
    let effective = Arc::clone(&slot.compiled.as_ref().unwrap().config);
    let restart_required = sherlock_config(2);
    slot.compiled.as_mut().unwrap().pending = Some(PendingConfig {
        config: Arc::clone(&restart_required),
        boundary: false,
    });
    let now = Instant::now();
    restart_and_settle(&mut slot, now);
    assert!(slot.native_run().unwrap().run > old_run.run);
    let current = slot.compiled.as_ref().unwrap();
    assert!(Arc::ptr_eq(&current.config, &effective));
    // Still pending, not dropped: only an operator restart applies it.
    assert!(current.pending.as_ref().is_some_and(
        |pending| !pending.boundary && Arc::ptr_eq(&pending.config, &restart_required)
    ));
    assert_eq!(slot.watchdog.last_recovery(), Some(now));
}

#[cfg(feature = "load")]
#[test]
fn watchdog_restart_reoffers_an_accepted_boundary_revision_to_the_new_instance() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(43);
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    let accepted = sherlock_config(2);
    slot.compiled.as_mut().unwrap().pending = Some(PendingConfig {
        config: Arc::clone(&accepted),
        boundary: true,
    });
    restart_and_settle(&mut slot, Instant::now());
    // The replacement is created from the effective revision; the accepted
    // boundary revision goes through its configure receiver, which Sherlock
    // applies immediately. It is not silently discarded.
    let current = slot.compiled.as_ref().unwrap();
    assert!(Arc::ptr_eq(&current.config, &accepted));
    assert!(current.pending.is_none());
    assert_eq!(slot.native_settings_revision(), Some(2));
}

#[test]
fn blocked_native_work_never_automatically_restarts() {
    let (mut slot, _) = receiver(41, SettingsApply::Applied);
    Arc::make_mut(
        slot.compiled
            .as_mut()
            .unwrap()
            .output
            .status
            .as_mut()
            .unwrap(),
    )
    .phase = NativePhase::Blocked;
    let run = slot.native_run();
    let now = Instant::now();
    slot.watchdog.arm_fresh(now);
    assert_eq!(
        slot.feed_watchdog(
            now + crate::watchdog::HARD_STALL,
            Some((1, 1, 0)),
            &[],
            false,
            true,
            &[],
        ),
        WatchdogAction::None,
    );
    assert!(
        slot.watchdog.frozen(),
        "blocked time must not spend recovery deadlines"
    );
    assert!(slot
        .restart_from_identity(now + crate::watchdog::HARD_STALL)
        .is_err());
    assert_eq!(slot.native_run(), run);
    assert_eq!(slot.state(), RunState::Running);
}

#[test]
fn a_watchdog_recovery_hold_freezes_the_active_clock() {
    let (mut slot, _) = receiver(44, SettingsApply::Applied);
    let start = Instant::now();
    slot.watchdog.arm_fresh(start);
    slot.watchdog
        .stamp_game_packet_tick(start + crate::watchdog::WEDGE);
    assert_eq!(
        slot.watchdog.observe(start + crate::watchdog::WEDGE, true),
        WatchdogAction::RequestAnchor
    );
    assert!(slot.watchdog.holds_script_actions());
    slot.sync_compiled_clue(false);
    let now = Instant::now();
    assert_eq!(
        slot.native_runtime.clock.now(now + Duration::from_secs(60)),
        slot.native_runtime.clock.now(now),
        "recovery-hold time must not spend family deadlines"
    );
}

struct Park;
impl crate::native::NativeMachine for Park {
    type Args = ();
    type Output = ();
    fn begin(_: (), _: &mut ActionContext<'_>) -> Result<Self, crate::native::ActionError> {
        Ok(Self)
    }
    fn poll(
        &mut self,
        _: &mut ActionContext<'_>,
    ) -> std::task::Poll<Result<(), crate::native::ActionError>> {
        std::task::Poll::Pending
    }
    fn cancel(&mut self) {}
}

struct RelogFrame {
    run: RunKey,
    evidence: EvidenceStamp,
    old_action: Option<std::task::Poll<Result<(), crate::native::ActionError>>>,
    stale_walk: Option<Result<(), crate::native::ActionError>>,
}

/// Parks one native action, then after a relog polls that handle and tries
/// a walk fenced by the pre-relog evidence.
struct Relog {
    frames: std::sync::mpsc::Sender<RelogFrame>,
    first: Option<EvidenceStamp>,
    handle: Option<crate::native::ActionHandle<Park>>,
}

impl Script for Relog {
    fn tick(&mut self, cx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let mut frame = RelogFrame {
            run: cx.cx.run(),
            evidence: cx.cx.evidence(),
            old_action: None,
            stale_walk: None,
        };
        match (&self.handle, self.first) {
            (None, _) => {
                self.first = Some(cx.cx.evidence());
                self.handle = Some(cx.actions.begin::<Park>((), &mut cx.cx).unwrap());
            }
            (Some(handle), Some(first)) => {
                frame.old_action = Some(cx.actions.poll(handle, &mut cx.cx));
                let request = crate::native::WalkRequest {
                    target: api::WorldTile {
                        x: 1,
                        z: 1,
                        level: 0,
                    },
                    loc_id: None,
                    radius: 0,
                    arrival: nav::contract::ArrivalKind::Reach,
                    options: crate::native::WalkOptions::default(),
                    required_after: first,
                    evidence: None,
                    cross: Vec::new().into_boxed_slice(),
                    protect: false,
                    food_guard: false,
                    allow: Default::default(),
                };
                frame.stale_walk = Some(
                    cx.actions
                        .begin::<crate::native::walk::Walk>(request, &mut cx.cx)
                        .map(drop),
                );
            }
            (Some(_), None) => unreachable!("the first frame is recorded with the handle"),
        }
        self.frames.send(frame).unwrap();
        Ok(ScriptFlow::Continue)
    }
}

#[test]
fn a_relog_advances_the_compiled_session_and_fences_pre_relog_evidence() {
    let (frames, received) = std::sync::mpsc::channel();
    let mut slot = SlotScript::new();
    slot.bind_incarnation(45);
    slot.start_test_script(
        Box::new(Relog {
            frames,
            first: None,
            handle: None,
        }),
        Some(selected()),
    )
    .unwrap();
    tick(&mut slot);
    let before = received.try_recv().unwrap();
    assert_eq!(slot.native_run(), Some(before.run));

    slot.reconnect_session_work();
    slot.on_is_up(true);
    assert_eq!(slot.state(), RunState::Running);
    tick(&mut slot);
    let after = received.try_recv().unwrap();

    assert_eq!(
        (after.run.slot, after.run.run),
        (before.run.slot, before.run.run)
    );
    assert_ne!(
        after.run.session, before.run.session,
        "the reconnect is a new session of the same run"
    );
    assert_eq!(slot.native_run(), Some(after.run));
    assert!(
        !after.evidence.meets(before.evidence),
        "pre-reconnect evidence cannot satisfy a post-reconnect freshness floor"
    );
    assert_eq!(
        after.old_action,
        Some(std::task::Poll::Ready(Err(
            crate::native::ActionError::Stale
        )))
    );
    assert_eq!(
        after.stale_walk,
        Some(Err(crate::native::ActionError::Stale)),
        "a walk fenced by the dropped session's evidence is stale"
    );
}

fn gatherer_snapshot() -> api::snapshot::GameSnapshot {
    FamilyPreparation::run(|families| {
        let selected = selected();
        let mut cx = PrepareContext {
            pin: selected.selected_pin().unwrap(),
            selected,
            banks: Arc::default(),
            families,
        };
        crate::gatherer::test_full_pack_fixture(&mut cx).unwrap().1
    })
    .unwrap()
    .join()
    .unwrap()
}

fn gatherer_tick(slot: &mut SlotScript, snapshot: &api::snapshot::GameSnapshot, tick: u32) {
    slot.on_game_tick(&mut ScriptCtx {
        driver: &mut NullDriver::default(),
        tick: u64::from(tick),
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled: crate::CompiledTick::default(),
    });
}

fn gatherer_slot(incarnation: u64) -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    let mut bag = SettingsBag::new();
    bag.insert("disposition".into(), serde_json::json!("Power"));
    bag.insert("deathPolicy".into(), serde_json::json!("Stop"));
    slot.start_compiled(
        "gatherer-lifecycle",
        crate::CompiledId("Gatherer"),
        Arc::new(bag),
        selected(),
        Arc::default(),
    )
    .unwrap();
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    slot
}

fn take_gatherer_drop_actions(slot: &mut SlotScript) -> Vec<crate::native::HostAction> {
    let mut actions = Vec::with_capacity(5);
    while let Some(action) = slot.take_native_action() {
        assert!(
            matches!(
                &action.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::Held {
                    action,
                    slot: Some(_),
                    ..
                }) if action == "Drop"
            ),
            "full inventory must enter disposal before gathering"
        );
        assert!(action.authority().live());
        actions.push(action);
    }
    actions
}

fn queue_gatherer_drop(
    slot: &mut SlotScript,
    snapshot: &api::snapshot::GameSnapshot,
    first_tick: u32,
) -> (u32, Vec<crate::native::HostAction>) {
    for tick in first_tick..first_tick + 16 {
        gatherer_tick(slot, snapshot, tick);
        if slot.has_native_actions() {
            let actions = take_gatherer_drop_actions(slot);
            assert_eq!(actions.len(), 5, "the full pack queues one five-drop batch");
            return (tick, actions);
        }
    }
    panic!(
        "Gatherer never queued its full-pack disposal: {:?}",
        slot.native_status()
    );
}

#[test]
fn gatherer_equips_with_the_observed_tool_action_before_gathering() {
    let mut snapshot = gatherer_snapshot();
    let mut stats = snapshot.stats().to_vec();
    stats.push(api::snapshot::StatView {
        index: 0,
        name: "attack".into(),
        base: 1,
        effective: 1,
        xp: 0,
        used: true,
    });
    snapshot.seed_stats(stats);
    let equipped = snapshot.equipment()[0].clone();
    let mut held = equipped.clone();
    held.container = api::snapshot::ItemContainer::Inventory;
    held.actions = vec![Some("Wield".into()), Some("Drop".into())];
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_inventory(vec![held.clone()], 28);
    let mut slot = gatherer_slot(92);
    let mut equip_tick = None;
    for tick in 1..17 {
        gatherer_tick(&mut slot, &snapshot, tick);
        if let Some(action) = slot.take_native_action() {
            let crate::native::HostEffect::Interaction(crate::shim::InteractReq::Held {
                name,
                action: operation,
                ..
            }) = &action.effect
            else {
                panic!("gathering must wait for observed equipment");
            };
            assert_eq!(name, held.def.name.as_ref().unwrap());
            assert!(
                held.actions
                    .iter()
                    .flatten()
                    .any(|candidate| candidate == operation),
                "equip request must resolve an available operation, got {operation}"
            );
            assert_ne!(operation, "Drop", "the tool is protected");
            equip_tick = Some(tick);
            break;
        }
    }
    let tick = equip_tick.expect("held usable tool must enter equipment admission");
    gatherer_tick(&mut slot, &snapshot, tick + 1);
    assert!(
        !slot.has_native_actions(),
        "dispatch is not observed equipment"
    );
    snapshot.seed_equipment(vec![equipped]);
    snapshot.seed_inventory(Vec::new(), 28);
    for next in tick + 2..tick + 10 {
        gatherer_tick(&mut slot, &snapshot, next);
        if let Some(action) = slot.take_native_action() {
            assert!(
                matches!(
                    action.effect,
                    crate::native::HostEffect::Walk(_)
                        | crate::native::HostEffect::Interaction(
                            crate::shim::InteractReq::Loc { .. }
                        )
                ),
                "observed equipment must advance to a resource, not repeat equip"
            );
            slot.stop();
            return;
        }
    }
    panic!("observed equipment must release gathering admission");
}

#[test]
fn gatherer_slot_stop_pause_and_watchdog_revoke_every_undrained_drop() {
    let snapshot = gatherer_snapshot();
    for boundary in 0..3 {
        let mut slot = gatherer_slot(80 + boundary);
        let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
        let old_run = slot.native_run().unwrap();
        match boundary {
            0 => slot.stop(),
            1 => slot.pause(),
            2 => restart_and_settle(&mut slot, Instant::now()),
            _ => unreachable!(),
        }
        assert!(actions.iter().all(|action| !action.authority().live()));
        assert!(
            slot.take_native_action().is_none(),
            "old queued drops must never drain"
        );
        if boundary == 0 {
            assert!(!slot.has_native_actions());
            continue;
        }
        if boundary == 1 {
            slot.resume();
        } else {
            assert_ne!(slot.native_run().unwrap(), old_run);
        }
        let (_, fresh) = queue_gatherer_drop(&mut slot, &snapshot, tick + 1);
        assert!(fresh.iter().all(|action| action.authority().live()));
        assert!(actions.iter().all(|action| !action.authority().live()));
        slot.stop();
        assert!(fresh.iter().all(|action| !action.authority().live()));
    }
}

#[test]
fn gatherer_session_change_replans_drop_from_observation_not_old_authority() {
    let snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(84);
    let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    let old_run = slot.native_run().unwrap();
    slot.reconnect_session_work();
    slot.on_is_up(true);
    assert!(actions.iter().all(|action| !action.authority().live()));
    assert!(slot.take_native_action().is_none());
    let (_, fresh) = queue_gatherer_drop(&mut slot, &snapshot, tick + 1);
    let new_run = slot.native_run().unwrap();
    assert_eq!((new_run.slot, new_run.run), (old_run.slot, old_run.run));
    assert_ne!(new_run.session, old_run.session);
    assert!(fresh
        .iter()
        .all(|action| action.authority().run() == new_run));
    assert!(actions.iter().all(|action| !action.authority().live()));
    slot.stop();
}

#[test]
fn gatherer_stop_policy_death_cancels_undrained_drop() {
    let mut snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(85);
    let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
        sequence: 1,
    }]);
    gatherer_tick(&mut slot, &snapshot, tick + 1);
    assert!(actions.iter().all(|action| !action.authority().live()));
    assert!(slot.take_native_action().is_none());
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "died");
    gatherer_tick(&mut slot, &snapshot, tick + 2);
    assert!(
        !slot.has_native_actions(),
        "latched death cannot resume disposal"
    );
    slot.stop();
}

#[test]
fn gatherer_active_gather_allocates_nothing_on_one_thousand_unchanged_polls() {
    let mut snapshot = gatherer_snapshot();
    snapshot.seed_inventory(Vec::new(), 28);
    let mut player = snapshot.local_player().unwrap().clone();
    player.player.actor.animation = 879;
    snapshot.seed_local_player(player);
    let mut slot = gatherer_slot(86);
    let mut clicked_at = None;
    for tick in 1..17 {
        gatherer_tick(&mut slot, &snapshot, tick);
        if let Some(action) = slot.take_native_action() {
            assert!(matches!(
                action.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { .. })
            ));
            let authority = action.authority();
            slot.complete_native_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: u64::from(tick),
                        sequence: u64::from(tick),
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            clicked_at = Some(tick);
            break;
        }
    }
    let first = clicked_at.expect("ready empty pack must click a real catalog tree") + 1;
    gatherer_tick(&mut slot, &snapshot, first);
    let allocations = allocation_counter::measure(|| {
        for tick in first + 1..first + 1001 {
            gatherer_tick(&mut slot, &snapshot, tick);
        }
    })
    .count_total;
    assert_eq!(
        allocations, 0,
        "unchanged animated gathering must not allocate"
    );
    assert!(
        !slot.has_native_actions(),
        "no extra click without a transition"
    );
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Working);
    println!("Gatherer steady polls=1000 allocations={allocations}");
    slot.stop();
}

#[test]
fn gatherer_delayed_drops_replan_after_two_ticks_without_assuming_progress() {
    let snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(87);
    let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    let mut sent_slots = Vec::new();
    for action in actions {
        sent_slots.push(complete_gatherer_drop(&mut slot, action, tick, None));
    }
    gatherer_tick(&mut slot, &snapshot, tick + 1);
    assert!(
        !slot.has_native_actions(),
        "accepted writes are still awaiting observation"
    );
    gatherer_tick(&mut slot, &snapshot, tick + 2);
    let mut resent_slots = Vec::new();
    while let Some(action) = slot.take_native_action() {
        if let crate::native::HostEffect::Interaction(crate::shim::InteractReq::Held {
            slot: Some(index),
            ..
        }) = action.effect
        {
            resent_slots.push(index);
        }
    }
    assert_eq!(
        resent_slots, sent_slots,
        "unsettled slots must be re-planned, not blocked after three polls"
    );
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Working);
    slot.stop();
}

fn complete_gatherer_drop(
    slot: &mut SlotScript,
    action: crate::native::HostAction,
    tick: u32,
    refused_slot: Option<i32>,
) -> i32 {
    let index = match &action.effect {
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Held {
            slot: Some(index),
            ..
        }) => *index,
        _ => panic!("expected slot-exact drop"),
    };
    let authority = action.authority();
    slot.complete_native_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: u64::from(tick),
                sequence: u64::from(tick),
            },
            accepted: refused_slot != Some(index),
            chat_since: 0,
        },
    );
    index
}

fn accept_gatherer_drop_actions(
    slot: &mut SlotScript,
    actions: Vec<crate::native::HostAction>,
    tick: u32,
    refused_slot: Option<i32>,
) -> Vec<i32> {
    let mut slots = Vec::with_capacity(actions.len());
    for action in actions {
        slots.push(complete_gatherer_drop(slot, action, tick, refused_slot));
    }
    slots
}

fn accept_gatherer_drops(slot: &mut SlotScript, tick: u32, refused_slot: Option<i32>) -> Vec<i32> {
    let mut slots = Vec::new();
    while let Some(action) = slot.take_native_action() {
        slots.push(complete_gatherer_drop(slot, action, tick, refused_slot));
    }
    slots
}

#[test]
fn gatherer_rejected_drop_replans_only_its_slot_then_observes_late_settlement() {
    let mut snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(88);
    let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    assert_eq!(
        accept_gatherer_drop_actions(&mut slot, actions, tick, Some(2)),
        [0, 1, 2, 3, 4]
    );
    gatherer_tick(&mut slot, &snapshot, tick + 1);
    assert_eq!(accept_gatherer_drops(&mut slot, tick + 1, None), [2]);
    let rows = snapshot
        .inventory()
        .iter()
        .filter(|row| row.slot > 4)
        .cloned()
        .collect();
    snapshot.seed_inventory(rows, 28);
    gatherer_tick(&mut slot, &snapshot, tick + 2);
    assert_eq!(
        accept_gatherer_drops(&mut slot, tick + 2, None),
        [5, 6, 7, 8, 9]
    );
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Working);
    slot.stop();
}

#[test]
fn gatherer_three_unsettled_rounds_block_without_counting_dispatch_as_progress() {
    let snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(89);
    let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    assert_eq!(
        accept_gatherer_drop_actions(&mut slot, actions, tick, None),
        [0, 1, 2, 3, 4]
    );
    for round in 1..=2 {
        gatherer_tick(&mut slot, &snapshot, tick + round * 2 - 1);
        assert!(!slot.has_native_actions());
        gatherer_tick(&mut slot, &snapshot, tick + round * 2);
        assert_eq!(
            accept_gatherer_drops(&mut slot, tick + round * 2, None),
            [0, 1, 2, 3, 4]
        );
    }
    gatherer_tick(&mut slot, &snapshot, tick + 5);
    gatherer_tick(&mut slot, &snapshot, tick + 6);
    assert!(!slot.has_native_actions());
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "inventory-blocked");
    slot.stop();
}

#[test]
fn gatherer_full_pack_chat_modal_consumes_one_of_five_packets() {
    let mut snapshot = gatherer_snapshot();
    snapshot.seed_chat_modal(123, vec!["Your inventory is too full.".into()]);
    let mut slot = gatherer_slot(90);
    for tick in 1..17 {
        gatherer_tick(&mut slot, &snapshot, tick);
        if !slot.has_native_actions() {
            continue;
        }
        let first = slot.take_native_action().unwrap();
        assert!(
            matches!(
                first.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
            ),
            "mining mesbox must close before disposal"
        );
        assert_eq!(accept_gatherer_drops(&mut slot, tick, None), [0, 1, 2, 3]);
        slot.stop();
        return;
    }
    panic!("full-pack mesbox never entered disposal");
}

#[test]
fn gatherer_rejected_click_observes_missing_target_before_retry() {
    let mut snapshot = gatherer_snapshot();
    snapshot.seed_inventory(Vec::new(), 28);
    let mut slot = gatherer_slot(91);
    let mut clicked_at = None;
    for tick in 1..17 {
        gatherer_tick(&mut slot, &snapshot, tick);
        if let Some(action) = slot.take_native_action() {
            assert!(matches!(
                action.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { .. })
            ));
            let authority = action.authority();
            slot.complete_native_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: u64::from(tick),
                        sequence: u64::from(tick),
                    },
                    accepted: false,
                    chat_since: 0,
                },
            );
            clicked_at = Some(tick);
            break;
        }
    }
    snapshot.seed_locs(Vec::new());
    let first = clicked_at.unwrap();
    for tick in first + 1..=first + 4 {
        gatherer_tick(&mut slot, &snapshot, tick);
        assert!(
            !slot.has_native_actions(),
            "missing target must never be retried"
        );
    }
    assert_eq!(
        slot.native_status()
            .unwrap()
            .failure
            .as_ref()
            .unwrap()
            .code
            .as_ref(),
        "resource-unavailable"
    );
    slot.stop();
}

#[test]
fn gatherer_counts_yield_observed_after_target_depletion() {
    let mut snapshot = gatherer_snapshot();
    let product = snapshot.inventory()[0].clone();
    snapshot.seed_inventory(Vec::new(), 28);
    let mut slot = gatherer_slot(92);
    let mut clicked_at = None;
    for tick in 1..17 {
        gatherer_tick(&mut slot, &snapshot, tick);
        if let Some(action) = slot.take_native_action() {
            let authority = action.authority();
            slot.complete_native_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: u64::from(tick),
                        sequence: u64::from(tick),
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            clicked_at = Some(tick);
            break;
        }
    }
    let first = clicked_at.unwrap();
    snapshot.seed_locs(Vec::new());
    gatherer_tick(&mut slot, &snapshot, first + 1);
    snapshot.seed_inventory(vec![product], 28);
    gatherer_tick(&mut slot, &snapshot, first + 2);
    let status = slot.native_status().unwrap();
    let yielded = status
        .fields
        .iter()
        .find(|field| field.key == "yielded")
        .unwrap();
    assert_eq!(yielded.value, crate::native::StatusValue::Integer(1));
    assert_eq!(
        slot.state(),
        RunState::Idle,
        "depleted area blocks and stops"
    );
    assert!(
        slot.retained.is_none(),
        "normal Stop releases retained work"
    );
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "resource-unavailable"
    );
    slot.stop();
}

#[test]
fn gatherer_exclusive_heap_stays_inside_the_eight_kib_target() {
    fn measure_prepared() -> (Arc<PreparedConfig>, allocation_counter::AllocationInfo) {
        FamilyPreparation::run(|families| {
            let selected = selected();
            let mut cx = PrepareContext {
                pin: selected.selected_pin().unwrap(),
                selected,
                banks: Arc::default(),
                families,
            };
            let mut prepared = None;
            let info = allocation_counter::measure(|| {
                prepared = Some(
                    (crate::gatherer::CARD.prepare)(&mut cx, 1, Arc::new(SettingsBag::new()))
                        .unwrap(),
                );
            });
            (prepared.unwrap(), info)
        })
        .unwrap()
        .join()
        .unwrap()
    }
    fn seat_delivery_bytes(slot: &SlotScript) -> usize {
        slot.seat_delivery.capacity() * std::mem::size_of::<SeatEffect>()
            + slot.seat_native_delivery.capacity()
                * std::mem::size_of::<crate::native::HostAction>()
    }
    let snapshot = gatherer_snapshot();
    let (cold_prepared, cold) = measure_prepared();
    let mut warm = gatherer_slot(92);
    let mut shared_slot = None;
    let shared_seat = allocation_counter::measure(|| {
        let mut slot = SlotScript::new();
        slot.bind_incarnation(94);
        slot.arm_compiled_seat(
            RunKey {
                slot: 94,
                run: 1,
                session: 0,
            },
            crate::CompiledId("Gatherer"),
        );
        shared_slot = Some(slot);
    });
    let (prepared, preparation) = measure_prepared();
    let prepare_bytes = preparation.bytes_current;
    let mut measured = None;
    let main = allocation_counter::measure(|| {
        let mut slot = gatherer_slot(93);
        let (tick, actions) = queue_gatherer_drop(&mut slot, &snapshot, 1);
        accept_gatherer_drop_actions(&mut slot, actions, tick, None);
        measured = Some(slot);
    });
    // Keep the historical pre-seat family bound: exclude the measured
    // SlotScript/I-seat baseline and the shared seat delivery buffers that
    // grow only when this five-action native batch is dispatched.
    assert!(
        shared_seat.bytes_current > 0,
        "shared slot seat baseline is measured"
    );
    let delivery_bytes = i64::try_from(
        seat_delivery_bytes(measured.as_ref().unwrap())
            .checked_sub(seat_delivery_bytes(shared_slot.as_ref().unwrap()))
            .expect("the active seat includes its shared delivery buffers"),
    )
    .expect("bounded seat delivery capacity fits the signed allocation counter");
    let slot_family_bytes = main
        .bytes_current
        .checked_sub(shared_seat.bytes_current)
        .and_then(|remaining| remaining.checked_sub(delivery_bytes))
        .expect("the measured Gatherer slot includes its shared seat and delivery baseline");
    let exclusive = prepare_bytes + slot_family_bytes;
    println!(
        "Gatherer exclusive heap: shared slot/seat={}B, shared delivery={delivery_bytes}B, prepared={prepare_bytes}, slot={main:?}, total={exclusive}",
        shared_seat.bytes_current
    );
    println!(
        "Gatherer preparation: first={cold:?}, cached={preparation:?}, shared_delta={}, worker_transient_peak={}",
        cold.bytes_current - prepare_bytes,
        i128::from(cold.bytes_max) - i128::from(cold.bytes_current),
    );
    assert!(
        (0..=8192).contains(&exclusive),
        "exclusive Gatherer heap exceeds 8 KiB"
    );
    measured.as_mut().unwrap().stop();
    warm.stop();
    drop(prepared);
    drop(cold_prepared);
    drop(shared_slot);
}

#[cfg(feature = "load")]
mod api_gather_seat {
    use super::*;
    use crate::api_gather::{GatherEnd, GatherPhase};
    use crate::isolate_fb::{DecodedSeatBatch, SeatInteractDecoded, SeatStartArgs, SeatStartInput};
    use crate::native::request::NativeKind;
    use crate::shim::InteractReq;
    use api::identity::RequestId;

    fn load_slot(incarnation: u64, data: bool) -> SlotScript {
        let mut slot = SlotScript::new();
        slot.bind_incarnation(incarnation);
        slot.start_load_with_settings_and_game_data(
            "export function tick() {}".into(),
            crate::load::LoadShape::NativeTick,
            None,
            vec![],
            &[],
            data.then(selected),
            Arc::default(),
        )
        .unwrap();
        assert_eq!(settle(&mut slot), StartOutcome::Ready);
        slot.on_is_up(true);
        slot
    }

    fn request_id(token: u64) -> RequestId {
        RequestId::isolate(token).expect("test request id")
    }

    fn run_in_seat_with_settings(slot: &mut SlotScript, token: u64, settings: SettingsBag) {
        let run = slot.seat.as_ref().expect("Load slot seat").run();
        slot.consume_seat_batch(
            DecodedSeatBatch {
                consumed_terminal_seq: 0,
                starts: vec![SeatStartInput {
                    id: request_id(token),
                    parent: None,
                    run: run.run,
                    session: run.session,
                    kind: NativeKind::Gather,
                    args: SeatStartArgs::Gather(settings),
                }],
                decisions: Vec::new(),
                cancels: Vec::new(),
                mirror_revokes: Vec::new(),
                reqs: Vec::new(),
            },
            0,
            &[],
        );
    }

    fn gather_settings() -> SettingsBag {
        let mut settings = SettingsBag::new();
        settings.insert("disposition".into(), serde_json::json!("Power"));
        settings.insert("deathPolicy".into(), serde_json::json!("Stop"));
        settings
    }

    fn run(slot: &mut SlotScript, token: u64) {
        slot.consume_api_control(&InteractReq::GatherRun {
            request_id: token,
            settings: Arc::new(gather_settings()),
        });
    }

    fn run_in_seat(slot: &mut SlotScript, token: u64) {
        run_in_seat_with_settings(slot, token, gather_settings());
    }

    fn stop(slot: &mut SlotScript, token: u64) {
        slot.consume_api_control(&InteractReq::GatherStop { request_id: token });
    }

    fn queue_stop_in_seat(slot: &mut SlotScript, token: u64) {
        slot.consume_seat_batch(
            DecodedSeatBatch {
                consumed_terminal_seq: 0,
                starts: Vec::new(),
                decisions: Vec::new(),
                cancels: Vec::new(),
                mirror_revokes: Vec::new(),
                reqs: vec![SeatInteractDecoded {
                    lineage: Some(request_id(token)),
                    mirror_owner: None,
                    request: InteractReq::GatherStop { request_id: token },
                }],
            },
            0,
            &[],
        );
    }

    fn stop_in_seat(slot: &mut SlotScript, token: u64) {
        queue_stop_in_seat(slot, token);
        let (_, rows, _) = slot.drain_host_interacts();
        assert!(
            rows.is_empty(),
            "typed GatherStop is consumed as a reserved control"
        );
    }
    fn poll(slot: &mut SlotScript, snapshot: &api::snapshot::GameSnapshot, tick: u64, hold: bool) {
        slot.tick_api(&mut ScriptCtx {
            driver: &mut NullDriver::default(),
            tick,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: crate::CompiledTick {
                hold,
                ..Default::default()
            },
        });
    }
    // Match the native seat portion of SlotScript::on_game_tick while
    // excluding the independent Load-isolate JS tick command queue.
    fn poll_native_api_seat(
        slot: &mut SlotScript,
        snapshot: &api::snapshot::GameSnapshot,
        tick: u64,
        hold: bool,
    ) {
        poll(slot, snapshot, tick, hold);
        slot.capture_native_seat_rows();
        slot.settle_api_seat_terminals();
    }

    fn poll_through_slot(
        slot: &mut SlotScript,
        snapshot: &api::snapshot::GameSnapshot,
        tick: u64,
        hold: bool,
    ) {
        slot.on_game_tick(&mut ScriptCtx {
            driver: &mut NullDriver::default(),
            tick,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: crate::CompiledTick {
                hold,
                ..Default::default()
            },
        });
    }

    fn installed(slot: &mut SlotScript, snapshot: &api::snapshot::GameSnapshot) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            poll(slot, snapshot, 1, false);
            if slot
                .api
                .as_ref()
                .unwrap()
                .page
                .as_ref()
                .is_some_and(|page| page.phase == GatherPhase::Running)
            {
                break;
            }
            assert!(Instant::now() < deadline, "API preparation stalled");
            std::thread::yield_now();
        }
        assert!(
            slot.api
                .as_ref()
                .unwrap()
                .page
                .as_ref()
                .unwrap()
                .status
                .is_none(),
            "install page has no status until the first tick"
        );
    }

    #[test]
    fn load_api_gather_walk_permissions_follow_its_live_card_settings() {
        let snapshot = gatherer_snapshot();
        let mut slot = load_slot(206, true);
        assert_eq!(slot.native_walk_permissions(), None);
        assert_eq!(slot.api_gather_walk_permissions(), None);

        let mut settings = SettingsBag::new();
        settings.insert("disposition".into(), serde_json::json!("Power"));
        settings.insert("deathPolicy".into(), serde_json::json!("Stop"));
        settings.insert("allowTeleports".into(), serde_json::json!(true));
        settings.insert("allowWilderness".into(), serde_json::json!(true));
        settings.insert("allowDangerZones".into(), serde_json::json!(true));
        slot.consume_api_control(&InteractReq::GatherRun {
            request_id: 206,
            settings: Arc::new(settings),
        });
        installed(&mut slot, &snapshot);

        assert_eq!(slot.native_walk_permissions(), None);
        assert_eq!(
            slot.api_gather_walk_permissions(),
            Some(crate::native::WalkPermissions {
                allow_teleports: true,
                allow_wilderness: true,
                allow_danger_zones: true,
            })
        );

        stop(&mut slot, 206);
        assert_eq!(slot.native_walk_permissions(), None);
        assert_eq!(slot.api_gather_walk_permissions(), None);
        slot.stop();
    }

    #[test]
    fn load_api_gather_defaults_keep_walk_permissions_refused() {
        let snapshot = gatherer_snapshot();
        let mut slot = load_slot(207, true);
        run(&mut slot, 207);
        installed(&mut slot, &snapshot);

        assert_eq!(slot.native_walk_permissions(), None);
        assert_eq!(
            slot.api_gather_walk_permissions(),
            Some(crate::native::WalkPermissions::default())
        );

        stop(&mut slot, 207);
        assert_eq!(slot.native_walk_permissions(), None);
        assert_eq!(slot.api_gather_walk_permissions(), None);
        slot.stop();
    }

    #[test]
    fn stop_during_preparation_is_prompt_clears_retention_and_never_installs() {
        let snapshot = gatherer_snapshot();
        let mut slot = load_slot(201, true);
        run(&mut slot, 201);
        let retention = Arc::clone(slot.retained.as_ref().unwrap());
        retention.lock().unwrap().gather().yielded = 9;
        stop(&mut slot, 201);
        assert!(!slot.api_owns_foreground());
        assert!(matches!(
            slot.api.as_ref().unwrap().terminal,
            Some(GatherEnd::Stopped { token: 201, .. })
        ));
        assert!(retention.lock().unwrap().gather().is_fresh());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !slot.api.as_ref().unwrap().retiring.is_empty() {
            poll(&mut slot, &snapshot, 1, false);
            assert!(Instant::now() < deadline, "retired preparation not reaped");
            std::thread::yield_now();
        }
        assert!(slot.api.as_ref().unwrap().page.is_none());
        assert!(!slot.has_native_actions());
        stop(&mut slot, 201);
        assert!(matches!(
            slot.api.as_ref().unwrap().terminal,
            Some(GatherEnd::Stopped { token: 201, .. })
        ));
        slot.stop();
    }

    #[test]
    fn api_stop_revokes_five_undrained_native_drops_and_preserves_last_published_counts() {
        let snapshot = gatherer_snapshot();
        let mut slot = load_slot(202, true);
        run_in_seat(&mut slot, 202);
        installed(&mut slot, &snapshot);
        let mut actions = Vec::new();
        for tick in 2..18 {
            poll_through_slot(&mut slot, &snapshot, tick, false);
            if slot.has_native_actions() {
                actions = take_gatherer_drop_actions(&mut slot);
                break;
            }
        }
        assert_eq!(actions.len(), 5, "one bounded native drop batch");
        assert!(
            actions.iter().all(|action| action.authority().live()),
            "captured drops are live before Stop"
        );
        let published = crate::slot::api_seat::counts(slot.native_status().as_deref());
        let policy_override = api::run_policy::RunPolicyOverride {
            run_auto: None,
            energy_min: Some(api::run_policy::RunEnergyMin::Floor(77)),
        };
        queue_stop_in_seat(&mut slot, 202);
        assert!(
            slot.api_owns_foreground(),
            "the typed Stop retains the live foreground until its host drain"
        );
        slot.restore_interacts(vec![
            InteractReq::RunPolicyOverride {
                policy: Some(policy_override),
            },
            InteractReq::InspectAck {
                seq: 9,
                generation: 7,
            },
            InteractReq::SetCameraYaw { yaw: 777 },
        ]);
        let (policy, rows, owned) = slot.drain_host_interacts();
        assert_eq!(policy, Some(Some(policy_override)));
        assert!(
            !owned,
            "the typed Stop releases the foreground after its host drain"
        );
        assert_eq!(
            rows.into_iter()
                .map(|queued| queued.req)
                .collect::<Vec<_>>(),
            vec![
                InteractReq::InspectAck {
                    seq: 9,
                    generation: 7,
                },
                InteractReq::SetCameraYaw { yaw: 777 },
            ],
            "the typed Stop preserves unrelated host-local work"
        );
        assert!(actions.iter().all(|action| !action.authority().live()));
        assert!(
            slot.take_native_action().is_none(),
            "Stop before Driver drain sends no packets"
        );
        assert!(
            matches!(&slot.api.as_ref().unwrap().terminal, Some(GatherEnd::Stopped { token: 202, counts }) if counts == &published)
        );
        assert!(slot
            .retained
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .gather()
            .is_fresh());
        slot.stop();
    }

    #[test]
    fn reconnect_rekeys_actual_card_revokes_old_action_and_reposts_terminal() {
        let snapshot = gatherer_snapshot();
        let mut slot = load_slot(203, true);
        run_in_seat(&mut slot, 203);
        installed(&mut slot, &snapshot);
        for tick in 2..18 {
            poll_through_slot(&mut slot, &snapshot, tick, false);
            if slot.has_native_actions() {
                break;
            }
        }
        let old = slot
            .take_native_action()
            .expect("native Gatherer action")
            .authority();
        assert!(
            old.live(),
            "captured Gatherer action is live before reconnect"
        );
        let previous = old.run();
        assert!(slot.reconnect_session_work());
        assert!(slot.api_owns_foreground());
        slot.on_is_up(true);
        let rekeyed_session = slot
            .native_status()
            .expect("rekeyed Gatherer status")
            .run
            .session_key();
        slot.align_seat_session(rekeyed_session);
        poll_through_slot(&mut slot, &snapshot, 20, false);
        let next = slot.native_status().unwrap().run;
        assert_eq!(next.slot, previous.slot);
        assert_eq!(next.run, previous.run);
        assert_ne!(next.session, previous.session);
        stop_in_seat(&mut slot, 203);
        assert!(slot.reconnect_session_work());
        let page = slot.api.as_ref().unwrap();
        assert!(page.page.is_none());
        assert!(matches!(
            &page.terminal,
            Some(GatherEnd::Stopped { token: 203, .. })
        ));
        slot.stop();
    }

    #[test]
    fn hold_pause_reset_and_watchdog_keep_only_the_authorized_retention() {
        let snapshot = gatherer_snapshot();
        let mut slot = load_slot(204, true);
        run(&mut slot, 204);
        installed(&mut slot, &snapshot);
        for tick in 2..10 {
            poll(&mut slot, &snapshot, tick, true);
        }
        assert!(!slot.has_native_actions(), "hold ineligible native actions");
        assert!(slot.api_owns_foreground());
        slot.pause();
        let status = slot.native_status();
        gatherer_tick(&mut slot, &snapshot, 10);
        assert!(Arc::ptr_eq(
            status.as_ref().unwrap(),
            slot.native_status().as_ref().unwrap()
        ));
        slot.resume();
        let retention = Arc::clone(slot.retained.as_ref().unwrap());
        retention.lock().unwrap().gather().yielded = 7;
        slot.reset_session_work();
        assert!(!slot.api_owns_foreground());
        assert!(slot.api.as_ref().unwrap().terminal.is_none());
        assert_eq!(retention.lock().unwrap().gather().yielded, 7);
        run(&mut slot, 205);
        installed(&mut slot, &snapshot);
        poll(&mut slot, &snapshot, 12, true);
        let old_run = slot.native_status().unwrap().run;
        slot.teardown_api(StopReason::Replaced);
        assert!(slot.api.is_none());
        assert_eq!(retention.lock().unwrap().gather().yielded, 7);
        run(&mut slot, 206);
        installed(&mut slot, &snapshot);
        poll(&mut slot, &snapshot, 13, true);
        assert_ne!(
            old_run.run,
            slot.native_status().unwrap().run.run,
            "fresh action identity after seat recreation"
        );
        slot.stop();
        assert!(slot.api.is_none());
        assert!(slot.retained.is_none());
    }

    #[test]
    fn missing_facts_refuses_and_duplicate_run_cannot_resurrect_the_terminal() {
        let mut slot = load_slot(207, false);
        run(&mut slot, 207);
        assert!(
            matches!(&slot.api.as_ref().unwrap().terminal, Some(GatherEnd::Refused { token: 207, reason }) if reason.as_ref() == "unavailable:selected game data unavailable")
        );
        run(&mut slot, 207);
        assert!(!slot.api_owns_foreground());
        assert_eq!(
            slot.api
                .as_ref()
                .unwrap()
                .terminal
                .as_ref()
                .unwrap()
                .token(),
            207
        );
        slot.stop();
    }

    #[test]
    fn actual_blocked_terminal_wins_stop_race_and_next_run_keeps_retention() {
        let mut snapshot = gatherer_snapshot();
        let mut slot = load_slot(212, true);
        run(&mut slot, 212);
        installed(&mut slot, &snapshot);
        poll(&mut slot, &snapshot, 2, false);
        let retained = Arc::clone(slot.retained.as_ref().unwrap());
        retained.lock().unwrap().gather().yielded = 8;
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            type_: 0,
            username: None,
            text: "Oh dear, you are dead!".into(),
            sequence: 1,
        }]);
        poll(&mut slot, &snapshot, 3, false);
        assert!(!slot.api_owns_foreground());
        assert!(slot.native_status().is_none());
        assert!(
            matches!(&slot.api.as_ref().unwrap().terminal, Some(GatherEnd::Blocked { token: 212, failure, .. }) if failure.code.as_ref() == "died")
        );
        let first = slot.api.as_ref().unwrap().terminal.clone();
        stop(&mut slot, 212);
        poll(&mut slot, &snapshot, 4, false);
        assert_eq!(
            slot.api.as_ref().unwrap().terminal,
            first,
            "first terminal is never replaced by late Stop/ticks"
        );
        assert!(!retained.lock().unwrap().gather().is_fresh());
        snapshot.seed_chat_lines(Vec::new());
        run(&mut slot, 213);
        installed(&mut slot, &snapshot);
        poll(&mut slot, &snapshot, 5, true);
        assert_eq!(slot.api.as_ref().unwrap().page.as_ref().unwrap().token, 213);
        assert!(slot.api_owns_foreground());
        stop(&mut slot, 213);
        assert!(retained.lock().unwrap().gather().is_fresh());
        slot.stop();
    }

    #[test]
    fn warmed_load_seat_exclusive_heap_and_unchanged_polls_are_bounded() {
        let mut snapshot = gatherer_snapshot();
        let mut warm = gatherer_slot(208);
        let mut slot = load_slot(209, true);
        assert!(
            slot.api.is_none(),
            "established Load baseline has no seat allocation"
        );
        // The first park can grow parking_lot's process-global table.
        // Warm this thread before charging only this seat.
        let warm_lock = parking_lot::Mutex::new(());
        parking_lot::Condvar::new().wait_for(&mut warm_lock.lock(), Duration::from_millis(1));
        let (prepared, instance, worker) = FamilyPreparation::run(|families| {
            let selected = selected();
            let cx = PrepareContext {
                pin: selected.selected_pin().unwrap(),
                selected,
                banks: Arc::default(),
                families,
            };
            let bag = Arc::new(SettingsBag::new());
            let mut retained = RetainedMemory::default();
            let mut prepared = None;
            let mut instance = None;
            let measured = allocation_counter::measure(|| {
                let config = prepare_config(
                    cx.families,
                    crate::gatherer::CARD.id,
                    1,
                    bag,
                    Arc::clone(&cx.selected),
                    Arc::clone(&cx.banks),
                    None,
                )
                .unwrap();
                instance = Some(
                    (crate::gatherer::CARD.create)(
                        api::selected::RunKey {
                            slot: 210,
                            run: 1,
                            session: 1,
                        },
                        Arc::clone(&config),
                        &mut retained,
                    )
                    .unwrap(),
                );
                prepared = Some(config);
            });
            (prepared.unwrap(), instance.unwrap(), measured)
        })
        .unwrap()
        .join()
        .unwrap();
        let dispatched = allocation_counter::measure(|| run_in_seat(&mut slot, 209));
        let installed_heap = allocation_counter::measure(|| installed(&mut slot, &snapshot));
        let polled = allocation_counter::measure(|| {
            for tick in 2..18 {
                poll_native_api_seat(&mut slot, &snapshot, tick, false);
                if slot.has_native_actions() {
                    break;
                }
            }
        });
        assert!(slot.has_native_actions(), "actual native batch measured");
        let exclusive = worker.bytes_current
            + dispatched.bytes_current
            + installed_heap.bytes_current
            + polled.bytes_current;
        println!(
            "API gather warmed Load baseline: idle option={}B; worker prepared+instance={worker:?}; dispatch={dispatched:?}; install={installed_heap:?}; polls={polled:?}; exclusive={exclusive}B",
            std::mem::size_of::<Option<Box<crate::slot::api_seat::ApiSeat>>>()
        );
        assert_eq!(
            std::mem::size_of::<Option<Box<crate::slot::api_seat::ApiSeat>>>(),
            8
        );
        // Typed RouteTooLarge adds 40 B to inline receipts: 8,206 B measured, 242 B headroom.
        assert!(exclusive <= 8448, "exclusive API seat exceeds 8.25KiB");
        let stopped = allocation_counter::measure(|| stop_in_seat(&mut slot, 209));
        println!("API gather post-Stop retained={}B (exclusive live + Stop delta), Stop={stopped:?}, worker transient_peak={}B", exclusive + stopped.bytes_current, i128::from(worker.bytes_max) - i128::from(worker.bytes_current));
        assert!(!slot.api_owns_foreground());
        // A second session uses the retained ledger and an animated observation
        // to exercise the actual card's unchanged path without world copies.
        snapshot.seed_inventory(Vec::new(), 28);
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.actor.animation = 879;
        snapshot.seed_local_player(player);
        run_in_seat(&mut slot, 211);
        installed(&mut slot, &snapshot);
        let mut first = None;
        for tick in 20..36 {
            poll_through_slot(&mut slot, &snapshot, tick, false);
            if let Some(action) = slot.take_native_action() {
                let authority = action.authority();
                slot.complete_native_interaction(
                    &authority,
                    crate::native::InteractionReceipt {
                        request_id: action.request_id.get(),
                        evidence: EvidenceStamp {
                            run: authority.run(),
                            tick,
                            sequence: tick,
                        },
                        accepted: true,
                        chat_since: 0,
                    },
                );
                first = Some(tick + 1);
                break;
            }
        }
        let first = first.expect("live Gatherer click");
        poll_through_slot(&mut slot, &snapshot, first, false);
        let page = Arc::clone(slot.api.as_ref().unwrap().page.as_ref().unwrap());
        let steady = allocation_counter::measure(|| {
            for tick in first + 1..first + 1001 {
                poll_native_api_seat(&mut slot, &snapshot, tick, false);
            }
        });
        assert_eq!(
            steady.count_total, 0,
            "1,000 unchanged native API-seat polls allocate nothing"
        );
        assert!(
            !slot.has_native_actions(),
            "unchanged native polls emit no actions"
        );
        assert!(
            Arc::ptr_eq(&page, slot.api.as_ref().unwrap().page.as_ref().unwrap()),
            "unchanged status keeps the page identity"
        );
        println!(
            "API gather unchanged polls=1000 allocations={}",
            steady.count_total
        );
        slot.stop();
        warm.stop();
        drop(instance);
        drop(prepared);
    }
}

#[cfg(all(feature = "load", feature = "test-hooks"))]
#[test]
fn compiled_walk_decisions_keep_their_observed_epoch_across_later_ticks_and_restore() {
    use crate::shim::InteractReq;

    struct WalkingRows(i32);
    impl Script for WalkingRows {
        fn tick(&mut self, cx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            self.0 += 1;
            cx.queue_test_interaction(InteractReq::WalkTo {
                x: self.0,
                z: 0,
                level: 0,
            });
            Ok(ScriptFlow::Continue)
        }
    }

    let mut slot = SlotScript::new();
    slot.bind_incarnation(10);
    slot.start_test_script(Box::new(WalkingRows(0)), Some(selected()))
        .unwrap();
    slot.observe_walk_outcome(10, None);
    tick(&mut slot);
    slot.observe_walk_outcome(11, None);
    tick(&mut slot);
    let (_, older, _) = slot.drain_host_interacts();

    slot.observe_walk_outcome(12, None);
    tick(&mut slot);
    slot.restore_host_interacts(older);
    let (_, rows, _) = slot.drain_host_interacts();
    let decisions: Vec<_> = rows
        .into_iter()
        .map(|queued| {
            let InteractReq::WalkTo { x, .. } = queued.req else {
                panic!("expected walking decision")
            };
            (x, queued.observed_walk_outcome_seq)
        })
        .collect();
    assert_eq!(
        decisions,
        vec![(1, 10), (2, 11), (3, 12)],
        "later observations and Pause-fence restoration must not refresh older walking decisions"
    );
}

struct StopWork {
    _quiet: crate::native::QuietReadLease,
    cancels: Arc<AtomicU64>,
}

impl crate::native::NativeMachine for StopWork {
    type Args = Arc<AtomicU64>;
    type Output = ();

    fn begin(
        cancels: Self::Args,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, crate::native::ActionError> {
        cx.emit(crate::shim::InteractReq::Held {
            name: "Logs".into(),
            action: "Drop".into(),
            slot: Some(0),
            target_item_id: None,
        })?;
        Ok(Self {
            _quiet: cx.begin_quiet_read(41)?,
            cancels,
        })
    }

    fn poll(
        &mut self,
        _: &mut ActionContext<'_>,
    ) -> std::task::Poll<Result<(), crate::native::ActionError>> {
        std::task::Poll::Pending
    }

    fn cancel(&mut self) {
        self.cancels.fetch_add(1, Ordering::Relaxed);
    }
}

/// Exercise both terminal contracts, not just a card that returns Blocked.
struct TerminalReporter {
    phase: NativePhase,
    flow_blocked: bool,
    stops: Arc<AtomicU64>,
    cancels: Arc<AtomicU64>,
    work: Option<crate::native::ActionHandle<StopWork>>,
}

impl Script for TerminalReporter {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.work.is_none() {
            self.work = Some(
                tick.actions
                    .begin::<StopWork>(Arc::clone(&self.cancels), &mut tick.cx)
                    .expect("queue pending native work and hold a quiet lease before termination"),
            );
        }
        let failure = ScriptFailure {
            code: "no-route".into(),
            message: "no reachable bank under the walk permissions".into(),
        };
        tick.output.status(ScriptStatus {
            run: tick.cx.run(),
            card: crate::CompiledId("test"),
            phase: self.phase,
            active_settings: 1,
            pending_settings: None,
            fields: Arc::from([crate::native::StatusField {
                key: "destination",
                label: "Destination",
                value: crate::native::StatusValue::Text("bank".into()),
            }]),
            failure: (self.phase == NativePhase::Blocked).then(|| failure.clone()),
        });
        Ok(if self.flow_blocked {
            ScriptFlow::Blocked(failure)
        } else {
            ScriptFlow::Continue
        })
    }

    fn on_stop(&mut self, reason: StopReason) {
        assert_eq!(reason, StopReason::Operator, "use the normal Stop path");
        self.stops.fetch_add(1, Ordering::Relaxed);
    }
}
#[derive(Debug, Clone)]
pub(crate) struct CapturedStatusLine {
    pub(crate) slot: String,
    pub(crate) level: api::hostlog::Level,
    pub(crate) source: api::hostlog::Source,
    pub(crate) message: String,
}

#[derive(Default)]
struct StatusLineSink(std::sync::Mutex<Vec<CapturedStatusLine>>);

impl api::hostlog::Sink for StatusLineSink {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let Some(slot) = record.slot.filter(|slot| slot.starts_with("status-")) else {
            return;
        };
        self.0.lock().unwrap().push(CapturedStatusLine {
            slot: slot.to_owned(),
            level: record.level,
            source: record.source,
            message: record.message.to_owned(),
        });
    }
}

static STATUS_LINE_SINK: StatusLineSink = StatusLineSink(std::sync::Mutex::new(Vec::new()));

pub(crate) fn install_status_line_sink() {
    let _ = api::hostlog::install_sink(&STATUS_LINE_SINK);
}

pub(crate) fn status_lines_for(slot: &str) -> Vec<CapturedStatusLine> {
    STATUS_LINE_SINK
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.slot == slot)
        .cloned()
        .collect()
}

fn status_line_test_status(phase: NativePhase, failure: Option<ScriptFailure>) -> ScriptStatus {
    ScriptStatus {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        card: crate::CompiledId("Gatherer"),
        phase,
        active_settings: 1,
        pending_settings: None,
        fields: Arc::from([]),
        failure,
    }
}

#[test]
fn native_status_logs_each_phase_or_failure_change_once() {
    const SLOT: &str = "status-change-test";
    install_status_line_sink();
    let mut output = Output {
        account: Some(SLOT.into()),
        ..Output::default()
    };

    output.status(status_line_test_status(NativePhase::Working, None));
    output.status(status_line_test_status(NativePhase::Working, None));
    output.status(status_line_test_status(NativePhase::Waiting, None));

    let failure = ScriptFailure {
        code: Arc::from("walk-ended"),
        message: Arc::from("walk ended with Failed"),
    };
    output.status(status_line_test_status(
        NativePhase::Blocked,
        Some(failure.clone()),
    ));
    output.status(status_line_test_status(
        NativePhase::Blocked,
        Some(failure.clone()),
    ));
    output.status(status_line_test_status(
        NativePhase::Blocked,
        Some(ScriptFailure {
            message: Arc::from("walk ended with a different reason"),
            ..failure
        }),
    ));
    output.status(status_line_test_status(
        NativePhase::Blocked,
        Some(ScriptFailure {
            code: Arc::from("walk-ended"),
            message: Arc::from("walk ended with a different reason"),
        }),
    ));

    let lines = status_lines_for(SLOT);
    assert_eq!(
        lines.len(),
        4,
        "initial phase, two phase transitions, and one changed failure; identical statuses add none"
    );
    assert_eq!(lines[0].level, api::hostlog::Level::Info);
    assert_eq!(
        lines[0].message,
        "native Gatherer phase=working failure=none"
    );
    assert_eq!(lines[1].level, api::hostlog::Level::Info);
    assert_eq!(
        lines[1].message,
        "native Gatherer phase=waiting failure=none"
    );
    assert_eq!(lines[2].level, api::hostlog::Level::Info);
    assert_eq!(
        lines[2].message,
        "native Gatherer phase=blocked failure=walk-ended failure_message=walk ended with Failed"
    );
    assert_eq!(lines[3].level, api::hostlog::Level::Info);
    assert_eq!(
        lines[3].message,
        "native Gatherer phase=blocked failure=walk-ended failure_message=walk ended with a different reason"
    );
    assert!(
        lines
            .iter()
            .all(|line| line.source == api::hostlog::Source::Script),
        "native status updates use the script log source"
    );
}

#[test]
fn native_status_logs_present_fields_as_plain_text() {
    const SLOT: &str = "status-format-test";
    install_status_line_sink();
    let mut output = Output {
        account: Some(SLOT.into()),
        ..Output::default()
    };
    let mut working = status_line_test_status(NativePhase::Working, None);
    working.fields = vec![
        crate::native::StatusField {
            key: "waiting_for",
            label: "Waiting for",
            value: crate::native::StatusValue::Text(Arc::from("bank trip")),
        },
        crate::native::StatusField {
            key: "action_state",
            label: "Action",
            value: crate::native::StatusValue::Text(Arc::from("banking")),
        },
        crate::native::StatusField {
            key: "last_event",
            label: "Last event",
            value: crate::native::StatusValue::Text(Arc::from("bank trip due")),
        },
    ]
    .into();
    output.status(working);
    output.status(status_line_test_status(
        NativePhase::Blocked,
        Some(ScriptFailure {
            code: Arc::from("walk-ended"),
            message: Arc::from("walk ended with Failed"),
        }),
    ));

    let lines = status_lines_for(SLOT);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0].message,
        "native Gatherer phase=working failure=none waiting_for=bank trip action_state=banking last_event=bank trip due"
    );
    assert_eq!(
        lines[1].message,
        "native Gatherer phase=blocked failure=walk-ended failure_message=walk ended with Failed"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.message.contains("Some(") && !line.message.contains("None")),
        "present status text is plain and absent fields are omitted"
    );
}

#[test]
fn native_status_logs_gatherer_method_area_target_and_reason() {
    const SLOT: &str = "status-diagnose-test";
    install_status_line_sink();
    let mut output = Output {
        account: Some(SLOT.into()),
        ..Output::default()
    };
    let mut blocked = status_line_test_status(
        NativePhase::Blocked,
        Some(ScriptFailure {
            code: Arc::from("resource-unavailable"),
            message: Arc::from(
                "resource-unavailable: absent: 2; method=fishing.saltfish.op1; area=Custom (3086,3229,0) r12; planning=3092,3243,0",
            ),
        }),
    );
    blocked.fields = vec![
        crate::native::StatusField {
            key: "method",
            label: "Method",
            value: crate::native::StatusValue::Text(Arc::from("Net")),
        },
        crate::native::StatusField {
            key: "area",
            label: "Area",
            value: crate::native::StatusValue::Text(Arc::from("Custom (3086,3229,0) r12")),
        },
        crate::native::StatusField {
            key: "target",
            label: "Target",
            value: crate::native::StatusValue::Text(Arc::from("—")),
        },
        crate::native::StatusField {
            key: "last_event",
            label: "Last event",
            value: crate::native::StatusValue::Text(Arc::from("Network stall")),
        },
    ]
    .into();
    output.status(blocked);

    let lines = status_lines_for(SLOT);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].message,
        "native Gatherer phase=blocked failure=resource-unavailable failure_message=resource-unavailable: absent: 2; method=fishing.saltfish.op1; area=Custom (3086,3229,0) r12; planning=3092,3243,0 method=Net last_event=Network stall"
    );
}

#[test]
fn native_status_logs_gatherer_last_event_at_info() {
    const SLOT: &str = "status-last-event-info";
    install_status_line_sink();
    let mut output = Output {
        account: Some(SLOT.into()),
        ..Output::default()
    };
    let mut first = status_line_test_status(NativePhase::Working, None);
    first.fields = vec![crate::native::StatusField {
        key: "last_event",
        label: "Last event",
        value: crate::native::StatusValue::Text(Arc::from("started")),
    }]
    .into();
    output.status(first);
    let mut second = status_line_test_status(NativePhase::Working, None);
    second.fields = vec![crate::native::StatusField {
        key: "last_event",
        label: "Last event",
        value: crate::native::StatusValue::Text(Arc::from("waiting for world")),
    }]
    .into();
    output.status(second);

    let mut gathering = status_line_test_status(NativePhase::Working, None);
    gathering.fields = vec![crate::native::StatusField {
        key: "last_event",
        label: "Last event",
        value: crate::native::StatusValue::Text(Arc::from("gathering")),
    }]
    .into();
    output.status(gathering);

    let lines = status_lines_for(SLOT);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].level, api::hostlog::Level::Info);
    assert_eq!(lines[1].level, api::hostlog::Level::Info);
    assert_eq!(lines[2].level, api::hostlog::Level::Debug);
    assert!(
        lines[1].message.contains("last_event=waiting for world"),
        "{}",
        lines[1].message
    );
    assert!(
        lines[2].message.contains("last_event=gathering"),
        "{}",
        lines[2].message
    );
}

#[test]
fn terminal_native_blocked_stop_logs_once_through_output_status() {
    install_status_line_sink();
    for (slot_name, phase, flow_blocked) in [
        ("status-blocked-flow-stop", NativePhase::Working, true),
        ("status-blocked-status-stop", NativePhase::Blocked, false),
    ] {
        let stops = Arc::new(AtomicU64::new(0));
        let cancels = Arc::new(AtomicU64::new(0));
        let mut slot = SlotScript::new();
        slot.start_test_script(
            Box::new(TerminalReporter {
                phase,
                flow_blocked,
                stops: Arc::clone(&stops),
                cancels: Arc::clone(&cancels),
                work: None,
            }),
            Some(selected()),
        )
        .unwrap();
        slot.compiled.as_mut().unwrap().output.account = Some(slot_name.into());
        tick(&mut slot);

        assert_eq!(slot.state(), RunState::Idle);
        assert_eq!(stops.load(Ordering::Relaxed), 1);
        assert_eq!(cancels.load(Ordering::Relaxed), 1);
        let blocked_lines = status_lines_for(slot_name)
            .into_iter()
            .filter(|line| line.message.contains("phase=blocked"))
            .collect::<Vec<_>>();
        assert_eq!(blocked_lines.len(), 1, "{slot_name}");
        assert_eq!(blocked_lines[0].level, api::hostlog::Level::Info);
        assert_eq!(blocked_lines[0].source, api::hostlog::Source::Script);
        assert_eq!(
            blocked_lines[0].message,
            "native test phase=blocked failure=no-route failure_message=no reachable bank under the walk permissions"
        );
    }
}

#[test]
fn terminal_native_blocked_stops_cleans_up_and_preserves_reason() {
    for (phase, flow_blocked) in [(NativePhase::Working, true), (NativePhase::Blocked, false)] {
        let stops = Arc::new(AtomicU64::new(0));
        let cancels = Arc::new(AtomicU64::new(0));
        let mut slot = SlotScript::new();
        slot.start_test_script(
            Box::new(TerminalReporter {
                phase,
                flow_blocked,
                stops: Arc::clone(&stops),
                cancels: Arc::clone(&cancels),
                work: None,
            }),
            Some(selected()),
        )
        .unwrap();
        slot.attach_source_identity("compiled:test");
        let control_generation = slot.control_generation;
        #[cfg(feature = "load")]
        {
            slot.clue_stop_owed = false;
        }
        let run = slot.native_run().unwrap();
        tick(&mut slot);
        assert_eq!(slot.state(), RunState::Idle);
        assert!(!slot.want_run);
        assert!(slot.native_run().is_none());
        assert!(slot.compiled.is_none());
        assert_eq!(slot.control_generation, control_generation + 1);
        assert!(
            slot.source_identity().is_none(),
            "normal Stop clears identity"
        );
        #[cfg(feature = "load")]
        assert!(slot.clue_stop_owed, "normal Stop owes fresh clue cleanup");
        assert!(slot.retained.is_none(), "Stop releases retained work");
        assert!(!slot.has_native_actions());
        assert!(slot.native_quiet_read(Instant::now()).is_none());
        assert_eq!(stops.load(Ordering::Relaxed), 1);
        assert_eq!(cancels.load(Ordering::Relaxed), 1, "Stop cancels live work");
        let status = slot
            .native_status()
            .expect("terminal diagnostic survives Stop");
        assert_eq!(status.run, run);
        assert_eq!(status.phase, NativePhase::Blocked);
        assert_eq!(status.fields[0].key, "destination");
        assert_eq!(
            status.failure.as_ref().unwrap().message.as_ref(),
            "no reachable bank under the walk permissions"
        );
        assert_eq!(
            slot.lifecycle_receipt.as_ref().unwrap().state,
            ScriptTerminalState::Failed
        );
        let now = Instant::now() + crate::watchdog::HARD_STALL;
        assert_eq!(
            slot.feed_watchdog(now, Some((0, 0, 0)), &[], false, true, &[]),
            WatchdogAction::None
        );
        assert!(slot.restart_from_identity(now).is_err());
        slot.resume();
        slot.on_is_up(true);
        slot.reconnect_session_work();
        tick(&mut slot);
        assert_eq!(slot.state(), RunState::Idle, "no automatic resurrection");
        assert_eq!(slot.native_status().as_deref(), Some(status.as_ref()));
        assert_eq!(stops.load(Ordering::Relaxed), 1);
        slot.start_test_script(
            Box::new(TerminalReporter {
                phase: NativePhase::Waiting,
                flow_blocked: false,
                stops: Arc::clone(&stops),
                cancels: Arc::clone(&cancels),
                work: None,
            }),
            Some(selected()),
        )
        .unwrap();
        assert!(
            slot.native_status().is_none(),
            "explicit Start clears the old reason"
        );
        // The explicit Start is observed on a fresh server tick, not the
        // already-spent emission budget of the blocked tick.
        slot.on_game_tick(&mut ScriptCtx {
            driver: &mut NullDriver::default(),
            tick: 2,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::CompiledTick::default(),
        });
        assert_eq!(slot.state(), RunState::Running);
        assert!(slot.has_native_actions(), "the fixture really queues work");
        assert!(slot.native_quiet_read(Instant::now()).is_some());
        slot.stop();
        assert_eq!(stops.load(Ordering::Relaxed), 2);
        assert_eq!(cancels.load(Ordering::Relaxed), 2);
    }
}

#[test]
fn native_waiting_keeps_running_until_its_own_terminal_bound() {
    let stops = Arc::new(AtomicU64::new(0));
    let cancels = Arc::new(AtomicU64::new(0));
    let mut slot = SlotScript::new();
    slot.start_test_script(
        Box::new(TerminalReporter {
            phase: NativePhase::Waiting,
            flow_blocked: false,
            stops: Arc::clone(&stops),
            cancels: Arc::clone(&cancels),
            work: None,
        }),
        Some(selected()),
    )
    .unwrap();
    let run = slot.native_run();
    for _ in 0..3 {
        tick(&mut slot);
        assert_eq!(slot.state(), RunState::Running);
        assert_eq!(slot.native_run(), run);
        assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    }
    assert_eq!(stops.load(Ordering::Relaxed), 0);
    assert_eq!(cancels.load(Ordering::Relaxed), 0);
    slot.stop();
    assert_eq!(cancels.load(Ordering::Relaxed), 1);
}

#[test]
fn host_world_members_reaches_the_native_snapshot_view() {
    use api::selected::Truth;
    struct Reads(Arc<std::sync::Mutex<Vec<Truth>>>);
    impl Script for Reads {
        fn tick(&mut self, cx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            self.0
                .lock()
                .unwrap()
                .push(cx.cx.snapshot().world_members());
            Ok(ScriptFlow::Continue)
        }
    }
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut slot = SlotScript::new();
    slot.bind_incarnation(10);
    assert_eq!(slot.world_members(), Truth::Unknown);
    slot.bind_world_members(Truth::True);
    assert_eq!(slot.world_members(), Truth::True);
    slot.start_test_script(Box::new(Reads(Arc::clone(&seen))), Some(selected()))
        .unwrap();
    for world in [Truth::True, Truth::False, Truth::Unknown] {
        slot.on_game_tick(&mut ScriptCtx {
            driver: &mut NullDriver::default(),
            tick: 1,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
            compiled: crate::CompiledTick {
                world_members: world,
                ..Default::default()
            },
        });
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [Truth::True, Truth::False, Truth::Unknown]
    );
}

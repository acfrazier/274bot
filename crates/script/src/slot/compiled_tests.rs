use super::*;
use crate::ctx::test_support::NullDriver;
use std::sync::atomic::{AtomicU64, Ordering};

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()
}

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

#[test]
fn a_late_factory_reply_cannot_replace_a_new_run_after_stop() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(11);
    let retained = Arc::new(Mutex::new(RetainedMemory::default()));
    let held = retained.lock().unwrap();
    slot.retained = Some(Arc::clone(&retained));
    start(&mut slot, SettingsBag::new());
    // Keep the real worker handle so this test can force drain of a completed
    // old reply. In production Stop discards it, making resurrection impossible.
    let stale = slot.preparing.take().unwrap();
    slot.stop();
    start(&mut slot, SettingsBag::new());
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    let replacement = slot.native_run().unwrap();
    drop(held);
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
    assert!(!alice.post_settings_bag_fenced(&SettingsBag::new(), "compiled:test", alice_run.run));
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
        let (release, gate) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            gate.recv().unwrap();
            PreparationResult(Some(Ok(*run)))
        });
        if finished {
            release.send(()).unwrap();
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
        if !finished {
            release.send(()).unwrap();
        }
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

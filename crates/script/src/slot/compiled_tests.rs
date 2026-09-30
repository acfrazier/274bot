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

/// A Sherlock revision prepared exactly as a settings edit prepares it.
fn sherlock_config(revision: u64) -> Arc<PreparedConfig> {
    FamilyPreparation::run(move |worker| {
        prepare_config(
            worker,
            crate::CompiledId("Sherlock"),
            revision,
            Arc::new(SettingsBag::new()),
            selected(),
            Arc::default(),
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap()
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
                    radius: 0,
                    options: crate::FindOptions::default(),
                    required_after: first,
                    evidence: None,
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
    slot.start_compiled(
        "gatherer-lifecycle",
        crate::CompiledId("Gatherer"),
        Arc::new(SettingsBag::new()),
        selected(),
        Arc::default(),
    )
    .unwrap();
    assert_eq!(settle(&mut slot), StartOutcome::Ready);
    slot
}

fn queue_gatherer_drop(
    slot: &mut SlotScript,
    snapshot: &api::snapshot::GameSnapshot,
    first_tick: u32,
) -> (u32, Vec<crate::native::HostAuthority>) {
    for tick in first_tick..first_tick + 16 {
        gatherer_tick(slot, snapshot, tick);
        if slot.has_native_actions() {
            let ledger = slot.native_runtime.ledger.as_ref().unwrap();
            let authorities = ledger
                .outbox
                .iter()
                .map(|action| {
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
                    action.authority()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                authorities.len(),
                5,
                "the full pack queues one five-drop batch"
            );
            assert!(authorities.iter().all(crate::native::HostAuthority::live));
            return (tick, authorities);
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
        let (tick, authorities) = queue_gatherer_drop(&mut slot, &snapshot, 1);
        let old_run = slot.native_run().unwrap();
        match boundary {
            0 => slot.stop(),
            1 => slot.pause(),
            2 => restart_and_settle(&mut slot, Instant::now()),
            _ => unreachable!(),
        }
        assert!(authorities.iter().all(|authority| !authority.live()));
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
        assert!(fresh.iter().all(crate::native::HostAuthority::live));
        assert!(authorities.iter().all(|authority| !authority.live()));
        slot.stop();
        assert!(fresh.iter().all(|authority| !authority.live()));
    }
}

#[test]
fn gatherer_session_change_replans_drop_from_observation_not_old_authority() {
    let snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(84);
    let (tick, authorities) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    let old_run = slot.native_run().unwrap();
    slot.reconnect_session_work();
    slot.on_is_up(true);
    assert!(authorities.iter().all(|authority| !authority.live()));
    assert!(slot.take_native_action().is_none());
    let (_, fresh) = queue_gatherer_drop(&mut slot, &snapshot, tick + 1);
    let new_run = slot.native_run().unwrap();
    assert_eq!((new_run.slot, new_run.run), (old_run.slot, old_run.run));
    assert_ne!(new_run.session, old_run.session);
    assert!(fresh.iter().all(|authority| authority.run() == new_run));
    assert!(authorities.iter().all(|authority| !authority.live()));
    slot.stop();
}

#[test]
fn gatherer_death_observation_cancels_undrained_drop_without_retry() {
    let mut snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(85);
    let (tick, authorities) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
        sequence: 1,
    }]);
    gatherer_tick(&mut slot, &snapshot, tick + 1);
    assert!(authorities.iter().all(|authority| !authority.live()));
    assert!(slot.take_native_action().is_none());
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "died");
    assert!(!failure.retryable);
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
    let (tick, _) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    let mut sent_slots = Vec::new();
    while let Some(action) = slot.take_native_action() {
        if let crate::native::HostEffect::Interaction(crate::shim::InteractReq::Held {
            slot: Some(index),
            ..
        }) = &action.effect
        {
            sent_slots.push(*index);
        }
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
            },
        );
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

fn accept_gatherer_drops(slot: &mut SlotScript, tick: u32, refused_slot: Option<i32>) -> Vec<i32> {
    let mut slots = Vec::new();
    while let Some(action) = slot.take_native_action() {
        let crate::native::HostEffect::Interaction(crate::shim::InteractReq::Held {
            slot: Some(index),
            ..
        }) = &action.effect
        else {
            panic!("expected slot-exact drop")
        };
        slots.push(*index);
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
                accepted: refused_slot != Some(*index),
            },
        );
    }
    slots
}

#[test]
fn gatherer_rejected_drop_replans_only_its_slot_then_observes_late_settlement() {
    let mut snapshot = gatherer_snapshot();
    let mut slot = gatherer_slot(88);
    let (tick, _) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    assert_eq!(
        accept_gatherer_drops(&mut slot, tick, Some(2)),
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
    let (tick, _) = queue_gatherer_drop(&mut slot, &snapshot, 1);
    assert_eq!(
        accept_gatherer_drops(&mut slot, tick, None),
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
    assert!(failure.retryable);
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
        slot.retained
            .as_ref()
            .unwrap()
            .lock()
            .expect("retained memory")
            .gather()
            .yielded,
        1
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
    let snapshot = gatherer_snapshot();
    let (cold_prepared, cold) = measure_prepared();
    let mut warm = gatherer_slot(92);
    let (prepared, preparation) = measure_prepared();
    let prepare_bytes = preparation.bytes_current;
    let mut measured = None;
    let main = allocation_counter::measure(|| {
        let mut slot = gatherer_slot(93);
        let (tick, _) = queue_gatherer_drop(&mut slot, &snapshot, 1);
        accept_gatherer_drops(&mut slot, tick, None);
        measured = Some(slot);
    });
    // Preparation runs on its own thread; count its retained allocation
    // separately from the real slot's instance, ledger, status and paint.
    let exclusive = prepare_bytes + main.bytes_current;
    println!("Gatherer exclusive heap: prepared={prepare_bytes}, slot={main:?}, total={exclusive}");
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
}

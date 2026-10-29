use super::*;
use crate::ctx::test_support::NullDriver;
use crate::native::snapshot_source::{NativeFactsInput, SnapshotInput, TileInput};
use crate::native::Script;
use crate::surface::test_support::{StubSurfaceCard, StubSurfaceFactory, StubSurfaceHandle};
use crate::surface::{SurfaceReady, TickOutcome};

struct Noop;

impl Script for Noop {
    fn tick(
        &mut self,
        _ctx: &mut crate::native::NativeTick<'_>,
    ) -> Result<crate::native::ScriptFlow, crate::native::ScriptFailure> {
        Ok(crate::native::ScriptFlow::Continue)
    }
}

fn empty_snapshot(tick: u64) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 0,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: crate::native::snapshot_source::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn stub_surface_start() -> crate::surface::SurfaceStart {
    crate::surface::SurfaceStart {
        card: Arc::new(StubSurfaceCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
    }
}

fn stub_slot_with_incarnation(
    incarnation: u64,
) -> (SlotScript, Arc<StubSurfaceFactory>, StubSurfaceHandle) {
    let mut slot = SlotScript::new();
    if incarnation != 0 {
        slot.bind_incarnation(incarnation);
    }
    let factory = Arc::new(StubSurfaceFactory::new());
    slot.bind_surface_factory(factory.clone());
    slot.start_surface(stub_surface_start()).unwrap();
    let run = factory.last_run().expect("stub surface run");
    (slot, factory, run)
}

fn stub_slot() -> (SlotScript, Arc<StubSurfaceFactory>, StubSurfaceHandle) {
    stub_slot_with_incarnation(0)
}

fn ready_stub_slot_with_incarnation(
    incarnation: u64,
) -> (SlotScript, Arc<StubSurfaceFactory>, StubSurfaceHandle) {
    let (mut slot, factory, run) = stub_slot_with_incarnation(incarnation);
    run.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    (slot, factory, run)
}

fn ready_stub_slot() -> (SlotScript, Arc<StubSurfaceFactory>, StubSurfaceHandle) {
    ready_stub_slot_with_incarnation(0)
}

fn wait_state(slot: &mut SlotScript, want: RunState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while slot.state() != want && Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(slot.state(), want, "last_error={:?}", slot.last_error());
}

fn native_tick_at(
    slot: &mut SlotScript,
    selected: &api::game_data::SelectedGameData,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    hold: bool,
) {
    let mut driver = NullDriver::default();
    let mut ctx = compiled_ctx(&mut driver, Some(selected));
    ctx.tick = tick;
    ctx.here = here;
    ctx.compiled.hold = hold;
    slot.on_game_tick(&mut ctx);
}

fn start_api_gather_and_wait(slot: &mut SlotScript, selected: &api::game_data::SelectedGameData) {
    let run = slot.seat.as_ref().expect("owned native seat").run();
    let id = api::identity::RequestId::isolate(1).expect("isolate request");
    slot.consume_seat_batch(
        crate::native::seat_rows::DecodedSeatBatch {
            consumed_terminal_seq: 0,
            starts: vec![crate::native::seat_rows::SeatStartInput {
                id,
                parent: None,
                run: run.run,
                session: run.session,
                kind: crate::native::request::NativeKind::Gather,
                args: crate::native::seat_rows::SeatStartArgs::Gather(
                    crate::native::SettingsBag::new(),
                ),
            }],
            decisions: Vec::new(),
            cancels: Vec::new(),
            mirror_revokes: Vec::new(),
            reqs: Vec::new(),
        },
        0,
        &[],
    );
    let (policy, visible, owned) = slot.drain_host_interacts();
    assert!(policy.is_none());
    assert!(
        visible
            .iter()
            .all(|row| matches!(&row.req, crate::shim::InteractReq::LoopSettled)),
        "only surface lifecycle, not delegated gameplay, may reach host: {visible:?}"
    );
    assert!(
        owned,
        "the native API seat owns delegated Gatherer work: {:?}",
        slot.api.as_ref().and_then(|seat| seat.terminal.as_ref())
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut tick = 10;
    while slot.native_status().is_none() && Instant::now() < deadline {
        native_tick_at(slot, selected, tick, None, false);
        tick += 1;
        let (_, visible, _) = slot.drain_host_interacts();
        assert!(
            visible
                .iter()
                .all(|row| matches!(&row.req, crate::shim::InteractReq::LoopSettled)),
            "delegated Gatherer gameplay stays in the seat: {visible:?}"
        );
        std::thread::yield_now();
    }
    assert!(
        slot.native_status().is_some(),
        "the delegated Gatherer reaches its running status"
    );
    assert!(slot.api_owns_foreground());
}

#[test]
fn non_kept_stop_releases_the_seat_before_idle_and_rearms_on_next_start() {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(39);
    slot.start_test_script(Box::new(Noop), None).unwrap();
    assert!(slot.seat.is_some(), "compiled card owns a native seat");
    let root = slot.seat_probe().unwrap();
    assert!(root.compiled_owner_live);
    assert_eq!(
        root.live, 1,
        "an idle compiled step retains only its run owner"
    );
    assert_eq!(root.reserved_releases, 0);

    slot.stop();
    assert_eq!(slot.state(), RunState::Idle);
    assert!(slot.seat.is_none(), "Stop drops the old seat owner");

    slot.start_test_script(Box::new(Noop), None).unwrap();
    assert!(slot.seat.is_some(), "a new run arms a new seat");
    let root = slot.seat_probe().unwrap();
    assert!(root.compiled_owner_live);
    assert_eq!(root.live, 1);
    slot.stop_removed();
    assert!(slot.seat.is_none(), "removal is also a non-kept boundary");
}

#[test]
fn surface_without_native_wait_retains_gameplay_wedge_detection() {
    let (mut slot, _, _) = ready_stub_slot();
    assert!(slot.native_run().is_none());
    assert!(slot.evidence_wait_run.is_none());
    let here = Some((3200, 3200, 0));
    let started = Instant::now();
    slot.note_game_packet_tick(started);
    assert_eq!(
        slot.feed_watchdog(started, here, &[], false, true, &[]),
        WatchdogAction::None
    );
    let wedged = started + crate::watchdog::WEDGE;
    slot.watchdog.stamp_scheduler(wedged);
    slot.note_game_packet_tick(wedged);
    assert_eq!(
        slot.feed_watchdog(wedged, here, &[], false, true, &[]),
        WatchdogAction::RequestAnchor,
        "two absent native run keys do not authorize an evidence wait"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn surface_tick_error_clears_after_success_not_user_logs() {
    let (mut slot, _, run) = ready_stub_slot();
    run.queue_tick_outcomes([TickOutcome::Error {
        tick: 1,
        generation: 0,
        message: "first tick failure".into(),
    }]);
    run.queue_logs(["first tick failure".into()]);
    let first_logs = slot.drain_logs();
    assert!(
        first_logs
            .iter()
            .any(|line| line.contains("first tick failure")),
        "{first_logs:?}"
    );
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("first tick failure")),
        "{:?}",
        slot.last_error()
    );

    let mut held = empty_snapshot(2);
    held.hold = true;
    assert!(slot.post_snapshot_delta(&held, NativeFactsInput::default(), false));
    run.queue_logs(["tick completed normally".into()]);
    let _ = slot.drain_logs();
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("first tick failure")),
        "a paint-only held frame cannot recover a loop failure: {:?}",
        slot.last_error()
    );

    held.tick = 3;
    held.hold = false;
    assert!(slot.post_snapshot_delta(&held, NativeFactsInput::default(), false));
    run.queue_tick_outcomes([TickOutcome::Error {
        tick: 3,
        generation: 0,
        message: "second tick failure".into(),
    }]);
    run.queue_logs(["second tick failure".into()]);
    let second_logs = slot.drain_logs();
    assert!(
        second_logs
            .iter()
            .any(|line| line.contains("second tick failure")),
        "{second_logs:?}"
    );
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("second tick failure")),
        "a second failing tick cannot recover the active error: {:?}",
        slot.last_error()
    );

    held.tick = 4;
    assert!(slot.post_snapshot_delta(&held, NativeFactsInput::default(), false));
    run.queue_tick_outcomes([TickOutcome::Success {
        tick: 4,
        generation: 0,
    }]);
    run.queue_logs(["tick completed normally".into()]);
    let third_logs = slot.drain_logs();
    assert!(
        third_logs
            .iter()
            .any(|line| line == "tick completed normally"),
        "{third_logs:?}"
    );
    assert_eq!(
        slot.last_error(),
        None,
        "a successful same-generation tick clears the active error; a user log does not replace it"
    );
    assert!(
        slot.take_pending_logs()
            .iter()
            .any(|line| line.contains("first tick failure")),
        "the recovered error remains in the slot log"
    );
}

#[test]
fn reconnect_success_clears_stub_surface_error() {
    let (mut slot, _, run) = ready_stub_slot();
    run.queue_tick_outcomes([TickOutcome::Error {
        tick: 1,
        generation: 0,
        message: "session-one failure".into(),
    }]);
    let _ = slot.drain_logs();
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("session-one failure")),
        "{:?}",
        slot.last_error()
    );

    slot.reset_session_work();
    slot.on_is_up(true);
    run.queue_tick_outcomes([TickOutcome::Success {
        tick: 2,
        generation: 0,
    }]);
    let _ = slot.drain_logs();
    assert_eq!(
        slot.last_error(),
        None,
        "the first completed clean tick in the new session recovers the active diagnostic"
    );
}

#[test]
fn parked_stub_surface_does_not_clear_active_error() {
    let (mut slot, _, run) = ready_stub_slot();
    run.queue_tick_outcomes([TickOutcome::Error {
        tick: 2,
        generation: 0,
        message: "async iteration failure".into(),
    }]);
    let _ = slot.drain_logs();
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("async iteration failure")),
        "{:?}",
        slot.last_error()
    );

    run.queue_logs(["next async iteration parked".into()]);
    let _ = slot.drain_logs();
    assert!(
        slot.last_error()
            .is_some_and(|error| error.contains("async iteration failure")),
        "parking a new async iteration is not a completed success: {:?}",
        slot.last_error()
    );
}

#[test]
fn surface_success_clears_active_error_after_failure() {
    let (mut slot, _, run) = ready_stub_slot();
    run.queue_tick_outcomes([TickOutcome::Error {
        tick: 1,
        generation: 0,
        message: "transient failure".into(),
    }]);
    let _ = slot.drain_logs();
    assert!(slot
        .last_error()
        .is_some_and(|error| error.contains("transient failure")));

    run.queue_tick_outcomes([TickOutcome::Success {
        tick: 2,
        generation: 0,
    }]);
    let _ = slot.drain_logs();
    assert_eq!(
        slot.last_error(),
        None,
        "a slow tick that completed normally is still a successful recovery"
    );
}

#[test]
fn session_reset_discards_old_surface_policy_batch() {
    let (mut slot, _, run) = stub_slot_with_incarnation(1203);
    run.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    let policy = api::run_policy::RunPolicyOverride {
        run_auto: Some(false),
        energy_min: None,
    };
    run.queue_seat_batches([crate::native::seat_rows::QueuedSeatBatch {
        generation: 0,
        observed_walk_outcome_seq: 0,
        batch: crate::native::seat_rows::DecodedSeatBatch {
            consumed_terminal_seq: 0,
            starts: Vec::new(),
            decisions: Vec::new(),
            cancels: Vec::new(),
            mirror_revokes: Vec::new(),
            reqs: vec![crate::native::seat_rows::SeatInteractDecoded {
                lineage: None,
                mirror_owner: None,
                request: crate::interact::InteractReq::RunPolicyOverride {
                    policy: Some(policy),
                },
            }],
        },
        machine_events: Vec::new(),
        held_waits: Default::default(),
        held_fold: false,
    }]);

    run.set_generation(1);
    slot.reset_session_work();
    let (old_policy, old_requests, _) = slot.drain_host_interacts();
    assert!(
        old_policy.is_none() && old_requests.is_empty(),
        "the old connection generation stays fenced"
    );
}

#[test]
fn surface_stop_receipt_cleans_slot_work_and_allows_fresh_restart() {
    let (mut slot, factory, run) = ready_stub_slot();
    run.set_terminal(
        true,
        Some(crate::surface::ScriptStopReceipt {
            tick: 1,
            reason: "finished".into(),
        }),
    );
    run.queue_logs(["script requested stop".into()]);
    run.set_retirement_logs(Vec::<String>::new());
    let input = empty_snapshot(1);
    assert!(slot.post_snapshot_delta(&input, NativeFactsInput::default(), false));
    slot.store_last_world_id(Some(123));
    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
    let epoch = slot.work_epoch();
    let mut logs = Vec::new();
    while matches!(
        slot.state(),
        RunState::Starting | RunState::Running | RunState::Stopping
    ) {
        logs.extend(slot.drain_logs());
        std::thread::yield_now();
    }
    assert_eq!(slot.state(), RunState::Idle, "{logs:?}");
    assert!(logs
        .iter()
        .any(|line| line.contains("script requested stop")));
    assert!(!slot.has_instance());
    assert!(slot.pending_withdraw_x().is_none());
    assert_ne!(slot.work_epoch(), epoch);
    assert!(!slot.has_posted_snapshot());
    assert_eq!(slot.last_world_id(), None);
    assert!(slot.drain_interacts().is_empty());
    assert!(slot.last_error().unwrap().contains("script requested stop"));
    assert_eq!(
        slot.lifecycle_receipt(),
        Some(ScriptLifecycleReceipt {
            runtime_generation: 1,
            state: ScriptTerminalState::Stopped,
            tick: 1,
            reason: "finished".into(),
        })
    );
    assert_eq!(slot.take_pending_logs(), logs);
    slot.on_is_up(true);
    assert_eq!(
        slot.state(),
        RunState::Idle,
        "login cannot restart a stopped card"
    );

    slot.start_surface(stub_surface_start()).unwrap();
    let fresh_run = factory.last_run().expect("fresh stub surface run");
    fresh_run.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    assert_eq!(slot.state(), RunState::Running);
    assert!(slot.last_error().is_none());
    assert_eq!(
        slot.lifecycle_receipt(),
        None,
        "fresh Start clears the receipt"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    assert_eq!(
        slot.lifecycle_receipt(),
        None,
        "operator Stop is not a script-requested Stopped receipt"
    );
}

#[test]
fn surface_stop_receipt_ends_slot_cleanly_with_its_reason() {
    let (mut slot, _, run) = ready_stub_slot();
    run.set_terminal(
        true,
        Some(crate::surface::ScriptStopReceipt {
            tick: 1,
            reason: "BankFletcher: bank is out of 'Willow logs'".into(),
        }),
    );
    run.set_retirement_logs(Vec::<String>::new());
    let _ = slot.drain_logs();
    wait_state(&mut slot, RunState::Idle);
    assert_eq!(slot.state(), RunState::Idle);
    assert_eq!(
        slot.lifecycle_receipt(),
        Some(ScriptLifecycleReceipt {
            runtime_generation: 1,
            state: ScriptTerminalState::Stopped,
            tick: 1,
            reason: "BankFletcher: bank is out of 'Willow logs'".into(),
        }),
        "a finish is a clean Stopped receipt, never Failed"
    );
    slot.on_is_up(true);
    assert_eq!(
        slot.state(),
        RunState::Idle,
        "a relog cannot restart a finished script"
    );
}

#[test]
fn script_stop_receipt_logs_reason_once() {
    const SLOT: &str = "status-finish-log-once";
    crate::slot::compiled::tests::install_status_line_sink();
    api::hostlog::bind_slot(SLOT);
    let (mut slot, _, run) = ready_stub_slot();
    run.set_terminal(
        true,
        Some(crate::surface::ScriptStopReceipt {
            tick: 1,
            reason: "finish-log-once: out of logs".into(),
        }),
    );
    run.set_retirement_logs(Vec::<String>::new());
    let _ = slot.drain_logs();
    wait_state(&mut slot, RunState::Idle);
    let _ = slot.drain_logs();
    let _ = slot.drain_logs();
    let captured: Vec<_> = crate::slot::compiled::tests::status_lines_for(SLOT)
        .into_iter()
        .filter(|line| line.message.contains("finish-log-once: out of logs"))
        .collect();
    assert_eq!(
        captured.len(),
        1,
        "the finish reason reaches the host log exactly once: {captured:?}"
    );
    assert_eq!(captured[0].level, api::hostlog::Level::Info);
    assert_eq!(captured[0].source, api::hostlog::Source::Script);
    assert_eq!(
        captured[0].message,
        "script requested stop tick=1 reason=finish-log-once: out of logs"
    );
}

#[test]
fn delegated_finish_keeps_surface_running_without_receipt() {
    let (slot, _, _) = ready_stub_slot();
    assert_eq!(slot.state(), RunState::Running);
    assert_eq!(
        slot.lifecycle_receipt(),
        None,
        "a delegated finish is not a host stop"
    );
}

#[test]
fn stop_releases_snapshot_storage_and_restart_emits_keyframe() {
    let (mut slot, _, run) = ready_stub_slot();
    run.set_posted_facts(Some(crate::surface::PostedSnapshotFacts {
        withdraw_x_result_seq: 0,
        bank_op_result_seq: 0,
        walk_outcome_seq: 0,
        bank_selection: Default::default(),
        route_inspect: Default::default(),
    }));
    slot.store_last_world_id(Some(123));
    slot.last_error = Some("retained diagnostic".into());
    slot.pending_logs.push("retained log".into());
    slot.pause();
    assert!(slot.has_posted_snapshot());
    assert_eq!(slot.last_world_id(), Some(123));
    slot.resume();
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    assert!(!slot.has_posted_snapshot());
    assert_eq!(slot.last_world_id(), None);
    assert_eq!(
        slot.last_error, None,
        "operator Stop clears the active diagnostic"
    );
    assert_eq!(
        slot.pending_logs,
        ["retained log"],
        "Stop preserves diagnostic history for the panel"
    );
    slot.start_test_script(Box::new(Noop), None).unwrap();
}

#[test]
fn on_random_defaults_to_host_and_override_claims_handle() {
    use api::random::{DetectedRandom, RandomClaim, RandomKind};

    struct ClaimHandle;
    impl Script for ClaimHandle {
        fn tick(
            &mut self,
            _ctx: &mut crate::native::NativeTick<'_>,
        ) -> Result<crate::native::ScriptFlow, crate::native::ScriptFailure> {
            Ok(crate::native::ScriptFlow::Continue)
        }
        fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
            RandomClaim::Handle
        }
    }

    let ev = DetectedRandom {
        kind: RandomKind::Dialog,
        name: "genie".to_string(),
        ours: true,
        npc_index: Some(0),
    };

    // Default: Host.
    let mut s = SlotScript::new();
    s.start_test_script(Box::new(Noop), None).unwrap();
    assert_eq!(s.on_random(&ev), RandomClaim::Host);

    // Override: Handle.
    s.stop();
    s.start_test_script(Box::new(ClaimHandle), None).unwrap();
    assert_eq!(s.on_random(&ev), RandomClaim::Handle);

    // Paused: Host — the knock only fires while Running.
    s.pause();
    assert_eq!(s.on_random(&ev), RandomClaim::Host);

    // Idle (stopped): Host.
    s.stop();
    assert_eq!(s.on_random(&ev), RandomClaim::Host);
}

#[test]
fn ticks_counts_dispatched_ticks_since_start() {
    let mut s = SlotScript::new();
    s.start_test_script(Box::new(Noop), None).unwrap();
    let mut d = NullDriver::default();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 2,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    assert_eq!(s.ticks, 2);

    // Paused ticks do not count.
    s.pause();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 3,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    assert_eq!(s.ticks, 2);

    // A fresh Start resets the counter.
    s.stop();
    s.start_test_script(Box::new(Noop), None).unwrap();
    s.on_game_tick(&mut ScriptCtx {
        driver: &mut d,
        tick: 4,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::ctx::CompiledTick::default(),
    });
    assert_eq!(s.ticks, 1);
}

#[test]
fn pending_withdraw_x_pause_freezes_while_stop_and_reconnect_abort() {
    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Noop), None).unwrap();
    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
    assert_eq!(
        slot.pending_withdraw_x().unwrap().remaining,
        Duration::from_millis(3000)
    );

    slot.pause();
    let paused = slot
        .pending_withdraw_x()
        .expect("Pause retains pending work");
    assert!(paused.deadline.is_none(), "Pause freezes monotonic time");
    slot.resume();
    assert!(
        slot.pending_withdraw_x().unwrap().deadline.is_some(),
        "Resume restores the remaining deadline"
    );

    slot.stop();
    assert!(
        slot.pending_withdraw_x().is_none(),
        "Stop aborts pending work"
    );

    slot.start_test_script(Box::new(Noop), None).unwrap();
    slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
    let before_reset = slot.withdraw_x_result();
    slot.reset_session_work();
    assert!(
        slot.pending_withdraw_x().is_none(),
        "reconnect/session reset aborts pending work"
    );
    assert_eq!(
        slot.withdraw_x_result(),
        (before_reset.0.wrapping_add(1), false),
        "session reset publishes an explicit abort even if a later bank reuses the generation"
    );

    let settlement = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3).waiting_settlement();
    assert_eq!(settlement.remaining, Duration::from_millis(4000));

    let mut expired = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3);
    expired.deadline = Some(Instant::now() - Duration::from_millis(1));
    assert!(expired.expired(), "expiry uses monotonic wall time");
}

#[test]
fn pending_bank_op_pause_freezes_while_stop_and_reconnect_abort() {
    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Noop), None).unwrap();
    slot.set_pending_bank_op(Some(PendingBankOp::new(
        PendingBankOpKind::Deposit,
        1,
        3,
        0,
        7,
    )));
    assert_eq!(
        slot.pending_bank_op().unwrap().remaining,
        Duration::from_millis(2000)
    );

    slot.pause();
    assert!(
        slot.pending_bank_op().unwrap().deadline.is_none(),
        "Pause freezes the ordinary bank deadline"
    );
    slot.resume();
    assert!(slot.pending_bank_op().unwrap().deadline.is_some());

    slot.stop();
    assert!(slot.pending_bank_op().is_none(), "Stop drops old-slot work");

    slot.start_test_script(Box::new(Noop), None).unwrap();
    slot.set_pending_bank_op(Some(PendingBankOp::new(
        PendingBankOpKind::Withdraw,
        1,
        20,
        0,
        7,
    )));
    assert_eq!(
        slot.pending_bank_op().unwrap().remaining,
        Duration::from_millis(4000)
    );
    let before = slot.bank_op_result();
    slot.reset_session_work();
    assert!(slot.pending_bank_op().is_none());
    assert_eq!(slot.bank_op_result(), (before.0.wrapping_add(1), false));
}

#[test]
fn fenced_settings_reject_stale_identity_generation_and_unchanged_bag() {
    let (mut slot, _, _) = ready_stub_slot();
    slot.attach_source_identity("catalog:ChickenKiller");
    let gen = slot.runtime_generation();
    let mut bag = serde_json::Map::new();
    bag.insert("x".into(), serde_json::json!(1));
    assert!(slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen));
    assert!(
        !slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen),
        "unchanged bag is not reposted"
    );
    assert!(!slot.post_settings_bag_fenced(&bag, "catalog:Other", gen));
    assert!(!slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen.wrapping_add(1)));
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    assert!(slot.source_identity().is_none());
    assert_ne!(slot.runtime_generation(), gen);
    assert!(!slot.post_settings_bag_fenced(&bag, "catalog:ChickenKiller", gen));
}

#[test]
fn settings_update_reaches_start_queued_behind_reap() {
    let (mut slot, factory, first_run) = ready_stub_slot();
    first_run.defer_stop_receipt();
    slot.stop();
    slot.start_surface(stub_surface_start())
        .expect("Start while Stopping is queued");
    slot.attach_source_identity("catalog:Example");
    let generation = slot.runtime_generation();
    let bag = serde_json::Map::from_iter([("amount".to_string(), serde_json::json!(9))]);
    assert!(slot.post_settings_bag_fenced(&bag, "catalog:Example", generation));
    assert_eq!(
        slot.surface_start
            .as_ref()
            .and_then(|identity| identity.settings_bag.as_deref()),
        Some(&bag)
    );

    assert!(first_run.finish_stop());
    wait_state(&mut slot, RunState::Starting);
    let queued_run = factory.last_run().expect("queued stub surface run");
    queued_run.set_readiness(SurfaceReady::Ready);
    wait_state(&mut slot, RunState::Running);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn stop_clears_identity_and_bumps_runtime_generation() {
    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Noop), None).unwrap();
    slot.attach_source_identity("file:shared.ts");
    let gen = slot.runtime_generation();
    slot.stop();
    assert!(slot.source_identity().is_none());
    assert_eq!(slot.state(), RunState::Idle);
    assert_ne!(
        slot.runtime_generation(),
        gen,
        "Stop must invalidate the previous execution generation"
    );
}

#[test]
fn slot_stop_drains_surface_logs_once() {
    let (mut slot, _, run) = ready_stub_slot();
    run.set_retirement_logs(["stopped-ok".into()]);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    let logs = slot.take_pending_logs();
    let hits = logs
        .iter()
        .filter(|line| line.contains("stopped-ok"))
        .count();
    assert_eq!(hits, 1, "exactly one retirement log after take: {logs:?}");
    slot.stop();
    let again = slot.take_pending_logs();
    assert!(
        again.iter().all(|line| !line.contains("stopped-ok")),
        "second stop must not redeliver retirement logs: {again:?}"
    );
}

#[test]
fn surface_self_stop_receipt_runs_cleanup_once() {
    let (mut slot, _, run) = ready_stub_slot();
    run.set_terminal(
        true,
        Some(crate::surface::ScriptStopReceipt {
            tick: 1,
            reason: "done".into(),
        }),
    );
    run.queue_logs(["script requested stop".into()]);
    run.set_retirement_logs(["stopped-ok".into()]);
    let _ = slot.drain_logs();
    wait_state(&mut slot, RunState::Idle);
    let logs = slot.take_pending_logs();
    let hits = logs
        .iter()
        .filter(|line| line.contains("stopped-ok"))
        .count();
    assert_eq!(hits, 1, "self-stop retirement log once: {logs:?}");
    assert!(logs
        .iter()
        .any(|line| line.contains("script requested stop")));
    slot.stop();
    let again = slot.take_pending_logs();
    assert!(
        again.iter().all(|line| !line.contains("stopped-ok")),
        "join after self-stop then Drop must not redeliver retirement logs: {again:?}"
    );
}

#[test]
fn watchdog_restart_folds_surface_retirement_logs_into_pending() {
    let (mut slot, factory, run) = ready_stub_slot();
    run.set_retirement_logs(["stopped-ok".into()]);
    run.defer_stop_receipt();
    slot.restart_from_identity(Instant::now())
        .expect("restart from retained surface identity");
    wait_state(&mut slot, RunState::Stopping);
    assert!(run.finish_stop());
    wait_state(&mut slot, RunState::Starting);
    let replacement = factory.last_run().expect("replacement stub surface");
    replacement.set_readiness(SurfaceReady::Ready);
    wait_state(&mut slot, RunState::Running);
    let logs = slot.take_pending_logs();
    assert!(
        logs.iter().any(|line| line.contains("stopped-ok")),
        "retired surface logs land in pending_logs: {logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn restart_still_refuses_pause() {
    let (mut slot, _, _) = ready_stub_slot();
    slot.pause();
    let err = slot.restart_from_identity(Instant::now()).unwrap_err();
    assert!(
        err.contains("not running") || err.contains("pause") || err.contains("frozen"),
        "{err}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn surface_start_waits_for_stub_readiness() {
    let t0 = Instant::now();
    let (mut slot, _, run) = stub_slot();
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "Start must not wait on setup: {:?}",
        t0.elapsed()
    );
    assert_eq!(slot.state(), RunState::Starting);
    run.set_readiness(SurfaceReady::Ready);
    wait_state(&mut slot, RunState::Running);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn stub_surface_setup_failure_clears_slot_start_state() {
    let (mut slot, _, run) = stub_slot();
    slot.attach_source_identity("file:bad.ts");
    assert_eq!(
        slot.poll_start(),
        StartPoll::Pending,
        "setup has not settled"
    );
    run.set_readiness(SurfaceReady::Failed("zz-setup-proof".into()));
    wait_state(&mut slot, RunState::Idle);
    let err = slot.last_error().unwrap_or("").to_string();
    assert!(err.contains("zz-setup-proof"), "{err}");
    assert_eq!(
        slot.poll_start(),
        StartPoll::Settled(StartOutcome::Failed(err))
    );
    assert_eq!(slot.source_identity(), None);
    assert!(slot.surface_start.is_none());
    assert_eq!(
        slot.runtime_generation(),
        0,
        "a Start that never reached Ready is not a Start"
    );
}

#[test]
fn stop_during_starting_ends_idle_and_cancels_the_start() {
    let (mut slot, _, _) = stub_slot();
    assert_eq!(slot.state(), RunState::Starting);
    slot.stop();
    assert_eq!(slot.state(), RunState::Stopping);
    wait_state(&mut slot, RunState::Idle);
    assert_eq!(
        slot.poll_start(),
        StartPoll::Settled(StartOutcome::Cancelled)
    );
    assert_eq!(slot.last_error(), None);
}

#[test]
fn watchdog_restart_during_stop_reap_never_revives_the_slot() {
    let (mut slot, _, run) = ready_stub_slot();
    run.defer_stop_receipt();
    slot.stop();
    assert_eq!(slot.state(), RunState::Stopping);
    assert!(slot.restart_from_identity(Instant::now()).is_err());
    assert!(run.finish_stop());
    wait_state(&mut slot, RunState::Idle);
    for _ in 0..20 {
        slot.observe_lifecycle();
    }
    assert_eq!(slot.state(), RunState::Idle);
    assert!(!slot.surface_active());
}

#[test]
fn stub_stop_returns_and_queued_start_waits_for_reap() {
    let (mut slot, factory, first_run) = ready_stub_slot();
    first_run.set_retirement_logs(["stopped-ok".into()]);
    first_run.defer_stop_receipt();
    let t0 = Instant::now();
    slot.stop();
    assert!(
        t0.elapsed() < Duration::from_millis(200),
        "Stop blocked: {:?}",
        t0.elapsed()
    );
    assert_eq!(slot.state(), RunState::Stopping);
    slot.start_surface(stub_surface_start())
        .expect("Start while Stopping is queued");
    let generation = slot.runtime_generation();
    assert!(
        slot.start_surface(stub_surface_start()).is_err(),
        "a second queued Start is refused"
    );
    assert!(first_run.finish_stop());
    wait_state(&mut slot, RunState::Starting);
    let replacement = factory.last_run().expect("queued replacement surface");
    replacement.set_readiness(SurfaceReady::Ready);
    wait_state(&mut slot, RunState::Running);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    assert_eq!(
        slot.runtime_generation(),
        generation,
        "the generation read after Start is the one that runs"
    );
    let logs = slot.take_pending_logs();
    assert!(
        logs.iter().any(|line| line.contains("stopped-ok")),
        "retired surface logs arrive before the queued Start: {logs:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn stub_surface_start_all_over_n_members_does_not_block() {
    let t0 = Instant::now();
    let mut slots: Vec<_> = (0..4).map(|_| stub_slot()).collect();
    for (slot, _, _) in &mut slots {
        assert_eq!(slot.state(), RunState::Starting);
    }
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "Start all blocked per member: {:?}",
        t0.elapsed()
    );
    for (slot, _, run) in &mut slots {
        run.set_readiness(SurfaceReady::Ready);
        wait_state(slot, RunState::Running);
        slot.stop();
        wait_state(slot, RunState::Idle);
    }
}

/// A compiled card that queues one walk per tick onto the ctx's sink —
/// the queue the slot parks there — and nothing else.
#[derive(Default)]
struct Walker;

impl Script for Walker {
    fn tick(
        &mut self,
        ctx: &mut crate::native::NativeTick<'_>,
    ) -> Result<crate::native::ScriptFlow, crate::native::ScriptFailure> {
        if let Some(sink) = ctx.frame.compiled.interacts.as_mut() {
            sink.push(crate::shim::InteractReq::Walk {
                x: 3,
                z: 4,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
                avoid: Vec::new(),
                cross: Vec::new(),
            });
        }
        Ok(crate::native::ScriptFlow::Continue)
    }
}

/// The walk `Walker` queues, for comparing whole requests.
fn walker_walk() -> crate::shim::InteractReq {
    crate::shim::InteractReq::Walk {
        x: 3,
        z: 4,
        level: 0,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id: 0,
        avoid: Vec::new(),
        cross: Vec::new(),
    }
}

/// A ctx with no views wired and nothing parked: a compiled tick over it
/// is the slot's own queue, installed by the slot.
fn compiled_ctx<'a>(
    driver: &'a mut dyn api::interact::Driver,
    selected: Option<&'a api::game_data::SelectedGameData>,
) -> ScriptCtx<'a> {
    ScriptCtx {
        driver,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: crate::CompiledTick {
            condition: None,
            poison_unknown_pulse: None,
            cure_poison: crate::survival::CurePoison::WhenNeeded,
            walk_danger: None,
            selected,
            reach: None,
            reach_flood: None,
            bank_memory: None,
            collision: None,
            hold: false,
            world_members: api::selected::Truth::Unknown,
            interacts: None,
            service_gate: None,
        },
    }
}

#[test]
fn compiled_panic_moves_from_active_error_to_preserved_stop_history() {
    struct Panics;
    impl Script for Panics {
        fn tick(
            &mut self,
            _ctx: &mut crate::native::NativeTick<'_>,
        ) -> Result<crate::native::ScriptFlow, crate::native::ScriptFailure> {
            panic!("compiled boom")
        }
    }

    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Panics), None).unwrap();
    let mut driver = NullDriver::default();
    slot.on_game_tick(&mut compiled_ctx(&mut driver, None));
    assert_eq!(slot.state(), RunState::Error);
    assert!(slot
        .last_error()
        .is_some_and(|error| error.contains("compiled boom")));
    slot.stop();
    assert_eq!(slot.last_error(), None, "explicit Stop clears active error");
    assert!(
        slot.take_pending_logs()
            .iter()
            .any(|line| line.contains("compiled boom")),
        "the panic remains in diagnostic history"
    );
}

#[test]
fn a_compiled_tick_queues_onto_the_slot_and_one_drain_takes_it() {
    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Walker), None).unwrap();
    let mut d = NullDriver::default();
    slot.on_game_tick(&mut compiled_ctx(&mut d, None));
    assert_eq!(
        slot.drain_interacts(),
        vec![walker_walk()],
        "the compiled card's verbs ride the slot's own drain"
    );
    assert!(
        slot.drain_interacts().is_empty(),
        "the drain takes the queue, it never replays it"
    );

    // A paused card is not ticked, so it queues nothing.
    slot.pause();
    slot.on_game_tick(&mut compiled_ctx(&mut d, None));
    assert!(slot.drain_interacts().is_empty());

    // Stop drops the instance and the queue it had not spent.
    slot.resume();
    slot.on_game_tick(&mut compiled_ctx(&mut d, None));
    slot.stop();
    assert!(
        slot.drain_interacts().is_empty(),
        "Stop must not leak a dead card's requests into the next Start"
    );
}

/// MAZE-RANDOM-1: a native run teleported onto the Maze square stayed
/// `Working` there. Held, the guardian owns the square and the card keeps
/// its frame; unheld (the guardian gave the Maze up, or random events are
/// off), the run blocks with the reason and the card is not ticked.
#[test]
fn an_unheld_native_run_on_a_random_trap_square_blocks() {
    let maze = (2891, 4555, 0);
    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Walker), None).unwrap();
    let mut d = NullDriver::default();

    let mut held = compiled_ctx(&mut d, None);
    held.here = Some(maze);
    held.compiled.hold = true;
    slot.on_game_tick(&mut held);
    assert_eq!(slot.drain_interacts(), vec![walker_walk()]);
    assert_ne!(
        slot.native_status().map(|status| status.phase),
        Some(crate::native::NativePhase::Blocked),
        "a held frame leaves the Maze to the guardian"
    );

    let mut unheld = compiled_ctx(&mut d, None);
    unheld.here = Some(maze);
    slot.on_game_tick(&mut unheld);
    assert!(
        slot.drain_interacts().is_empty(),
        "the card is not ticked inside the Maze"
    );
    let status = slot.native_status().expect("blocked status");
    assert_eq!(status.phase, crate::native::NativePhase::Blocked);
    let failure = status.failure.as_ref().expect("blocked failure");
    assert_eq!(&*failure.code, "random-trapped");
    assert!(
        failure.message.contains("Maze") && failure.message.contains("(2891, 4555, 0)"),
        "{}",
        failure.message
    );
    assert_eq!(
        slot.state(),
        RunState::Idle,
        "the trapped run takes the Stop"
    );
    assert!(!slot.has_instance());
    assert_eq!(
        slot.lifecycle_receipt().map(|receipt| receipt.state),
        Some(ScriptTerminalState::Failed)
    );

    // The Mime stage is the other trap square.
    let mut slot = SlotScript::new();
    slot.start_test_script(Box::new(Walker), None).unwrap();
    let mut mime = compiled_ctx(&mut d, None);
    mime.here = Some((31 * 64 + 10, 74 * 64 + 10, 0));
    slot.on_game_tick(&mut mime);
    let status = slot.native_status().expect("blocked status");
    assert!(status.failure.as_ref().is_some_and(
        |failure| &*failure.code == "random-trapped" && failure.message.contains("Mime")
    ));
}

fn is_test_walk(req: &crate::interact::InteractReq) -> bool {
    matches!(
        req,
        crate::interact::InteractReq::WalkNear {
            x: 2820,
            z: 3556,
            ..
        }
    )
}

fn queued_stub_walk_batch(generation: u64) -> crate::native::seat_rows::QueuedSeatBatch {
    crate::native::seat_rows::QueuedSeatBatch {
        generation,
        observed_walk_outcome_seq: 0,
        batch: crate::native::seat_rows::DecodedSeatBatch {
            consumed_terminal_seq: 0,
            starts: Vec::new(),
            decisions: Vec::new(),
            cancels: Vec::new(),
            mirror_revokes: Vec::new(),
            reqs: vec![crate::native::seat_rows::SeatInteractDecoded {
                lineage: None,
                mirror_owner: None,
                request: crate::interact::InteractReq::WalkNear {
                    x: 2820,
                    z: 3556,
                    level: 0,
                    radius: 1,
                    compat_tile_intent: false,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    request_id: 0,
                    avoid: Vec::new(),
                    cross: Vec::new(),
                },
            }],
        },
        machine_events: Vec::new(),
        held_waits: Default::default(),
        held_fold: false,
    }
}

fn align_ready_test_seat(slot: &mut SlotScript) {
    let ready = slot
        .seat
        .as_ref()
        .expect("surface seat")
        .run()
        .session_key();
    assert_eq!(
        ready.map(api::identity::SessionKey::epoch),
        Some(slot.work_epoch()),
        "the test's ready session matches its new connection"
    );
    slot.align_seat_session(ready);
}

fn run_stub_walk_tick(slot: &mut SlotScript, n: u64) -> (i32, i32, i32) {
    align_ready_test_seat(slot);
    let here = (2823, 3555, 0);
    let mut input = empty_snapshot(n);
    input.here = Some(TileInput {
        x: here.0,
        z: here.1,
        level: here.2,
    });
    assert!(slot.post_snapshot_delta(&input, NativeFactsInput::default(), false));
    let mut driver = NullDriver::default();
    let mut ctx = compiled_ctx(&mut driver, None);
    ctx.tick = n;
    ctx.here = Some(here);
    slot.on_game_tick(&mut ctx);
    here
}

fn held_stub_walk_tick(slot: &mut SlotScript, run: &StubSurfaceHandle, n: u64, now: Instant) {
    let here = run_stub_walk_tick(slot, n);
    let (_, game_rows, _) = slot.drain_host_interacts();
    assert!(
        game_rows.iter().all(|row| !matches!(
            &row.req,
            crate::interact::InteractReq::WaitEnqueued | crate::interact::InteractReq::WaitSettled
        )),
        "seat wait facts stay on the watchdog lane: {game_rows:?}"
    );
    run.queue_lifecycle([
        crate::interact::InteractReq::WaitEnqueued,
        crate::interact::InteractReq::LoopSettled,
    ]);
    let facts = slot.drain_lifecycle();
    slot.note_game_packet_tick(now);
    slot.feed_watchdog(now, Some(here), &[], false, true, &facts);
    assert!(
        slot.watchdog().wait_active(),
        "held walk tick {n} has no parked wait; lifecycle={facts:?}, logs={:?}",
        slot.drain_logs()
    );
}

#[test]
fn stub_walk_held_across_reconnect_remains_a_parked_wait() {
    let (mut slot, _, run) = ready_stub_slot_with_incarnation(1204);
    run.set_execution_active(true);
    let now = Instant::now();
    held_stub_walk_tick(&mut slot, &run, 1, now);
    assert!(slot.watchdog().wait_active(), "the walk is parked");

    assert!(slot.reconnect_session_work(), "the surface work is carried");
    slot.on_is_up(true);
    let reconnected_at = Instant::now();
    held_stub_walk_tick(&mut slot, &run, 2, reconnected_at);
    assert!(
        slot.watchdog().wait_active(),
        "the held walk is still the parked wait"
    );
    slot.note_game_packet_tick(reconnected_at + Duration::from_secs(5));
    assert_eq!(
        slot.feed_watchdog(
            reconnected_at + crate::watchdog::SCHEDULER_WARN + Duration::from_secs(1),
            Some((2823, 3555, 0)),
            &[],
            false,
            true,
            &[],
        ),
        crate::watchdog::WatchdogAction::None,
        "no hung-loop warning while the held walk runs"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn stub_walk_in_slot_custody_at_reconnect_requests_fresh_walk() {
    let (mut slot, _, run) = ready_stub_slot_with_incarnation(1207);
    run.queue_seat_batches([queued_stub_walk_batch(0)]);
    let batches = slot.surface.as_ref().unwrap().drain_seat_batches();
    assert!(
        batches.iter().any(|queued| queued
            .batch
            .reqs
            .iter()
            .any(|row| is_test_walk(&row.request))),
        "the producer emits the walk: {batches:?}"
    );
    for queued in batches {
        slot.consume_seat_batch(
            queued.batch,
            queued.observed_walk_outcome_seq,
            &queued.machine_events,
        );
    }
    let in_custody = slot
        .seat_interact_delivery
        .iter()
        .any(|queued| is_test_walk(&queued.req))
        || slot
            .seat
            .as_ref()
            .is_some_and(|seat| seat.undelivered_walk_ids().next().is_some());
    assert!(
        in_custody,
        "the walk is in slot custody, not in the surface queue"
    );

    assert!(slot.reconnect_session_work());
    slot.on_is_up(true);
    let (_, discarded, _) = slot.drain_host_interacts();
    assert!(
        !discarded.iter().any(|row| is_test_walk(&row.req)),
        "the old-session walk is discarded: {discarded:?}"
    );

    run.set_generation(1);
    align_ready_test_seat(&mut slot);
    run.queue_seat_batches([queued_stub_walk_batch(1)]);
    let (_, fresh, _) = slot.drain_host_interacts();
    let fresh: Vec<_> = fresh
        .into_iter()
        .filter(|row| is_test_walk(&row.req))
        .collect();
    assert_eq!(
        fresh.len(),
        1,
        "the resumed wait requests exactly one fresh walk: {fresh:?}"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn stop_ends_stub_surface_after_reconnect() {
    let (mut slot, _, run) = ready_stub_slot_with_incarnation(1205);
    run.set_execution_active(true);
    assert!(slot.reconnect_session_work());
    slot.on_is_up(true);
    assert!(slot.reconnect_session_work());
    assert_eq!(slot.state(), RunState::Paused);
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
    assert!(slot.surface.is_none(), "the held surface run is gone");
}
#[test]
fn ignored_maze_trap_cleans_up_core_api_work() {
    use api::random::{DetectedRandom, RandomClaim, RandomKind};

    let selected =
        api::game_data::for_revision(api::selected::ClientRevision::R289).expect("selected data");
    let (mut slot, _, run) = ready_stub_slot_with_incarnation(1206);
    slot.surface_start
        .as_mut()
        .expect("surface identity")
        .game_data = Some(Arc::clone(&selected));
    run.set_ignored_randoms(vec!["Maze".into(), "Mime".into()]);
    for (kind, name) in [(RandomKind::Maze, "Maze"), (RandomKind::Mime, "Mime")] {
        assert_eq!(
            slot.on_random(&DetectedRandom {
                kind,
                name: name.into(),
                ours: true,
                npc_index: None,
            }),
            RandomClaim::Handle,
            "ignored {name} random"
        );
    }

    let maze = (2891, 4555, 0);
    let mut held = empty_snapshot(20);
    held.hold = true;
    assert!(slot.post_snapshot_delta(&held, NativeFactsInput::default(), false));
    native_tick_at(&mut slot, &selected, 20, Some(maze), true);
    assert_eq!(slot.state(), RunState::Running);
    assert!(
        slot.lifecycle_receipt().is_none(),
        "guardian-held Maze remains frozen, not terminal"
    );

    slot.pause();
    native_tick_at(&mut slot, &selected, 21, Some(maze), false);
    assert_eq!(slot.state(), RunState::Paused);
    assert!(
        slot.lifecycle_receipt().is_none(),
        "paused scripts are not trap-stopped"
    );
    slot.resume();

    start_api_gather_and_wait(&mut slot, &selected);
    native_tick_at(&mut slot, &selected, 22, Some(maze), false);
    assert!(
        !slot.api_owns_foreground(),
        "terminal trap cleanup revokes delegated API work"
    );
    let receipt = slot.lifecycle_receipt().expect("terminal failure receipt");
    assert_eq!(receipt.state, ScriptTerminalState::Failed);
    assert!(receipt.reason.starts_with("random-trapped:"));
    assert!(
        receipt.reason.contains("Maze"),
        "receipt retains the trapped event reason: {}",
        receipt.reason
    );
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn ignored_mime_trap_returns_core_terminal_receipt() {
    use api::random::{DetectedRandom, RandomClaim, RandomKind};

    let selected =
        api::game_data::for_revision(api::selected::ClientRevision::R289).expect("selected data");
    let (mut slot, _, run) = ready_stub_slot();
    run.set_ignored_randoms(vec!["Mime".into()]);
    assert_eq!(
        slot.on_random(&DetectedRandom {
            kind: RandomKind::Mime,
            name: "Mime".into(),
            ours: true,
            npc_index: None,
        }),
        RandomClaim::Handle
    );

    let mime = (31 * 64 + 10, 74 * 64 + 10, 0);
    native_tick_at(&mut slot, &selected, 1, Some(mime), false);
    let receipt = slot.lifecycle_receipt().expect("terminal failure receipt");
    assert_eq!(receipt.state, ScriptTerminalState::Failed);
    assert!(receipt.reason.starts_with("random-trapped:"));
    assert!(
        receipt.reason.contains("Mime"),
        "receipt retains the trapped event reason: {}",
        receipt.reason
    );
    wait_state(&mut slot, RunState::Idle);
}
#[test]
fn surface_watchdog_holds_pen_recovery_until_the_player_leaves() {
    let (mut slot, _, _) = ready_stub_slot();

    let now = Instant::now();
    let inside_pen = (3340, 3250, 0);
    let settled = [crate::shim::InteractReq::LoopSettled];
    slot.note_game_packet_tick(now);
    slot.feed_watchdog(now, Some(inside_pen), &[], false, true, &settled);
    slot.note_game_packet_tick(now + crate::watchdog::WEDGE);
    assert_eq!(
        slot.feed_watchdog(
            now + crate::watchdog::WEDGE,
            Some(inside_pen),
            &[],
            false,
            true,
            &settled,
        ),
        crate::watchdog::WatchdogAction::RequestAnchor
    );

    let anchor = [
        crate::shim::InteractReq::RecoveryAnchor {
            x: 3368,
            z: 3274,
            level: 0,
        },
        crate::shim::InteractReq::LoopSettled,
    ];
    slot.note_game_packet_tick(now + crate::watchdog::WEDGE + Duration::from_secs(1));
    let action = slot.feed_watchdog(
        now + crate::watchdog::WEDGE + Duration::from_secs(1),
        Some(inside_pen),
        &[],
        false,
        true,
        &anchor,
    );
    assert_eq!(
        format!("{action:?}"),
        "HoldInFightPen",
        "a surface script must not walk toward an unreachable lobby anchor"
    );
    assert!(
        !slot.watchdog().holds_script_actions(),
        "the active surface run keeps running while recovery is held"
    );
    let logs = slot.take_pending_logs();
    assert_eq!(logs.len(), 1, "entering the hold is logged once: {logs:?}");
    assert_eq!(
        logs[0],
        "watchdog: holding recovery in a Duel Arena fight pen; the recovery anchor is outside the pen"
    );

    slot.note_game_packet_tick(now + crate::watchdog::WEDGE + Duration::from_secs(20));
    assert_eq!(
        slot.feed_watchdog(
            now + crate::watchdog::WEDGE + Duration::from_secs(20),
            Some(inside_pen),
            &[],
            false,
            true,
            &settled,
        ),
        crate::watchdog::WatchdogAction::None,
        "the pen hold must not repeatedly request the same recovery"
    );
    assert!(slot.take_pending_logs().is_empty());

    let outside_pen = (3368, 3274, 0);
    slot.note_game_packet_tick(now + crate::watchdog::WEDGE + Duration::from_secs(30));
    assert_eq!(
        slot.feed_watchdog(
            now + crate::watchdog::WEDGE + Duration::from_secs(30),
            Some(outside_pen),
            &[],
            false,
            true,
            &settled,
        ),
        crate::watchdog::WatchdogAction::None
    );
    slot.note_game_packet_tick(now + crate::watchdog::WEDGE * 2 + Duration::from_secs(30));
    assert_eq!(
        slot.feed_watchdog(
            now + crate::watchdog::WEDGE * 2 + Duration::from_secs(30),
            Some(outside_pen),
            &[],
            false,
            true,
            &settled,
        ),
        crate::watchdog::WatchdogAction::RequestAnchor,
        "ordinary recovery resumes after leaving the pen"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn surface_watchdog_restarts_hard_stall_while_pen_hold_survives_freeze_and_reset() {
    let (mut slot, _, _) = ready_stub_slot();

    let now = Instant::now();
    let inside_pen = (3340, 3250, 0);
    let settled = [crate::shim::InteractReq::LoopSettled];
    slot.note_game_packet_tick(now);
    slot.feed_watchdog(now, Some(inside_pen), &[], false, true, &settled);
    for seconds in (5..=crate::watchdog::WEDGE.as_secs()).step_by(5) {
        let tick_at = now + Duration::from_secs(seconds);
        slot.note_game_packet_tick(tick_at);
        let action = slot.feed_watchdog(tick_at, Some(inside_pen), &[], false, true, &[]);
        if seconds == crate::watchdog::WEDGE.as_secs() {
            assert_eq!(action, crate::watchdog::WatchdogAction::RequestAnchor);
        }
    }

    let anchor = crate::watchdog::Tile {
        x: 3368,
        z: 3274,
        level: 0,
    };
    let anchor_reply = [crate::shim::InteractReq::RecoveryAnchor {
        x: anchor.x,
        z: anchor.z,
        level: anchor.level,
    }];
    let held_at = now + crate::watchdog::WEDGE + Duration::from_secs(1);
    slot.note_game_packet_tick(held_at);
    assert_eq!(
        slot.feed_watchdog(held_at, Some(inside_pen), &[], false, true, &anchor_reply),
        crate::watchdog::WatchdogAction::HoldInFightPen
    );
    assert_eq!(slot.watchdog().held_fight_pen_anchor(), Some(anchor));
    assert!(slot.watchdog().last_recovery().is_none());

    let frozen_at = held_at + Duration::from_secs(1);
    slot.note_game_packet_tick(frozen_at);
    assert_eq!(
        slot.feed_watchdog(frozen_at, Some(inside_pen), &[], true, true, &[]),
        crate::watchdog::WatchdogAction::None
    );
    assert_eq!(slot.watchdog().held_fight_pen_anchor(), Some(anchor));

    let thawed_at = held_at + Duration::from_secs(2);
    slot.note_game_packet_tick(thawed_at);
    assert_eq!(
        slot.feed_watchdog(thawed_at, Some(inside_pen), &[], false, true, &[]),
        crate::watchdog::WatchdogAction::None
    );
    assert_eq!(slot.watchdog().held_fight_pen_anchor(), Some(anchor));
    assert!(slot.watchdog().last_recovery().is_none());

    let reset_at = held_at + Duration::from_secs(3);
    assert_eq!(
        slot.watchdog.on_session_reset(reset_at),
        crate::watchdog::WatchdogAction::None
    );
    assert_eq!(slot.watchdog().held_fight_pen_anchor(), Some(anchor));
    assert!(slot.watchdog().last_recovery().is_none());

    for seconds in (5..crate::watchdog::HARD_STALL.as_secs()).step_by(5) {
        let tick_at = reset_at + Duration::from_secs(seconds);
        slot.note_game_packet_tick(tick_at);
        slot.feed_watchdog(tick_at, Some(inside_pen), &[], false, true, &[]);
    }
    let stalled_at = reset_at + crate::watchdog::HARD_STALL;
    slot.note_game_packet_tick(stalled_at);
    assert_eq!(
        slot.feed_watchdog(stalled_at, Some(inside_pen), &[], false, true, &[]),
        crate::watchdog::WatchdogAction::Restart {
            reason: crate::watchdog::RestartReason::Stall,
        },
        "a held fight-pen recovery does not disable the hard-stall restart"
    );
    assert!(slot.watchdog().last_recovery().is_none());
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn surface_watchdog_still_restarts_a_hard_stall_outside_fight_pens() {
    let (mut slot, _, _) = ready_stub_slot();

    let now = Instant::now();
    let here = (3200, 3274, 0);
    slot.note_game_packet_tick(now);
    assert_eq!(
        slot.feed_watchdog(now, Some(here), &[], false, true, &[]),
        crate::watchdog::WatchdogAction::None
    );
    let outside_anchor = [crate::shim::InteractReq::RecoveryAnchor {
        x: 3220,
        z: 3274,
        level: 0,
    }];
    for seconds in (5..crate::watchdog::HARD_STALL.as_secs()).step_by(5) {
        let tick_at = now + Duration::from_secs(seconds);
        slot.note_game_packet_tick(tick_at);
        let lifecycle: &[crate::shim::InteractReq] =
            if seconds == crate::watchdog::HARD_STALL.as_secs() - 5 {
                &outside_anchor
            } else {
                &[]
            };
        let action = slot.feed_watchdog(tick_at, Some(here), &[], false, true, lifecycle);
        if seconds == crate::watchdog::SCHEDULER_WARN.as_secs() {
            assert_eq!(action, crate::watchdog::WatchdogAction::WarnHungLoop);
        } else if seconds == crate::watchdog::WEDGE.as_secs() {
            assert_eq!(action, crate::watchdog::WatchdogAction::RequestAnchor);
        } else if seconds == crate::watchdog::HARD_STALL.as_secs() - 5 {
            assert_eq!(
                action,
                crate::watchdog::WatchdogAction::ArmWalk {
                    x: 3220,
                    z: 3274,
                    level: 0,
                },
                "outside-pen anchor replies must reach normal recovery, not be held"
            );
        } else {
            assert_eq!(action, crate::watchdog::WatchdogAction::None);
        }
    }
    let stalled_at = now + crate::watchdog::HARD_STALL;
    slot.note_game_packet_tick(stalled_at);
    assert_eq!(
        slot.feed_watchdog(stalled_at, Some(here), &[], false, true, &[]),
        crate::watchdog::WatchdogAction::Restart {
            reason: crate::watchdog::RestartReason::Stall,
        },
        "the fight-pen hold must not disable genuine scheduler-stall recovery"
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn surface_watchdog_logs_packet_silence_and_clears_it_on_resume() {
    let (mut slot, _, _) = ready_stub_slot();

    let now = Instant::now();
    let here = (3200, 3274, 0);
    let settled = [crate::shim::InteractReq::LoopSettled];
    slot.note_game_packet_tick(now);
    slot.feed_watchdog(now, Some(here), &[], false, true, &settled);

    let quiet_at = now + crate::watchdog::SCHEDULER_WARN;
    assert_eq!(
        format!(
            "{:?}",
            slot.feed_watchdog(quiet_at, Some(here), &[], false, true, &settled)
        ),
        "WarnNoGamePackets"
    );
    assert_eq!(
        slot.take_pending_logs(),
        ["watchdog: no game packets for 10 s"]
    );

    let resumed_at = quiet_at + Duration::from_secs(1);
    slot.note_game_packet_tick(resumed_at);
    assert_eq!(
        slot.feed_watchdog(resumed_at, Some(here), &[], false, true, &settled),
        crate::watchdog::WatchdogAction::None
    );
    slot.stop();
    wait_state(&mut slot, RunState::Idle);
}

#[test]
fn native_only_slot_metrics_count_late_controls_without_logging() {
    let mut slot = SlotScript::new();
    assert_eq!(slot.slot_metrics(), SlotMetrics::default());
    slot.bind_incarnation(1);
    let run = api::selected::RunKey {
        slot: 11,
        run: 22,
        session: 33,
    };
    slot.arm_seat(run);
    let id = RequestId::isolate(1).unwrap();
    let seat = slot.seat.as_mut().unwrap();
    assert_eq!(
        seat.start(
            RequestStamp::new(id, None, None, ExecutionId::first(run)).unwrap(),
            crate::native::request::NativeKind::BankNearest,
            false,
        )
        .unwrap(),
        Admission::Admitted
    );
    seat.complete(id, false).unwrap();
    let result_seq = seat.results()[0].seq;
    seat.watermark(result_seq);
    for _ in 0..50 {
        seat.cancel(
            id,
            api::script_contract::CallbackCancelReason::ParentAborted,
        )
        .unwrap();
    }
    assert_eq!(
        slot.slot_metrics(),
        SlotMetrics {
            dropped: 0,
            late_controls: 50,
        }
    );
    assert!(slot.take_pending_logs().is_empty());
}

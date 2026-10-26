#![cfg(feature = "test-hooks")]

use script::{
    native::recovery_hints::{HintTile, RecoveryHintsCell},
    native::snapshot_source::{
        BankSelectionInput, NativeFactsInput, ReachViewInput, SnapshotInput,
    },
    surface::{
        test_support::StubSurfaceFactory, PostedSnapshotFacts, RouteInspectIdentities, SurfaceCard,
        SurfaceFamily, SurfaceKind, SurfaceReady, SurfaceStart,
    },
    RunState, SlotScript, StartOutcome, StartPoll,
};
use std::{any::Any, sync::Arc, time::Instant};

#[derive(Debug)]
struct TestCard;

impl SurfaceCard for TestCard {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Js
    }

    fn family(&self) -> Option<SurfaceFamily> {
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn start() -> SurfaceStart {
    SurfaceStart {
        card: Arc::new(TestCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::new(api::named_banks::NamedBankFacts::default()),
        loadouts: Arc::from([]),
    }
}

fn start_slot(slot: &mut SlotScript, factory: &Arc<StubSurfaceFactory>) {
    let surface_factory: Arc<dyn script::surface::SurfaceFactory> = factory.clone();
    slot.bind_surface_factory(surface_factory);
    slot.start_surface(start()).expect("stub surface start");
    let run = factory.last_run().expect("factory spawned a run");
    run.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    assert_eq!(slot.state(), RunState::Running);
    assert!(slot.surface_active());
}

fn empty_snapshot(tick: u64) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv: &[],
        inv_size: 0,
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
        scene_state: 2,
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
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn populated_facts() -> PostedSnapshotFacts {
    PostedSnapshotFacts {
        withdraw_x_result_seq: 11,
        bank_op_result_seq: 12,
        walk_outcome_seq: 13,
        bank_selection: BankSelectionInput {
            request_id: 14,
            generation: 15,
            bank_index: 2,
            kind: 3,
        },
        route_inspect: RouteInspectIdentities {
            latest: (21, 22, 23),
            prev: (31, 32, 33),
            replaced_id: 41,
            replaced_prev_id: 42,
            refused: [51, 52, 53],
        },
    }
}

#[test]
fn watchdog_restart_hints_are_isolated_between_slots_sharing_a_factory() {
    let factory = Arc::new(StubSurfaceFactory::new());
    let mut slot_a = SlotScript::new();
    let mut slot_b = SlotScript::new();
    start_slot(&mut slot_a, &factory);
    start_slot(&mut slot_b, &factory);

    let run_a = factory.run(0).expect("slot A run");
    let run_b = factory.run(1).expect("slot B run");
    let hints_a = run_a.recovery_hints();
    let hints_b = run_b.recovery_hints();
    assert!(!Arc::ptr_eq(&hints_a, &hints_b));
    assert!(!hints_a.pending());
    assert!(!hints_b.pending());

    slot_a
        .restart_from_identity(Instant::now())
        .expect("watchdog restart of slot A");
    assert!(hints_a.pending());
    assert!(!hints_b.pending());
    assert!(run_a.stop_called());
    assert!(!slot_a.surface_active());
    assert_eq!(slot_a.state(), RunState::Stopping);
    assert!(slot_b.surface_active());
    let _ = slot_a.poll_start();
    let restarted_a = factory.run(2).expect("slot A restarted run");
    assert_eq!(factory.spawn_attempts(), 3);
    restarted_a.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot_a.poll_start(), StartPoll::NotOwed);
    assert_eq!(slot_a.state(), RunState::Running);
    assert!(!hints_b.pending());
}

#[test]
fn same_slot_watchdog_restart_preserves_hint_cell_and_anchor() {
    let factory = Arc::new(StubSurfaceFactory::new());
    let mut slot = SlotScript::new();
    start_slot(&mut slot, &factory);
    let original = factory.run(0).expect("original run");
    let hints: Arc<RecoveryHintsCell> = original.recovery_hints();
    let anchor = HintTile {
        x: 3200,
        z: 3201,
        level: 2,
    };
    hints.set_anchor(Some(anchor));

    slot.restart_from_identity(Instant::now())
        .expect("watchdog restart");
    assert!(hints.pending());
    assert!(original.stop_called());

    // Stub stop receipts are synchronous. Lifecycle polling resolves the queued restart.
    let _ = slot.poll_start();
    let replacement = factory.run(1).expect("restart spawned a replacement run");
    assert_eq!(factory.spawn_attempts(), 2);
    let replacement_hints = replacement.recovery_hints();
    assert!(Arc::ptr_eq(&hints, &replacement_hints));
    assert_eq!(replacement_hints.anchor(), Some(anchor));
    assert!(replacement_hints.pending());
    replacement.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot.poll_start(), StartPoll::NotOwed);
    assert_eq!(slot.state(), RunState::Running);
    assert!(slot.surface_active());
}

#[test]
fn refused_typed_snapshot_keeps_the_active_run_and_clears_facts() {
    let factory = Arc::new(StubSurfaceFactory::new());
    let mut slot = SlotScript::new();
    start_slot(&mut slot, &factory);
    let run = factory.last_run().expect("active run");
    run.set_posted_facts(Some(populated_facts()));
    run.set_snapshot_acceptance(false, true);
    assert!(slot.posted_snapshot_facts().is_some());

    let accepted = slot.post_snapshot_delta(&empty_snapshot(1), NativeFactsInput::default(), false);
    assert!(!accepted);
    assert!(slot.surface_active());
    assert_eq!(slot.state(), RunState::Running);
    assert_eq!(factory.spawn_attempts(), 1);
    assert!(!run.stop_called());
    assert_eq!(slot.posted_snapshot_facts(), None);
}

#[test]
fn populated_posted_fact_reads_and_host_comparisons_allocate_nothing() {
    let factory = Arc::new(StubSurfaceFactory::new());
    let mut slot = SlotScript::new();
    start_slot(&mut slot, &factory);
    let run = factory.last_run().expect("active run");
    let expected = populated_facts();
    run.set_posted_facts(Some(expected));

    let compare_host_facts = |facts: Option<PostedSnapshotFacts>| {
        facts.is_some_and(|last| {
            let route = last.route_inspect;
            last.bank_selection
                == (BankSelectionInput {
                    request_id: 14,
                    generation: 15,
                    bank_index: 2,
                    kind: 3,
                })
                && last.withdraw_x_result_seq == 11
                && last.bank_op_result_seq == 12
                && last.walk_outcome_seq == 13
                && (route.latest.0, route.latest.1, route.latest.2) == (21, 22, 23)
                && (route.prev.0, route.prev.1, route.prev.2) == (31, 32, 33)
                && route.replaced_id == 41
                && route.replaced_prev_id == 42
                && route.refused == [51, 52, 53]
        })
    };

    let mut comparisons = (false, false, false);
    let info = allocation_counter::measure(|| {
        let first = slot.posted_snapshot_facts();
        let second = slot.posted_snapshot_facts();
        let third = slot.posted_snapshot_facts();
        comparisons = (
            compare_host_facts(first),
            compare_host_facts(second),
            compare_host_facts(third),
        );
    });
    assert_eq!(info.count_total, 0, "fact reads and comparisons allocate");
    assert_eq!(
        info.bytes_total, 0,
        "fact reads and comparisons allocate bytes"
    );
    assert_eq!(comparisons, (true, true, true));
    assert_eq!(run.posted_facts_reads(), 3);
}

#[test]
fn session_boundaries_clear_posted_facts_without_replacing_the_run() {
    use script::surface::test_support::StubSurfaceOperation;

    for reconnect in [false, true] {
        let factory = Arc::new(StubSurfaceFactory::new());
        let mut slot = SlotScript::new();
        start_slot(&mut slot, &factory);
        let run = factory.last_run().expect("active run");
        run.set_posted_facts(Some(populated_facts()));
        slot.store_last_world_id(Some(123));
        assert!(slot.posted_snapshot_facts().is_some());

        if reconnect {
            let _ = slot.reconnect_session_work();
        } else {
            slot.reset_session_work();
        }

        assert_eq!(slot.posted_snapshot_facts(), None);
        assert_eq!(slot.last_world_id(), None);
        assert!(slot.surface_active());
        assert_eq!(factory.spawn_attempts(), 1);
        assert!(!run.stop_called());
        let calls = run
            .operations()
            .into_iter()
            .filter(|operation| {
                if reconnect {
                    matches!(operation, StubSurfaceOperation::ReconnectSessionWork(_))
                } else {
                    matches!(operation, StubSurfaceOperation::ResetSessionWork(_))
                }
            })
            .count();
        assert_eq!(calls, 1, "one run receives one session boundary");
    }
}

#[test]
fn slot_size_stays_within_the_full_build_parent_layout() {
    let actual = std::mem::size_of::<SlotScript>();
    println!("S0d SlotScript layout: {actual} bytes");
    assert_eq!(
        actual, 1624,
        "the surface slot changed from its measured size"
    );
}

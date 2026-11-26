//! Conformance of the neutral adapter against the slot's seat page contract.
use api::identity::{ExecutionId, RequestId};
use api::script_contract::{
    AbortReason, CallbackCancelReason, DecisionSequence, Refusal, RequestStamp,
};
use api::selected::RunKey;
use script::api_combat::CombatEnd;
use script::api_gather::{GatherCounts, GatherEnd};
use script::api_progress::ProgressPage;
use script::interact::InteractReq;
use script::native::request::NativeKind;
use script::native::seat::{Admission, GameplaySeat, SeatDecisionStatus, SeatResult, SeatTerminal};
use script::native::seat_rows::{
    SeatPageInput, SeatStartArgs, MAX_SEAT_CANCELS, MAX_SEAT_GAMEPLAY_ROWS,
};
use script::native::snapshot_source::{NativeFactsInput, ReachViewInput, SnapshotInput};
use script::native::PromptPoint;
use script::surface::client::{Awaited, ClientError, SeatClient, SeatCompletion};
use seat_model::{run_trace, Bounds, Class, Outcome, Rules, SessionFamily, Step};

fn execution(run: u64, session: u64) -> ExecutionId {
    ExecutionId::first(RunKey {
        slot: 17,
        run,
        session,
    })
}

fn client() -> SeatClient {
    SeatClient::new(execution(23, 5))
}

fn result(id: RequestId, seq: u64, outcome: SeatTerminal) -> SeatResult {
    SeatResult {
        id,
        seq,
        outcome,
        prior_parent: None,
        session: None,
        authority_revoked: false,
    }
}
fn snapshot() -> SnapshotInput<'static> {
    SnapshotInput {
        tick: 0,
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

fn walk_request() -> InteractReq {
    InteractReq::WalkNear {
        x: 3200,
        z: 4100,
        level: 0,
        radius: 2,
        compat_tile_intent: false,
        allow_teleports: true,
        allow_wilderness: false,
        allow_bank_fetch: false,
        allow_prereq_dialogue: None,
        allow_quest_start: None,
        request_id: 0,
        track_door_crossing: false,
        avoid: Vec::new(),
        cross: Vec::new(),
    }
}

fn admitted_walk_outcome(
    id: RequestId,
    generation: u64,
    failed: bool,
    moving: Option<bool>,
) -> NativeFactsInput<'static> {
    NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_generation: generation,
        walk_outcome_failed: failed,
        walk_outcome_no_path: failed,
        walk_outcome_x: 3200,
        walk_outcome_z: 4100,
        walk_outcome_level: 0,
        walk_outcome_radius: 2,
        walk_outcome_allow_teleports: true,
        walk_outcome_request_id: id.get(),
        local_player_moving: moving,
        ..Default::default()
    }
}

#[test]
fn queued_batch_count_does_not_drain_or_repartition_rows() {
    let mut client = client();
    assert_eq!(client.queued_batch_count(), 0);
    client.await_next_tick().unwrap();
    assert_eq!(client.queued_batch_count(), 1);
    assert_eq!(client.queued_batch_count(), 1);

    let first = client.drain_seat_batches();
    assert_eq!(first.len(), 1);
    let row_count = first[0].batch.reqs.len();
    assert_eq!(client.queued_batch_count(), 0);

    client.restore_seat_batches(first);
    assert_eq!(client.queued_batch_count(), 1);
    let restored = client.drain_seat_batches();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].batch.reqs.len(), row_count);
    assert_eq!(client.queued_batch_count(), 0);
}

#[test]
fn stamped_start_and_exact_terminal_are_consumed_once() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    assert_eq!(
        client.awaiting(),
        Some(Awaited::Seat {
            id,
            kind: NativeKind::BankPlan
        })
    );
    let mut batches = client.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    let batch = batches.pop().unwrap();
    assert_eq!(batch.generation, 1);
    assert_eq!(batch.batch.starts.len(), 1);
    assert_eq!(batch.batch.starts[0].run, 23);
    assert_eq!(batch.batch.starts[0].session, 5);
    assert_eq!(batch.batch.starts[0].id, id);
    assert!(batch.machine_events.is_empty());
    assert!(batch.batch.mirror_revokes.is_empty());

    let results = [result(id, 8, SeatTerminal::Done)];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        8,
        false,
    );
    assert!(
        !client.completion_ready(),
        "the captured release floor excludes an old terminal"
    );
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        7,
        false,
    );
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal {
            result: results[0],
            value: None,
        })
    );
    assert_eq!(client.take_completion(), None);
    assert!(client.awaiting().is_none());
    let batches = client.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].batch.consumed_terminal_seq, 8);

    let empty_page = SeatPageInput::new(&[], &[], &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(empty_page),
            ..Default::default()
        },
        8,
        false,
    );
    assert!(
        client.drain_seat_batches().is_empty(),
        "the empty page acknowledges the sent watermark"
    );
}

#[test]
fn typed_families_never_resume_from_a_bare_done_terminal() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::Gather, SeatStartArgs::Empty, None)
        .unwrap();
    let results = [result(id, 1, SeatTerminal::Done)];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        0,
        false,
    );
    assert!(!client.completion_ready());
    let wrong = GatherEnd::Stopped {
        token: id.get() + 1,
        counts: GatherCounts::default(),
    };
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            api_gather_outcome: Some(&wrong),
            ..Default::default()
        },
        0,
        false,
    );
    assert!(!client.completion_ready());
    let end = GatherEnd::Stopped {
        token: id.get(),
        counts: GatherCounts::default(),
    };
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            api_gather_outcome: Some(&end),
            ..Default::default()
        },
        0,
        false,
    );
    assert_eq!(client.take_completion(), Some(SeatCompletion::Gather(end)));

    let combat_id = client
        .start_awaited(NativeKind::Combat, SeatStartArgs::Empty, None)
        .unwrap();
    let combat_results = [result(combat_id, 2, SeatTerminal::Failed)];
    let combat_page = SeatPageInput::new(&[], &combat_results, &[], 0).unwrap();
    let combat = CombatEnd::Stopped {
        token: combat_id.get(),
    };
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(combat_page),
            api_combat_outcome: Some(&combat),
            ..Default::default()
        },
        0,
        false,
    );
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Combat(combat))
    );

    let progress_id = client
        .start_awaited(NativeKind::QuestProgress, SeatStartArgs::Empty, None)
        .unwrap();
    let progress_results = [result(progress_id, 3, SeatTerminal::Refused(Refusal::Busy))];
    let progress_page = SeatPageInput::new(&[], &progress_results, &[], 0).unwrap();
    let progress = ProgressPage::Refused {
        token: progress_id.get(),
        reason: "busy".into(),
    };
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(progress_page),
            api_progress: Some(&progress),
            ..Default::default()
        },
        0,
        false,
    );
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Progress(progress))
    );
}

#[test]
fn walk_completion_waits_for_a_matching_host_outcome() {
    let mut client = client();
    let input = snapshot();
    let id = client.queue_walk(walk_request(), 71, &input).unwrap();
    let batch = client.drain_seat_batches().pop().unwrap();
    assert!(matches!(
        &batch.batch.reqs[0].request,
        InteractReq::WalkNear { request_id, .. } if *request_id == id.get()
    ));
    assert_eq!(client.awaiting(), Some(Awaited::Walk { id }));

    let mismatched = NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_generation: 71,
        walk_outcome_failed: true,
        walk_outcome_x: 3200,
        walk_outcome_z: 4100,
        walk_outcome_level: 0,
        walk_outcome_radius: 2,
        walk_outcome_allow_teleports: true,
        walk_outcome_request_id: id.get() + 1,
        ..Default::default()
    };
    client.accept_delivery(&input, &mismatched, 0, false);
    assert!(
        !client.completion_ready(),
        "a different request cannot settle this walk"
    );
    assert_eq!(client.awaiting(), Some(Awaited::Walk { id }));

    let matching = NativeFactsInput {
        walk_outcome_seq: 2,
        walk_outcome_generation: 71,
        walk_outcome_failed: true,
        walk_outcome_x: 3200,
        walk_outcome_z: 4100,
        walk_outcome_level: 0,
        walk_outcome_radius: 2,
        walk_outcome_allow_teleports: true,
        walk_outcome_request_id: id.get(),
        walk_outcome_no_path: true,
        ..Default::default()
    };
    client.accept_delivery(&input, &matching, 0, false);
    let Some(SeatCompletion::Walk(walk)) = client.take_completion() else {
        panic!("matching walk outcome settles the wait")
    };
    assert_eq!(
        (
            walk.id,
            walk.sequence,
            walk.generation,
            walk.x,
            walk.z,
            walk.radius,
            walk.allow_teleports,
            walk.no_path,
        ),
        (id, 2, 71, 3200, 4100, 2, true, true)
    );
}

#[test]
fn host_refusal_settles_after_admission_advances_route_generation() {
    let mut client = client();
    let input = snapshot();
    let id = client.queue_walk(walk_request(), 71, &input).unwrap();
    client.drain_seat_batches();
    client.accept_delivery(&input, &admitted_walk_outcome(id, 72, true, None), 0, false);
    let Some(SeatCompletion::Walk(walk)) = client.take_completion() else {
        panic!("host-refused walk must complete even when admission advances the generation")
    };
    assert_eq!((walk.id, walk.generation, walk.sequence), (id, 72, 1));
    assert!(walk.failed);
    assert!(walk.no_path);
}

#[test]
fn route_end_without_arrival_settles_after_admission_advances_generation() {
    let mut client = client();
    let input = snapshot();
    let id = client.queue_walk(walk_request(), 71, &input).unwrap();
    client.drain_seat_batches();
    client.accept_delivery(
        &input,
        &admitted_walk_outcome(id, 72, false, Some(false)),
        0,
        false,
    );
    let Some(SeatCompletion::Walk(walk)) = client.take_completion() else {
        panic!("ended walk without arrival must complete after its route generation advances")
    };
    assert_eq!((walk.id, walk.generation, walk.sequence), (id, 72, 1));
    assert!(!walk.failed);
}

#[test]
fn starts_and_cancels_retry_until_a_page_acknowledges_them() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    client.cancel(id, CallbackCancelReason::HookThrew).unwrap();
    client
        .cancel(id, CallbackCancelReason::ParentAborted)
        .unwrap();

    let first = client.drain_seat_batches().pop().unwrap();
    assert_eq!(first.batch.starts.len(), 1);
    assert_eq!(first.batch.cancels.len(), 1);
    assert_eq!(first.batch.cancels[0].of, id);
    assert_eq!(
        first.batch.cancels[0].reason,
        CallbackCancelReason::ParentAborted
    );
    let retry = client.drain_seat_batches().pop().unwrap();
    assert_eq!(retry.batch.starts[0].id, id);
    assert_eq!(retry.batch.cancels, first.batch.cancels);
    assert_eq!(client.pending_starts(), 1);

    let results = [result(id, 4, SeatTerminal::Refused(Refusal::Busy))];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        0,
        false,
    );
    assert_eq!(client.pending_starts(), 0);
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal {
            result: results[0],
            value: None,
        })
    );

    let watermark = client.drain_seat_batches().pop().unwrap();
    assert!(watermark.batch.starts.is_empty());
    assert!(watermark.batch.cancels.is_empty());
    assert_eq!(watermark.batch.consumed_terminal_seq, 4);
    let retry = client.drain_seat_batches().pop().unwrap();
    assert_eq!(retry.batch.consumed_terminal_seq, 4);

    let empty_page = SeatPageInput::new(&[], &[], &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(empty_page),
            ..Default::default()
        },
        0,
        false,
    );
    assert!(client.drain_seat_batches().is_empty());
}

#[test]
fn reset_discards_stale_starts_cancels_and_restored_rows() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    client
        .cancel(id, CallbackCancelReason::ParentAborted)
        .unwrap();
    client.loop_settled().unwrap();

    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.starts.len(), 1);
    assert_eq!(batch.batch.cancels.len(), 1);
    client.restore_seat_batches(vec![batch]);
    assert_eq!(client.pending_starts(), 1);

    client.reset(None);
    assert_eq!(client.pending_starts(), 0);
    assert!(client.drain_seat_batches().is_empty());
    assert!(client.drain_lifecycle().is_empty());
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Aborted {
            awaited: Awaited::Seat {
                id,
                kind: NativeKind::BankPlan,
            },
            reason: AbortReason::Reset,
        })
    );
}

#[test]
fn align_execution_rekeys_a_pending_start_without_aborting_its_wait() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let execution = execution(29, 6);
    client.align_execution(execution);

    assert_eq!(client.execution(), execution);
    assert_eq!(
        client.awaiting(),
        Some(Awaited::Seat {
            id,
            kind: NativeKind::BankPlan
        })
    );
    assert!(!client.completion_ready());
    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.starts.len(), 1);
    assert_eq!(batch.batch.starts[0].id, id);
    assert_eq!(batch.batch.starts[0].run, 29);
    assert_eq!(batch.batch.starts[0].session, 6);
}

#[test]
fn reset_advances_the_work_generation_for_new_batches() {
    let mut client = client();
    let old_id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let old_generation = client.drain_seat_batches()[0].generation;

    let new_generation = client.reset(None);
    assert_eq!(new_generation, old_generation + 1);
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Aborted {
            awaited: Awaited::Seat {
                id: old_id,
                kind: NativeKind::BankPlan,
            },
            reason: AbortReason::Reset,
        })
    );
    let new_id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let new_batch = client.drain_seat_batches().pop().unwrap();
    assert_ne!(new_id, old_id);
    assert_eq!(new_batch.generation, new_generation);
    assert_eq!(new_batch.batch.starts[0].id, new_id);
}

#[test]
fn reset_preserves_a_completion_already_copied_from_the_page() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let results = [result(id, 6, SeatTerminal::Done)];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        0,
        false,
    );
    assert!(client.completion_ready());

    client.reset(None);
    assert!(client.completion_ready());
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal {
            result: results[0],
            value: None,
        })
    );
}

#[test]
fn reconnect_preserves_and_requeues_the_awaited_seat_start() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let old_batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(old_batch.batch.starts[0].id, id);

    let new_execution = execution(24, 6);
    let generation = client.reconnect(Some(new_execution));
    assert_eq!(generation, 2);
    assert_eq!(
        client.awaiting(),
        Some(Awaited::Seat {
            id,
            kind: NativeKind::BankPlan
        })
    );
    assert!(!client.completion_ready());
    assert_eq!(client.pending_starts(), 0);

    client.accept_delivery(&snapshot(), &NativeFactsInput::default(), 0, false);
    assert!(!client.completion_ready());
    assert_eq!(
        client.awaiting(),
        Some(Awaited::Seat {
            id,
            kind: NativeKind::BankPlan
        })
    );
    assert_eq!(client.pending_starts(), 0);

    client.accept_delivery(&snapshot(), &NativeFactsInput::default(), 0, true);
    assert!(!client.completion_ready());
    assert_eq!(
        client.awaiting(),
        Some(Awaited::Seat {
            id,
            kind: NativeKind::BankPlan
        })
    );
    let requeued = client.drain_seat_batches().pop().unwrap();
    assert_eq!(requeued.generation, generation);
    assert_eq!(requeued.batch.starts.len(), 1);
    assert_eq!(requeued.batch.starts[0].id, id);
    assert_eq!(requeued.batch.starts[0].run, 24);
    assert_eq!(requeued.batch.starts[0].session, 6);
}

#[test]
fn reconnect_settles_a_next_tick_wait_on_the_first_full_tick() {
    let mut client = client();
    client.await_next_tick().unwrap();
    client.reconnect(None);

    assert_eq!(client.awaiting(), Some(Awaited::NextTick));
    assert!(!client.completion_ready());
    client.accept_delivery(&snapshot(), &NativeFactsInput::default(), 0, false);
    assert!(!client.completion_ready());
    assert_eq!(client.awaiting(), Some(Awaited::NextTick));

    client.accept_delivery(&snapshot(), &NativeFactsInput::default(), 0, true);
    assert_eq!(client.take_completion(), Some(SeatCompletion::NextTick));
    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.reqs.len(), 2);
    assert!(matches!(
        &batch.batch.reqs[0].request,
        InteractReq::WaitEnqueued
    ));
    assert!(matches!(
        &batch.batch.reqs[1].request,
        InteractReq::WaitSettled
    ));
}

#[test]
fn reconnect_rearms_an_unsent_walk_on_the_first_full_tick() {
    let mut client = client();
    let input = snapshot();
    let id = client.queue_walk(walk_request(), 71, &input).unwrap();
    client.reconnect(Some(execution(24, 6)));

    assert_eq!(client.awaiting(), Some(Awaited::Walk { id }));
    assert!(!client.completion_ready());
    client.accept_delivery(&input, &NativeFactsInput::default(), 0, false);
    assert!(!client.completion_ready());

    let new_generation = 93;
    client.accept_delivery(
        &input,
        &NativeFactsInput {
            walk_outcome_generation: new_generation,
            ..Default::default()
        },
        0,
        true,
    );
    assert_eq!(client.awaiting(), Some(Awaited::Walk { id }));
    assert!(!client.completion_ready());
    let batch = client.drain_seat_batches().pop().unwrap();
    let rearmed_walks = batch
        .batch
        .reqs
        .iter()
        .filter(|row| {
            matches!(
                &row.request,
                InteractReq::WalkNear { request_id, .. } if *request_id == id.get()
            )
        })
        .count();
    assert_eq!(rearmed_walks, 1);

    client.accept_delivery(
        &input,
        &NativeFactsInput {
            walk_outcome_seq: 1,
            walk_outcome_generation: new_generation,
            walk_outcome_failed: true,
            walk_outcome_x: 3200,
            walk_outcome_z: 4100,
            walk_outcome_level: 0,
            walk_outcome_radius: 2,
            walk_outcome_allow_teleports: true,
            walk_outcome_request_id: id.get(),
            ..Default::default()
        },
        0,
        false,
    );
    let Some(SeatCompletion::Walk(walk)) = client.take_completion() else {
        panic!("the re-armed walk keeps its original request token")
    };
    assert_eq!(
        (walk.id, walk.generation, walk.sequence),
        (id, new_generation, 1)
    );
}

#[test]
fn reconnect_rearmed_walk_settles_on_admitted_generation() {
    let mut client = client();
    let input = snapshot();
    let id = client.queue_walk(walk_request(), 71, &input).unwrap();
    client.reconnect(Some(execution(24, 6)));
    client.accept_delivery(
        &input,
        &NativeFactsInput {
            walk_outcome_generation: 93,
            ..Default::default()
        },
        0,
        true,
    );
    client.drain_seat_batches();
    client.accept_delivery(&input, &admitted_walk_outcome(id, 94, true, None), 0, false);
    let Some(SeatCompletion::Walk(walk)) = client.take_completion() else {
        panic!("re-armed walk must settle on its new session's admitted generation")
    };
    assert_eq!((walk.id, walk.generation, walk.sequence), (id, 94, 1));
    assert!(walk.failed);
}

#[test]
fn reconnect_does_not_replay_an_already_sent_walk() {
    let mut client = client();
    let input = snapshot();
    let id = client.queue_walk(walk_request(), 71, &input).unwrap();
    let sent = client.drain_seat_batches().pop().unwrap();
    assert!(sent.batch.reqs.iter().any(|row| matches!(
        &row.request,
        InteractReq::WalkNear { request_id, .. } if *request_id == id.get()
    )));

    client.reconnect(Some(execution(24, 6)));
    client.accept_delivery(
        &input,
        &NativeFactsInput {
            walk_outcome_generation: 93,
            ..Default::default()
        },
        0,
        true,
    );
    assert_eq!(client.awaiting(), Some(Awaited::Walk { id }));
    assert!(!client.completion_ready());
    let batches = client.drain_seat_batches();
    let walk_rows = batches
        .iter()
        .flat_map(|batch| &batch.batch.reqs)
        .filter(|row| matches!(&row.request, InteractReq::WalkNear { .. }))
        .count();
    assert_eq!(walk_rows, 0);
}

#[test]
fn cancel_capacity_rejects_the_first_row_beyond_the_bound() {
    let mut client = client();
    for raw in 1..=MAX_SEAT_CANCELS {
        let id = RequestId::isolate(1000 + raw as u64).unwrap();
        client.cancel(id, CallbackCancelReason::HookThrew).unwrap();
    }
    assert_eq!(
        client.cancel(
            RequestId::isolate(1000 + MAX_SEAT_CANCELS as u64 + 1).unwrap(),
            CallbackCancelReason::HookThrew
        ),
        Err(ClientError::CancelsFull)
    );
    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.cancels.len(), MAX_SEAT_CANCELS);
}

#[test]
fn completion_waits_until_lifecycle_capacity_is_available() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let results = [result(id, 1, SeatTerminal::Done)];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        0,
        false,
    );
    for _ in 1..MAX_SEAT_GAMEPLAY_ROWS {
        client.loop_settled().unwrap();
    }

    assert!(client.completion_ready());
    assert_eq!(client.take_completion(), None);
    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.reqs.len(), MAX_SEAT_GAMEPLAY_ROWS);
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal {
            result: results[0],
            value: None,
        })
    );
}

#[test]
fn lifecycle_rows_share_the_seat_batch_path() {
    let mut client = client();
    client.await_next_tick().unwrap();
    client.loop_settled().unwrap();
    client.accept_delivery(&snapshot(), &NativeFactsInput::default(), 0, true);
    assert_eq!(client.take_completion(), Some(SeatCompletion::NextTick));

    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.reqs.len(), 3);
    assert!(matches!(
        &batch.batch.reqs[0].request,
        InteractReq::WaitEnqueued
    ));
    assert!(matches!(
        &batch.batch.reqs[1].request,
        InteractReq::LoopSettled
    ));
    assert!(matches!(
        &batch.batch.reqs[2].request,
        InteractReq::WaitSettled
    ));
    assert!(client.drain_lifecycle().is_empty());
}

#[test]
fn restored_held_wait_rows_count_toward_the_gameplay_bound() {
    let mut producer = client();
    producer.await_next_tick().unwrap();
    for _ in 0..MAX_SEAT_GAMEPLAY_ROWS - 3 {
        producer.loop_settled().unwrap();
    }
    let mut batch = producer.drain_seat_batches().pop().unwrap();
    batch.held_waits.enqueue();
    batch.held_waits.settle();
    batch.held_fold = true;
    assert_eq!(batch.held_waits.rows(), 2);
    assert_eq!(batch.rows(), MAX_SEAT_GAMEPLAY_ROWS);

    let mut client = client();
    assert_eq!(client.held_fold_len(), 0);
    client.restore_seat_batches(vec![batch]);
    assert_eq!(client.held_fold_len(), 1);
    assert_eq!(client.held_fold_len(), 1);
    assert_eq!(client.await_next_tick(), Err(ClientError::RowsFull));
    assert_eq!(client.awaiting(), None);
    assert_eq!(client.drain_seat_batches().len(), 1);
    assert_eq!(client.held_fold_len(), 0);
}

#[test]
fn terminal_page_consumes_every_result_sequence() {
    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let other = RequestId::isolate(500).unwrap();
    let results = [
        result(id, 1, SeatTerminal::Done),
        result(other, 2, SeatTerminal::Done),
    ];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        &snapshot(),
        &NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        0,
        false,
    );

    let batch = client.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.consumed_terminal_seq, 2);
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal {
            result: results[0],
            value: None,
        })
    );
}

#[test]
fn model_trace_drives_the_client_terminal_receipt() {
    let model_id = 1;
    let steps = [
        Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Session(SessionFamily::Hunt),
            group: 1,
            bytes: 0,
            hooks: 0,
        },
        Step::Admit(model_id),
        Step::Prompt(model_id),
        Step::Decide {
            of: model_id,
            complete: true,
        },
        Step::Controls,
    ];
    let trace = run_trace(Rules::Chosen, Bounds::production(), &steps);
    assert!(trace.failure.is_none(), "{}", trace.text);
    assert_eq!(trace.final_state.outcome(model_id), Some(Outcome::Done));
    let terminal_order = trace.final_state.terminal_order();
    assert_eq!(terminal_order, [model_id]);
    let sequence = terminal_order
        .iter()
        .position(|id| *id == model_id)
        .unwrap() as u64
        + 1;

    let mut client = client();
    let mut seat = GameplaySeat::<u32>::new(execution(23, 5).run());
    let mut client_id = None;
    let mut expected = None;
    for step in &steps {
        match step {
            Step::Start { class, .. } => {
                assert_eq!(*class, Class::Session(SessionFamily::Hunt));
                client_id = Some(
                    client
                        .start_awaited(NativeKind::Hunt, SeatStartArgs::Empty, None)
                        .unwrap(),
                );
            }
            Step::Admit(id) => {
                assert_eq!(*id, model_id);
                let batch = client.drain_seat_batches().pop().unwrap();
                let [start] = batch.batch.starts.as_slice() else {
                    panic!("one modeled start reaches the real seat");
                };
                assert_eq!(start.id, client_id.unwrap());
                let stamp =
                    RequestStamp::new(start.id, start.parent, None, execution(23, 5)).unwrap();
                assert_eq!(
                    seat.start(stamp, start.kind, false).unwrap(),
                    Admission::Admitted
                );
            }
            Step::Prompt(id) => {
                assert_eq!(*id, model_id);
                let point = PromptPoint::from_contract_row(
                    script::native::abi::HOOK_CHARACTERIZATIONS
                        .iter()
                        .find(|row| row.member() == "sustain")
                        .expect("Hunt sustain contract row"),
                )
                .expect("awaited Hunt point");
                assert_eq!(
                    seat.prompt_with_details(client_id.unwrap(), point, &[]),
                    Ok(DecisionSequence::FIRST)
                );
            }
            Step::Decide { of, complete } => {
                assert_eq!(*of, model_id);
                assert!(*complete);
                assert_eq!(
                    seat.decide_with_answer(client_id.unwrap(), DecisionSequence::FIRST, vec![]),
                    SeatDecisionStatus::Consumed
                );
            }
            Step::Controls => {
                assert_eq!(trace.final_state.outcome(model_id), Some(Outcome::Done));
                seat.complete(client_id.unwrap(), false).unwrap();
                let terminal = seat.results()[0];
                assert_eq!(terminal.seq, sequence);
                assert_eq!(terminal.outcome, SeatTerminal::Done);
                expected = Some(SeatCompletion::Terminal {
                    result: terminal,
                    value: None,
                });
                let page = SeatPageInput::new(&[], seat.results(), &[], 0).unwrap();
                client.accept_delivery(
                    &snapshot(),
                    &NativeFactsInput {
                        seat_page: Some(page),
                        ..Default::default()
                    },
                    0,
                    false,
                );
            }
            _ => unreachable!("the trace follows fixture 1's prompt, decision, and receipt"),
        }
    }
    assert_eq!(client.take_completion(), expected);
}
#[test]
fn queued_interactions_are_exact_rows_and_stay_inside_the_seat_bound() {
    let request = InteractReq::Loc {
        x: 3200,
        z: 3201,
        level: 0,
        action: "Use-quickly".into(),
        id: Some(42),
    };
    let mut queued = client();
    queued.queue_interaction(request.clone()).unwrap();
    let batch = queued.drain_seat_batches().pop().unwrap();
    assert_eq!(batch.batch.reqs.len(), 1);
    assert_eq!(batch.batch.reqs[0].request, request);
    assert_eq!(batch.batch.reqs[0].lineage, None);
    assert_eq!(batch.batch.reqs[0].mirror_owner, None);
    for op in [
        InteractReq::Close,
        InteractReq::SetNoteMode { on: true },
        InteractReq::Deposit {
            name: "Logs".into(),
            id: None,
        },
        InteractReq::Withdraw {
            name: "Logs".into(),
            action: "Withdraw 1".into(),
        },
        InteractReq::WalkNearestBank,
        InteractReq::PuzzleMove {
            id: 2749,
            slot: 0,
            component: 6979,
            generation: 1,
        },
    ] {
        queued.queue_interaction(op.clone()).unwrap();
        let batch = queued.drain_seat_batches().pop().unwrap();
        assert_eq!(batch.batch.reqs.len(), 1);
        assert_eq!(batch.batch.reqs[0].request, op);
    }
    assert_eq!(
        queued.queue_interaction(walk_request()),
        Err(ClientError::NotInteraction)
    );

    let mut full = client();
    for _ in 0..MAX_SEAT_GAMEPLAY_ROWS {
        full.queue_interaction(InteractReq::Npc {
            name: "Banker".into(),
            action: "Bank".into(),
            index: None,
        })
        .unwrap();
    }
    assert_eq!(
        full.queue_interaction(InteractReq::Player {
            name: "bob".into(),
            action: "Trade".into(),
        }),
        Err(ClientError::RowsFull)
    );

    let mut waiting = client();
    waiting.await_next_tick().unwrap();
    assert_eq!(
        waiting.queue_interaction(InteractReq::Player {
            name: "bob".into(),
            action: "Trade".into(),
        }),
        Err(ClientError::AlreadyWaiting)
    );
}

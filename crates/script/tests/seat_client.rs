//! Conformance of the neutral adapter against the slot's seat page contract.
use api::identity::{ExecutionId, RequestId};
use api::script_contract::{AbortReason, CallbackCancelReason, Refusal};
use api::selected::RunKey;
use script::api_combat::CombatEnd;
use script::api_gather::{GatherCounts, GatherEnd};
use script::api_progress::ProgressPage;
use script::native::request::NativeKind;
use script::native::seat::{SeatResult, SeatTerminal};
use script::native::seat_rows::{SeatPageInput, SeatStartArgs, MAX_SEAT_STARTS};
use script::native::snapshot_source::NativeFactsInput;
use script::surface::client::{Awaited, ClientError, SeatClient, SeatCompletion};
use seat_model::{Bounds, Class, Outcome, Rules, State, Step};

fn client() -> SeatClient {
    SeatClient::new(ExecutionId::first(RunKey {
        slot: 17,
        run: 23,
        session: 5,
    }))
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
        NativeFactsInput {
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
        NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        7,
        false,
    );
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal(results[0]))
    );
    assert_eq!(client.take_completion(), None);
    assert!(client.awaiting().is_none());
    let batches = client.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].batch.consumed_terminal_seq, 8);
    assert!(
        client.drain_seat_batches().is_empty(),
        "watermark is sent once"
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
        NativeFactsInput {
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
        NativeFactsInput {
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
        NativeFactsInput {
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
        NativeFactsInput {
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
        NativeFactsInput {
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
fn pause_hold_and_session_reset_preserve_only_the_owed_completion() {
    let mut client = client();
    let id = RequestId::isolate(33).unwrap();
    client.await_walk(id).unwrap();
    let native = NativeFactsInput {
        walk_outcome_seq: 19,
        walk_outcome_request_id: id.get(),
        walk_outcome_generation: 71,
        walk_outcome_x: 3200,
        ..Default::default()
    };
    client.accept_delivery(native, 0, true); // held wake stores but does not resume
    assert!(client.completion_ready());
    assert_eq!(client.awaiting(), Some(Awaited::Walk { id, after_seq: 0 }));
    client.accept_delivery(NativeFactsInput::default(), 0, false);
    let Some(SeatCompletion::Walk(walk)) = client.take_completion() else {
        panic!("typed walk completion")
    };
    assert_eq!(
        (walk.id, walk.sequence, walk.generation, walk.x),
        (id, 19, 71, 3200)
    );
    assert_eq!(client.take_completion(), None);

    client.await_next_tick().unwrap();
    assert_eq!(client.reset(None), 1);
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Aborted(AbortReason::Reset))
    );
    assert!(client.awaiting().is_none());
}

#[test]
fn capacity_refuses_without_reusing_id_or_queuing_a_partial_start() {
    let mut client = client();
    for seq in 1..=MAX_SEAT_STARTS {
        let id = client
            .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
            .unwrap();
        let results = [result(id, seq as u64, SeatTerminal::Refused(Refusal::Busy))];
        let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
        client.accept_delivery(
            NativeFactsInput {
                seat_page: Some(page),
                ..Default::default()
            },
            0,
            false,
        );
        assert_eq!(
            client.take_completion(),
            Some(SeatCompletion::Terminal(results[0]))
        );
        client.drain_lifecycle();
    }
    assert_eq!(client.pending_starts(), MAX_SEAT_STARTS);
    assert_eq!(
        client.start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None),
        Err(ClientError::StartsFull)
    );
    client
        .cancel(
            RequestId::isolate(1).unwrap(),
            CallbackCancelReason::ParentAborted,
        )
        .unwrap();
    let mut batches = client.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    let batch = batches.pop().unwrap();
    assert_eq!(batch.batch.starts.len(), MAX_SEAT_STARTS);
    assert_eq!(batch.batch.cancels.len(), 1);
    client.restore_seat_batches(vec![batch]);
    assert_eq!(client.pending_starts(), MAX_SEAT_STARTS);
    assert_eq!(
        client.drain_seat_batches()[0].batch.starts.len(),
        MAX_SEAT_STARTS
    );
}

#[test]
fn model_start_admit_complete_trace_drives_client_terminal() {
    let mut model = State::new(Rules::Chosen, Bounds::production());
    let model_id = model.next_id();
    model
        .apply(&Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Capability,
            group: 1,
            bytes: 0,
            hooks: 0,
        })
        .unwrap();
    model.apply(&Step::Admit(model_id)).unwrap();
    model.apply(&Step::Complete(model_id)).unwrap();
    model.check().unwrap();
    assert_eq!(model.outcome(model_id), Some(Outcome::Done));

    let mut client = client();
    let id = client
        .start_awaited(NativeKind::BankPlan, SeatStartArgs::Empty, None)
        .unwrap();
    let results = [result(id, 1, SeatTerminal::Done)];
    let page = SeatPageInput::new(&[], &results, &[], 0).unwrap();
    client.accept_delivery(
        NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
        0,
        false,
    );
    assert_eq!(
        client.take_completion(),
        Some(SeatCompletion::Terminal(results[0]))
    );
    assert_eq!(
        client.drain_seat_batches()[0].batch.consumed_terminal_seq,
        1
    );
}

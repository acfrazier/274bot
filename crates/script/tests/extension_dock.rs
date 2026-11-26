use script::surface::extension::{
    ExtensionDock, ExtensionEnd, ExtensionEndCause, ExtensionEntry, ExtensionFaultCode,
    ExtensionMode, ExtensionPayload, ExtensionValue, RunBinding,
};

fn binding() -> RunBinding {
    RunBinding::new(7, 11, 13)
}

fn args() -> ExtensionValue {
    ExtensionValue::Named(vec![(
        "label".to_owned(),
        ExtensionValue::String("step".to_owned()),
    )])
}

fn stage(dock: &ExtensionDock, mode: ExtensionMode) -> u64 {
    dock.stage(binding(), ExtensionEntry::Tick, "expect.tile", mode, args())
        .expect("the call is admitted")
}

#[test]
fn a_staged_call_is_invisible_until_an_accepted_dispatch() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");

    let sequence = stage(&dock, ExtensionMode::Request);
    assert!(dock.take(binding()).expect("host take").is_none());
    dock.finish_dispatch(binding(), true)
        .expect("accepted dispatch commits");

    let call = dock
        .take(binding())
        .expect("host take")
        .expect("committed call");
    assert_eq!(call.binding, binding());
    assert_eq!(call.sequence, sequence);
    assert_eq!(call.name, "expect.tile");
    assert_eq!(call.mode, ExtensionMode::Request);
    assert_eq!(call.arguments, args());
}

#[test]
fn a_rejected_dispatch_discards_calls_without_reusing_sequences() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");

    let discarded = stage(&dock, ExtensionMode::Request);
    dock.finish_dispatch(binding(), false)
        .expect("refused dispatch discards its staging");
    assert!(dock.take(binding()).expect("host take").is_none());

    let admitted = stage(&dock, ExtensionMode::Request);
    assert!(admitted > discarded);
    dock.finish_dispatch(binding(), true)
        .expect("accepted dispatch commits");
    assert_eq!(
        dock.take(binding())
            .expect("host take")
            .expect("committed call")
            .sequence,
        admitted
    );
}

#[test]
fn completion_is_binding_and_sequence_owned_and_consumed_once() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");
    let sequence = stage(&dock, ExtensionMode::Await);
    dock.finish_dispatch(binding(), true)
        .expect("accepted dispatch commits");
    let call = dock
        .take(binding())
        .expect("host take")
        .expect("awaited call");
    assert_eq!(call.sequence, sequence);

    let wrong_binding = RunBinding::new(7, 12, 13);
    assert!(dock
        .complete(wrong_binding, sequence, ExtensionPayload::Boolean(true))
        .is_err());
    dock.complete(binding(), sequence, ExtensionPayload::Boolean(true))
        .expect("matching completion");
    assert!(dock
        .complete(binding(), sequence, ExtensionPayload::Unit)
        .is_err());
    assert_eq!(
        dock.take_completion(binding(), sequence)
            .expect("resume payload")
            .expect("one completion"),
        ExtensionPayload::Boolean(true)
    );
    assert!(dock.take_completion(binding(), sequence).is_err());
}

#[test]
fn the_first_fault_survives_seal_revocation_and_close() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");
    dock.latch_fault(
        binding(),
        Some(4),
        ExtensionEntry::Paint,
        ExtensionFaultCode::WrongEntry,
        "paint cannot issue scenario requests",
    );
    let first = dock.fault().expect("durable first fault");

    dock.seal(binding()).expect("seal");
    assert!(!dock.is_fenced());
    assert!(dock
        .stage(
            binding(),
            ExtensionEntry::Tick,
            "expect.tile",
            ExtensionMode::Request,
            args(),
        )
        .is_err());
    dock.revoke();
    assert!(dock.is_fenced());
    dock.close(ExtensionEnd::new(binding(), ExtensionEndCause::Stopped));

    assert_eq!(dock.fault(), Some(first.clone()));
    assert_eq!(dock.end().expect("retained end").first_fault, Some(first));
    assert!(dock.take(binding()).expect("closed host take").is_none());
}

#[test]
fn queue_overflow_refuses_the_whole_call_and_latches_failure() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");

    for _ in 0..64 {
        stage(&dock, ExtensionMode::Request);
        dock.finish_dispatch(binding(), true)
            .expect("bounded call commits");
    }
    assert!(dock
        .stage(
            binding(),
            ExtensionEntry::Tick,
            "expect.tile",
            ExtensionMode::Request,
            args(),
        )
        .is_err());
    assert!(dock.fault().is_some());

    let mut count = 0;
    while dock.take(binding()).expect("host take").is_some() {
        count += 1;
    }
    assert_eq!(count, 64);
}

#[test]
fn the_owner_can_revoke_egress_without_discarding_the_terminal_evidence() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");
    stage(&dock, ExtensionMode::Request);
    dock.finish_dispatch(binding(), true)
        .expect("commit one request");
    stage(&dock, ExtensionMode::Await);

    dock.publish_revocation();
    assert!(dock.is_revoked());
    assert!(dock.take(binding()).expect("fenced take").is_none());
    dock.close(ExtensionEnd::new(binding(), ExtensionEndCause::Stopped));

    let end = dock.end().expect("post-close terminal evidence");
    assert_eq!(end.disposal_counts.staged, 1);
    assert_eq!(end.disposal_counts.queued, 1);
}

#[test]
fn a_finish_call_commits_once_and_closes_later_admissions() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("first claim");

    let sequence = stage(&dock, ExtensionMode::Finish);
    dock.finish_dispatch(binding(), true)
        .expect("accepted finish commits");
    assert!(dock.is_fenced());
    assert!(dock
        .stage(
            binding(),
            ExtensionEntry::Tick,
            "expect.tile",
            ExtensionMode::Request,
            args(),
        )
        .is_err());
    assert_eq!(
        dock.take(binding())
            .expect("host take")
            .expect("finish call")
            .sequence,
        sequence
    );
}

#[test]
fn request_handles_can_be_awaited_only_after_host_take() {
    let dock = ExtensionDock::new();
    dock.claim(binding()).expect("dock claims once");
    let sequence = stage(&dock, ExtensionMode::Request);
    dock.finish_dispatch(binding(), true)
        .expect("accepted dispatch commits request");
    assert!(
        dock.begin_await(binding(), sequence).is_err(),
        "the host must first take the request"
    );

    dock.take(binding())
        .expect("host reads dock")
        .expect("request is queued");
    dock.begin_await(binding(), sequence)
        .expect("taken request can own one await");
    dock.finish_dispatch(binding(), true)
        .expect("accepted await dispatch commits ownership");
    assert_eq!(dock.pending_await(binding()), Some(sequence));
    assert_eq!(dock.pending_await(RunBinding::new(88, 3, 4)), None);
    dock.complete(binding(), sequence, ExtensionPayload::Boolean(false))
        .expect("matching completion is admitted");
    assert_eq!(dock.pending_await(binding()), None);
    assert!(dock.completion_ready(binding(), sequence));
    assert_eq!(
        dock.take_completion(binding(), sequence)
            .expect("VM resumes its own await"),
        Some(ExtensionPayload::Boolean(false))
    );
    assert!(
        dock.complete(binding(), sequence, ExtensionPayload::Unit)
            .is_err(),
        "consumed completion cannot be reused"
    );
}

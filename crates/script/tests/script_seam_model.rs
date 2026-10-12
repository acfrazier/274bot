//! F0 chosen-rule conformance. The real dev-only oracle executes transitions;
//! this test never implements a second forest, ledger, encoder or service.
use api::identity::{RequestId, Settlement};
use api::script_contract as frozen;
use script::native::custody::CallbackCustody;
use script::native::divergence::{DivergenceStatus, DIVERGENCES};
use script::native::request::{NativeKind, PendingPayload};
use seat_model::{Bounds, Class, Outcome, Rules, SessionFamily, State, Step, Trigger};

fn apply(state: &mut State, step: Step) {
    state
        .apply(&step)
        .unwrap_or_else(|error| panic!("{step:?}: {error}"));
    state
        .check()
        .unwrap_or_else(|error| panic!("{step:?}: {error:?}"));
}
fn admitted(state: &mut State, parent: Option<u16>, class: Class, group: u8) -> u16 {
    let id = state.next_id();
    apply(
        state,
        Step::Start {
            parent,
            local_parent: None,
            class,
            group,
            bytes: 0,
            hooks: 0,
        },
    );
    apply(state, Step::Admit(id));
    assert!(
        state.entry(id).is_some(),
        "test precondition: admitted id {id}"
    );
    id
}
fn request(id: u16) -> RequestId {
    RequestId::isolate(u64::from(id)).unwrap()
}
fn custody(id: u16) -> CallbackCustody<()> {
    CallbackCustody::new(
        request(id),
        PendingPayload::new(Vec::new(), 0, None).unwrap(),
        None,
    )
}

fn outcome_class_matches(frozen: &frozen::Outcome<(), ()>, model: Outcome) -> bool {
    match model {
        Outcome::Done => matches!(
            frozen.settlement(),
            Settlement::Completed(frozen::Completion::Done(()))
        ),
        Outcome::Failed => matches!(frozen.settlement(), Settlement::Failed(())),
        Outcome::Aborted(_) => matches!(frozen.settlement(), Settlement::Cancelled(_)),
        Outcome::Refused(_) => matches!(
            frozen.settlement(),
            Settlement::Completed(frozen::Completion::Refused(_))
        ),
    }
}

#[test]
fn frozen_bounds_match_chosen_model() {
    let model = Bounds::production();
    assert_eq!(frozen::MAX_ROOTS, model.roots, "MAX_ROOTS drift from SG");
    assert_eq!(frozen::MAX_LIVE_LEAVES, model.roots);
    assert_eq!(frozen::MAX_LIVE, model.live);
    assert_eq!(frozen::MAX_LINEAGE_DEPTH, model.depth);
    assert_eq!(frozen::MAX_RESULTS, model.credits);
    assert_eq!(frozen::MAX_PENDING_STARTS, model.pending);
    assert_eq!(frozen::REVOKED_RING, model.history);
    assert_eq!(frozen::MAX_INTERACT_REQS, model.rows);
    assert_eq!(frozen::MAX_ENCODED_GAMEPLAY, model.gameplay);
    assert_eq!(frozen::MAX_START_CHARGE, seat_model::MAX_START_CHARGE);
    assert_eq!(frozen::START_METADATA, seat_model::START_METADATA);
    assert_eq!(frozen::HOOK_CHARGE, seat_model::HOOK_CHARGE);
    assert_eq!(frozen::MAX_HOOKS, usize::from(seat_model::MAX_HOOKS));
    assert_eq!(frozen::CONTROL_RESERVATION, seat_model::CONTROL_RESERVATION);
    assert_eq!(frozen::MAX_CANCEL_CARRY, model.live + model.pending);
    assert_eq!(
        frozen::CONTROL_RESERVATION + frozen::MAX_ENCODED_GAMEPLAY,
        model.rows
    );
}

#[test]
fn model_refusal_variants_are_exhaustively_translated_to_frozen_vocabulary() {
    fn translate(value: seat_model::Refusal) -> frozen::Refusal {
        match value {
            seat_model::Refusal::Busy => frozen::Refusal::Busy,
            seat_model::Refusal::ParentBusy => frozen::Refusal::ParentBusy,
            seat_model::Refusal::ParentNotLive => frozen::Refusal::ParentNotLive,
            seat_model::Refusal::LineageDepth => frozen::Refusal::LineageDepth,
            seat_model::Refusal::ArgsTooLarge => frozen::Refusal::ArgsTooLarge,
            seat_model::Refusal::HistoryExpired => frozen::Refusal::HistoryExpired,
        }
    }
    // Exercise labels from the chosen model, not just a same-source list.
    let mut state = State::new(Rules::Chosen, Bounds::production());
    let id = state.next_id();
    apply(
        &mut state,
        Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Capability,
            group: 1,
            bytes: frozen::MAX_START_CHARGE,
            hooks: 1,
        },
    );
    let Some(Outcome::Refused(reason)) = state.outcome(id) else {
        panic!("oversize start did not refuse")
    };
    assert_eq!(translate(reason), frozen::Refusal::ArgsTooLarge);
    assert_eq!(state.retained(), 0, "local refusal consumes no seat credit");
    assert!(outcome_class_matches(
        &frozen::Outcome::refused(translate(reason)),
        state.outcome(id).unwrap()
    ));
}

#[test]
fn every_reviewed_profile_prefix_executes_in_the_real_chosen_oracle() {
    // Reuse the oracle's named fixtures; do not copy its transition rules or
    // run an unbounded explorer. Exhaustive suffix searches remain SG evidence.
    for profile in seat_model::profiles() {
        let mut state = State::new(Rules::Chosen, profile.bounds());
        for step in profile.prefix() {
            apply(&mut state, step);
        }
    }
}

#[test]
fn foreground_predicate_and_accepted_completion_authority_match_model() {
    for (model_family, native_family) in [
        (SessionFamily::Hunt, frozen::SessionFamily::Hunt),
        (SessionFamily::Gather, frozen::SessionFamily::Gather),
        (SessionFamily::Combat, frozen::SessionFamily::Combat),
        (SessionFamily::Progress, frozen::SessionFamily::Progress),
    ] {
        assert_eq!(
            native_family.foreground(),
            Some(model_family.owns_foreground())
        );
        let mut state = State::new(Rules::Chosen, Bounds::production());
        let session = admitted(&mut state, None, Class::Session(model_family), 0);
        let child = admitted(&mut state, Some(session), Class::Capability, 2);
        apply(
            &mut state,
            Step::Emit {
                of: Some(child),
                release: false,
            },
        );
        apply(&mut state, Step::Complete(child));
        apply(&mut state, Step::Page);
        apply(&mut state, Step::Watermark);
        apply(&mut state, Step::Take);
        apply(&mut state, Step::Dispatch);
        assert_eq!(
            state.dispatched(),
            1,
            "ordinary completion must not revoke the captured lease"
        );
        assert_eq!(
            frozen::dispatch_disposition(
                frozen::EmissionClass::Gameplay,
                Some(request(session)),
                state.foreground_session().map(request),
                frozen::AdmissionWitness::Admitted,
                frozen::EmissionProof::Proven,
                frozen::AuthorityBoundary::Preserved,
                false
            ),
            frozen::DispatchDisposition::Dispatch
        );
        let outcome: frozen::Outcome<(), ()> = frozen::Outcome::done(());
        assert!(!outcome.revokes_authority());
    }
}

#[test]
fn revocation_labels_and_leaf_first_order_match_the_boundary_contract() {
    for (trigger, reason) in [
        (Trigger::Superseded, frozen::AbortReason::Superseded),
        (Trigger::ParentAborted, frozen::AbortReason::ParentAborted),
        (Trigger::ManualCombat, frozen::AbortReason::UserInput),
    ] {
        let mut state = State::new(Rules::Chosen, Bounds::production());
        let root = admitted(&mut state, None, Class::Capability, 1);
        let child = admitted(&mut state, Some(root), Class::Capability, 2);
        apply(&mut state, Step::Revoke { of: root, trigger });
        assert_eq!(state.terminal_order(), vec![child, root]);
        assert_eq!(state.outcome(root), Some(Outcome::Aborted(trigger)));
        assert_eq!(
            state.outcome(child),
            Some(Outcome::Aborted(Trigger::ParentAborted))
        );
        let outcome: frozen::Outcome<(), ()> = frozen::Outcome::aborted(reason);
        assert_eq!(outcome.abort_reason(), Some(reason));
        assert!(
            matches!(outcome.settlement(), Settlement::Cancelled(value) if *value == reason.cancellation())
        );
        assert!(outcome_class_matches(
            &frozen::Outcome::aborted(reason),
            state.outcome(root).unwrap()
        ));
    }
    let mut walk = State::new(Rules::Chosen, Bounds::production());
    let root = admitted(&mut walk, None, Class::Capability, 1);
    apply(
        &mut walk,
        Step::Emit {
            of: Some(root),
            release: false,
        },
    );
    apply(
        &mut walk,
        Step::Revoke {
            of: root,
            trigger: Trigger::ManualWalk,
        },
    );
    apply(&mut walk, Step::Take);
    apply(&mut walk, Step::Dispatch);
    assert_eq!(walk.outcome(root), Some(Outcome::Done));
    assert_eq!(walk.dispatched(), 0);
    assert!(!frozen::Outcome::<bool, ()>::done(false).revokes_authority());
}

#[test]
fn terminal_receipt_retires_decide_and_cancel_custody_against_model() {
    for cancel in [false, true] {
        let mut state = State::new(Rules::Chosen, Bounds::production());
        let owner = admitted(&mut state, None, Class::Capability, 1);
        let child = admitted(&mut state, Some(owner), Class::Capability, 2);
        let mut carried = custody(owner);
        carried.acknowledge_live();
        assert!(carried.invoke_prompt(frozen::DecisionSequence::FIRST));
        assert!(!carried.invoke_prompt(frozen::DecisionSequence::FIRST));
        apply(&mut state, Step::Prompt(owner));
        assert!(carried.answer(frozen::DecisionSequence::FIRST, ()));
        apply(
            &mut state,
            Step::Decide {
                of: owner,
                complete: true,
            },
        );
        if cancel {
            assert!(carried.cancel(frozen::CallbackCancelReason::HookThrew));
            assert!(carried.decide().is_none());
            apply(
                &mut state,
                Step::Cancel {
                    of: owner,
                    throw: true,
                },
            );
        }
        apply(&mut state, Step::Controls);
        assert_eq!(
            state.outcome(owner),
            Some(if cancel {
                Outcome::Failed
            } else {
                Outcome::Done
            })
        );
        let terminal = if cancel {
            frozen::Outcome::failed(())
        } else {
            frozen::Outcome::done(())
        };
        assert!(outcome_class_matches(
            &terminal,
            state.outcome(owner).unwrap()
        ));
        assert_eq!(
            state.entry(child).unwrap().parent,
            None,
            "completion/HookThrew never cancels a detached child"
        );
        assert_eq!(state.carry_len(), 1);
        apply(&mut state, Step::Page);
        assert!(carried.receive_terminal());
        assert!(!carried.receive_terminal());
        assert!(carried.decide().is_none());
        assert!(carried.cancellation().is_none());
        assert_eq!(state.carry_len(), 0);
        for _ in 0..50 {
            apply(
                &mut state,
                if cancel {
                    Step::Cancel {
                        of: owner,
                        throw: true,
                    }
                } else {
                    Step::Decide {
                        of: owner,
                        complete: false,
                    }
                },
            );
            apply(&mut state, Step::Controls);
            apply(&mut state, Step::Page);
            assert!(!carried.answer(frozen::DecisionSequence::FIRST, ()));
            assert!(!carried.cancel(frozen::CallbackCancelReason::HookThrew));
        }
        assert_eq!(state.settlements(owner), 1);
        assert!(state.late_controls() >= 50);
        apply(&mut state, Step::Watermark);
        assert_eq!(state.retained(), 0);
    }
}

#[test]
fn acknowledged_payload_retirement_does_not_erase_hybrid_parent_custody() {
    let parent = request(1);
    let mut carried: CallbackCustody<()> = CallbackCustody::new(
        request(2),
        PendingPayload::new(vec![1, 2], 1, None).unwrap(),
        Some(parent),
    );
    carried.acknowledge_live();
    assert!(carried.payload().is_none());
    assert_eq!(carried.local_parent(), Some(parent));
    assert!(carried.cancel(frozen::CallbackCancelReason::ParentAborted));
    assert!(carried.receive_terminal());
    assert!(carried.local_parent().is_none());
    // The oracle's pending/lost/deferred cancellation compositions exercise
    // the separate never-admit/credit semantics using its existing fixtures.
    for profile in [
        seat_model::Profile::V5_03PendingCancel,
        seat_model::Profile::V5_03CreditDeferred,
    ] {
        let mut state = State::new(Rules::Chosen, profile.bounds());
        for step in profile.prefix() {
            apply(&mut state, step);
        }
    }
}

#[test]
fn byte_custody_uses_capacity_and_independent_gather_settings_limit() {
    let buffer = Vec::with_capacity(
        frozen::MAX_START_CHARGE - frozen::START_METADATA - frozen::HOOK_CHARGE * frozen::MAX_HOOKS,
    );
    let payload = PendingPayload::new(
        buffer,
        frozen::MAX_HOOKS,
        Some(frozen::GATHER_SETTINGS_MAX_BYTES),
    )
    .unwrap();
    assert_eq!(payload.charge(), frozen::MAX_START_CHARGE);
    assert!(
        payload.bytes().is_empty(),
        "capacity, not length, was charged"
    );
    assert_eq!(
        PendingPayload::new(Vec::with_capacity(frozen::MAX_START_CHARGE), 0, None).unwrap_err(),
        frozen::Refusal::ArgsTooLarge
    );
    assert_eq!(
        PendingPayload::new(Vec::new(), frozen::MAX_HOOKS + 1, None).unwrap_err(),
        frozen::Refusal::ArgsTooLarge
    );
    assert_eq!(
        PendingPayload::new(Vec::new(), 0, Some(frozen::GATHER_SETTINGS_MAX_BYTES + 1))
            .unwrap_err(),
        frozen::Refusal::ArgsTooLarge
    );
    assert!(
        PendingPayload::new(vec![0; frozen::GATHER_SETTINGS_MAX_BYTES + 1], 0, None).is_ok(),
        "4 KiB is not a general start cap"
    );
}

#[test]
fn gameplay_overflow_cannot_displace_reserved_release_or_control_rows() {
    let mut state = State::new(Rules::Chosen, Bounds::production());
    let owner = admitted(&mut state, None, Class::Capability, 1);
    apply(&mut state, Step::Prompt(owner));
    apply(
        &mut state,
        Step::Decide {
            of: owner,
            complete: false,
        },
    );
    for _ in 0..frozen::MAX_INTERACT_REQS {
        apply(
            &mut state,
            Step::Emit {
                of: None,
                release: false,
            },
        );
    }
    apply(
        &mut state,
        Step::Emit {
            of: Some(owner),
            release: true,
        },
    );
    apply(&mut state, Step::Take);
    assert_eq!(
        state
            .encoded_rows()
            .iter()
            .filter(|(_, release)| !release)
            .count(),
        frozen::MAX_ENCODED_GAMEPLAY
    );
    assert!(state.encoded_rows().contains(&(Some(owner), true)));
    assert_eq!(state.carry_len(), 1);
    apply(&mut state, Step::Dispatch);
    assert_eq!(state.release_dispatched(), 1);
    apply(&mut state, Step::Controls);
    assert_eq!(state.entry(owner).unwrap().decisions, 1);
}

#[test]
fn guardian_hold_drops_gameplay_but_preserves_release_and_immediate_host_actions() {
    for family in [SessionFamily::Gather, SessionFamily::Hunt] {
        let mut state = State::new(Rules::Chosen, Bounds::production());
        let session = admitted(&mut state, None, Class::Session(family), 1);
        apply(
            &mut state,
            Step::Emit {
                of: Some(session),
                release: false,
            },
        );
        apply(
            &mut state,
            Step::Emit {
                of: Some(session),
                release: true,
            },
        );
        apply(&mut state, Step::Take);
        apply(&mut state, Step::SessionHeld);
        let foreground = state.foreground_session();
        apply(
            &mut state,
            Step::Emit {
                of: Some(session),
                release: false,
            },
        );
        apply(&mut state, Step::HostAction { action: 1 });
        apply(&mut state, Step::HostAction { action: 2 });
        assert_eq!(state.host_actions_dispatched(), 2);
        assert_eq!(state.dispatched(), 0);
        assert_eq!(state.live(), 1);
        apply(&mut state, Step::SessionResumed);
        assert_eq!(state.foreground_session(), foreground);
        apply(&mut state, Step::Dispatch);
        assert_eq!(state.release_dispatched(), 1);
        assert_eq!(state.dispatched(), 0);
        assert_eq!(
            frozen::dispatch_disposition(
                frozen::EmissionClass::Host,
                None,
                foreground.map(request),
                frozen::AdmissionWitness::Pending,
                frozen::EmissionProof::InvalidExternal,
                frozen::AuthorityBoundary::Revoked,
                true
            ),
            frozen::DispatchDisposition::Dispatch
        );
    }
}

#[test]
fn kept_reconnect_carries_controls_and_releases_without_replaying_gameplay() {
    let mut state = State::new(Rules::Chosen, Bounds::production());
    let owner = admitted(&mut state, None, Class::Capability, 1);
    apply(&mut state, Step::Prompt(owner));
    apply(
        &mut state,
        Step::Decide {
            of: owner,
            complete: false,
        },
    );
    apply(
        &mut state,
        Step::Emit {
            of: Some(owner),
            release: false,
        },
    );
    apply(
        &mut state,
        Step::Emit {
            of: Some(owner),
            release: true,
        },
    );
    apply(&mut state, Step::Take);
    apply(&mut state, Step::Reconnect { keep: true });
    assert_eq!(state.live(), 1);
    assert_eq!(state.carry_len(), 1);
    apply(&mut state, Step::Dispatch);
    assert_eq!(state.dispatched(), 0);
    assert_eq!(state.release_dispatched(), 1);
    apply(&mut state, Step::Controls);
    assert_eq!(state.entry(owner).unwrap().decisions, 1);
}

#[test]
fn class_corrections_and_open_row_8a_are_explicit_not_activated() {
    assert_eq!(
        NativeKind::BankNearest.policy().class(),
        frozen::RequestClass::Capability
    );
    assert_eq!(
        NativeKind::PeriodicBank.policy().class(),
        frozen::RequestClass::Capability
    );
    assert_eq!(
        NativeKind::QuestProgress.policy().class(),
        frozen::RequestClass::Session(frozen::SessionFamily::Progress)
    );
    let open: Vec<_> = DIVERGENCES.iter().filter(|row| !row.approved()).collect();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id(), "8a");
    assert_eq!(open[0].status(), DivergenceStatus::OpenRow8a);
    assert!(DIVERGENCES.iter().all(|row| !row.source().is_empty()));
    assert_eq!(DIVERGENCES.len(), 31);
    assert!(!DIVERGENCES.iter().any(|row| row.id() == "29"));
}

#[test]
fn notification_custody_invokes_each_sequence_once_without_tick_coalescing() {
    let mut carried = custody(1);
    assert!(!carried.consume_event(0));
    for sequence in 1..=7 {
        assert!(carried.consume_event(sequence));
        assert!(!carried.consume_event(sequence));
    }
    assert!(!carried.consume_event(3));
    assert!(carried.invoke_prompt(frozen::DecisionSequence::FIRST));
    assert!(carried.answer(frozen::DecisionSequence::FIRST, ()));
    carried.decisions_consumed(1);
    assert!(
        !carried.answer(frozen::DecisionSequence::FIRST, ()),
        "an acknowledged answer cannot be carried again"
    );
    assert!(carried.receive_terminal());
    assert!(!carried.consume_event(8));
    assert!(!carried.invoke_prompt(frozen::DecisionSequence::FIRST));
    assert!(!carried.answer(frozen::DecisionSequence::FIRST, ()));
}

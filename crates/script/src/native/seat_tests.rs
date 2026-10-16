use super::*;
use api::identity::{ExecutionId, RequestId};

fn run() -> RunKey {
    RunKey {
        slot: 1,
        run: 7,
        session: 0,
    }
}

fn id(raw: u64) -> RequestId {
    RequestId::isolate(raw).unwrap()
}

fn stamp(raw: u64, parent: Option<u64>) -> RequestStamp {
    RequestStamp::new(id(raw), parent.map(id), None, ExecutionId::first(run())).unwrap()
}

fn start(seat: &mut GameplaySeat<u32>, raw: u64, parent: Option<u64>, kind: NativeKind) {
    assert_eq!(
        seat.start(stamp(raw, parent), kind, false).unwrap(),
        Admission::Admitted
    );
}

#[test]
fn seat_adopts_all_children_and_preserves_started_slot_rule() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    start(&mut seat, 2, Some(1), NativeKind::Walk);
    start(&mut seat, 3, Some(2), NativeKind::BankNearest);
    seat.complete(id(2), false).unwrap();
    let adopted = seat.entry(id(3)).unwrap();
    assert_eq!(adopted.parent, Some(id(1)));
    assert_eq!(adopted.origin, ChildOrigin::Adopted);
    assert_eq!(adopted.depth, 2);
    start(&mut seat, 4, Some(1), NativeKind::Shop);
    assert_eq!(
        seat.start(stamp(5, Some(1)), NativeKind::Modals, false)
            .unwrap(),
        Admission::Refused(Refusal::ParentBusy)
    );
    seat.complete(id(1), false).unwrap();
    assert!(seat.entry(id(3)).unwrap().parent.is_none());
    assert!(seat.entry(id(4)).unwrap().parent.is_none());
    assert_eq!(seat.entry(id(3)).unwrap().depth, 1);
}

#[test]
fn seat_enforces_root_leaf_reservation_and_projected_replacement() {
    let mut seat = GameplaySeat::<u32>::new(run());
    for index in 1..=4 {
        start(&mut seat, index, None, NativeKind::BankNearest);
    }
    assert_eq!(
        seat.start(stamp(5, None), NativeKind::Shop, false).unwrap(),
        Admission::Refused(Refusal::Busy)
    );
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 4, None, NativeKind::BankNearest);
    start(&mut seat, 5, None, NativeKind::BankNearest);
    start(&mut seat, 6, None, NativeKind::BankNearest);
    start(&mut seat, 1, None, NativeKind::Hunt);
    start(&mut seat, 2, Some(1), NativeKind::Walk);
    start(&mut seat, 3, Some(2), NativeKind::Shop);
    assert_eq!(
        seat.start(stamp(7, Some(1)), NativeKind::Fire, false)
            .unwrap(),
        Admission::Refused(Refusal::ParentBusy)
    );
    seat.complete(id(2), false).unwrap();
    assert_eq!(
        seat.start(stamp(8, Some(1)), NativeKind::Fire, false)
            .unwrap(),
        Admission::Refused(Refusal::Busy)
    );
}

#[test]
fn seat_refused_root_never_earns_post_terminal_owner_authority() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    assert_eq!(
        seat.start(stamp(2, None), NativeKind::BankNearest, false)
            .unwrap(),
        Admission::Refused(Refusal::Busy)
    );
    assert_eq!(
        seat.push(
            17,
            EmissionClass::Gameplay,
            EmissionSource::Owner(id(2)),
            None
        ),
        Err(SeatError::ProtocolDefect),
        "a retained refusal does not turn a never-admitted owner into a root"
    );
    assert_eq!(seat.dropped(), 1);
    seat.complete(id(1), false).unwrap();
    seat.push(
        18,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.take();
    let mut dispatched = Vec::new();
    seat.dispatch_into(&mut dispatched);
    assert_eq!(
        dispatched,
        [18],
        "ordinary completion can still rebase to root"
    );
}

#[test]
fn seat_revokes_adopted_subtree_leaf_first_without_reparenting_to_strangers() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    start(&mut seat, 2, Some(1), NativeKind::Walk);
    start(&mut seat, 3, Some(2), NativeKind::BankNearest);
    seat.complete(id(2), false).unwrap();
    seat.revoke(id(1), AbortReason::UserInput).unwrap();
    assert_eq!(
        seat.results().iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![id(2), id(3), id(1)]
    );
    assert_eq!(
        seat.results().last().unwrap().outcome,
        SeatTerminal::Aborted(AbortReason::UserInput)
    );
    assert_eq!(
        seat.results()[1].outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );
}

#[test]
fn seat_pending_owner_rows_wait_for_admission_and_keep_push_time_session() {
    let mut seat = GameplaySeat::<u32>::new(run());
    seat.push(
        1,
        EmissionClass::Gameplay,
        EmissionSource::Pending {
            owner: id(1),
            parent: None,
            kind: NativeKind::Gather,
        },
        None,
    )
    .unwrap();
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert!(delivered.is_empty());
    seat.push(
        2,
        EmissionClass::Gameplay,
        EmissionSource::Pending {
            owner: id(1),
            parent: None,
            kind: NativeKind::Gather,
        },
        None,
    )
    .unwrap();
    start(&mut seat, 1, None, NativeKind::Gather);
    seat.take();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [2]);
    seat.push(3, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [2]);
}

#[test]
fn seat_pending_mirror_row_waits_for_its_nearest_seat_ancestor() {
    let mut seat = GameplaySeat::<u32>::new(run());
    seat.push(
        5,
        EmissionClass::Gameplay,
        EmissionSource::PendingMirror {
            owner: id(90),
            pending: id(1),
            parent: None,
            kind: NativeKind::Gather,
        },
        None,
    )
    .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    seat.push(
        6,
        EmissionClass::Gameplay,
        EmissionSource::PendingMirror {
            owner: id(90),
            pending: id(1),
            parent: None,
            kind: NativeKind::Gather,
        },
        None,
    )
    .unwrap();
    start(&mut seat, 1, None, NativeKind::Gather);
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [6]);
}

#[test]
fn seat_completion_preserves_pinned_rows_but_late_push_becomes_root() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Shop);
    seat.push(
        1,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.complete(id(1), false).unwrap();
    seat.push(2, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [1, 2]);
    assert!(!seat.results()[0].authority_revoked);
}

#[test]
fn seat_root_gameplay_queued_during_foreground_stays_blocked_after_terminal() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Gather);
    seat.push(10, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.push(
        11,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.complete(id(1), false).unwrap();
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [11], "the accepted owner lease survives Done");
    assert_eq!(seat.dropped(), 1, "the old root cannot leak after Done");

    seat.push(12, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [11, 12], "a new root is unowned after Done");
}

#[test]
fn seat_delayed_wire_row_retains_session_after_owner_completion() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Gather);
    start(&mut seat, 2, Some(1), NativeKind::BankPlan);
    seat.complete(id(2), false).unwrap();
    seat.complete(id(1), false).unwrap();
    assert_eq!(seat.results()[0].session, Some(id(1)));
    // The isolate emitted this while 2 was live, but the batch arrived
    // after both ordinary terminals. Physical lineage is still 2.
    seat.push(
        8,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(2)),
        None,
    )
    .unwrap();
    start(&mut seat, 3, None, NativeKind::Combat);
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(
        output.is_empty(),
        "the later Combat foreground excludes Gather"
    );
    assert_eq!(seat.dropped(), 1);
}

#[test]
fn seat_revocation_drops_pinned_rows_in_queue_and_wire_even_after_terminal_retirement() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Shop);
    seat.push(
        1,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.take();
    seat.push(
        2,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.revoke(id(1), AbortReason::Superseded).unwrap();
    seat.watermark(1);
    seat.dispatch_into(&mut Vec::new());
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert!(delivered.is_empty());
    assert_eq!(seat.dropped(), 2);
}

#[test]
fn seat_release_reservation_bypasses_foreground_and_coalesces() {
    use api::script_contract::Release;
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Gather);
    for value in 0..MAX_INTERACT_REQS {
        seat.push(
            value as u32,
            EmissionClass::Gameplay,
            EmissionSource::Root,
            None,
        )
        .unwrap();
    }
    seat.push(
        500,
        EmissionClass::Release,
        EmissionSource::Owner(id(1)),
        Some(Release::AbortWalk(id(3))),
    )
    .unwrap();
    seat.push(
        501,
        EmissionClass::Release,
        EmissionSource::Owner(id(1)),
        Some(Release::AbortWalk(id(3))),
    )
    .unwrap();
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [500]);
    assert_eq!(seat.dropped(), MAX_INTERACT_REQS as u64);
}

#[test]
fn seat_guardian_hold_freezes_forest_controls_and_pages_but_drops_gameplay() {
    use api::script_contract::Release;
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Gather);
    seat.push(
        1,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.push(
        2,
        EmissionClass::Release,
        EmissionSource::Owner(id(1)),
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.take();
    seat.hold(true);
    seat.push(
        3,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.complete(id(1), false).unwrap();
    seat.cancel(id(1), CallbackCancelReason::ParentAborted)
        .unwrap();
    seat.watermark(1);
    assert_eq!(seat.live().len(), 1);
    assert!(seat.results().is_empty());
    seat.hold(false);
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [2]);
    assert_eq!(seat.dropped(), 2);
}

#[test]
fn seat_controls_cancel_first_and_ignore_late_duplicates() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    assert_eq!(seat.prompt(id(1)).unwrap(), Some(1));
    seat.cancel(id(1), CallbackCancelReason::HookThrew).unwrap();
    seat.decide(id(1), 1);
    seat.cancel(id(1), CallbackCancelReason::HookThrew).unwrap();
    assert_eq!(seat.late_controls(), 2);
    assert_eq!(seat.results()[0].outcome, SeatTerminal::Failed);
}

#[test]
fn seat_credit_deferral_then_watermark_and_cancelled_pending_result() {
    let mut seat = GameplaySeat::<u32>::new(run());
    for value in 1..=MAX_RESULTS as u64 {
        assert_eq!(
            seat.start(stamp(value, None), NativeKind::Shop, false)
                .unwrap(),
            Admission::Admitted
        );
        seat.complete(id(value), false).unwrap();
    }
    assert_eq!(seat.results().len(), MAX_RESULTS);
    assert_eq!(
        seat.start(stamp(33, Some(1)), NativeKind::Walk, true)
            .unwrap(),
        Admission::Deferred
    );
    seat.watermark(1);
    assert_eq!(
        seat.start(stamp(33, Some(1)), NativeKind::Walk, true)
            .unwrap(),
        Admission::Cancelled
    );
    assert_eq!(
        seat.results().last().unwrap().outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );
}

#[test]
fn seat_kept_reconnect_preserves_terminal_credit_and_release_but_drops_gameplay() {
    use api::script_contract::Release;
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    seat.push(
        1,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    seat.push(
        2,
        EmissionClass::Release,
        EmissionSource::Owner(id(1)),
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.take();
    seat.complete(id(1), false).unwrap();
    seat.reconnect(1);
    assert_eq!(seat.run().session, 1);
    assert_eq!(seat.results().len(), 1);
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [2]);
    assert_eq!(seat.dropped(), 1);
}

#[test]
fn seat_stop_is_silent_reset_clears_generation_and_host_actions_skip_seat() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    seat.stop();
    assert!(seat.live().is_empty() && seat.results().is_empty());
    assert_eq!(
        seat.start(stamp(2, None), NativeKind::Shop, false).unwrap(),
        Admission::Stale
    );
    seat.reset(run());
    start(&mut seat, 2, None, NativeKind::Shop);
    assert_eq!(seat.live().len(), 1);
}

#[test]
fn seat_walk_drive_waits_for_earlier_reserved_release() {
    use api::script_contract::Release;
    let mut seat = GameplaySeat::<u32>::new(run());
    for value in 1..=MAX_WALK_RELEASES as u64 {
        start(&mut seat, value, None, NativeKind::Walk);
        seat.complete(id(value), false).unwrap();
    }
    assert_eq!(
        seat.start(stamp(17, None), NativeKind::Walk, false)
            .unwrap(),
        Admission::Deferred
    );
    seat.push(
        1,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(id(1))),
    )
    .unwrap();
    seat.push(
        2,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(id(1))),
    )
    .unwrap();
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [1]);
    assert_eq!(
        seat.start(stamp(17, None), NativeKind::Walk, false)
            .unwrap(),
        Admission::Admitted
    );
}

#[test]
fn seat_invalid_external_gameplay_is_defective_even_during_guardian_hold() {
    let mut seat = GameplaySeat::<u32>::new(run());
    seat.hold(true);
    assert_eq!(
        seat.push(
            1,
            EmissionClass::Gameplay,
            EmissionSource::InvalidExternal(id(99)),
            None,
        ),
        Err(SeatError::ProtocolDefect)
    );
    assert_eq!(seat.dropped(), 1);
}

#[test]
fn seat_reset_advances_generation_floor_without_reusing_request_ids() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Shop);
    seat.complete(id(1), false).unwrap();
    let mut next = run();
    next.session += 1;
    seat.reset(next);
    let old = RequestStamp::new(id(1), None, None, ExecutionId::first(run())).unwrap();
    assert_eq!(
        seat.start(old, NativeKind::Shop, false).unwrap(),
        Admission::Stale
    );
    let child = RequestStamp::new(id(2), Some(id(1)), None, ExecutionId::first(next)).unwrap();
    assert_eq!(
        seat.start(child, NativeKind::Shop, false).unwrap(),
        Admission::Refused(Refusal::HistoryExpired)
    );
    let fresh = RequestStamp::new(id(3), None, None, ExecutionId::first(next)).unwrap();
    assert_eq!(
        seat.start(fresh, NativeKind::Shop, false).unwrap(),
        Admission::Admitted
    );
}

#[test]
fn seat_wire_walk_row_waits_for_release_without_losing_its_payload() {
    use api::script_contract::Release;
    let mut seat = GameplaySeat::<u32>::new(run());
    for value in 1..=MAX_WALK_RELEASES as u64 {
        assert!(seat.reserve_walk(id(value)));
    }
    seat.push_walk(17, EmissionSource::Root, id(17)).unwrap();
    seat.take();
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert!(delivered.is_empty());
    seat.push(
        1,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(id(1))),
    )
    .unwrap();
    seat.take();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [1]);
    seat.take();
    seat.dispatch_into(&mut delivered);
    assert_eq!(delivered, [1, 17]);
}

#[test]
fn seat_mirror_revoke_after_take_invalidates_pinned_wire_row() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Gather);
    seat.push(
        2,
        EmissionClass::Gameplay,
        EmissionSource::Mirror {
            owner: id(2),
            parent: Some(id(1)),
        },
        None,
    )
    .unwrap();
    seat.take();
    assert!(seat.revoke_mirror_owner(id(2)));
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert!(delivered.is_empty());
    assert_eq!(seat.dropped(), 1);
    assert!(seat.revoke_mirror_owner(id(2)));
    assert_eq!(seat.dropped(), 1, "resent revoke is idempotent");
}

#[test]
fn seat_mirror_cascade_revokes_each_descendant_wire_lease() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    for owner in [2, 3] {
        seat.push(
            owner as u32,
            EmissionClass::Gameplay,
            EmissionSource::Mirror {
                owner: id(owner),
                parent: Some(id(1)),
            },
            None,
        )
        .unwrap();
    }
    seat.take();
    assert!(seat.revoke_mirror_owner(id(2)));
    assert!(seat.revoke_mirror_owner(id(3)));
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert!(delivered.is_empty());
    assert_eq!(seat.dropped(), 2);
}

#[test]
fn seat_mirror_revoke_level_retries_and_rejects_gaps_atomically() {
    let mut seat = GameplaySeat::<u32>::new(run());
    seat.push(
        1,
        EmissionClass::Gameplay,
        EmissionSource::Mirror {
            owner: id(2),
            parent: None,
        },
        None,
    )
    .unwrap();
    seat.take();
    assert_eq!(
        seat.validate_mirror_revokes([(id(2), 1), (id(3), 3)]),
        Err(SeatError::ProtocolDefect)
    );
    assert_eq!(seat.mirror_revokes_consumed(), 0);
    seat.validate_mirror_revokes([(id(2), 1)]).unwrap();
    assert!(seat.apply_mirror_revoke(id(2), 1).unwrap());
    let mut delivered = Vec::new();
    seat.dispatch_into(&mut delivered);
    assert!(delivered.is_empty());
    assert_eq!(seat.dropped(), 1);
    seat.validate_mirror_revokes([(id(2), 1)]).unwrap();
    assert!(seat.apply_mirror_revoke(id(2), 1).unwrap());
    assert_eq!(seat.late_controls(), 1);
    seat.reconnect(1);
    assert_eq!(seat.mirror_revokes_consumed(), 1);
    let mut next = run();
    next.session = 2;
    seat.reset(next);
    assert_eq!(seat.mirror_revokes_consumed(), 0);
}

#[test]
fn seat_matches_chosen_model_for_completed_child_lease_and_foreground() {
    use seat_model::{Bounds, Class, Rules, SessionFamily, State, Step};

    let mut model = State::new(Rules::Chosen, Bounds::production());
    let mut seat = GameplaySeat::<u32>::new(run());
    model
        .apply(&Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Session(SessionFamily::Gather),
            group: 0,
            bytes: 0,
            hooks: 0,
        })
        .unwrap();
    model.apply(&Step::Admit(1)).unwrap();
    start(&mut seat, 1, None, NativeKind::Gather);
    model
        .apply(&Step::Start {
            parent: Some(1),
            local_parent: None,
            class: Class::Capability,
            group: 1,
            bytes: 0,
            hooks: 0,
        })
        .unwrap();
    model.apply(&Step::Admit(2)).unwrap();
    start(&mut seat, 2, Some(1), NativeKind::Walk);
    model
        .apply(&Step::Emit {
            of: Some(2),
            release: false,
        })
        .unwrap();
    seat.push(
        9,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(2)),
        None,
    )
    .unwrap();
    model.apply(&Step::Complete(2)).unwrap();
    seat.complete(id(2), false).unwrap();
    model.apply(&Step::Take).unwrap();
    seat.take();
    model.apply(&Step::Dispatch).unwrap();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [9]);
    assert_eq!(model.dispatched(), output.len());
    assert_eq!(model.dropped(), seat.dropped() as usize);
    model.check().unwrap();
}

#[test]
fn seat_matches_chosen_model_for_capability_before_session_filter() {
    use seat_model::{Bounds, Class, Rules, SessionFamily, State, Step};

    let mut model = State::new(Rules::Chosen, Bounds::production());
    let mut seat = GameplaySeat::<u32>::new(run());
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
    model.apply(&Step::Admit(1)).unwrap();
    start(&mut seat, 1, None, NativeKind::Walk);
    model
        .apply(&Step::Emit {
            of: Some(1),
            release: false,
        })
        .unwrap();
    seat.push(
        3,
        EmissionClass::Gameplay,
        EmissionSource::Owner(id(1)),
        None,
    )
    .unwrap();
    model
        .apply(&Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Session(SessionFamily::Gather),
            group: 0,
            bytes: 0,
            hooks: 0,
        })
        .unwrap();
    model.apply(&Step::Admit(2)).unwrap();
    start(&mut seat, 2, None, NativeKind::Gather);
    model.apply(&Step::Take).unwrap();
    seat.take();
    model.apply(&Step::Dispatch).unwrap();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    assert_eq!(model.dispatched(), output.len());
    assert_eq!(model.dropped(), seat.dropped() as usize);
    model.check().unwrap();
}

#[test]
fn run_policy_uses_reserved_row_and_coalesces_latest_at_encoder_capacity() {
    let mut seat = GameplaySeat::<u32>::new(run());
    for value in 0..MAX_ENCODED_GAMEPLAY {
        seat.push(
            value as u32,
            EmissionClass::Gameplay,
            EmissionSource::Root,
            None,
        )
        .unwrap();
    }
    seat.push(900, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.push_policy(901, EmissionSource::Root).unwrap();
    seat.push_policy(902, EmissionSource::Root).unwrap();
    seat.take();
    let mut dispatched = Vec::new();
    seat.dispatch_into(&mut dispatched);
    assert_eq!(dispatched.len(), MAX_ENCODED_GAMEPLAY + 1);
    assert_eq!(dispatched[0], 902);
    assert_eq!(seat.dropped(), 2);
}

#[test]
fn guardian_frees_unarmed_walk_reservation_but_keeps_armed_and_queued_releases() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Walk);
    seat.push_walk(10, EmissionSource::Owner(id(1)), id(10))
        .unwrap();
    seat.take();
    assert_eq!(seat.reserved_releases(), 2);
    assert!(seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(10) && slot.state == WalkReleaseState::Reserved));
    seat.hold(true);
    assert!(seat.walk_releases.iter().all(Option::is_none));
    seat.hold(false);
    start(&mut seat, 2, None, NativeKind::Walk);

    seat.push_walk(20, EmissionSource::Owner(id(1)), id(20))
        .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [20]);
    assert_eq!(seat.reserved_releases(), 2);
    assert!(seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(20) && slot.state == WalkReleaseState::Armed));
    seat.hold(true);
    assert!(seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(20) && slot.state == WalkReleaseState::Armed));
    seat.hold(false);

    start(&mut seat, 3, None, NativeKind::Walk);
    assert!(seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(3) && slot.state == WalkReleaseState::Reserved));
    seat.reconnect(1);
    assert_eq!(seat.reserved_releases(), 1);
    assert!(!seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(3)));
    assert!(seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(20) && slot.state == WalkReleaseState::Armed));
    seat.push(
        21,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(api::script_contract::Release::AbortWalk(id(20))),
    )
    .unwrap();
    seat.hold(true);
    assert_eq!(seat.reserved_releases(), 1);
    assert!(seat
        .walk_releases
        .iter()
        .flatten()
        .any(|slot| slot.id == id(20) && slot.state == WalkReleaseState::Queued));
    seat.hold(false);
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [20, 21]);
    assert!(seat.walk_releases.iter().all(Option::is_none));
}

#[test]
fn native_prompt_custody_freezes_on_hold_survives_kept_reconnect_and_clears_on_reset() {
    let mut seat = GameplaySeat::<u32>::new(run());
    start(&mut seat, 1, None, NativeKind::Hunt);
    let point = PromptPoint::from_contract_row(
        crate::native::abi::HOOK_CHARACTERIZATIONS
            .iter()
            .find(|row| row.member() == "sustain")
            .expect("awaited sustain contract row"),
    )
    .expect("awaited hook point");
    let first = DecisionSequence::FIRST;
    assert_eq!(seat.prompt_with_details(id(1), point, &[1, 2]), Ok(first));
    seat.hold(true);
    assert_eq!(
        seat.prompt_with_details(id(1), point, &[]),
        Err(SeatError::Held)
    );
    assert_eq!(
        seat.decide_with_answer(id(1), first, vec![7]),
        SeatDecisionStatus::Held
    );
    assert_eq!(seat.prompt_payload(id(1)).unwrap().args, [1, 2]);
    seat.hold(false);
    seat.reconnect(run().session + 1);
    let prompt = seat.prompt_payload(id(1)).expect("kept prompt");
    assert_eq!(
        (prompt.seq, prompt.point.member(), prompt.args.as_slice()),
        (first, "sustain", &[1, 2][..])
    );
    assert_eq!(
        seat.decide_with_answer(id(1), first, vec![7]),
        SeatDecisionStatus::Consumed
    );
    assert_eq!(seat.prompt_payload(id(1)).map(|prompt| prompt.seq), None);
    assert_eq!(seat.take_answer(id(1)).unwrap().answer, [7]);
    assert_eq!(
        seat.prompt_with_details(id(1), point, &[3]),
        Ok(first.next().unwrap())
    );
    seat.reset(run());
    assert!(seat.prompt_payload(id(1)).is_none());
    assert!(seat.take_answer(id(1)).is_none());
    assert!(seat.live().is_empty());
}

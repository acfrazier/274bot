use super::*;
use api::identity::{ExecutionId, RequestId};
use api::script_contract::Release;

fn fixture_run() -> RunKey {
    RunKey {
        slot: 13,
        run: 28,
        session: 0,
    }
}

fn fixture_id(raw: u64) -> RequestId {
    RequestId::isolate(raw).unwrap()
}

fn fixture_stamp(raw: u64, parent: Option<u64>) -> RequestStamp {
    RequestStamp::new(
        fixture_id(raw),
        parent.map(fixture_id),
        None,
        ExecutionId::first(fixture_run()),
    )
    .unwrap()
}

fn fixture_start(seat: &mut GameplaySeat<u32>, raw: u64, parent: Option<u64>, kind: NativeKind) {
    assert_eq!(
        seat.start(fixture_stamp(raw, parent), kind, false).unwrap(),
        Admission::Admitted,
        "start {raw} as {kind:?} under {parent:?}"
    );
}

fn fixture_push_owner(seat: &mut GameplaySeat<u32>, payload: u32, owner: u64) {
    seat.push(
        payload,
        EmissionClass::Gameplay,
        EmissionSource::Owner(fixture_id(owner)),
        None,
    )
    .unwrap();
}

/// §28.7 fixture 1: exercise the seat-visible prompt/Decide/terminal receipt.
/// `sustain` cadence and Hunt/JS `countKill` behavior are outside GameplaySeat.
#[test]
fn fixture_01_prompt_decide_and_terminal_receipt() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);

    assert_eq!(seat.prompt(fixture_id(1)).unwrap(), Some(1));
    seat.decide(fixture_id(1), 1);
    assert_eq!(seat.live()[0].decisions_consumed, 1);
    assert_eq!(seat.live()[0].prompt, None);

    seat.complete(fixture_id(1), false).unwrap();
    assert_eq!(seat.results()[0].outcome, SeatTerminal::Done);
    seat.watermark(1);
    assert!(seat.results().is_empty());
}

/// §28.7 fixture 2: test exclusive replacement, ParentBusy, and root capacity.
/// Awaited `leave`, Promise ordering, and family operations are not seat APIs.
#[test]
fn fixture_02_replacement_parent_busy_and_root_capacity() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Shop);
    fixture_start(&mut seat, 2, None, NativeKind::Shop);
    assert_eq!(
        seat.results()[0].outcome,
        SeatTerminal::Aborted(AbortReason::Superseded)
    );
    assert_eq!(seat.live().len(), 1);

    fixture_start(&mut seat, 3, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 4, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 5, None, NativeKind::BankNearest);
    assert_eq!(
        seat.start(fixture_stamp(6, None), NativeKind::BankNearest, false)
            .unwrap(),
        Admission::Refused(Refusal::Busy)
    );

    let mut child_seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut child_seat, 1, None, NativeKind::Hunt);
    fixture_start(&mut child_seat, 2, Some(1), NativeKind::Walk);
    assert_eq!(
        child_seat
            .start(fixture_stamp(3, Some(1)), NativeKind::Fire, false)
            .unwrap(),
        Admission::Refused(Refusal::ParentBusy)
    );
}

/// §28.7 fixture 3: takeover results and captured gameplay leases are distinct.
/// JS settlement, held-click, physical host release, and family takeover remain external.
#[test]
fn fixture_03_manual_takeover_revokes_rows_and_keeps_result_classes() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    fixture_start(&mut seat, 2, Some(1), NativeKind::Walk);
    fixture_start(&mut seat, 3, Some(2), NativeKind::BankNearest);
    fixture_push_owner(&mut seat, 20, 2);
    seat.push(
        15,
        EmissionClass::Lifecycle,
        EmissionSource::Owner(fixture_id(1)),
        None,
    )
    .unwrap();
    fixture_push_owner(&mut seat, 30, 3);
    seat.take();

    seat.manual_walk(fixture_id(2)).unwrap();
    assert_eq!(seat.results()[0].id, fixture_id(3));
    assert_eq!(
        seat.results()[0].outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );
    let walk = seat.results().last().unwrap();
    assert_eq!(walk.id, fixture_id(2));
    assert_eq!(walk.outcome, SeatTerminal::Done);
    assert!(walk.authority_revoked);
    assert_eq!(
        seat.start(fixture_stamp(4, Some(2)), NativeKind::BankNearest, false,)
            .unwrap(),
        Admission::Refused(Refusal::ParentNotLive)
    );
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [15], "lifecycle bypasses gameplay revocation");
    assert_eq!(seat.dropped(), 2);
    seat.push(
        16,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(fixture_id(2))),
    )
    .unwrap();
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [15, 16], "reserved release bypasses foreground");
    assert_eq!(seat.dropped(), 2);

    let mut combat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut combat, 10, None, NativeKind::Combat);
    fixture_start(&mut combat, 11, Some(10), NativeKind::Walk);
    combat
        .revoke(fixture_id(10), AbortReason::UserInput)
        .unwrap();
    assert_eq!(combat.results()[0].id, fixture_id(11));
    assert_eq!(
        combat.results()[1].outcome,
        SeatTerminal::Aborted(AbortReason::UserInput)
    );
}

/// §28.7 fixture 4: kept reconnect drops gameplay but retains terminals/releases.
/// `SceneReady`, re-arm, deadlines, and per-family reconnect execution are external.
#[test]
fn fixture_04_kept_reconnect_preserves_release_and_terminal() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Walk);
    fixture_push_owner(&mut seat, 10, 1);
    seat.push(
        11,
        EmissionClass::Release,
        EmissionSource::Owner(fixture_id(1)),
        Some(Release::AbortWalk(fixture_id(1))),
    )
    .unwrap();
    seat.take();
    seat.complete(fixture_id(1), false).unwrap();

    seat.reconnect(4);
    assert_eq!(seat.run().session, 4);
    assert_eq!(seat.results().len(), 1);
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [11]);
    assert_eq!(seat.dropped(), 1);
}

/// §28.7 fixture 5: test the real root, lineage-depth, and leaf bounds.
/// The 257 callback cap, progress ring, generated-table failures, and product-adapter
/// bounds are not represented by GameplaySeat.
#[test]
fn fixture_05_forest_root_and_depth_bounds() {
    let mut roots = GameplaySeat::<u32>::new(fixture_run());
    for raw in 1..=MAX_ROOTS as u64 {
        fixture_start(&mut roots, raw, None, NativeKind::BankNearest);
    }
    assert_eq!(
        roots
            .start(
                fixture_stamp(MAX_ROOTS as u64 + 1, None),
                NativeKind::BankNearest,
                false,
            )
            .unwrap(),
        Admission::Refused(Refusal::Busy)
    );

    let mut full = GameplaySeat::<u32>::new(fixture_run());
    let chains = [
        (
            1,
            2,
            3,
            4,
            NativeKind::Walk,
            NativeKind::Shop,
            NativeKind::Fire,
        ),
        (
            5,
            6,
            7,
            8,
            NativeKind::Dialogue,
            NativeKind::MakeX,
            NativeKind::PartnerTrade,
        ),
        (
            9,
            10,
            11,
            12,
            NativeKind::Teleport,
            NativeKind::Reach,
            NativeKind::Prayer,
        ),
        (
            13,
            14,
            15,
            16,
            NativeKind::Autocast,
            NativeKind::Special,
            NativeKind::Modals,
        ),
    ];
    for (root, child, grandchild, leaf, child_kind, grandchild_kind, leaf_kind) in chains {
        fixture_start(&mut full, root, None, NativeKind::BankNearest);
        fixture_start(&mut full, child, Some(root), child_kind);
        fixture_start(&mut full, grandchild, Some(child), grandchild_kind);
        fixture_start(&mut full, leaf, Some(grandchild), leaf_kind);
    }
    assert_eq!(full.live().len(), MAX_LIVE);
    fixture_start(&mut full, 17, Some(3), NativeKind::Fire);
    assert_eq!(full.live().len(), MAX_LIVE);
    assert_eq!(
        full.results()
            .iter()
            .find(|result| result.id == fixture_id(4))
            .unwrap()
            .outcome,
        SeatTerminal::Aborted(AbortReason::Superseded)
    );
    assert_eq!(
        full.start(fixture_stamp(18, Some(3)), NativeKind::Clue, false)
            .unwrap(),
        Admission::Refused(Refusal::ParentBusy),
        "the started-child slot takes precedence over the full-seat bound"
    );
    assert_eq!(full.live().len(), MAX_LIVE);

    let mut depth = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut depth, 1, None, NativeKind::BankNearest);
    fixture_start(&mut depth, 2, Some(1), NativeKind::Walk);
    fixture_start(&mut depth, 3, Some(2), NativeKind::Shop);
    fixture_start(&mut depth, 4, Some(3), NativeKind::Fire);
    assert_eq!(depth.live()[3].depth as usize, MAX_LINEAGE_DEPTH);
    assert_eq!(
        depth
            .start(fixture_stamp(5, Some(4)), NativeKind::Clue, false)
            .unwrap(),
        Admission::Refused(Refusal::LineageDepth)
    );
}

/// §28.7 fixture 6: direct stamped parent attribution is the reachable seat part.
/// `await Execution.delayTicks(1)` and CPED continuation propagation need the engine.
#[test]
fn fixture_06_stamped_child_keeps_its_parent() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    let child = seat
        .live()
        .iter()
        .find(|entry| entry.id == fixture_id(2))
        .unwrap();
    assert_eq!(child.parent, Some(fixture_id(1)));
    assert_eq!(child.depth, 2);
}

/// §28.7 fixture 7: a completed intermediate promotes its child; a later child
/// can start and both survive root completion. JS scheduling/result values are external.
#[test]
fn fixture_07_sequential_child_starts_and_root_promotion() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, Some(1), NativeKind::Walk);
    fixture_start(&mut seat, 3, Some(2), NativeKind::Shop);
    seat.complete(fixture_id(2), false).unwrap();
    fixture_start(&mut seat, 4, Some(1), NativeKind::Fire);

    seat.complete(fixture_id(1), false).unwrap();
    for raw in [3, 4] {
        let entry = seat
            .live()
            .iter()
            .find(|entry| entry.id == fixture_id(raw))
            .unwrap();
        assert_eq!(entry.parent, None);
        assert_eq!(entry.depth, 1);
    }
}

/// §28.7 fixture 8: check the foreground predicate in both start orders.
/// `loop`/tick-listener/delayUntil root scheduling and family callbacks need JS.
#[test]
fn fixture_08_session_foreground_matches_family_and_start_order() {
    let families = [
        NativeKind::Hunt,
        NativeKind::Gather,
        NativeKind::Combat,
        NativeKind::QuestProgress,
    ];
    for (index, family) in families.iter().copied().enumerate() {
        let first_id = 1 + index as u64 * 10;
        let session_id = first_id + 1;
        let mut capability_first = GameplaySeat::<u32>::new(fixture_run());
        fixture_start(
            &mut capability_first,
            first_id,
            None,
            NativeKind::BankNearest,
        );
        fixture_push_owner(&mut capability_first, 80, first_id);
        fixture_start(&mut capability_first, session_id, None, family);
        capability_first.take();
        let mut output = Vec::new();
        capability_first.dispatch_into(&mut output);
        if family == NativeKind::Hunt {
            assert_eq!(output, [80]);
            assert_eq!(capability_first.dropped(), 0);
        } else {
            assert!(output.is_empty());
            assert_eq!(capability_first.dropped(), 1);
        }

        fixture_push_owner(&mut capability_first, 81, first_id);
        capability_first.take();
        capability_first.dispatch_into(&mut output);
        if family == NativeKind::Hunt {
            assert_eq!(output, [80, 81]);
            assert_eq!(capability_first.dropped(), 0);
        } else {
            assert!(output.is_empty());
            assert_eq!(capability_first.dropped(), 2);
        }
        let expected_foreground = if family == NativeKind::Hunt {
            None
        } else {
            Some(fixture_id(session_id))
        };
        assert_eq!(capability_first.foreground(), expected_foreground);

        let mut session_first = GameplaySeat::<u32>::new(fixture_run());
        fixture_start(&mut session_first, first_id, None, family);
        assert_eq!(
            session_first
                .start(
                    fixture_stamp(session_id, None),
                    NativeKind::BankNearest,
                    false,
                )
                .unwrap(),
            Admission::Refused(Refusal::Busy)
        );
        session_first
            .push(82, EmissionClass::Gameplay, EmissionSource::Root, None)
            .unwrap();
        session_first.take();
        let mut session_output = Vec::new();
        session_first.dispatch_into(&mut session_output);
        if family == NativeKind::Hunt {
            assert_eq!(session_output, [82]);
        } else {
            assert!(session_output.is_empty());
            assert_eq!(session_first.dropped(), 1);
        }
    }
}

/// §28.7 fixture 9: the seat can preserve an ordinary accepted lease; it cannot
/// execute V8/CPED or prove continuation-preserved embedder data.
#[test]
fn fixture_09_ordinary_completion_keeps_accepted_owner_authority() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    fixture_push_owner(&mut seat, 20, 2);
    seat.complete(fixture_id(2), false).unwrap();
    seat.complete(fixture_id(1), false).unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [20]);
    assert_eq!(seat.dropped(), 0);
}

/// §28.7 fixture 9b: a root emission stays an explicit root at the seat boundary.
/// Exception unwinding, Deno termination, and the next V8 `enter_root` probe are
/// not observable through GameplaySeat.
#[test]
fn fixture_09b_root_emission_is_not_reconstructed_from_a_terminal() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Shop);
    seat.complete(fixture_id(1), false).unwrap();
    seat.push(90, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [90]);
}

/// §28.7 fixture 10: held seat state is frozen, held gameplay is dropped, and
/// releases remain dispatchable. Real pause verbs/lifecycle interruption are external.
#[test]
fn fixture_10_guardian_hold_freezes_requests_and_keeps_releases() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    assert_eq!(seat.prompt(fixture_id(1)).unwrap(), Some(1));
    fixture_push_owner(&mut seat, 10, 1);
    seat.take();
    seat.push(
        12,
        EmissionClass::Release,
        EmissionSource::Owner(fixture_id(1)),
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.hold(true);
    fixture_push_owner(&mut seat, 11, 1);
    seat.complete(fixture_id(1), false).unwrap();
    seat.cancel(fixture_id(1), CallbackCancelReason::HookThrew)
        .unwrap();
    seat.decide(fixture_id(1), 1);
    assert_eq!(
        seat.start(fixture_stamp(2, None), NativeKind::Shop, false)
            .unwrap(),
        Admission::Deferred
    );
    seat.push(
        12,
        EmissionClass::Release,
        EmissionSource::Owner(fixture_id(1)),
        Some(Release::GatherStop),
    )
    .unwrap();
    assert_eq!(seat.live().len(), 1);
    assert!(seat.results().is_empty());
    assert_eq!(seat.foreground(), Some(fixture_id(1)));
    assert_eq!(seat.dropped(), 2);

    let mut output = Vec::new();
    seat.take();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    seat.hold(false);
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [12], "the release queued before hold survives once");
    assert_eq!(seat.live().len(), 1);
    assert_eq!(seat.live()[0].prompt, Some(1));
    assert_eq!(seat.live()[0].decisions_consumed, 0);
}

/// §28.7 fixture 11: HookThrew is Failed, not revocation; detached children live.
/// Thrown-value identity, `afterDeposit`, and bank-family behavior need product hooks.
#[test]
fn fixture_11_hook_throw_fails_only_the_named_entry() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Shop);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    fixture_push_owner(&mut seat, 20, 2);
    seat.cancel(fixture_id(1), CallbackCancelReason::HookThrew)
        .unwrap();
    assert_eq!(seat.results()[0].outcome, SeatTerminal::Failed);
    assert_eq!(seat.live().len(), 1);
    assert_eq!(seat.live()[0].id, fixture_id(2));
    assert_eq!(seat.live()[0].parent, None);
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [20]);
}

/// §28.7 fixture 12: reconnect drops Shop-independent gameplay but keeps releases.
/// Shop partial-transfer totals and post-relog family clicks need the real adapter.
#[test]
fn fixture_12_reconnect_keeps_release_and_drops_gameplay() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Shop);
    fixture_push_owner(&mut seat, 10, 1);
    seat.push(
        11,
        EmissionClass::Release,
        EmissionSource::Owner(fixture_id(1)),
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.take();
    seat.reconnect(8);
    assert_eq!(seat.live().len(), 1);
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [11]);
    assert_eq!(seat.dropped(), 1);
}

/// §28.7 fixture 13: prompt and Decide state survive kept reconnect.
/// Page/keyframe loss and real hook invocation counts are not seat state.
#[test]
fn fixture_13_prompt_and_decide_survive_kept_reconnect() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    assert_eq!(seat.prompt(fixture_id(1)).unwrap(), Some(1));
    seat.reconnect(1);
    assert_eq!(seat.live()[0].prompt, Some(1));
    assert_eq!(seat.live()[0].decisions_consumed, 0);
    seat.decide(fixture_id(1), 1);
    assert_eq!(seat.live()[0].prompt, None);
    assert_eq!(seat.live()[0].decisions_consumed, 1);
}

/// §28.7 fixture 13b: terminal receipt retires result state and a late Decide counts.
/// Dropped acknowledgement pages, exactly-once JS settlement, and logs are external.
#[test]
fn fixture_13b_terminal_receipt_retires_decide_and_counts_late_copy() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    assert_eq!(seat.prompt(fixture_id(1)).unwrap(), Some(1));
    seat.decide(fixture_id(1), 1);
    seat.complete(fixture_id(1), false).unwrap();
    seat.watermark(1);
    assert!(seat.results().is_empty());
    seat.decide(fixture_id(1), 1);
    assert_eq!(seat.late_controls(), 1);
}

/// §28.7 fixture 13c: Cancel is applied before the now-late Decide.
/// Real throw identity, wire order, and duplicate-adapter behavior need I-wire.
#[test]
fn fixture_13c_cancel_replaces_pending_decide() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    assert_eq!(seat.prompt(fixture_id(1)).unwrap(), Some(1));
    seat.cancel(fixture_id(1), CallbackCancelReason::ParentAborted)
        .unwrap();
    seat.decide(fixture_id(1), 1);
    assert_eq!(
        seat.results()[0].outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );
    assert_eq!(seat.late_controls(), 1);
}

/// §28.7 fixture 14: terminal sequence retirement is monotonic at the seat.
/// Sticky `acknowledged_ever`, keyframe delivery, and start replay are adapter state.
#[test]
fn fixture_14_watermark_retires_only_acknowledged_results() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    seat.complete(fixture_id(1), false).unwrap();
    fixture_start(&mut seat, 2, None, NativeKind::BankNearest);
    seat.complete(fixture_id(2), false).unwrap();
    seat.watermark(1);
    assert_eq!(seat.results().len(), 1);
    assert_eq!(seat.results()[0].id, fixture_id(2));
    seat.watermark(2);
    assert!(seat.results().is_empty());
}

/// §28.7 fixture 15: a prior accepted lease is independently checked against the
/// later foreground; a late root action is not reconstructed from its old owner.
/// Cross-thread promise/gate timing requires the actual host and isolate.
#[test]
fn fixture_15_completion_and_late_push_dispatch_windows() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    fixture_push_owner(&mut seat, 20, 2);
    seat.complete(fixture_id(2), false).unwrap();
    seat.complete(fixture_id(1), false).unwrap();
    fixture_start(&mut seat, 3, None, NativeKind::Gather);
    seat.push(30, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    assert_eq!(seat.dropped(), 2);
    assert_eq!(seat.foreground(), Some(fixture_id(3)));

    let mut revoked = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut revoked, 10, None, NativeKind::BankNearest);
    fixture_push_owner(&mut revoked, 100, 10);
    revoked.take();
    revoked
        .revoke(fixture_id(10), AbortReason::Superseded)
        .unwrap();
    revoked.watermark(1);
    let mut revoked_output = Vec::new();
    revoked.dispatch_into(&mut revoked_output);
    assert!(revoked_output.is_empty());
    assert_eq!(revoked.dropped(), 1);
}

/// §28.7 fixture 15: an accepted pre-completion Owner row keeps its pin, but
/// `Owner(id)` pushed after ordinary completion must be treated as a new root.
#[test]
fn fixture_15_post_completion_owner_push_is_root() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    fixture_push_owner(&mut seat, 20, 2);
    seat.complete(fixture_id(2), false).unwrap();
    fixture_push_owner(&mut seat, 21, 2);

    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [20]);
    assert_eq!(seat.dropped(), 1);
}

/// §28.7 fixture 16: ManualWalk retains Done while revoking the captured
/// Session-descendant lease and aborting the child.
#[test]
fn fixture_16_manual_walk_keeps_done_but_revokes_session_lease() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    fixture_start(&mut seat, 2, Some(1), NativeKind::Walk);
    fixture_start(&mut seat, 3, Some(2), NativeKind::BankNearest);
    fixture_push_owner(&mut seat, 30, 3);
    seat.take();
    seat.manual_walk(fixture_id(2)).unwrap();
    let walk = seat
        .results()
        .iter()
        .find(|result| result.id == fixture_id(2))
        .unwrap();
    assert_eq!(walk.outcome, SeatTerminal::Done);
    assert!(walk.authority_revoked);
    assert_eq!(walk.session, Some(fixture_id(1)));
    assert_eq!(
        seat.results()
            .iter()
            .find(|result| result.id == fixture_id(3))
            .unwrap()
            .outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    assert_eq!(seat.dropped(), 1);
}

/// §28.7 fixture 17: root completion promotes every direct child independently.
/// Actual import/pump/listener/delayUntil root behavior requires CPED and product JS.
#[test]
fn fixture_17_root_completion_promotes_all_children() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, Some(1), NativeKind::Walk);
    fixture_start(&mut seat, 3, Some(2), NativeKind::Shop);
    seat.complete(fixture_id(2), false).unwrap();
    fixture_start(&mut seat, 4, Some(1), NativeKind::Fire);
    seat.complete(fixture_id(1), false).unwrap();

    assert_eq!(seat.live().len(), 2);
    for raw in [3, 4] {
        let child = seat
            .live()
            .iter()
            .find(|entry| entry.id == fixture_id(raw))
            .unwrap();
        assert_eq!(child.parent, None);
        assert_eq!(child.depth, 1);
    }
}

/// §28.7 fixture 18: the explicit RequestStamp parent is represented correctly.
/// AsyncVariable scope/derived values and once-per-run Deno logging are unavailable here.
#[test]
fn fixture_18_stamped_child_does_not_invent_async_scope() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    let stamp = fixture_stamp(2, Some(1));
    assert_eq!(
        seat.start(stamp, NativeKind::BankNearest, false).unwrap(),
        Admission::Admitted
    );
    assert_eq!(seat.live()[1].parent, Some(fixture_id(1)));
    assert_eq!(seat.live()[1].origin, ChildOrigin::Started);
}

/// §28.7 fixture 19: explicit Root and Owner sources stay distinct.
/// `queueMicrotask` inheritance under hooks/`loop()` requires the locked engine.
#[test]
fn fixture_19_root_and_hook_emissions_keep_distinct_sources() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    fixture_push_owner(&mut seat, 10, 1);
    seat.push(11, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [10, 11]);
}

/// §28.7 fixture 20: same-group nested replacement at the four-root boundary.
/// Four-root family flows and encoded refusal rows belong to product/I-wire.
#[test]
fn fixture_20_full_roots_allow_projected_nested_replacement() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, Some(1), NativeKind::Shop);
    fixture_start(&mut seat, 3, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 4, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 5, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 6, Some(1), NativeKind::Shop);
    assert_eq!(
        seat.results()
            .iter()
            .find(|result| result.id == fixture_id(2))
            .unwrap()
            .outcome,
        SeatTerminal::Aborted(AbortReason::Superseded)
    );
    let replacement = seat
        .live()
        .iter()
        .find(|entry| entry.id == fixture_id(6))
        .unwrap();
    assert_eq!(replacement.parent, Some(fixture_id(1)));
    assert_eq!(
        seat.live()
            .iter()
            .filter(|entry| entry.parent.is_none())
            .count(),
        MAX_ROOTS
    );
}

/// §28.7 fixture 21: completing an inclusive ancestor adopts the descendant
/// without mutating an unrelated root.
/// Real Banking.open() ancestry and adapter-side local-parent links are external.
#[test]
fn fixture_21_inclusive_ancestry_adopts_without_stranger_reparenting() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, Some(1), NativeKind::Walk);
    fixture_start(&mut seat, 3, Some(2), NativeKind::Shop);
    fixture_start(&mut seat, 4, None, NativeKind::BankNearest);
    seat.complete(fixture_id(2), false).unwrap();
    let adopted = seat
        .live()
        .iter()
        .find(|entry| entry.id == fixture_id(3))
        .unwrap();
    assert_eq!(adopted.parent, Some(fixture_id(1)));
    assert_eq!(adopted.origin, ChildOrigin::Adopted);
    seat.complete(fixture_id(1), false).unwrap();
    assert!(seat
        .live()
        .iter()
        .any(|entry| entry.id == fixture_id(3) && entry.parent.is_none()));
    assert!(seat
        .live()
        .iter()
        .any(|entry| entry.id == fixture_id(4) && entry.parent.is_none()));
}

/// §28.7 fixture 22: an adopted child consumes the last leaf reservation; a new
/// sibling is refused Busy, while the allowed children survive later promotion.
/// `runMachine` Promise settlement order is not seat state.
#[test]
fn fixture_22_adopted_child_uses_future_root_reservation() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 3, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 4, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 5, Some(1), NativeKind::Walk);
    fixture_start(&mut seat, 6, Some(5), NativeKind::Shop);
    seat.complete(fixture_id(5), false).unwrap();
    assert_eq!(
        seat.live()
            .iter()
            .find(|entry| entry.id == fixture_id(6))
            .unwrap()
            .origin,
        ChildOrigin::Adopted
    );
    assert_eq!(
        seat.start(fixture_stamp(7, Some(1)), NativeKind::Fire, false)
            .unwrap(),
        Admission::Refused(Refusal::Busy)
    );
}

/// §28.7 fixture 23: seat admission remains family-neutral and accepts each
/// declared NativeKind as a root; per-member JS paths/coercion are not exposed.
#[test]
fn fixture_23_all_native_kinds_are_admitted_as_roots() {
    for kind in NativeKind::ALL {
        let mut seat = GameplaySeat::<u32>::new(fixture_run());
        assert_eq!(
            seat.start(fixture_stamp(1, None), kind, false).unwrap(),
            Admission::Admitted,
            "{kind:?}"
        );
        assert_eq!(seat.live()[0].kind, kind);
    }
}

/// §28.7 fixture 24: result credits retire through a watermark and release rows
/// encode before gameplay. Real page snapshots, promise settlement, and auto-AbortWalk
/// generation require the page/family/host layers.
#[test]
fn fixture_24_terminal_credit_watermark_and_release_delivery() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    for raw in 1..=MAX_RESULTS as u64 {
        fixture_start(&mut seat, raw, None, NativeKind::BankNearest);
        seat.complete(fixture_id(raw), false).unwrap();
    }
    assert_eq!(
        seat.start(
            fixture_stamp(MAX_RESULTS as u64 + 1, None),
            NativeKind::BankNearest,
            false,
        )
        .unwrap(),
        Admission::Deferred
    );
    seat.watermark(1);
    fixture_start(
        &mut seat,
        MAX_RESULTS as u64 + 1,
        None,
        NativeKind::BankNearest,
    );

    let mut releases = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut releases, 1, None, NativeKind::Hunt);
    releases
        .push(50, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    releases
        .push(
            51,
            EmissionClass::Release,
            EmissionSource::Root,
            Some(Release::AbortWalk(fixture_id(99))),
        )
        .unwrap();
    releases.take();
    let mut output = Vec::new();
    releases.dispatch_into(&mut output);
    assert_eq!(output, [51, 50]);
}

/// §28.7 fixture 24: four independent Walk-headed forests stay admitted.
/// Manual takeover converts their depth-four trees to sixteen ordered
/// terminals without losing credits; the slot fixture proves physical releases.
#[test]
fn fixture_24_four_walk_heads_publish_sixteen_takeover_results() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    for root in [1, 5, 9, 13] {
        fixture_start(&mut seat, root, None, NativeKind::Walk);
        fixture_start(&mut seat, root + 1, Some(root), NativeKind::BankNearest);
        fixture_start(
            &mut seat,
            root + 2,
            Some(root + 1),
            NativeKind::PeriodicBank,
        );
        fixture_start(&mut seat, root + 3, Some(root + 2), NativeKind::Trade);
    }
    assert_eq!(seat.live().len(), MAX_LIVE);
    for root in [1, 5, 9, 13] {
        seat.manual_walk(fixture_id(root)).unwrap();
    }
    assert!(seat.live().is_empty());
    assert_eq!(seat.results().len(), MAX_LIVE);
    for (index, root) in [1, 5, 9, 13].into_iter().enumerate() {
        for offset in 0..3 {
            let result = seat.results()[index * 4 + offset];
            assert_eq!(result.id, fixture_id(root + 3 - offset as u64));
            assert_eq!(
                result.outcome,
                SeatTerminal::Aborted(AbortReason::ParentAborted)
            );
        }
        let result = seat.results()[index * 4 + 3];
        assert_eq!(result.id, fixture_id(root));
        assert_eq!(result.outcome, SeatTerminal::Done);
        assert!(result.authority_revoked);
    }
    for (ordinal, result) in seat.results().iter().enumerate() {
        assert_eq!(result.seq, ordinal as u64 + 1);
    }
    fixture_start(&mut seat, 17, None, NativeKind::BankNearest);
    seat.watermark(1);
    assert_eq!(seat.results().len(), MAX_LIVE - 1);
    assert_eq!(seat.live().len(), 1);
}

/// §28.7 fixture 24(b): a slow result consumer cannot let a seventeenth
/// replacement exceed the 32 shared credits while sixteen live entries stay
/// admitted. The deferred start retries at the first terminal watermark.
#[test]
fn fixture_24_full_live_and_slow_results_defer_seventeenth_supersession() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    for root in [1, 5, 9, 13] {
        fixture_start(&mut seat, root, None, NativeKind::BankNearest);
        fixture_start(&mut seat, root + 1, Some(root), NativeKind::PeriodicBank);
        fixture_start(&mut seat, root + 2, Some(root + 1), NativeKind::Trade);
        fixture_start(
            &mut seat,
            root + 3,
            Some(root + 2),
            if root == 1 {
                NativeKind::Prayer
            } else {
                NativeKind::BankNearest
            },
        );
    }
    assert_eq!(seat.live().len(), MAX_LIVE);
    for raw in 17..=32 {
        fixture_start(&mut seat, raw, Some(3), NativeKind::Prayer);
    }
    assert_eq!(seat.live().len(), MAX_LIVE);
    assert_eq!(seat.results().len(), MAX_RESULTS - MAX_LIVE);
    assert!(seat
        .results()
        .iter()
        .all(|result| { result.outcome == SeatTerminal::Aborted(AbortReason::Superseded) }));
    assert_eq!(
        seat.start(fixture_stamp(33, Some(3)), NativeKind::Prayer, false)
            .unwrap(),
        Admission::Deferred
    );
    seat.watermark(1);
    fixture_start(&mut seat, 33, Some(3), NativeKind::Prayer);
    assert_eq!(seat.live().len(), MAX_LIVE);
    assert_eq!(seat.results().len(), MAX_RESULTS - MAX_LIVE);
    assert!(seat.live().iter().any(|entry| entry.id == fixture_id(33)));
}

/// §28.7 fixture 25: credit deferral, cancellation, stale stamps, and a lower
/// unseen ID after a higher ID are all direct start cases. SceneReady/adapter carry
/// custody remains outside GameplaySeat.
#[test]
fn fixture_25_pending_start_credit_stale_and_out_of_order_ids() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    for raw in 1..=MAX_RESULTS as u64 {
        fixture_start(&mut seat, raw, None, NativeKind::BankNearest);
        seat.complete(fixture_id(raw), false).unwrap();
    }
    let pending_id = MAX_RESULTS as u64 + 1;
    assert_eq!(
        seat.start(fixture_stamp(pending_id, None), NativeKind::Shop, true)
            .unwrap(),
        Admission::Deferred
    );
    assert_eq!(seat.results().len(), MAX_RESULTS);
    seat.watermark(1);
    assert_eq!(
        seat.start(fixture_stamp(pending_id, None), NativeKind::Shop, true)
            .unwrap(),
        Admission::Cancelled
    );
    assert_eq!(
        seat.results().last().unwrap().outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );

    let mut stale_run = fixture_run();
    stale_run.session += 1;
    let stale =
        RequestStamp::new(fixture_id(100), None, None, ExecutionId::first(stale_run)).unwrap();
    assert_eq!(
        seat.start(stale, NativeKind::Shop, false).unwrap(),
        Admission::Stale
    );
    seat.watermark(2);
    fixture_start(&mut seat, 100, None, NativeKind::Shop);

    let mut reordered = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut reordered, 9, None, NativeKind::BankNearest);
    reordered.complete(fixture_id(9), false).unwrap();
    assert_eq!(reordered.lookup(fixture_id(7)), AuthorityStatus::Completed);
    assert_eq!(
        reordered
            .start(fixture_stamp(7, None), NativeKind::BankNearest, false)
            .unwrap(),
        Admission::Admitted,
        "Adup checks live/results, not the retained high-water lookup"
    );
}

/// §28.7 fixture 26: BankNearest is a Capability child and its action is filtered
/// by a Progress foreground. `leave`/progress reads and refusal projection are product APIs.
#[test]
fn fixture_26_bank_nearest_child_obeys_progress_foreground() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Shop);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    assert_eq!(seat.live()[1].parent, Some(fixture_id(1)));
    fixture_start(&mut seat, 3, None, NativeKind::QuestProgress);
    fixture_push_owner(&mut seat, 20, 2);
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    assert_eq!(seat.dropped(), 1);
    assert!(seat.live().iter().any(|entry| entry.id == fixture_id(2)));
}

/// §28.7 fixture 27: PendingMirror captures its nearest seat ancestry and admission
/// witness; an accepted Mirror lease survives parent completion and watermark.
/// Reverse family cutover and host dispatch need the integration adapter.
#[test]
fn fixture_27_pending_mirror_captures_nearest_seat_ancestor() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    fixture_start(&mut seat, 2, Some(1), NativeKind::BankNearest);
    seat.push(
        27,
        EmissionClass::Gameplay,
        EmissionSource::PendingMirror {
            owner: fixture_id(90),
            pending: fixture_id(3),
            parent: Some(fixture_id(2)),
            kind: NativeKind::Walk,
        },
        None,
    )
    .unwrap();
    seat.take();
    fixture_start(&mut seat, 3, Some(2), NativeKind::Walk);
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [27]);
    assert_eq!(seat.dropped(), 0);
    let mut completed_parent = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut completed_parent, 1, None, NativeKind::Gather);
    fixture_start(&mut completed_parent, 2, Some(1), NativeKind::BankNearest);
    completed_parent
        .push(
            28,
            EmissionClass::Gameplay,
            EmissionSource::Mirror {
                owner: fixture_id(91),
                parent: Some(fixture_id(2)),
            },
            None,
        )
        .unwrap();
    completed_parent.take();
    completed_parent.complete(fixture_id(2), false).unwrap();
    completed_parent.watermark(1);
    let mut completed_output = Vec::new();
    completed_parent.dispatch_into(&mut completed_output);
    assert_eq!(completed_output, [28]);
    assert_eq!(completed_parent.dropped(), 0);
}

/// §28.7 fixture 28: an admission witness set after Take still authorizes the
/// same in-flight row; an unadmitted row is dropped and never replayed.
/// Full decoded batch ordering and cancellation settlement require I-wire.
#[test]
fn fixture_28_pending_rows_require_admission_before_dispatch() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    seat.complete(fixture_id(1), false).unwrap();
    seat.watermark(1);

    seat.push(
        28,
        EmissionClass::Gameplay,
        EmissionSource::Pending {
            owner: fixture_id(2),
            parent: None,
            kind: NativeKind::Gather,
        },
        None,
    )
    .unwrap();
    seat.take();
    fixture_start(&mut seat, 2, None, NativeKind::Gather);
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [28]);

    seat.push(
        29,
        EmissionClass::Gameplay,
        EmissionSource::Pending {
            owner: fixture_id(3),
            parent: Some(fixture_id(2)),
            kind: NativeKind::BankNearest,
        },
        None,
    )
    .unwrap();
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [28]);
    fixture_start(&mut seat, 3, Some(2), NativeKind::BankNearest);
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [28]);
    assert_eq!(seat.dropped(), 1);
}

/// §28.7 fixture 29: test all 18 reserved release slots, release priority,
/// gameplay order, and the 141-row encoded gameplay cap. Physical arming,
/// FlatBuffers, and host consumption are outside this seat.
#[test]
fn fixture_29_release_priority_and_gameplay_encoder_cap() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    for raw in 1..=MAX_WALK_RELEASES as u64 {
        seat.push(
            raw as u32 - 1,
            EmissionClass::Release,
            EmissionSource::Root,
            Some(Release::AbortWalk(fixture_id(200 + raw))),
        )
        .unwrap();
    }
    seat.push(
        MAX_WALK_RELEASES as u32,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.push(
        MAX_WALK_RELEASES as u32 + 1,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::CombatStop),
    )
    .unwrap();
    for value in 0..(MAX_ENCODED_GAMEPLAY as u32 + 9) {
        seat.push(
            1_000 + value,
            EmissionClass::Gameplay,
            EmissionSource::Root,
            None,
        )
        .unwrap();
    }
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    let release_count = MAX_WALK_RELEASES + 2;
    let expected_releases = (0..release_count as u32).collect::<Vec<_>>();
    let expected_gameplay = (1_000..1_000 + MAX_ENCODED_GAMEPLAY as u32).collect::<Vec<_>>();
    assert_eq!(&output[..release_count], expected_releases.as_slice());
    assert_eq!(&output[release_count..], expected_gameplay.as_slice());
    assert_eq!(seat.dropped(), 9);
}

/// §28.7 fixture 30: 50 late copies of each control retire without reviving carry.
/// The separate slot consumer fixture checks the read-only SeatProbe and zero-log behavior.
#[test]
fn fixture_30_fifty_late_decide_and_cancel_copies_are_counted() {
    for cancel in [false, true] {
        let mut seat = GameplaySeat::<u32>::new(fixture_run());
        fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
        seat.complete(fixture_id(1), false).unwrap();
        seat.watermark(1);
        for _ in 0..50 {
            if cancel {
                seat.cancel(fixture_id(1), CallbackCancelReason::ParentAborted)
                    .unwrap();
            } else {
                seat.decide(fixture_id(1), 1);
            }
        }
        assert_eq!(seat.late_controls(), 50);
        assert!(seat.results().is_empty());
    }
}

/// §27 §6 `chosen_guardian_actions_dispatch_outside_session_queue`: with Gather
/// foreground, the seat rejects host rows; ordered execution before/after Stop
/// cannot be tested by GameplaySeat.
#[test]
fn guardian_gather_host_actions_are_rejected_at_the_seat_boundary() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, None, NativeKind::Gather);
    assert_eq!(seat.foreground(), Some(fixture_id(2)));
    assert_eq!(
        seat.push(5, EmissionClass::Host, EmissionSource::Root, None),
        Err(SeatError::InvalidAuthority)
    );
    seat.stop();
    assert_eq!(
        seat.push(6, EmissionClass::Host, EmissionSource::Root, None),
        Err(SeatError::InvalidAuthority)
    );
    assert!(!seat.has_rows());
}

/// §27 §6 `chosen_hunt_guardian_actions_dispatch_outside_session_queue`: Hunt has
/// no foreground, but host actions remain outside the seat API and are not dispatched here.
#[test]
fn guardian_hunt_host_actions_are_rejected_at_the_seat_boundary() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    fixture_start(&mut seat, 2, None, NativeKind::Hunt);
    assert_eq!(seat.foreground(), None);
    assert_eq!(
        seat.push(5, EmissionClass::Host, EmissionSource::Root, None),
        Err(SeatError::InvalidAuthority)
    );
    seat.stop();
    assert_eq!(
        seat.push(6, EmissionClass::Host, EmissionSource::Root, None),
        Err(SeatError::InvalidAuthority)
    );
    assert!(!seat.has_rows());
}

fn assert_guardian_hold_product_case(kind: NativeKind, expected_foreground: Option<RequestId>) {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, kind);
    assert_eq!(seat.foreground(), expected_foreground);
    assert_eq!(seat.prompt(fixture_id(1)).unwrap(), Some(1));

    fixture_push_owner(&mut seat, 1, 1);
    seat.take();
    seat.push(
        2,
        EmissionClass::Gameplay,
        EmissionSource::Pending {
            owner: fixture_id(2),
            parent: Some(fixture_id(1)),
            kind: NativeKind::BankNearest,
        },
        None,
    )
    .unwrap();
    seat.push(
        3,
        EmissionClass::Gameplay,
        EmissionSource::Mirror {
            owner: fixture_id(3),
            parent: Some(fixture_id(1)),
        },
        None,
    )
    .unwrap();
    seat.hold(true);
    assert_eq!(seat.dropped(), 3);

    fixture_push_owner(&mut seat, 4, 1);
    for (payload, source) in [
        (
            5,
            EmissionSource::Pending {
                owner: fixture_id(4),
                parent: Some(fixture_id(1)),
                kind: NativeKind::BankNearest,
            },
        ),
        (
            6,
            EmissionSource::Mirror {
                owner: fixture_id(5),
                parent: Some(fixture_id(1)),
            },
        ),
        (7, EmissionSource::Root),
    ] {
        seat.push(payload, EmissionClass::Gameplay, source, None)
            .unwrap();
    }
    assert_eq!(seat.dropped(), 7);
    assert!(!seat.has_rows());

    assert_eq!(
        seat.start(fixture_stamp(8, Some(1)), NativeKind::BankNearest, false)
            .unwrap(),
        Admission::Deferred
    );
    seat.cancel(fixture_id(8), CallbackCancelReason::ParentAborted)
        .unwrap();
    // Adapter-side Start+Cancel custody is not inspectable here; retry it with
    // `cancelled = true` after resume to exercise the seat's publication boundary.
    seat.push(
        100,
        EmissionClass::Release,
        EmissionSource::Owner(fixture_id(1)),
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.push(
        101,
        EmissionClass::Release,
        EmissionSource::Owner(fixture_id(1)),
        Some(Release::CombatStop),
    )
    .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    assert_eq!(seat.live().len(), 1);
    assert!(seat.results().is_empty());
    assert_eq!(seat.live()[0].prompt, Some(1));
    assert_eq!(seat.live()[0].decisions_consumed, 0);
    assert_eq!(seat.foreground(), expected_foreground);

    seat.hold(false);
    assert_eq!(
        seat.start(fixture_stamp(8, Some(1)), NativeKind::BankNearest, true)
            .unwrap(),
        Admission::Cancelled
    );
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [100, 101]);
    assert_eq!(seat.dropped(), 7);
    assert_eq!(seat.results().last().unwrap().id, fixture_id(8));
    assert_eq!(
        seat.results().last().unwrap().outcome,
        SeatTerminal::Aborted(AbortReason::ParentAborted)
    );
}

/// §27 §6 `chosen_guardian_hold_drops_gameplay_and_preserves_controls`: exactly
/// seven Gather-held gameplay rows drop, two releases survive, and foreground remains.
#[test]
fn guardian_gather_hold_drops_gameplay_and_preserves_controls() {
    assert_guardian_hold_product_case(NativeKind::Gather, Some(fixture_id(1)));
}

/// §27 §6 `chosen_hunt_guardian_hold_drops_gameplay_and_preserves_controls`: exactly
/// seven Hunt-held gameplay rows drop, two releases survive, and no foreground is created.
#[test]
fn guardian_hunt_hold_drops_gameplay_and_preserves_controls() {
    assert_guardian_hold_product_case(NativeKind::Hunt, None);
}

/// §27 §6 `chosen_mutation_held_cancel_executes_breaks_guardian_hold`: Cancel
/// must not complete or revoke a live request while held.
#[test]
fn guardian_mutation_held_cancel_is_frozen_until_resume() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    seat.hold(true);
    seat.cancel(fixture_id(1), CallbackCancelReason::HookThrew)
        .unwrap();
    assert_eq!(seat.live().len(), 1);
    assert!(seat.results().is_empty());
    seat.hold(false);
    seat.cancel(fixture_id(1), CallbackCancelReason::HookThrew)
        .unwrap();
    assert_eq!(seat.results()[0].outcome, SeatTerminal::Failed);
}

/// §27 §6 `chosen_mutation_held_decide_executes_breaks_guardian_hold`: Decide
/// remains pending through hold and is consumed only after resume.
#[test]
fn guardian_mutation_held_decide_is_frozen_until_resume() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Hunt);
    seat.prompt(fixture_id(1)).unwrap();
    seat.hold(true);
    seat.decide(fixture_id(1), 1);
    assert_eq!(seat.live()[0].prompt, Some(1));
    assert_eq!(seat.live()[0].decisions_consumed, 0);
    seat.hold(false);
    seat.decide(fixture_id(1), 1);
    assert_eq!(seat.live()[0].prompt, None);
    assert_eq!(seat.live()[0].decisions_consumed, 1);
}

/// §27 §6 `chosen_mutation_held_release_dispatch_breaks_guardian_hold`: an
/// enqueued release is retained, not dispatched, until the hold ends.
#[test]
fn guardian_mutation_held_release_waits_for_resume() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    seat.hold(true);
    seat.push(
        10,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
    seat.hold(false);
    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [10]);
}

/// §27 §6 `chosen_mutation_hold_clears_carry_breaks_control_custody`: the seat
/// has no Cancel-carry queue, so this checks the reachable retained-result ledger.
#[test]
fn guardian_mutation_hold_freezes_result_retirement() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    seat.complete(fixture_id(1), false).unwrap();
    seat.hold(true);
    seat.watermark(1);
    assert_eq!(seat.results().len(), 1);
    seat.hold(false);
    seat.watermark(1);
    assert!(seat.results().is_empty());
}

/// §27 §6 `chosen_mutation_hold_drops_pending_breaks_pending_start_custody`:
/// Deferred is not admission; the producer can retry the same ID after resume.
#[test]
fn guardian_mutation_hold_preserves_deferred_start_identity() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    seat.hold(true);
    assert_eq!(
        seat.start(fixture_stamp(2, Some(1)), NativeKind::BankNearest, false)
            .unwrap(),
        Admission::Deferred
    );
    assert_eq!(seat.lookup(fixture_id(2)), AuthorityStatus::Pending);
    seat.hold(false);
    assert_eq!(
        seat.start(fixture_stamp(2, Some(1)), NativeKind::BankNearest, false)
            .unwrap(),
        Admission::Admitted
    );
}

/// §27 §6 `chosen_mutation_resume_completes_foreground_breaks_guardian_hold`:
/// resume itself does not alter live entries, results, or foreground.
#[test]
fn guardian_mutation_resume_does_not_complete_foreground() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    seat.hold(true);
    seat.hold(false);
    assert_eq!(seat.foreground(), Some(fixture_id(1)));
    assert_eq!(seat.live().len(), 1);
    assert!(seat.results().is_empty());
}

/// §27 §6 `chosen_mutation_controls_clears_carry_breaks_control_custody`:
/// no control-carry API exists here; normal result retention remains testable.
#[test]
fn guardian_mutation_controls_do_not_clear_retained_results() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    seat.complete(fixture_id(1), false).unwrap();
    seat.watermark(0);
    assert_eq!(seat.results().len(), 1);
    seat.watermark(1);
    assert!(seat.results().is_empty());
}

/// §27 §6 `chosen_mutation_controls_drops_pending_breaks_pending_start_custody`:
/// the direct seat can report credit deferral, but pending serialization is adapter-owned.
#[test]
fn guardian_mutation_controls_leave_deferred_start_for_retry() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    for raw in 1..=MAX_RESULTS as u64 {
        fixture_start(&mut seat, raw, None, NativeKind::BankNearest);
        seat.complete(fixture_id(raw), false).unwrap();
    }
    let pending = MAX_RESULTS as u64 + 1;
    assert_eq!(
        seat.start(fixture_stamp(pending, None), NativeKind::Shop, false)
            .unwrap(),
        Admission::Deferred
    );
    seat.watermark(1);
    assert_eq!(
        seat.start(fixture_stamp(pending, None), NativeKind::Shop, false)
            .unwrap(),
        Admission::Admitted
    );
}

/// §27 §6 `chosen_mutation_duplicate_release_breaks_row_conservation`: duplicate
/// AbortWalk controls for the same owner coalesce to one physical row.
#[test]
fn guardian_mutation_duplicate_release_coalesces() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    seat.push(
        1,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(fixture_id(50))),
    )
    .unwrap();
    seat.push(
        2,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(fixture_id(50))),
    )
    .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [1]);
}

/// §27 §6 `chosen_mutation_held_tick_keeps_releases_breaks_row_conservation`:
/// HeldTick discards both queued and in-flight releases rather than retaining them.
#[test]
fn guardian_mutation_held_tick_discards_queued_and_wire_releases() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    seat.push(
        10,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::GatherStop),
    )
    .unwrap();
    seat.take();
    seat.push(
        11,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::CombatStop),
    )
    .unwrap();
    seat.held_tick();
    assert!(!seat.has_rows());
    assert_eq!(seat.dropped(), 2);
    assert_eq!(seat.live().len(), 1);
    let mut output = Vec::new();
    seat.take();
    seat.dispatch_into(&mut output);
    assert!(output.is_empty());
}

/// §27 §6 `chosen_mutation_held_gameplay_queue_breaks_row_conservation`: gameplay
/// pushed during hold is dropped immediately, while release custody remains queued.
#[test]
fn guardian_mutation_held_gameplay_never_enters_queue() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::BankNearest);
    seat.hold(true);
    fixture_push_owner(&mut seat, 10, 1);
    assert_eq!(seat.dropped(), 1);
    assert!(!seat.has_rows());
    seat.push(
        11,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::GatherStop),
    )
    .unwrap();
    assert!(seat.has_rows());
}

/// §27 §6 `chosen_session_capture_handoff_covers_all_family_pairs`: all sixteen
/// old/new Session pairs exercise captured authority against the new foreground.
/// The real family scheduler and host action stream are outside this seat.
#[test]
fn guardian_session_capture_handoff_covers_all_family_pairs() {
    let families = [
        NativeKind::Hunt,
        NativeKind::Gather,
        NativeKind::Combat,
        NativeKind::QuestProgress,
    ];
    for (old_index, old_family) in families.iter().copied().enumerate() {
        for (new_index, new_family) in families.iter().copied().enumerate() {
            let base = (old_index * families.len() + new_index) as u64 * 10;
            let root_id = base + 1;
            let first_id = base + 2;
            let second_id = base + 3;
            let mut seat = GameplaySeat::<u32>::new(fixture_run());
            fixture_start(&mut seat, root_id, None, NativeKind::BankNearest);
            fixture_start(&mut seat, first_id, None, old_family);
            fixture_push_owner(&mut seat, 1, first_id);
            seat.complete(fixture_id(first_id), false).unwrap();
            fixture_start(&mut seat, second_id, None, new_family);
            seat.take();
            let mut output = Vec::new();
            seat.dispatch_into(&mut output);

            let should_dispatch = new_family == NativeKind::Hunt;
            let expected_dispatched = if should_dispatch { 1 } else { 0 };
            let expected_dropped = if should_dispatch { 0 } else { 1 };
            assert_eq!(
                output.len(),
                expected_dispatched,
                "{old_family:?} -> {new_family:?}"
            );
            assert_eq!(
                seat.dropped(),
                expected_dropped,
                "{old_family:?} -> {new_family:?}"
            );
            assert!(seat
                .live()
                .iter()
                .any(|entry| entry.id == fixture_id(root_id)));
            assert!(seat
                .live()
                .iter()
                .any(|entry| entry.id == fixture_id(second_id)));
            let expected_foreground = if new_family == NativeKind::Hunt {
                None
            } else {
                Some(fixture_id(second_id))
            };
            assert_eq!(seat.foreground(), expected_foreground);
        }
    }
}

/// A root row pushed under Gather is checked against foreground only when dispatched.
#[test]
fn root_gameplay_row_uses_foreground_at_dispatch_time() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut seat, 1, None, NativeKind::Gather);
    seat.push(30, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.complete(fixture_id(1), false).unwrap();

    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);

    assert_eq!(output, [30]);
    assert_eq!(seat.dropped(), 0);
}

#[test]
fn normal_walk_terminals_free_slots_before_later_gameplay_dispatch() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    let mut walks = Vec::new();
    for raw in 1..=(MAX_WALK_RELEASES as u64 + 1) {
        fixture_start(&mut seat, raw, None, NativeKind::Walk);
        seat.push_walk(
            raw as u32,
            EmissionSource::Owner(fixture_id(raw)),
            fixture_id(raw),
        )
        .unwrap();
        seat.take();
        seat.dispatch_into(&mut walks);
        assert_eq!(
            seat.reserved_releases(),
            1,
            "walk {raw} keeps its reservation until terminal"
        );

        seat.complete(fixture_id(raw), false).unwrap();
        assert_eq!(
            seat.reserved_releases(),
            0,
            "normal terminal frees walk {raw}'s reservation"
        );
    }
    assert_eq!(walks.len(), MAX_WALK_RELEASES + 1);

    seat.push(200, EmissionClass::Gameplay, EmissionSource::Root, None)
        .unwrap();
    seat.take();
    let mut gameplay = Vec::new();
    seat.dispatch_into(&mut gameplay);
    assert_eq!(gameplay, [200]);
}

#[test]
fn walk_release_retirement_is_terminal_scoped_and_queue_safe() {
    let mut unrelated = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut unrelated, 1, None, NativeKind::BankNearest);
    unrelated
        .push(
            40,
            EmissionClass::Release,
            EmissionSource::Root,
            Some(Release::AbortWalk(fixture_id(1))),
        )
        .unwrap();
    unrelated.complete(fixture_id(1), false).unwrap();
    assert_eq!(unrelated.reserved_releases(), 1);

    unrelated.take();
    let mut output = Vec::new();
    unrelated.dispatch_into(&mut output);
    assert_eq!(output, [40]);
    assert_eq!(unrelated.reserved_releases(), 0);

    let mut walk = GameplaySeat::<u32>::new(fixture_run());
    fixture_start(&mut walk, 1, None, NativeKind::Walk);
    walk.push(
        41,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(fixture_id(1))),
    )
    .unwrap();
    walk.complete(fixture_id(1), false).unwrap();
    assert_eq!(
        walk.reserved_releases(),
        1,
        "a queued AbortWalk remains reserved through the walk terminal"
    );

    walk.take();
    let mut output = Vec::new();
    walk.dispatch_into(&mut output);
    assert_eq!(output, [41]);
    assert_eq!(walk.reserved_releases(), 0);
}

#[test]
fn physical_walk_terminal_retires_only_armed_unqueued_slot() {
    let mut seat = GameplaySeat::<u32>::new(fixture_run());
    assert!(seat.reserve_walk(fixture_id(50)));
    seat.push_walk(50, EmissionSource::Root, fixture_id(50))
        .unwrap();
    seat.take();
    let mut output = Vec::new();
    seat.dispatch_into(&mut output);
    assert_eq!(seat.reserved_releases(), 1);

    seat.walk_terminal(fixture_id(50));
    assert_eq!(seat.reserved_releases(), 0);

    assert!(seat.reserve_walk(fixture_id(51)));
    seat.walk_terminal(fixture_id(51));
    assert_eq!(
        seat.reserved_releases(),
        1,
        "a terminal cannot free a reservation that was never armed"
    );
    seat.push(
        51,
        EmissionClass::Release,
        EmissionSource::Root,
        Some(Release::AbortWalk(fixture_id(51))),
    )
    .unwrap();
    seat.walk_terminal(fixture_id(51));
    assert_eq!(
        seat.reserved_releases(),
        1,
        "a queued AbortWalk remains reserved"
    );

    seat.take();
    seat.dispatch_into(&mut output);
    assert_eq!(output, [50, 51]);
    assert_eq!(seat.reserved_releases(), 0);
}

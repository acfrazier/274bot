//! Tests of the hold model: hand-written scenarios, the findings at the
//! head commit, and the model builder's self-check mutations.

use hold_model::{
    Action, Begin, Bounds, Cause, Env, Info, Key, Kind, Limits, Machine, Model, Mutation, OwnerAct,
    State, Violation, explore_profile, profile, release_traces,
};

fn frame(tick: bool, wake: bool, drift: bool, act: OwnerAct) -> Action {
    Action::Frame(hold_model::FrameInput {
        tick,
        wake,
        drift,
        stall: false,
        act,
    })
}

/// Apply `actions` in order; every step must pass the invariants.
fn run(model: &Model, actions: &[Action]) -> (State, Vec<Info>) {
    let mut state = model.initial_state();
    let mut infos = Vec::new();
    for action in actions {
        let (next, info) = model
            .step_detail(&state, action)
            .unwrap_or_else(|v: Violation| panic!("{action:?}: {} {}", v.name, v.detail));
        state = next;
        infos.push(info);
    }
    (state, infos)
}

fn model() -> Model {
    Model::new(Bounds::standard(), Mutation::None)
}

#[test]
fn a_dialogue_wait_holds_the_press_until_its_page_is_seen_and_closed() {
    let (state, infos) = run(
        &model(),
        &[
            // The owner sends its Talk and takes a modal wait.
            frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
            // The page opens; the owner sees it at its next poll.
            Action::ModalOpen,
            frame(true, false, false, OwnerAct::Poll),
            Action::ModalClose,
            // No poll this frame: nothing holds, the host presses.
            frame(false, false, false, OwnerAct::NoPoll),
        ],
    );
    assert!(infos[0].deferred && !infos[0].pressed, "held at the send");
    assert_eq!(infos[2].releases, vec![(Kind::Modal, Cause::Page)]);
    assert!(!infos[2].pressed, "the open modal keeps the host idle");
    assert!(infos[4].pressed && !infos[4].deferred);
    assert!(state.run_on);
}

#[test]
fn the_host_ends_a_hung_hold_one_tick_after_the_owners_deadline() {
    let model = Model::new(
        Bounds {
            env: Env::NONE,
            ..Bounds::standard()
        },
        Mutation::None,
    );
    let (_, infos) = run(
        &model,
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
            Action::ModalOpen,
            frame(false, true, false, OwnerAct::Send),
            Action::ModalClose,
            Action::Wait,
            // The active acknowledgement reached its bound; the sample fixes the tick.
            frame(false, false, false, OwnerAct::NoPoll),
            // The next tick: the owner stalls, the host ends the hold.
            frame(true, false, false, OwnerAct::Stall),
        ],
    );
    assert!(infos[5].deferred, "the bound is reached, the margin is not");
    assert_eq!(infos[6].expired, Some((Key::Modal(2), 1, 2)));
    assert_eq!(infos[6].releases, vec![(Kind::Modal, Cause::HostTerminal)]);
    assert!(infos[6].pressed, "the terminal release lets the press out");
}

#[test]
fn a_paused_choice_or_main_acknowledgement_keeps_its_active_clock_hold() {
    let (_, infos) = run(
        &model(),
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
            Action::ModalOpen,
            frame(false, true, false, OwnerAct::Send),
            Action::Pause,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
        ],
    );
    assert!(
        infos[8].deferred && infos[9].deferred,
        "Pause stops the clock"
    );
    assert_eq!(infos[9].expired, None);
}

/// A known limit, not a defect: Open, Continue, the end gap and the journal
/// window use observed ticks. Pause does not stop those deadlines.
/// The host ends the hold one tick after the deadline and logs a failure,
/// although the owner is only paused.
#[test]
fn a_paused_dialogue_loses_a_tick_bounded_gap_hold() {
    let (_, infos) = run(
        &model(),
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
            Action::ModalOpen,
            frame(false, true, false, OwnerAct::Send),
            Action::ModalClose,
            // The acknowledgement arrived and the chat closed: the same
            // lease becomes a one-tick end gap, due at tick 2.
            frame(false, true, false, OwnerAct::ToGap(1)),
            Action::Pause,
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
        ],
    );
    assert!(infos[6].deferred, "the gap is held until its deadline");
    assert_eq!(infos[7].expired, Some((Key::Modal(2), 2, 3)));
    assert_eq!(infos[7].releases, vec![(Kind::Modal, Cause::HostTerminal)]);
}

#[test]
fn open_and_continue_waits_keep_their_tick_deadline_through_pause() {
    for continuation in [false, true] {
        let mut actions = vec![frame(true, false, false, OwnerAct::Begin(Begin::Modal))];
        if continuation {
            actions.extend([
                Action::ModalOpen,
                frame(false, true, false, OwnerAct::Continue),
                Action::ModalClose,
            ]);
        }
        actions.extend([
            Action::Pause,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
        ]);
        let (_, infos) = run(&model(), &actions);
        let last = infos.last().unwrap();
        assert!(
            infos[infos.len() - 2].deferred,
            "the deadline retains its margin"
        );
        let id = if continuation { 2 } else { 1 };
        assert_eq!(last.expired, Some((Key::Modal(id), 4, 5)));
        assert!(last.pressed && !last.deferred);
    }
}

#[test]
fn a_queued_modal_has_no_deadline_until_its_talk_is_accepted() {
    let model = model();
    let (state, infos) = run(
        &model,
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::QueuedModal)),
            Action::Pause,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
            frame(true, false, false, OwnerAct::NoPoll),
        ],
    );
    for info in infos.iter().filter(|info| info.wait.is_some()) {
        assert!(info.deferred && !info.pressed && info.expired.is_none());
    }
    assert_eq!(infos[0].wait, Some((Key::Modal(1), 1)));
    assert_eq!(infos[9].wait, Some((Key::Modal(1), 5)));
    assert!(matches!(
        state.lease,
        hold_model::Lease::Modal(hold_model::Modal {
            phase: hold_model::ModalPhase::AwaitingDispatch,
            ledger: hold_model::LedgerBound::AwaitingDispatch,
            ..
        })
    ));
    assert_eq!(state.due, None);
    let (resumed, _) = model.step_detail(&state, &Action::Resume).unwrap();
    let (accepted, info) = model
        .step_detail(&resumed, &frame(false, true, false, OwnerAct::Accept))
        .unwrap();
    assert!(matches!(
        accepted.lease,
        hold_model::Lease::Modal(hold_model::Modal {
            phase: hold_model::ModalPhase::Open { until: 8 },
            ledger: hold_model::LedgerBound::Tick(8),
            ..
        })
    ));
    assert_eq!(info.wait, Some((Key::Modal(1), 8)));
    assert!(info.deferred && info.expired.is_none());
}

/// The steps of finding F2: the journal read sends its click in a frame where
/// the lease's own 10 s wall bound passes between the owner's poll and the
/// host's sample. The read needs a pause first: its own bound on the active
/// clock (8 s) is shorter than the wall bound (10 s), unless the clock
/// stopped. The last step is the frame of the click.
fn f2_steps() -> [Action; 7] {
    [
        frame(true, false, false, OwnerAct::Begin(Begin::Quiet)),
        Action::Pause,
        Action::Wait,
        Action::Wait,
        Action::Wait,
        Action::Resume,
        frame(false, true, true, OwnerAct::Poll),
    ]
}

/// F2 of `HOLD-MODEL-AB`, fixed: the click takes a fresh wall bound from its
/// own poll, so it never goes out against a hold that ends in the same frame.
#[test]
fn the_journal_click_keeps_its_hold_when_the_begin_bound_passes_in_its_own_frame() {
    let model = model();
    let (state, infos) = run(&model, &f2_steps());
    assert!(
        infos[6].emitted && infos[6].deferred && !infos[6].pressed,
        "the click goes out with a live hold: {:?}",
        infos[6]
    );
    assert!(state.hold_rec.is_some_and(|hold| !hold.released));
}

/// The ledger as it was at `aa5e8bf2c` is a mutation that the frozen
/// invariants catch: the same steps end in a press after the click (S6).
#[test]
fn the_earlier_ledger_loses_the_click_hold_to_the_wall_bound_in_its_own_frame() {
    let model = Model::new(Bounds::standard(), Mutation::F2QuietClickNoRenew);
    let mut state = model.initial_state();
    for (index, action) in f2_steps().iter().enumerate() {
        match model.step_detail(&state, action) {
            Ok((next, _)) => state = next,
            Err(violation) => {
                assert_eq!(index, 6);
                assert_eq!(violation.name, "S6-press-after-send");
                return;
            }
        }
    }
    panic!("the last frame must violate S6");
}

/// The paint cap stays at the begin when the click renews the hold: at the
/// frame of the click the page has stopped being hidden (wall 4, cap 4), and
/// the lease goes on with a hold bound after the cap.
#[test]
fn the_paint_cap_does_not_follow_the_renewed_hold() {
    let (state, _) = run(&model(), &f2_steps());
    assert!(matches!(
        state.lease,
        hold_model::Lease::Quiet(q) if q.held && q.paint_until == 4 && q.hold_until > q.paint_until
    ));
}

#[test]
fn stop_and_a_session_boundary_end_the_hold_at_the_next_frame() {
    for begin in [Begin::Modal, Begin::QueuedModal] {
        for end in [Action::Stop, Action::Relog] {
            let (_, infos) = run(
                &model(),
                &[
                    frame(true, false, false, OwnerAct::Begin(begin)),
                    end,
                    frame(true, false, false, OwnerAct::NoPoll),
                ],
            );
            assert!(!infos[2].deferred, "{begin:?}: {end:?}");
            assert!(infos[2].wait.is_none(), "{begin:?}: {end:?}");
        }
    }
}

#[test]
fn the_compat_click_renews_the_hold_after_a_long_pause() {
    let (_, infos) = run(
        &model(),
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::Compat)),
            Action::Pause,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            Action::Wait,
            Action::Resume,
            // The hold from the begin has passed; the click takes a new one.
            frame(true, false, false, OwnerAct::Poll),
        ],
    );
    assert!(infos[7].emitted && infos[7].deferred && !infos[7].pressed);
}

/// F1 of `HOLD-MODEL-AB`: the host ended the acknowledgement hold at its
/// terminal release (the owner was not polled while its clock ran), and the
/// end gap that follows is the same lease renewed to a later deadline. The
/// head host holds the renewed wait. The shortest trace of the earlier host
/// is these six steps.
fn f1_trace() -> [Action; 6] {
    [
        frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
        Action::ModalOpen,
        frame(false, true, true, OwnerAct::Send),
        frame(true, false, false, OwnerAct::Stall),
        Action::ModalClose,
        frame(false, true, false, OwnerAct::ToGap(1)),
    ]
}

#[test]
fn a_renewed_end_gap_holds_again_after_the_host_ended_the_acknowledgement_hold() {
    let (state, infos) = run(&model(), &f1_trace());
    assert_eq!(
        infos[3].expired,
        Some((Key::Modal(2), 1, 2)),
        "the host ended the acknowledgement hold at the deadline plus the margin"
    );
    assert!(
        infos[5].deferred && !infos[5].pressed && infos[5].wait.is_some(),
        "the renewed wait has a hold of its own"
    );
    assert!(state.hold_rec.is_some_and(|hold| !hold.released));
}

#[test]
fn the_earlier_host_presses_during_the_renewed_end_gap() {
    let earlier = Model::new(Bounds::standard(), Mutation::F1RenewalKeepsRelease);
    let mut state = earlier.initial_state();
    let steps = f1_trace();
    for action in &steps[..5] {
        state = earlier
            .step_detail(&state, action)
            .expect("the first five steps are clean")
            .0;
    }
    let (_, info, violation) = earlier.step_unchecked(&state, &steps[5]);
    assert!(info.pressed && !info.deferred, "the earlier host presses");
    let names: Vec<_> = violation.iter().map(|v| v.name).collect();
    assert_eq!(names, ["S1-press-while-live"]);
}

/// A repeat of the deadline after the terminal release is no renewal: the
/// hold stays ended. The host re-arms only for a later deadline.
#[test]
fn the_head_does_not_rearm_a_hold_for_a_repeat_of_the_deadline() {
    let model = model();
    let mut state = model.initial_state();
    for action in &f1_trace()[..5] {
        state = model.step_detail(&state, action).expect("clean").0;
    }
    let released = state.hold_rec.expect("the hold exists");
    assert!(released.released && !released.boundary);
    // The owner changes nothing at its next poll: the same key and deadline.
    let (state, info) = model
        .step_detail(&state, &frame(true, false, false, OwnerAct::Stall))
        .expect("clean");
    assert!(!info.deferred && info.pressed, "the key holds no more");
    assert!(state.hold_rec.is_some_and(|hold| hold.released));
}

#[test]
fn the_head_has_no_finding_in_the_smoke_profile() {
    let run = explore_profile(
        &profile("smoke").expect("smoke profile"),
        Mutation::None,
        Some(7),
        Limits::default(),
        true,
    );
    assert!(run.complete);
    assert!(
        run.failures.is_empty(),
        "F1 is fixed: {:?}",
        run.failures
            .iter()
            .map(|(v, trace)| (v.name, trace.len()))
            .collect::<Vec<_>>()
    );
}

/// F2 is fixed: the quiet profiles have no finding. The shortest trace of
/// the earlier ledger, in the profile where the bounds are equal, is 4 steps.
#[test]
fn the_head_has_no_finding_in_the_quiet_profiles() {
    for (name, horizon) in [("quiet", 7), ("alt", 7), ("handoff", 6)] {
        let run = explore_profile(
            &profile(name).expect("profile"),
            Mutation::None,
            Some(horizon),
            Limits::default(),
            true,
        );
        assert!(run.complete, "{name}");
        assert!(
            run.failures.is_empty(),
            "{name}: F2 is fixed: {:?}",
            run.failures
                .iter()
                .map(|(v, trace)| (v.name, trace.len()))
                .collect::<Vec<_>>()
        );
    }
}

/// The earlier ledger is counted without the stall, as before finding N1:
/// the stall finds a shorter trace of the same kind (see
/// `the_earlier_product_dispatches_after_expiry_and_s7_catches_it`).
#[test]
fn the_earlier_ledger_fails_s6_in_the_quiet_profiles() {
    for (name, horizon, steps) in [("alt", 5, 4), ("quiet", 7, 6)] {
        let mut quiet = profile(name).expect("profile");
        quiet.bounds.stall = false;
        let run = explore_profile(
            &quiet,
            Mutation::F2QuietClickNoRenew,
            Some(horizon),
            Limits::default(),
            true,
        );
        let found: Vec<_> = run
            .failures
            .iter()
            .map(|(v, trace)| (v.name, trace.len()))
            .collect();
        assert!(
            found.contains(&("S6-press-after-send", steps)),
            "{name}: {found:?}"
        );
        // The dispatched click without a hold is also S7, one step later: a
        // trace with the run already on.
        assert!(
            found
                .iter()
                .any(|(name, _)| *name == "S7-dispatch-without-hold"),
            "{name}: {found:?}"
        );
        assert_eq!(found.len(), 2, "{name}: {found:?}");
    }
}

/// The earlier host (the product at `0a3e700f8`) is a mutation that the
/// frozen invariants catch: S1 and S4, in the six steps of F1.
#[test]
fn the_earlier_host_fails_s1_and_s4_in_the_smoke_profile() {
    let run = explore_profile(
        &profile("smoke").expect("smoke profile"),
        Mutation::F1RenewalKeepsRelease,
        Some(7),
        Limits::default(),
        true,
    );
    assert!(run.complete);
    let mut found: Vec<(&str, usize)> = run
        .failures
        .iter()
        .map(|(v, trace)| (v.name, trace.len()))
        .collect();
    found.sort_unstable();
    assert_eq!(
        found,
        [("S1-press-while-live", 6), ("S4-released-state-carried", 6)],
        "the end gap that follows an acknowledgement is the same lease renewed, so a host release of the acknowledgement wait stays on it"
    );
}

/// The model builder's self-checks: every frozen invariant fails under a
/// mutation of the transitions that it is meant to catch.
#[test]
fn each_self_check_mutation_fails_its_invariant() {
    let base = Bounds {
        max_tick: 3,
        max_wall: 4,
        max_pages: 3,
        ..Bounds::standard()
    };
    let quiet_only = Bounds {
        modal: false,
        compat: false,
        env: Env::NONE,
        ..base
    };
    let cases = [
        (
            Mutation::CheckMarginZero,
            "S3-early-terminal-release",
            3,
            base,
        ),
        (
            Mutation::CheckMarginTwo,
            "L-hold-outlives-deadline",
            4,
            base,
        ),
        (Mutation::CheckStopKeepsLease, "S2-hold-after-end", 4, base),
        (
            Mutation::CheckCompatClickNoRenew,
            "S6-press-after-send",
            6,
            base,
        ),
        (
            Mutation::CheckSendKeepsKey,
            "S4-released-state-carried",
            6,
            base,
        ),
        (Mutation::CheckStaleHold, "S0-hold-without-wait", 2, base),
        (
            Mutation::CheckQuietNoWall,
            "S5-paint-hidden-past-cap",
            6,
            quiet_only,
        ),
    ];
    for (mutation, expected, horizon, bounds) in cases {
        let run = explore_profile(
            &hold_model::Profile {
                name: "check",
                bounds,
            },
            mutation,
            Some(horizon),
            Limits::default(),
            true,
        );
        assert!(
            run.failures.iter().any(|(v, _)| v.name == expected),
            "{} must fail {expected}; found {:?}",
            mutation.name(),
            run.failures.iter().map(|(v, _)| v.name).collect::<Vec<_>>()
        );
    }
}

/// Every lease kind reaches a normal release (its page, its bound, its drop),
/// and the host's terminal release and the session releases exist.
#[test]
fn every_lease_kind_reaches_its_releases() {
    let bounds = Bounds {
        horizon: 9,
        max_tick: 3,
        max_wall: 4,
        max_ids: 3,
        max_pages: 2,
        max_epochs: 1,
        env: Env {
            session: true,
            ..Env::NONE
        },
        ..Bounds::standard()
    };
    let (_, found) = release_traces(bounds, Mutation::None, 9, Limits::default(), 8);
    let has = |kind: Kind, cause: Cause| found.iter().any(|(r, _)| *r == (kind, cause));
    for (kind, cause) in [
        (Kind::Modal, Cause::Page),
        (Kind::Modal, Cause::Bound),
        (Kind::Modal, Cause::Dropped),
        (Kind::Modal, Cause::Session),
        (Kind::Modal, Cause::HostTerminal),
        (Kind::Quiet, Cause::Page),
        (Kind::Quiet, Cause::Bound),
        (Kind::Quiet, Cause::Session),
        (Kind::Quiet, Cause::Wall),
        (Kind::Compat, Cause::Page),
        (Kind::Compat, Cause::Bound),
        (Kind::Compat, Cause::Wall),
        (Kind::Compat, Cause::Session),
    ] {
        assert!(has(kind, cause), "no trace reaches {kind:?} {cause:?}");
    }
    // Every listed trace replays to a transition that has the release.
    let model = Model::new(bounds, Mutation::None);
    for (release, traces) in &found {
        for trace in traces {
            let mut state = model.initial();
            let mut hit = false;
            for action in trace {
                let (next, info) = model.step_detail(&state, action).expect("a clean trace");
                hit = info.releases.contains(release);
                state = next;
            }
            assert!(hit, "the last step of a {release:?} trace must release");
        }
    }
}

/// Phase 2: each earlier design of the hold is caught by a frozen invariant.
/// The shortest trace has the length given.
#[test]
fn each_earlier_design_fails_a_frozen_invariant() {
    let modal_only = Bounds {
        quiet: false,
        compat: false,
        ..Bounds::standard()
    };
    let compat_only = Bounds {
        modal: false,
        quiet: false,
        same_frame_begin: true,
        ..Bounds::standard()
    };
    let cases = [
        (
            Mutation::M1HoldTwoTicks,
            "S1-press-while-live",
            3,
            modal_only,
        ),
        (
            Mutation::M1HoldTwoTicks,
            "L-hold-outlives-deadline",
            4,
            modal_only,
        ),
        (
            Mutation::M2HoldThreeTicks,
            "S1-press-while-live",
            4,
            modal_only,
        ),
        (
            Mutation::M3FixedTerminal,
            "S3-early-terminal-release",
            4,
            modal_only,
        ),
        (
            Mutation::M3FixedTerminal,
            "S1-press-while-live",
            4,
            modal_only,
        ),
        (
            Mutation::M5ActiveToWall,
            "S1-press-while-live",
            5,
            modal_only,
        ),
        (
            Mutation::M5ActiveToWall,
            "S3-early-terminal-release",
            6,
            modal_only,
        ),
        (
            Mutation::M6CompatNoRenew,
            "S6-press-after-send",
            4,
            compat_only,
        ),
        (
            Mutation::M4CompatConstantKey,
            "S1-press-while-live",
            3,
            compat_only,
        ),
    ];
    for (mutation, expected, steps, bounds) in cases {
        let run = explore_profile(
            &hold_model::Profile {
                name: "phase2",
                bounds,
            },
            mutation,
            Some(steps),
            Limits::default(),
            true,
        );
        let found = run.failures.iter().find(|(v, _)| v.name == expected);
        assert!(
            found.is_some_and(|(_, trace)| trace.len() == steps),
            "{} must fail {expected} in {steps} steps; found {:?}",
            mutation.name(),
            run.failures
                .iter()
                .map(|(v, t)| (v.name, t.len()))
                .collect::<Vec<_>>()
        );
    }
}

// ---- finding N1: an expired hold makes its queued verb stale ----

/// The steps of finding N1 for the journal read: the owner begins, and in a
/// later frame it polls (its click goes out, and the poll renews the hold to
/// four wall units from there), and the process stalls past every hold before
/// the host's sample. `run_on` says whether the run is already on, so that no
/// press follows. The last step is the frame of the click.
fn n1_quiet_steps(run_on: bool) -> Vec<Action> {
    let mut steps = Vec::new();
    if run_on {
        // No lease yet: the host presses and the run is on.
        steps.push(frame(true, false, false, OwnerAct::Stall));
    }
    steps.push(frame(true, false, false, OwnerAct::Begin(Begin::Quiet)));
    steps.push(Action::Frame(hold_model::FrameInput {
        tick: true,
        wake: false,
        drift: false,
        stall: true,
        act: OwnerAct::Poll,
    }));
    steps
}

/// The same for the compat read: its click is the first poll.
fn n1_compat_steps(run_on: bool) -> Vec<Action> {
    let mut steps = Vec::new();
    if run_on {
        steps.push(frame(true, false, false, OwnerAct::Stall));
    }
    steps.push(frame(true, false, false, OwnerAct::Begin(Begin::Compat)));
    steps.push(Action::Frame(hold_model::FrameInput {
        tick: true,
        wake: false,
        drift: false,
        stall: true,
        act: OwnerAct::Poll,
    }));
    steps
}

fn stalled_model(mutation: Mutation) -> Model {
    Model::new(
        Bounds {
            stall: true,
            ..Bounds::standard()
        },
        mutation,
    )
}

/// The head drops the click that the owner queued when the sample ends its
/// hold by expiry. The press is the host's ordinary decision after an expired
/// hold; it cannot close the page of a click that never went out.
#[test]
fn an_expired_hold_makes_the_queued_click_stale() {
    for steps in [
        n1_quiet_steps(false),
        n1_quiet_steps(true),
        n1_compat_steps(false),
        n1_compat_steps(true),
    ] {
        let (_, infos) = run(&stalled_model(Mutation::None), &steps);
        let last = infos.last().expect("a frame");
        assert!(last.emitted, "the owner sent its click");
        assert!(last.dropped && !last.dispatched(), "the click is stale");
        assert!(!last.deferred, "the hold expired at the sample");
    }
}

/// S1 to S6 cannot see a dispatch after expiry when the run is already on:
/// no press follows, so S6 is silent, and no lease is live at the press
/// decision, so S1 is silent. S7 catches the product at `c2ca4cf66`, and
/// the dispatch of the click is the only wrong thing in the frame.
#[test]
fn the_earlier_product_dispatches_after_expiry_and_s7_catches_it() {
    let model = stalled_model(Mutation::N1ExpiredClickDispatched);
    for (steps, kind) in [
        (n1_quiet_steps(true), "quiet"),
        (n1_compat_steps(true), "compat"),
    ] {
        let mut state = model.initial_state();
        let last = steps.len() - 1;
        for (index, action) in steps.iter().enumerate() {
            let (next, info, violation) = model.step_unchecked(&state, action);
            if index == last {
                let violation = violation.unwrap_or_else(|| panic!("{kind}: S7 must fire"));
                assert_eq!(violation.name, "S7-dispatch-without-hold", "{kind}");
                assert!(info.dispatched() && !info.deferred, "{kind}: {info:?}");
                assert!(!info.pressed, "{kind}: run is on, so S6 cannot see it");
            } else {
                assert!(violation.is_none(), "{kind}: step {index}");
            }
            state = next;
        }
    }
    // With the press, S6 sees it too (it is checked before S7).
    let mut state = model.initial_state();
    let mut last = None;
    for action in &n1_quiet_steps(false) {
        let (next, _, violation) = model.step_unchecked(&state, action);
        last = violation;
        state = next;
    }
    assert_eq!(
        last.map(|violation| violation.name),
        Some("S6-press-after-send")
    );
}

/// The shortest trace of N1 in each profile that has a lease with a wall
/// bound, found by the explorer.
#[test]
fn the_earlier_product_fails_s7_in_the_profiles() {
    for (name, horizon, steps) in [("smoke", 4, 3), ("alt", 4, 3), ("quiet", 4, 3)] {
        let run = explore_profile(
            &profile(name).expect("profile"),
            Mutation::N1ExpiredClickDispatched,
            Some(horizon),
            Limits::default(),
            true,
        );
        let found = run
            .failures
            .iter()
            .find(|(v, _)| v.name == "S7-dispatch-without-hold");
        assert!(
            found.is_some_and(|(_, trace)| trace.len() == steps),
            "{name}: S7 in {steps} steps; found {:?}",
            run.failures
                .iter()
                .map(|(v, t)| (v.name, t.len()))
                .collect::<Vec<_>>()
        );
    }
}

/// The modal lease cannot reach the case. It has no wall bound: the owner's
/// deadline is on the active clock or in ticks, and the host ends such a hold
/// only a tick after the owner's deadline, in a later frame. A stall in the
/// frame of a send leaves the hold live, and the head has no finding in the
/// modal profile for any stall.
#[test]
fn a_stall_never_strands_a_dialogue_verb() {
    let model = stalled_model(Mutation::None);
    let (_, infos) = run(
        &model,
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
            Action::ModalOpen,
            Action::Frame(hold_model::FrameInput {
                tick: false,
                wake: true,
                drift: false,
                stall: true,
                act: OwnerAct::Send,
            }),
        ],
    );
    let send = &infos[2];
    assert!(send.emitted && send.dispatched(), "the send goes out");
    assert!(send.deferred, "with a live hold");
    let run = explore_profile(
        &profile("modal").expect("profile"),
        Mutation::None,
        Some(9),
        Limits::default(),
        true,
    );
    assert!(
        run.complete && run.failures.is_empty(),
        "{:?}",
        run.failures.iter().map(|(v, _)| v.name).collect::<Vec<_>>()
    );
}

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
            Action::Wait,
            Action::Wait,
            Action::Wait,
            // The active clock reached the bound: the sample fixes the tick.
            frame(false, false, false, OwnerAct::NoPoll),
            // The next tick: the owner stalls, the host ends the hold.
            frame(true, false, false, OwnerAct::Stall),
        ],
    );
    assert!(infos[4].deferred, "the bound is reached, the margin is not");
    assert_eq!(infos[5].expired, Some((Key::Modal(1), 1, 2)));
    assert_eq!(infos[5].releases, vec![(Kind::Modal, Cause::HostTerminal)]);
    assert!(infos[5].pressed, "the terminal release lets the press out");
}

#[test]
fn a_paused_dialogue_keeps_its_active_clock_hold() {
    let (_, infos) = run(
        &model(),
        &[
            frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
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
        infos[6].deferred && infos[7].deferred,
        "Pause stops the clock"
    );
    assert_eq!(infos[7].expired, None);
}

/// A known limit, not a defect: a wait that the owner bounds in ticks (the end
/// gap, the journal window) is not on the active clock, so Pause does not stop
/// it. The host ends the hold one tick after the deadline and logs a failure,
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

/// A known limit, finding F2: the journal read sends its click in a frame
/// where the lease's own 10 s wall bound passes between the owner's poll and
/// the host's sample. The hold ends in the same frame and the press follows
/// the click. The read needs a pause first: its own bound on the active clock
/// (8 s) is shorter than the wall bound (10 s), unless the clock stopped.
#[test]
fn the_journal_click_can_lose_its_hold_to_the_wall_bound_in_its_own_frame() {
    let model = model();
    let mut state = model.initial_state();
    let steps = [
        frame(true, false, false, OwnerAct::Begin(Begin::Quiet)),
        Action::Pause,
        Action::Wait,
        Action::Wait,
        Action::Wait,
        Action::Resume,
        frame(false, true, true, OwnerAct::Poll),
    ];
    for (index, action) in steps.iter().enumerate() {
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

#[test]
fn stop_and_a_session_boundary_end_the_hold_at_the_next_frame() {
    for end in [Action::Stop, Action::Relog] {
        let (_, infos) = run(
            &model(),
            &[
                frame(true, false, false, OwnerAct::Begin(Begin::Modal)),
                end,
                frame(true, false, false, OwnerAct::NoPoll),
            ],
        );
        assert!(!infos[2].deferred, "{end:?}");
        assert!(infos[2].wait.is_none(), "{end:?}");
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

#[test]
fn the_head_findings_are_the_two_known_ones_in_the_smoke_profile() {
    let run = explore_profile(
        &profile("smoke").expect("smoke profile"),
        Mutation::None,
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

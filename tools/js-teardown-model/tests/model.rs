//! Tests of the teardown and retirement models: the profiles end clean and
//! exhausted, every mutation fails the invariant that must catch it, and the
//! four invariants of the C23 slice have hand-written scenarios.

use js_teardown_model::teardown::{
    Action, Bounds, Event, Kind, Mutation, Owner, Phase, State, Teardown, Who,
};
use js_teardown_model::{
    Limits, explore_retire, explore_teardown, retire, retire_profile, retire_profiles,
    teardown_profile,
};

fn model() -> Teardown {
    Teardown::new(Bounds::standard(), Mutation::None)
}

/// Apply `actions` in order; every step must pass the invariants.
fn run(model: &Teardown, actions: &[Action]) -> (State, Vec<Event>) {
    let mut state = model.initial_state();
    let mut events = Vec::new();
    for action in actions {
        assert!(
            model.enabled(&state, action),
            "{action:?} is not offered in {}",
            state.summary()
        );
        let (next, step) = model
            .step_detail(&state, action)
            .unwrap_or_else(|v| panic!("{action:?}: {} {}", v.name, v.detail));
        state = next;
        events.extend(step);
    }
    (state, events)
}

/// The violation that `actions` end in.
fn violation(model: &Teardown, actions: &[Action]) -> &'static str {
    let mut state = model.initial_state();
    for action in actions {
        match model.step_detail(&state, action) {
            Ok((next, _)) => state = next,
            Err(v) => return v.name,
        }
    }
    panic!("no step of {actions:?} violates an invariant");
}

#[test]
fn every_teardown_profile_ends_clean_and_exhausted_at_the_head() {
    for name in ["smoke", "ticks", "frames", "no-refusal", "stop"] {
        let profile = teardown_profile(name).expect("a named profile");
        let result = explore_teardown(&profile, Mutation::None, None, Limits::default(), true);
        assert!(result.cap.is_none(), "{name}: a cap stopped the run");
        assert!(
            result.failure.is_none() && result.failures.is_empty(),
            "{name}: {:?}",
            result.failure.as_ref().map(|(v, _)| v)
        );
        assert!(result.complete, "{name}: incomplete");
        // The state space is exhausted before the horizon, so the horizon
        // does not bound what the profile covers.
        assert!(
            result.deepest < result.horizon,
            "{name}: the horizon {} cuts the search at {}",
            result.horizon,
            result.deepest
        );
    }
}

#[test]
fn every_retire_profile_ends_clean_and_exhausted_at_the_head() {
    for profile in retire_profiles() {
        let result = explore_retire(
            &profile,
            retire::Mutation::None,
            None,
            Limits::default(),
            true,
        );
        assert!(
            result.cap.is_none(),
            "{}: a cap stopped the run",
            profile.name
        );
        assert!(
            result.failure.is_none() && result.failures.is_empty(),
            "{}: {:?}",
            profile.name,
            result.failure.as_ref().map(|(v, _)| v)
        );
        assert!(
            result.complete && result.deepest < result.horizon,
            "{}",
            profile.name
        );
    }
}

/// The longer profiles run in the nightly (see README.md); a developer can
/// run them here with `JS_TEARDOWN_MODEL_FULL=1`.
#[test]
fn the_full_profiles_end_clean_when_asked() {
    if std::env::var_os("JS_TEARDOWN_MODEL_FULL").is_none() {
        return;
    }
    for name in ["clock", "full"] {
        let profile = teardown_profile(name).expect("a named profile");
        let result = explore_teardown(&profile, Mutation::None, None, Limits::default(), true);
        assert!(result.complete && result.cap.is_none(), "{name}");
        assert!(
            result.failures.is_empty() && result.failure.is_none(),
            "{name}"
        );
        assert!(result.deepest < result.horizon, "{name}");
    }
}

#[test]
fn every_teardown_mutation_fails_the_invariant_that_must_catch_it() {
    for mutation in Mutation::ALL {
        let Some(expected) = mutation.caught_by() else {
            continue;
        };
        // The hook protocol alone is enough for the mutations of Stop and
        // Drop; the others need a second eval or Pause.
        let profile = teardown_profile(match mutation {
            Mutation::R1StopTerminatesInHook
            | Mutation::R4HookDeadlineIntoDone
            | Mutation::R6HookEnterTwice
            | Mutation::R7StopEntersHook
            | Mutation::D1DropOverwritesHook => "stop",
            _ => "ticks",
        })
        .expect("a named profile");
        let result = explore_teardown(&profile, mutation, None, Limits::default(), true);
        assert!(
            result.failures.iter().any(|(v, _)| v.name == expected),
            "{}: no violation named {expected}; found {:?}",
            mutation.name(),
            result
                .failures
                .iter()
                .map(|(v, _)| v.name)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn every_retire_mutation_fails_the_invariant_that_must_catch_it() {
    for mutation in retire::Mutation::ALL {
        let Some(expected) = mutation.caught_by() else {
            continue;
        };
        // One room is enough for each mutation; `retire-cap2` is only
        // exhausted for the head.
        let profile = retire_profile("retire").expect("a named profile");
        let result = explore_retire(&profile, mutation, None, Limits::default(), true);
        assert!(
            result.failures.iter().any(|(v, _)| v.name == expected),
            "{}: no violation named {expected}; found {:?}",
            mutation.name(),
            result
                .failures
                .iter()
                .map(|(v, _)| v.name)
                .collect::<Vec<_>>()
        );
    }
}

/// Finding D1, at `a3a50d098`: Drop overwrote a running hook with Done.
/// Drop's termination lands before the isolate thread's pre-hook cancel,
/// the cancel eats it, and the supervisor's hook deadline (guarded by
/// `Hook`) no longer fires: the body runs unbounded.
#[test]
fn finding_d1_raw_drop_in_the_hook_window_leaves_the_body_unbounded() {
    let profile = teardown_profile("stop").expect("a named profile");
    let result = explore_teardown(
        &profile,
        Mutation::D1DropOverwritesHook,
        None,
        Limits::default(),
        true,
    );
    let (violation, trace) = result
        .failures
        .iter()
        .find(|(v, _)| v.name == "e-hook-unbounded")
        .expect("D1 is found");
    assert!(violation.detail.contains("Done"));
    // script stop, enter hook, guard, Drop, Drop's queue and terminate, the
    // thread's cancel, its registration.
    assert_eq!(trace.len(), 8, "{trace:?}");
    assert!(trace.contains(&Action::Drop) && trace.contains(&Action::HookCancel));
    // The head keeps the hook, so the same steps end bounded.
    let (state, events) = run(
        &model(),
        &[
            Action::ScriptStop,
            Action::EnterHook,
            Action::HookGuard,
            Action::Drop,
            Action::DropSend,
            Action::DropFire,
            Action::HookCancel,
            Action::HookRegister { refuse: false },
        ],
    );
    assert_eq!(state.phase, Phase::Hook);
    assert!(events.contains(&Event::DropKeptHook));
    assert_eq!(state.entries.len(), 1, "the hook deadline is registered");
}

// Invariant 4: Pause never cuts an eval that owns execution, and always
// blocks the next entry.

#[test]
fn pause_blocks_the_next_tick_but_lets_the_owning_eval_run_to_its_horizon() {
    let m = model();
    // Pause looks at the eval, finds it inside its horizon and arms the
    // eval's own deadline; it then publishes its intent and takes the
    // deadline's owner. The eval keeps running.
    let pause = [
        Action::Enter(Kind::Tick),
        Action::Pause,
        Action::Look,
        Action::Register {
            by: Who::Host,
            refuse: false,
        },
        Action::PauseSet,
        Action::PauseArm,
    ];
    let (state, events) = run(&m, &pause);
    assert!(events.contains(&Event::PauseSet));
    let exec = state.exec.expect("the eval still owns execution");
    assert!(exec.interrupt.is_none(), "Pause did not cut it");
    assert_eq!(exec.armed, Some(Owner::Pause), "Pause owns the diagnostic");
    assert_eq!(
        state.entries.len(),
        1,
        "one registration, at the eval's own horizon"
    );
    // It finishes, and the next tick is refused.
    let mut and_then = pause.to_vec();
    and_then.extend([Action::Finish, Action::Enter(Kind::Tick)]);
    let (state, events) = run(&m, &and_then);
    assert!(state.exec.is_none());
    assert!(events.contains(&Event::EntryRefusedPause(Kind::Tick)));
}

#[test]
fn a_frame_or_lifecycle_eval_enters_while_paused() {
    let m = Teardown::new(
        Bounds {
            frames: true,
            ..Bounds::standard()
        },
        Mutation::None,
    );
    let (_, events) = run(
        &m,
        &[
            Action::Pause,
            Action::PauseSet,
            Action::PauseArm,
            Action::Enter(Kind::Frame),
        ],
    );
    assert!(events.contains(&Event::EntryAccepted(Kind::Frame)));
}

#[test]
fn a_pause_deadline_that_finds_pause_resumed_cuts_nothing() {
    let m = model();
    let (state, events) = run(
        &m,
        &[
            Action::Enter(Kind::Tick),
            Action::Pause,
            Action::Look,
            Action::Register {
                by: Who::Host,
                refuse: false,
            },
            Action::PauseSet,
            Action::PauseArm,
            Action::Resume,
            Action::Advance,
            Action::Fire(0),
        ],
    );
    assert!(events.contains(&Event::DeadlinePauseResumed));
    assert!(state.exec.expect("still active").interrupt.is_none());
    assert_eq!(state.flag, 0);
}

#[test]
fn the_pause_check_outside_the_lock_lets_a_tick_in() {
    let bad = Teardown::new(Bounds::standard(), Mutation::R2PauseCheckedOutsideLock);
    assert_eq!(
        violation(
            &bad,
            &[
                Action::EnterCheck(Kind::Tick),
                Action::Pause,
                Action::PauseSet,
                Action::EnterCommit,
            ],
        ),
        "d-pause-did-not-block-entry"
    );
}

// Invariant 1: the eval that armed a terminate request consumes it, or the
// isolate thread cancels it, and no later eval or onStop sees it.

#[test]
fn a_deadline_of_a_finished_eval_is_stale_for_the_next_one() {
    let m = model();
    let (state, events) = run(
        &m,
        &[
            Action::Enter(Kind::Tick),
            Action::Reset,
            Action::Register {
                by: Who::Host,
                refuse: false,
            },
            Action::Finish,
            Action::Enter(Kind::Tick),
            Action::Advance,
            Action::Fire(0),
        ],
    );
    assert!(events.contains(&Event::DeadlineStale));
    assert!(state.exec.expect("eval 2").interrupt.is_none());
    assert_eq!(state.flag, 0);
}

#[test]
fn finish_cancels_the_request_armed_for_its_eval() {
    let m = model();
    let (state, events) = run(
        &m,
        &[
            Action::Enter(Kind::Tick),
            Action::Advance,
            Action::Advance,
            Action::Poll,
            Action::Look,
            Action::Finish,
            Action::Enter(Kind::Tick),
        ],
    );
    assert!(events.contains(&Event::PollFired));
    assert!(events.contains(&Event::FinishCut { consumed: false }));
    assert_eq!(
        state.flag, 0,
        "the next eval enters with no request pending"
    );
    assert_eq!(state.exec.expect("eval 2").id, 2);
}

#[test]
fn a_request_fired_after_the_call_is_cancelled_and_the_cut_is_not_reported() {
    let m = model();
    let (_, events) = run(
        &m,
        &[
            Action::Enter(Kind::Tick),
            Action::Advance,
            Action::Advance,
            Action::Poll,
            Action::Look,
            Action::Finish,
        ],
    );
    assert!(events.contains(&Event::FinishCut { consumed: false }));
    let (_, events) = run(
        &m,
        &[
            Action::Enter(Kind::Tick),
            Action::Advance,
            Action::Advance,
            Action::Poll,
            Action::Look,
            Action::Deliver,
            Action::Finish,
        ],
    );
    assert!(events.contains(&Event::Delivered { consumed: true }));
    assert!(events.contains(&Event::FinishCut { consumed: true }));
}

#[test]
fn stop_terminates_the_unwinding_tick_and_the_hook_starts_with_nothing_pending() {
    let m = model();
    let (state, events) = run(
        &m,
        &[
            Action::Enter(Kind::Tick),
            Action::StopSend,
            Action::StopLock,
            Action::Deliver,
            Action::Finish,
            Action::StopCommand,
            Action::EnterHook,
            Action::HookGuard,
            Action::HookCancel,
            Action::HookRegister { refuse: false },
        ],
    );
    assert!(events.contains(&Event::StopTerminates));
    assert!(events.contains(&Event::HookRan));
    assert_eq!(state.phase, Phase::Hook);
    assert_eq!(state.flag, 0);
}

// Invariant 2: onStop runs at most once, and only after no eval is active.

#[test]
fn the_hook_is_refused_after_the_script_already_stopped_itself() {
    let m = model();
    let (state, events) = run(
        &m,
        &[
            Action::ScriptStop,
            Action::EnterHook,
            Action::HookGuard,
            Action::HookCancel,
            Action::HookRegister { refuse: false },
            Action::HookDone,
            Action::Complete,
            Action::CancelAfter,
        ],
    );
    assert_eq!(state.hook_runs, 1);
    assert_eq!(state.phase, Phase::Done);
    assert!(events.contains(&Event::HookRan));
}

#[test]
fn the_hook_does_not_start_inside_an_active_eval() {
    let bad = Teardown::new(Bounds::standard(), Mutation::R7StopEntersHook);
    assert_eq!(
        violation(
            &bad,
            &[
                Action::Enter(Kind::Tick),
                Action::StopSend,
                Action::StopLock
            ]
        ),
        "b-on-stop-with-active-eval"
    );
}

#[test]
fn a_hook_deadline_fires_in_the_hook_only() {
    let m = model();
    let (state, events) = run(
        &m,
        &[
            Action::ScriptStop,
            Action::EnterHook,
            Action::HookGuard,
            Action::HookCancel,
            Action::HookRegister { refuse: false },
            Action::HookDone,
            Action::Complete,
            Action::CancelAfter,
            Action::Advance,
            Action::Fire(0),
        ],
    );
    assert!(events.contains(&Event::HookDeadlineStale));
    assert_eq!(state.flag, 0, "nothing is terminated after Done");
}

// Invariant 3: see retire.rs.

fn retire_run(actions: &[retire::Action]) -> Result<retire::State, &'static str> {
    let model = retire::Retire::new(retire::Bounds::standard(), retire::Mutation::None);
    let mut state = model.initial_state();
    for action in actions {
        assert!(
            model.enabled(&state, action),
            "{action:?} is not offered in {}",
            state.summary()
        );
        state = model.step_detail(&state, action).map_err(|v| v.name)?;
    }
    Ok(state)
}

#[test]
fn a_gameplay_credit_moves_to_a_published_quarantine_before_its_receipt() {
    use retire::Action::*;
    let state = retire_run(&[Retire(0), Begin, Advance, Advance, Settle, Finish, Send])
        .expect("the head's order");
    let row = state.rows[0];
    assert_eq!(row.custody, retire::Custody::Quarantined);
    assert_eq!(row.thread, retire::Thread::Alive);
    assert!(matches!(row.receipt, retire::Receipt::Sent(1)));
}

#[test]
fn a_validation_credit_waits_for_the_join() {
    use retire::Action::*;
    // Row 2 is the validation row. Quarantined, it has no receipt until reap
    // joins its finished thread.
    let state = retire_run(&[Retire(2), Begin, Advance, Advance, Settle, Finish, Send])
        .expect("the head's order");
    assert!(matches!(state.rows[2].receipt, retire::Receipt::None));
    let state = retire_run(&[
        Retire(2),
        Begin,
        Advance,
        Advance,
        Settle,
        Finish,
        Send,
        Exit(2),
        Reap,
    ])
    .expect("reap joins it");
    assert!(matches!(state.rows[2].receipt, retire::Receipt::Sent(1)));
    assert_eq!(state.rows[2].thread, retire::Thread::Joined);
}

#[test]
fn a_timeout_with_the_quarantine_full_keeps_the_row_and_its_charge() {
    use retire::Action::*;
    // Cap 1: row 0 takes the room; row 1 stays retiring past its deadline.
    let state = retire_run(&[
        Retire(0),
        Retire(1),
        Begin,
        Advance,
        Advance,
        Settle,
        Settle,
        Finish,
        Send,
    ])
    .expect("the head's order");
    assert_eq!(state.rows[0].custody, retire::Custody::Quarantined);
    assert_eq!(state.rows[1].custody, retire::Custody::Retiring);
    assert!(matches!(state.rows[1].receipt, retire::Receipt::None));
}

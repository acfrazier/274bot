//! Tests of the lifecycle model: the profiles end clean and exhausted, every
//! mutation fails the invariant that must catch it, the traces that LD1
//! requires the model to accept are replayed step by step, and each finding
//! of the model has its shortest trace.

use luau_model::lifecycle::{
    Action, Alloc, Binding, Bounds, Call, Event, Fail, Intent, Inv, Latch, Lifecycle, Mutation, Op,
    Receipt, Ret, RunState, RunnerPhase, Shapes, Site, State, Verdict, Wait, Wake, Who,
    checks::CHECKS,
};
use luau_model::{GOALS, Limits, explore_lifecycle, goal_traces, profile, profiles};

fn model(bounds: Bounds) -> Lifecycle {
    Lifecycle::new(bounds, Mutation::None)
}

/// Apply `actions` in order; every step must be enabled and pass the
/// invariants.
fn run(model: &Lifecycle, actions: &[Action]) -> (State, Vec<Event>) {
    run_from(model, model.initial_state(), actions)
}

fn run_from(model: &Lifecycle, mut state: State, actions: &[Action]) -> (State, Vec<Event>) {
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
fn violation(model: &Lifecycle, actions: &[Action]) -> &'static str {
    let mut state = model.initial_state();
    for action in actions {
        assert!(
            model.enabled(&state, action),
            "{action:?} is not offered in {}",
            state.summary()
        );
        match model.step_detail(&state, action) {
            Ok((next, _)) => state = next,
            Err(v) => return v.name,
        }
    }
    panic!("no step of {actions:?} violates an invariant");
}

fn load_table(hook: bool, paint: bool) -> Action {
    Action::Vm(Op::Return(Ret::Table { hook, paint }))
}

const START: Action = Action::Start(Who::Operator);
const TICK: Action = Action::Dispatch(Wake::Tick);
const RETURN: Action = Action::Vm(Op::Return(Ret::Plain));
const FAIL: Action = Action::Vm(Op::Fail);
const STOP_CALL: Action = Action::Vm(Op::Call(Binding::Stop));

/// A run that loaded with `onStop` and `onPaint` and is `Running`.
fn running() -> Vec<Action> {
    vec![START, load_table(true, true), Action::Observe]
}

/// The smallest profile that catches each mutation. Each profile is
/// exhausted, so the choice only keeps the test lane short.
fn catching_profile(mutation: Mutation) -> &'static str {
    use Mutation::*;
    match mutation {
        None => unreachable!("the head has no catching profile"),
        OnStopAfterTrip
        | OnStopInDispatch
        | DeadlineCarries
        | VerdictFromError
        | RowsAfterStop
        | DoneTwice
        | SampledClock
        | NoExitCheck
        | EffectAfterTrip
        | DrainBeforeFailure
        | PendingFailure
        | HookFailureLost
        | HookFailureReplacesReceipt
        | HookSkippedWhilePaused
        | PausedPaint
        | LoadUngated
        | FailedLoadLeak
        | PaintRearmsDeadline
        | GapEffect
        | StopContinues
        | LatchOverwritten
        | HookActionLegal
        | F2StopIgnoresSelfStop
        | CheckGapUnbounded
        | CheckHookTwice
        | CheckPauseNoRefusal
        | HookOutsideMutex
        | HookUngated
        | CheckDoneDropped
        | CheckStopKeepsRun => "smoke",
        SeatPageIgnoresPause
        | ResumeTwice
        | LostWake
        | ReenterTick
        | HeldTickResumes
        | AckWithoutStore
        | AckOnRefusal
        | LentSkipsSeatPage
        | BareSeatEnd
        | F1PaintWaitSuspends
        | ResumeWithoutCompletion => "waits",
        SpawnBeforeRetire
        | QueuedStartLost
        | StopKeepsQueuedStart
        | F3RestartIgnoresSelfStop
        | SelfStopAcceptsDispatch
        | HookRunsWithoutEntry
        | CheckRemoveKeepsVm => "control",
        MemoryVerdictFromError | LostRefusal | EntryThroughCall | LostErrorIgnored => "memory",
        SharedRestartDock => "dock-restart",
        CompletionOverwrite
        | ConsumeTwice
        | CallsAfterClose
        | CaughtHookRefusalLost
        | UnobservedRefusalLost
        | RevokeKeepsPayload
        | RevokeClearsFirstFault
        | WakeBetweenFreezeAndStop
        | PostFreezeAction
        | BlockingFleetLock => "fence",
        ConsumeWhilePaused
        | HeldConsumes
        | RefusedDispatchLosesPayload
        | SessionCancelsExtensionWait => "dock",
        CommitFailedDispatch | HookFailureNotInEnd => "dock-trips",
        GuessedStopGeneration
        | ReplacementWithOldEnd
        | FailedReplacementRestoresAuthorization
        | ForgetEarlierExecution
        | RepeatLastTableStop
        | LoadingMissingFromLedger
        | StopUnderRunnerLock
        | FileCompileUnderRunner
        | StaleFinalTicket
        | FileCompileHoldsSharedAdmission => "runner",
        ForcedStopSatisfiesNaturalStop => "runner-natural",
        UnlinkedEarlierTicket => "runner-chain",
    }
}

// ---- the profiles and the mutations ----

#[test]
fn every_unit_profile_ends_clean_and_exhausted_at_the_head() {
    for profile in profiles().into_iter().filter(|p| !p.nightly) {
        let result = explore_lifecycle(&profile, Mutation::None, None, Limits::default(), true);
        assert!(
            result.cap.is_none(),
            "{}: a cap stopped the run",
            profile.name
        );
        assert!(
            result.failure.is_none() && result.failures.is_empty(),
            "{}: {:?}",
            profile.name,
            result
                .failures
                .iter()
                .map(|(v, _)| v.name)
                .collect::<Vec<_>>()
        );
        assert!(result.complete, "{}: incomplete", profile.name);
        // The state space is exhausted before the horizon, so the horizon
        // does not bound what the profile covers.
        assert!(
            result.deepest < result.horizon,
            "{}: the horizon {} cuts the search at {}",
            profile.name,
            result.horizon,
            result.deepest
        );
    }
}

/// The longer profiles run in the nightly (see README.md); a developer can
/// run them here with `LUAU_MODEL_FULL=1`.
#[test]
fn the_nightly_profiles_end_clean_when_asked() {
    if std::env::var_os("LUAU_MODEL_FULL").is_none() {
        return;
    }
    for profile in profiles().into_iter().filter(|p| p.nightly) {
        let result = explore_lifecycle(&profile, Mutation::None, None, Limits::default(), true);
        assert!(result.complete && result.cap.is_none(), "{}", profile.name);
        assert!(
            result.failures.is_empty() && result.failure.is_none(),
            "{}",
            profile.name
        );
        assert!(result.deepest < result.horizon, "{}", profile.name);
    }
}

#[test]
fn every_mutation_fails_the_invariant_that_must_catch_it() {
    // Every violation name that some run of the model can raise.
    let mut fired = std::collections::BTreeSet::new();
    for mutation in Mutation::ALL {
        let Some(expected) = mutation.caught_by() else {
            continue;
        };
        let profile = profile(catching_profile(mutation)).expect("a named profile");
        let result = explore_lifecycle(&profile, mutation, None, Limits::default(), true);
        assert!(
            result.failures.iter().any(|(v, _)| v.name == expected),
            "{} in {}: no violation named {expected}; found {:?}",
            mutation.name(),
            profile.name,
            result
                .failures
                .iter()
                .map(|(v, _)| v.name)
                .collect::<Vec<_>>()
        );
        fired.extend(result.failures.iter().map(|(v, _)| v.name));
    }
    // A check that no mutation can make fail is dead: nothing shows that it
    // would catch the defect it names.
    for check in CHECKS {
        assert!(
            fired.contains(check.name),
            "no mutation makes the check {} fail",
            check.name
        );
    }
}

#[test]
fn mutation_names_are_unique_and_parse_back() {
    for (index, mutation) in Mutation::ALL.iter().enumerate() {
        assert_eq!(Mutation::parse(mutation.name()), Some(*mutation));
        for other in &Mutation::ALL[index + 1..] {
            assert_ne!(mutation.name(), other.name());
        }
    }
    assert_eq!(Mutation::ALL[0], Mutation::None);
}

#[test]
fn a_section_can_always_end_and_a_completion_can_always_be_read() {
    // TA, the termination assumption of LD1 6.2: every section that holds the
    // slot mutex ends. The model checks the half that it can: no reachable
    // state inside a section lacks a step, and a Stopping slot always offers
    // the observe step that retires it (P1).
    for name in [
        "smoke", "control", "waits", "watchdog", "memory", "dock", "runner",
    ] {
        let profile = profile(name).expect("a named profile");
        let m = model(profile.bounds);
        let result = explore_lifecycle(&profile, Mutation::None, None, Limits::default(), true);
        for node in 0..result.states {
            let state = result.state_of(node);
            let mut offered = Vec::new();
            luau_model::Machine::actions(&m, state, &mut offered);
            if state.sec.is_some() {
                assert!(
                    !offered.is_empty(),
                    "{name}: a section is stuck in {}",
                    state.summary()
                );
            }
            if state.slot.chan.is_some() {
                assert!(
                    offered.contains(&Action::Observe),
                    "{name}: a completion waits and Observe is not offered in {}",
                    state.summary()
                );
            }
        }
    }
}

#[test]
fn the_head_reaches_each_rule_that_the_conformance_tests_drive() {
    let mut reached: Vec<Event> = Vec::new();
    for name in ["control", "waits", "watchdog", "memory"] {
        let profile = profile(name).expect("a named profile");
        let (run, found) = goal_traces(
            profile.bounds,
            Mutation::None,
            profile.bounds.horizon,
            Limits::default(),
            3,
        );
        assert!(run.failures.is_empty() && run.failure.is_none(), "{name}");
        reached.extend(found.into_iter().map(|(event, _)| event));
    }
    for goal in GOALS {
        assert!(reached.contains(&goal), "no unit profile reaches {goal:?}");
    }
}

// ---- the four traces that LD1 6.5 requires the model to accept ----

#[test]
fn trace_one_stop_start_observe_spawns_the_replacement() {
    let m = model(Bounds {
        starts: 2,
        ctl: 2,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([
        Action::Stop,
        RETURN,
        START,
        Action::Observe,
        load_table(true, true),
    ]);
    let (state, events) = run(&m, &trace);
    assert_eq!(state.slot.state, RunState::Starting);
    assert!(events.contains(&Event::QueuedStartSpawned));
    assert_eq!(state.run.as_ref().map(|r| r.id), Some(2));
}

#[test]
fn trace_two_stop_start_stop_observe_ends_idle() {
    let m = model(Bounds {
        starts: 2,
        ctl: 3,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([Action::Stop, RETURN, START, Action::Stop, Action::Observe]);
    let (state, events) = run(&m, &trace);
    assert_eq!(state.slot.state, RunState::Idle);
    assert!(state.run.is_none());
    assert!(events.contains(&Event::QueuedStartDropped));
}

#[test]
fn trace_three_self_stop_then_the_deadline_passes_while_the_entry_unwinds() {
    // LD1 4.3 and the review of round 3: I7 a holds unless the script stopped
    // itself first. `api.stop` at the last unit of the budget, and the exit
    // after the deadline, is accepted.
    let m = model(Bounds {
        clock: true,
        self_stop: true,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([
        TICK,
        Action::Advance,
        STOP_CALL,
        Action::Advance,
        FAIL,
        // The dispatch is accepted; the log drain stops the run.
        Action::Observe,
        RETURN,
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::DispatchSelfStopped));
    assert!(
        events.contains(&Event::HookRan),
        "onStop runs after api.stop"
    );
    assert_eq!(state.slot.receipt, Receipt::Stopped);
    assert_eq!(state.slot.state, RunState::Idle);
}

#[test]
fn trace_four_tick_and_paint_together_pass_the_dispatch_budget() {
    let m = model(Bounds {
        clock: true,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([
        TICK,
        Action::Advance,
        RETURN,
        Action::Next,
        Action::Advance,
        // The Paint entry reaches its return after the dispatch budget: the
        // check trips, and the error unwinds out of the entry.
        RETURN,
        FAIL,
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::DispatchFailed(Fail::Watchdog)));
    assert_eq!(state.slot.receipt, Receipt::Failed);
    assert_eq!(state.slot.state, RunState::Stopping);
}

#[test]
fn self_stop_followed_by_an_internal_panic_is_accepted_and_skips_the_hook() {
    // The review of round 3, requirement 3: I11 a holds for a failure that
    // is the first terminal cause. A panic after `api.stop` is not.
    let m = model(Bounds {
        panics: 1,
        self_stop: true,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([TICK, STOP_CALL, Action::Vm(Op::Panic), Action::Observe]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::InternalAfterSelfStop));
    assert!(events.contains(&Event::DispatchSelfStopped));
    assert!(events.contains(&Event::HookSkipped));
    assert_eq!(state.slot.receipt, Receipt::Stopped);
    assert!(
        state.slot.card.hook,
        "the internal failure is reported once"
    );
    assert_eq!(state.slot.hook_logs, 1);
}

#[test]
fn a_memory_event_after_a_self_stop_does_not_change_the_latch() {
    let m = model(Bounds {
        caps: 1,
        self_stop: true,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([
        TICK,
        STOP_CALL,
        Action::Vm(Op::Alloc(Alloc::Refuse(Site::Host))),
        FAIL,
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::DispatchSelfStopped));
    assert_eq!(
        state.run.as_ref().map(|r| r.latch),
        Some(Latch::SelfStop),
        "the first terminal cause stays"
    );
}

// ---- the rules of the check ----

#[test]
fn a_trip_in_a_tick_entry_is_one_failed_dispatch() {
    let m = model(Bounds {
        clock: true,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([
        TICK,
        Action::Advance,
        Action::Advance,
        // Straight-line code; the next check point trips, and the error
        // unwinds out of the entry.
        Action::Vm(Op::Point),
        FAIL,
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::Tripped(Fail::Watchdog)));
    assert!(state.run.is_none(), "the run is retired in the same call");
    assert_eq!(state.slot.receipt, Receipt::Failed);
}

/// LD1 3.4 and 6.5: several operations can run between the deadline and the
/// next check point, not one. The model offers two (`R_gap`), the next check
/// point still trips the entry, and a third operation is not offered.
#[test]
fn two_operations_after_the_deadline_still_trip_at_the_next_check_point() {
    let m = model(Bounds {
        clock: true,
        ..Bounds::minimal()
    });
    let gap = Action::Vm(Op::Gap);
    let mut trace = running();
    trace.extend([TICK, Action::Advance, Action::Advance, gap, gap]);
    let (state, _) = run(&m, &trace);
    assert!(!m.enabled(&state, &gap), "a third operation exceeds R_gap");
    trace.extend([Action::Vm(Op::Point), FAIL]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::Tripped(Fail::Watchdog)));
    assert_eq!(state.slot.receipt, Receipt::Failed);
}

#[test]
fn a_binding_after_the_deadline_trips_before_it_has_an_effect() {
    let m = model(Bounds {
        clock: true,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([
        TICK,
        Action::Advance,
        Action::Advance,
        Action::Vm(Op::Call(Binding::Row)),
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::Tripped(Fail::Watchdog)));
    assert_eq!(
        state.run.as_ref().map(|r| r.rows),
        Some(0),
        "the row was not queued"
    );
}

#[test]
fn pause_does_not_lose_the_suspended_invocation_or_its_completion() {
    let m = model(Bounds {
        waits: 1,
        seat_waits: true,
        ctl: 2,
        ticks: 2,
        paint: Shapes::Either,
        ..Bounds::minimal()
    });
    // A script with no `onPaint`, so the dispatch ends with its Tick entry.
    let mut trace = vec![START, load_table(true, false), Action::Observe];
    trace.extend([
        TICK,
        Action::Vm(Op::Call(Binding::Wait(Wait::Seat { typed: false }))),
        Action::Pause,
        Action::Arrive,
        Action::Resume,
    ]);
    let (state, _) = run(&m, &trace);
    let run_state = state.run.as_ref().expect("the run lives");
    assert_eq!(run_state.inv, Inv::Suspended(Wait::Seat { typed: false }));
    assert!(
        state.host.comp.is_some(),
        "the host still owes the completion"
    );
    // The first accepted dispatch after Resume delivers it, once.
    let (state, events) = run_from(&m, state, &[TICK, RETURN]);
    assert!(events.contains(&Event::InvocationResumed));
    assert!(events.contains(&Event::CompletionAcked));
    assert!(state.host.comp.is_none());
}

#[test]
fn a_held_tick_stores_the_completion_and_does_not_resume() {
    let m = model(Bounds {
        waits: 1,
        seat_waits: true,
        ticks: 2,
        held: 1,
        holds: 1,
        paint: Shapes::Either,
        ..Bounds::minimal()
    });
    let mut trace = vec![START, load_table(true, false), Action::Observe];
    trace.extend([
        TICK,
        Action::Vm(Op::Call(Binding::Wait(Wait::Seat { typed: false }))),
        Action::Arrive,
        Action::HoldOn,
        TICK,
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::CompletionStored));
    assert!(events.contains(&Event::CompletionAcked));
    assert!(!events.contains(&Event::InvocationResumed));
    let run_state = state.run.as_ref().expect("the run lives");
    assert!(
        run_state.stored.is_some(),
        "the run holds what the host acknowledged"
    );
    assert_eq!(run_state.inv, Inv::Suspended(Wait::Seat { typed: false }));
}

#[test]
fn memory_events_trip_at_every_site() {
    for site in [Site::Host, Site::Binding, Site::Guard] {
        let m = model(Bounds {
            caps: 1,
            ..Bounds::minimal()
        });
        let mut trace = running();
        trace.extend([TICK, Action::Vm(Op::Alloc(Alloc::Refuse(site))), FAIL]);
        let (state, events) = run(&m, &trace);
        assert!(events.contains(&Event::Tripped(Fail::Memory)), "{site:?}");
        assert_eq!(state.slot.receipt, Receipt::Failed);
    }
}

#[test]
fn a_lost_error_trips_with_internal() {
    let m = model(Bounds {
        caps: 1,
        ..Bounds::minimal()
    });
    let mut trace = running();
    trace.extend([TICK, Action::Vm(Op::Alloc(Alloc::Lost)), FAIL]);
    let (_, events) = run(&m, &trace);
    assert!(events.contains(&Event::Tripped(Fail::Internal)));
}

#[test]
fn an_e1_crossing_that_no_check_point_sees_is_not_a_cap_event() {
    let m = model(Bounds {
        caps: 1,
        paint: Shapes::Either,
        ..Bounds::minimal()
    });
    let mut trace = vec![START, load_table(true, false), Action::Observe];
    trace.extend([
        TICK,
        Action::Vm(Op::Alloc(Alloc::OverS)),
        Action::Vm(Op::Free),
        RETURN,
    ]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::DispatchAccepted));
    assert_eq!(state.run.as_ref().map(|r| r.latch), Some(Latch::Empty));
}

#[test]
fn the_latch_is_written_once() {
    let m = Lifecycle::new(
        Bounds {
            clock: true,
            self_stop: true,
            ..Bounds::minimal()
        },
        Mutation::LatchOverwritten,
    );
    let mut trace = running();
    trace.extend([TICK, STOP_CALL, Action::Advance, Action::Advance, FAIL]);
    // Head: accepted (trace three). The mutation: a watchdog trip replaces
    // the self-stop.
    assert_eq!(violation(&m, &trace), "i11b-latch-overwritten");
}

// ---- the findings of this model: LD1 taken as written ----

/// F1. LD1 6.1 lists "yield at a wait" for a Paint entry. No rule says what
/// the run does with the yield. The head refuses a waiting binding in a Paint
/// entry (an error before the native call, as in `onStop`).
#[test]
fn finding_f1_a_wait_in_a_paint_entry_has_no_resumer() {
    let bounds = profile("waits").expect("a named profile").bounds;
    let m = Lifecycle::new(bounds, Mutation::F1PaintWaitSuspends);
    let result = explore_lifecycle(
        &profile("waits").expect("a named profile"),
        Mutation::F1PaintWaitSuspends,
        None,
        Limits::default(),
        true,
    );
    let (found, trace) = result
        .failures
        .iter()
        .find(|(v, _)| v.name == "i6-paint-suspends")
        .expect("F1 is found");
    assert!(found.detail.contains("Paint"));
    // Start, Load, the tick, the Tick entry returns, Next, the wait.
    assert_eq!(trace.len(), 5, "{trace:?}");
    // The head does not offer the wait in a Paint entry at all.
    let head = model(bounds);
    let mut state = head.initial_state();
    for action in &trace[..trace.len() - 1] {
        state = head
            .step_detail(&state, action)
            .expect("the prefix is legal")
            .0;
    }
    assert!(!head.enabled(&state, &trace[trace.len() - 1]));
    assert!(m.enabled(&state, &trace[trace.len() - 1]));
}

/// F2. LD1 4.3 has the Stopped receipt written by the log drain. An operator
/// Stop that comes first leaves no receipt, while the first cause of the run
/// is the script's own request. The head writes the receipt from the latch at
/// the Stop.
#[test]
fn finding_f2_an_operator_stop_that_overtakes_the_log_drain_keeps_the_first_cause() {
    let bounds = Bounds {
        ctl: 2,
        clock: true,
        self_stop: true,
        ..Bounds::minimal()
    };
    let design = Lifecycle::new(bounds, Mutation::F2StopIgnoresSelfStop);
    let mut trace = vec![START, load_table(false, false)];
    trace.extend([Action::Observe, TICK, STOP_CALL, FAIL, Action::Stop]);
    // `load_table(false, false)` is not offered in these bounds: use the shape
    // that the bounds offer.
    let trace: Vec<Action> = trace
        .into_iter()
        .map(|a| {
            if a == load_table(false, false) {
                load_table(true, true)
            } else {
                a
            }
        })
        .collect();
    assert_eq!(trace.len(), 7);
    assert_eq!(violation(&design, &trace), "i11b-wrong-receipt");
    let (state, _) = run(&model(bounds), &trace[..6]);
    let head = model(bounds)
        .step_detail(&state, &Action::Stop)
        .expect("the head accepts the same Stop")
        .0;
    assert_eq!(head.slot.receipt, Receipt::Stopped);
}

/// F3. A run that called `api.stop` waits for the log drain. The progress
/// watchdog can restart it first, and the replacement runs although the
/// script asked to stop. The head does not restart a run that has stopped.
#[test]
fn finding_f3_the_watchdog_does_not_restart_a_run_that_asked_to_stop() {
    let bounds = Bounds {
        starts: 2,
        restarts: 1,
        self_stop: true,
        ..Bounds::minimal()
    };
    let design = Lifecycle::new(bounds, Mutation::F3RestartIgnoresSelfStop);
    let mut trace = running();
    trace.extend([TICK, STOP_CALL, FAIL, Action::Restart]);
    assert_eq!(violation(&design, &trace), "i11b-selfstop-overridden");
    let (state, _) = run(&model(bounds), &trace[..trace.len() - 1]);
    assert!(!model(bounds).enabled(&state, &Action::Restart));
}

/// The slot gate (`state == Running && want_run`) closes a dispatch before it
/// reaches a paused run, so the run's own refusal is unreachable at the head.
/// The designs that bypass the gate reach it, and I2 catches them.
#[test]
fn a_paused_run_never_receives_a_dispatch_at_the_head() {
    let bounds = Bounds {
        ctl: 2,
        ticks: 2,
        wakes: 1,
        ..Bounds::minimal()
    };
    let head = model(bounds);
    let mut trace = running();
    trace.push(Action::Pause);
    let (state, _) = run(&head, &trace);
    assert_eq!(state.slot.state, RunState::Paused);
    assert!(state.run.as_ref().is_some_and(|r| r.paused));
    assert!(!head.enabled(&state, &TICK));
    assert!(!head.enabled(&state, &Action::Dispatch(Wake::SeatPage)));
    let design = Lifecycle::new(bounds, Mutation::SeatPageIgnoresPause);
    assert!(design.enabled(&state, &Action::Dispatch(Wake::SeatPage)));
}

// ---- the runner side (LD2) ----

fn runner_bounds() -> Bounds {
    Bounds {
        starts: 2,
        ctl: 3,
        ticks: 1,
        fence: true,
        runner: true,
        ..Bounds::minimal()
    }
}

/// The runner Starts, fences, Stops through its own intent and passes.
fn clean_runner_trace() -> Vec<Action> {
    vec![
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        Action::Fence,
        Action::Post(Intent::Stop { serial: 1 }),
        Action::Pump,
        // The hook runs inside the Stop.
        RETURN,
        Action::Observe,
    ]
}

#[test]
fn the_final_gate_passes_a_run_that_the_runner_retired_with_its_own_ticket() {
    let m = model(runner_bounds());
    let mut trace = clean_runner_trace();
    trace.push(Action::FinalGate);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::Ticketed));
    assert!(events.contains(&Event::Verdict(Verdict::Pass)));
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Pass));
}

/// LD2 T03: the ticket is at generation g+1, a queued Start moves it to g+2,
/// the factory fails, and `fail_setup` gives g+1 back. The generation
/// equals the ticket's and the slot is Idle. The Start serial is the
/// evidence, and the gate refuses.
#[test]
fn t03_a_failed_queued_replacement_gives_the_generation_back_and_the_gate_still_fails() {
    let m = model(runner_bounds());
    let mut trace = vec![
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        Action::Fence,
        Action::Post(Intent::Stop { serial: 1 }),
        Action::Pump,
        RETURN,
        // The operator's Start queues behind the Stop; its spawn fails.
        START,
        Action::Observe,
        Action::Vm(Op::Return(Ret::Other)),
    ];
    let (state, events) = run(&m, &trace);
    assert_eq!(state.slot.state, RunState::Idle);
    assert!(state.slot.diag, "the failed setup leaves its diagnostic");
    let ticket_after = state
        .runner
        .ledger
        .iter()
        .flatten()
        .find_map(|e| match e.state {
            luau_model::lifecycle::ExecState::Ticketed(t) => Some(t.gen_after),
            _ => None,
        })
        .expect("the runner's Stop is ticketed");
    assert_eq!(
        state.slot.generation, ticket_after,
        "fail_setup restored the generation of the ticket"
    );
    assert!(events.contains(&Event::StartQueued));
    trace.push(Action::FinalGate);
    let (state, _) = run(&m, &trace);
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Fail));
    // The design that compares generations passes it.
    let design = Lifecycle::new(
        runner_bounds(),
        Mutation::FailedReplacementRestoresAuthorization,
    );
    assert_eq!(violation(&design, &trace), "l-pass-unsound");
}

#[test]
fn a_foreign_stop_is_not_the_runners_retirement() {
    let m = model(runner_bounds());
    let trace = vec![
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        Action::Fence,
        Action::Stop,
        RETURN,
        Action::Observe,
        Action::FinalGate,
    ];
    let (state, _) = run(&m, &trace);
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Fail));
    assert!(state.runner.foreign_end);
    let design = Lifecycle::new(runner_bounds(), Mutation::GuessedStopGeneration);
    assert_eq!(violation(&design, &trace), "l-pass-unsound");
}

/// LD2 3.2 and 5.4: the final ticket belongs to the slot at the moment of the
/// runner's Stop. A later operator Stop of the idle slot moves the generation
/// (the ticket says 2, the live slot says 3), and the final gate compares the
/// live Idle slot with the final ticket. The 11-step trace is the one that
/// the gate review of round 2 found on the first head.
#[test]
fn an_operator_stop_of_the_idle_slot_after_the_runner_ticket_fails_the_final_gate() {
    let m = model(runner_bounds());
    let mut trace = clean_runner_trace();
    trace.extend([Action::Stop, Action::FinalGate]);
    assert_eq!(trace.len(), 11);
    let (state, _) = run(&m, &trace);
    let ticket_after = state
        .runner
        .ledger
        .iter()
        .flatten()
        .find_map(|exec| match exec.state {
            luau_model::lifecycle::ExecState::Ticketed(t) => Some(t.gen_after),
            _ => None,
        })
        .expect("the runner's Stop is ticketed");
    assert_eq!((ticket_after, state.slot.generation), (2, 3));
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Fail));
    // The design that takes the ticket as proof passes it.
    let design = Lifecycle::new(runner_bounds(), Mutation::StaleFinalTicket);
    assert_eq!(violation(&design, &trace), "l-pass-unsound");
}

fn chain_bounds() -> Bounds {
    Bounds {
        starts: 2,
        ctl: 3,
        ticks: 0,
        fence: true,
        runner: true,
        plan: 2,
        ..Bounds::minimal()
    }
}

/// Two planned executions: the runner's own Stop retires the first, the
/// next Start moves the generation by one from that ticket, the runner's own
/// Stop retires the second, and the gate passes the chain (LD2 3.2).
fn chain_trace(foreign_stop_between: bool) -> Vec<Action> {
    let mut trace = vec![
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        Action::Fence,
        Action::Post(Intent::Stop { serial: 1 }),
        Action::Pump,
        RETURN,
        Action::Observe,
    ];
    if foreign_stop_between {
        trace.push(Action::Stop);
    }
    trace.extend([
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        Action::Post(Intent::Stop { serial: 2 }),
        Action::Pump,
        RETURN,
        Action::Observe,
        Action::FinalGate,
    ]);
    trace
}

#[test]
fn a_chain_of_two_planned_executions_passes_the_final_gate() {
    let m = model(chain_bounds());
    let (state, _) = run(&m, &chain_trace(false));
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Pass));
    assert_eq!(state.slot.serial, 2);
}

/// LD2 3.2: an earlier ticket is linked to the next admission of the chain,
/// not compared with the final live slot. An operator Stop of the idle slot
/// between the two executions moves the generation by one more, and the gate
/// fails the chain although the final ticket matches the live slot.
#[test]
fn an_operator_stop_between_two_planned_executions_breaks_the_chain() {
    let m = model(chain_bounds());
    let trace = chain_trace(true);
    let (state, _) = run(&m, &trace);
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Fail));
    let design = Lifecycle::new(chain_bounds(), Mutation::UnlinkedEarlierTicket);
    assert_eq!(violation(&design, &trace), "l-pass-unsound");
}

/// LD2 2.4: a refused call in the Load entry writes the first fault, and
/// revocation and close keep it. The earlier design that cleared the latch on
/// revocation passed every check until the retained history existed.
#[test]
fn revocation_and_close_keep_the_first_fault() {
    let bounds = Bounds {
        dock: true,
        ctl: 2,
        ..Bounds::minimal()
    };
    let m = model(bounds);
    let call = Action::Vm(Op::Call(Binding::Ext { awaited: false }));
    let trace = vec![
        START,
        call,
        load_table(true, true),
        Action::Revoke,
        Action::Stop,
        RETURN,
        Action::Observe,
    ];
    let (state, _) = run(&m, &trace);
    let closed = state.closed_dock.expect("the dock closed");
    assert_eq!(closed.faults, 1);
    assert!(closed.end.is_some_and(|end| end.fault));
    let design = Lifecycle::new(bounds, Mutation::RevokeClearsFirstFault);
    assert_eq!(violation(&design, &trace), "d-first-fault-cleared");
}

#[test]
fn a_stop_intent_for_a_retired_execution_is_refused() {
    let m = model(runner_bounds());
    let mut trace = clean_runner_trace();
    trace.extend([START, load_table(true, true), Action::Observe]);
    let (state, _) = run(&m, &trace);
    // The replacement is live; the runner has nothing to stop: its own
    // execution is retired, and the replacement is not in its ledger plan.
    assert_eq!(state.run.as_ref().map(|r| r.serial), Some(2));
    assert!(!m.enabled(&state, &Action::Post(Intent::Stop { serial: 1 })));
}

fn natural_bounds() -> Bounds {
    Bounds {
        self_stop: true,
        natural: true,
        ..runner_bounds()
    }
}

/// LD2 4.1 and T30: a plan that requires the natural self-stop. The script
/// calls `api.stop`, the slot stops it, and the idle slot gets the one
/// housekeeping Stop of the runner. The ticket and the natural witness pass
/// the final gate, and the end is not the foreign end of an unplanned Stop.
#[test]
fn a_natural_self_stop_retired_by_the_housekeeping_stop_passes_the_final_gate() {
    let m = model(natural_bounds());
    let trace = vec![
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        TICK,
        STOP_CALL,
        FAIL,
        // The slot reads the request and stops the run: `onStop` runs.
        Action::Observe,
        RETURN,
        Action::Fence,
        Action::Post(Intent::Stop { serial: 1 }),
        Action::Pump,
        Action::FinalGate,
    ];
    let (state, events) = run(&m, &trace);
    assert!(!state.runner.foreign_end);
    assert!(events.contains(&Event::Stopped(
        luau_model::lifecycle::StopReason::ScriptStop
    )));
    assert!(events.contains(&Event::Ticketed));
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Pass));
}

/// LD2 T30: a forced Stop cannot manufacture the natural self-stop. The
/// runner's own Stop retires the run with a ticket and a clean end, and the
/// plan that requires the natural stop still fails.
#[test]
fn a_forced_stop_does_not_satisfy_a_natural_stop_plan() {
    let m = model(natural_bounds());
    let mut trace = clean_runner_trace();
    trace.push(Action::FinalGate);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::Ticketed));
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Fail));
    // The same trace passes when the plan does not ask for the natural stop.
    let plain = model(runner_bounds());
    let (state, _) = run(&plain, &trace);
    assert_eq!(state.runner.phase, RunnerPhase::Done(Verdict::Pass));
    // The design that lets the ticket stand for the witness passes it.
    let design = Lifecycle::new(natural_bounds(), Mutation::ForcedStopSatisfiesNaturalStop);
    assert_eq!(violation(&design, &trace), "l-pass-unsound");
}

/// A self-stop that the plan did not ask for is an end that the runner did
/// not order, and the housekeeping Stop is offered only after a natural end.
#[test]
fn an_unplanned_self_stop_is_foreign_and_no_housekeeping_stop_follows() {
    let plain = model(Bounds {
        self_stop: true,
        ..runner_bounds()
    });
    let trace = vec![
        Action::Post(Intent::Start),
        Action::Pump,
        load_table(true, true),
        Action::Observe,
        TICK,
        STOP_CALL,
        FAIL,
        Action::Observe,
        RETURN,
    ];
    let (state, _) = run(&plain, &trace);
    assert!(state.runner.foreign_end);
    assert!(!plain.enabled(&state, &Action::Post(Intent::Stop { serial: 1 })));
    // With the natural plan the same end is the witness and the Stop is offered.
    let natural = model(natural_bounds());
    let (state, _) = run(&natural, &trace);
    assert!(!state.runner.foreign_end);
    assert!(natural.enabled(&state, &Action::Post(Intent::Stop { serial: 1 })));
}

#[test]
fn a_retained_start_cannot_claim_the_dock() {
    let bounds = Bounds {
        starts: 2,
        restarts: 1,
        dock: true,
        ..Bounds::minimal()
    };
    let m = model(bounds);
    let mut trace = running();
    trace.extend([Action::Restart, RETURN, Action::Observe]);
    let (state, events) = run(&m, &trace);
    assert!(
        events.contains(&Event::SpawnFailed),
        "the claim failed before any script code ran"
    );
    assert_eq!(state.slot.state, RunState::Idle);
    assert!(state.slot.diag);
    let design = Lifecycle::new(bounds, Mutation::SharedRestartDock);
    assert_eq!(violation(&design, &trace), "d-dock-shared");
}

#[test]
fn a_staged_call_is_discarded_when_the_script_stops_itself_and_queued_otherwise() {
    let bounds = Bounds {
        ticks: 2,
        dock: true,
        self_stop: true,
        paint: Shapes::Either,
        ..Bounds::minimal()
    };
    let m = model(bounds);
    let call = Action::Vm(Op::Call(Binding::Ext { awaited: false }));
    let mut queued = vec![
        START,
        load_table(true, false),
        Action::Observe,
        TICK,
        call,
        RETURN,
    ];
    let (state, _) = run(&m, &queued);
    assert_eq!(
        state.run.as_ref().map(|r| r.dock.call),
        Some(Call::Queued { awaited: false })
    );
    queued.truncate(4);
    queued.extend([call, STOP_CALL, FAIL]);
    let (state, events) = run(&m, &queued);
    assert!(events.contains(&Event::DispatchSelfStopped));
    assert_eq!(
        state.run.as_ref().map(|r| r.dock.call),
        Some(Call::Discarded)
    );
}

#[test]
fn the_entry_fence_denies_every_wake_and_the_stop_still_runs_the_hook() {
    let bounds = Bounds {
        ticks: 2,
        ctl: 2,
        fence: true,
        ..Bounds::minimal()
    };
    let m = model(bounds);
    let mut trace = running();
    trace.extend([Action::Fence, TICK]);
    let (state, events) = run(&m, &trace);
    assert!(events.contains(&Event::Denied));
    assert!(state.sec.is_none(), "a denied wake enters nothing");
    let (_, events) = run_from(&m, state, &[Action::Stop, RETURN]);
    assert!(events.contains(&Event::HookRan));
}

// ---- the explorer's own results ----

#[test]
fn the_longer_horizon_does_not_change_a_profile_that_is_exhausted() {
    let profile = profile("smoke").expect("a named profile");
    let short = explore_lifecycle(&profile, Mutation::None, None, Limits::default(), true);
    let long = explore_lifecycle(
        &profile,
        Mutation::None,
        Some(profile.bounds.horizon + 10),
        Limits::default(),
        true,
    );
    assert_eq!(short.states, long.states);
    assert_eq!(short.transitions, long.transitions);
}

#[test]
fn a_cap_stops_the_run_and_marks_it_incomplete() {
    let profile = profile("control").expect("a named profile");
    let result = explore_lifecycle(
        &profile,
        Mutation::None,
        None,
        Limits {
            state_cap: 100,
            ..Limits::default()
        },
        true,
    );
    assert!(!result.complete);
    assert!(result.cap.is_some());
}

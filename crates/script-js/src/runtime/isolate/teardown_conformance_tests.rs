//! Conformance of the teardown functions to `tools/js-teardown-model`.
//!
//! Each trace is a path of the head model. [`Replay`] drives the real
//! functions of `teardown.rs` through it, with a V8 double that records the
//! pending termination request, and compares the real state with the model's
//! after every step: the phase, Pause intent, the active eval with its
//! deadline owner and its interrupt, the identity counter, whether V8 holds a
//! request, the registrations with the supervisor, the hook's flags and
//! `machines_halted`. A step where the two differ fails the test and names
//! the field.
//!
//! The drivers call the functions of `teardown.rs` in the order of the
//! model's actions, one action per lock section, the way the product calls
//! them: `watchdog_look` then `begin_arm` or `fire_execution_interrupt`,
//! `begin_stop`, `revoke`, `enter_teardown_hook`, `hook_deadline`,
//! `finish_teardown_hook`, `run_deadline`. What the tests do not run is the
//! glue that owns a `Runtime` (`teardown_once`, `arm_hook_deadline`,
//! `call_interruptible`) and the supervisor's own queue; the isolate tests
//! and the live cells cover those.
//!
//! Three seeded divergences ([`Seed`]) put an earlier behaviour of the code
//! into the driver. The tests require each to fail, so a conformance test
//! that cannot see a change is itself a failure.

use super::*;
use js_teardown_model::teardown as model;
use js_teardown_model::{teardown_goal_traces, teardown_profile, Limits, TEARDOWN_GOALS};
use model::{Action, Event, Host, Kind, Mutation, Owner, Phase, Who};
use std::cell::Cell;

/// One model time unit is one watchdog horizon, and the hook budget is one.
const UNIT: Duration = EXECUTION_WATCHDOG_HORIZON;

#[derive(Default)]
struct FakeV8 {
    pending: Cell<bool>,
    terminated: Cell<u32>,
}

impl Terminate for FakeV8 {
    fn terminate(&self) {
        self.pending.set(true);
        self.terminated.set(self.terminated.get() + 1);
    }
}

impl CancelTerminate for FakeV8 {
    fn cancel_terminate(&mut self) {
        self.pending.set(false);
    }
}

/// A behaviour of the code that the driver can put back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seed {
    /// The head: the real functions.
    Head,
    /// `Drop` overwrites a running hook with Done (D1, `a3a50d098`).
    DropOverwritesHook,
    /// `finish_interruptible_execution` leaves V8's request pending.
    FinishKeepsRequest,
    /// Stop terminates whatever phase it finds.
    StopTerminatesInHook,
}

const SEEDS: [Seed; 3] = [
    Seed::DropOverwritesHook,
    Seed::FinishKeepsRequest,
    Seed::StopTerminatesInHook,
];

fn model_owner(owner: ExecutionInterrupt) -> Owner {
    match owner {
        ExecutionInterrupt::Watchdog => Owner::Watchdog,
        ExecutionInterrupt::Pause => Owner::Pause,
        ExecutionInterrupt::SessionReset => Owner::Reset,
        ExecutionInterrupt::NoDeadline(_) => Owner::NoDeadline,
    }
}

struct Replay {
    teardown: Mutex<Teardown>,
    v8: FakeV8,
    base: Instant,
    now: u8,
    /// The supervisor's registrations, in the model's order.
    entries: Vec<(model::Entry, DeadlineKind)>,
    host_arm: Option<(DeadlineArm, ExecutionInterrupt)>,
    host_refused: Option<DeadlineArm>,
    thread_arm: Option<DeadlineArm>,
    thread_refused: Option<DeadlineArm>,
    look: Option<WatchdogLook>,
    hook_at: Option<Instant>,
    hook_runs: u8,
    seed: Seed,
}

type Check = Result<(), String>;

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Check {
    if ok {
        Ok(())
    } else {
        Err(what())
    }
}

impl Replay {
    fn new(seed: Seed) -> Self {
        let mut teardown = Teardown::new();
        // The hook budget of the model is one unit.
        teardown.test_hook_timeout = Some(UNIT);
        Self {
            teardown: Mutex::new(teardown),
            v8: FakeV8::default(),
            base: Instant::now(),
            now: 0,
            entries: Vec::new(),
            host_arm: None,
            host_refused: None,
            thread_arm: None,
            thread_refused: None,
            look: None,
            hook_at: None,
            hook_runs: 0,
            seed,
        }
    }

    fn at(&self, units: u8) -> Instant {
        self.base + UNIT * u32::from(units)
    }

    fn units(&self, at: Instant) -> u8 {
        u8::try_from((at - self.base).as_millis() / UNIT.as_millis()).unwrap_or(u8::MAX)
    }

    fn register(&mut self, id: u64, owner: ExecutionInterrupt, at: Instant) {
        let entry = model::Entry::Exec {
            id: u8::try_from(id).unwrap_or(u8::MAX),
            owner: model_owner(owner),
            at: self.units(at),
        };
        self.insert(entry, DeadlineKind::Execution { id, owner });
    }

    fn insert(&mut self, entry: model::Entry, kind: DeadlineKind) {
        self.entries.push((entry, kind));
        self.entries.sort_by_key(|(entry, _)| *entry);
    }

    /// Run the real functions of one model action.
    fn apply(&mut self, action: Action, events: &[Event], next: &model::State) -> Check {
        let now = self.at(self.now);
        let had = |event: Event| events.contains(&event);
        match action {
            Action::Advance => self.now += 1,
            Action::Enter(kind) => {
                let accepted = match kind {
                    Kind::Tick => begin_interruptible_execution(
                        &self.teardown,
                        100 + u64::from(self.now),
                        1,
                        now,
                    ),
                    Kind::Frame => begin_frame_execution(
                        &self.teardown,
                        super::super::FRAME_CALLBACK_TICK,
                        1,
                        now,
                    ),
                    Kind::Lifecycle => begin_lifecycle_hook_execution(&self.teardown, 1, now),
                };
                let want = events.iter().any(|e| matches!(e, Event::EntryAccepted(_)));
                ensure(accepted == want, || {
                    format!("{kind:?} entry: code accepted={accepted}, model accepted={want}")
                })?;
            }
            Action::EnterCheck(_) | Action::EnterCommit => {
                return Err("a mutation-only action in a head trace".into());
            }
            Action::Deliver => {
                note_interrupt_consumed(&self.teardown, || self.v8.pending.get());
            }
            Action::ArmWatchdog => {
                self.thread_arm = begin_arm(&self.teardown, ExecutionInterrupt::Watchdog);
                ensure(
                    self.thread_arm.is_some() == had(Event::Armed(Owner::Watchdog)),
                    || "the thread's watchdog arm: code and model disagree".into(),
                )?;
            }
            Action::Register { by, refuse } => {
                let (arm, owner) = match by {
                    Who::Host => self.host_arm.take().ok_or("no host arm to register")?,
                    Who::Thread => (
                        self.thread_arm.take().ok_or("no thread arm to register")?,
                        ExecutionInterrupt::Watchdog,
                    ),
                };
                if refuse {
                    match by {
                        Who::Host => self.host_refused = Some(arm),
                        Who::Thread => self.thread_refused = Some(arm),
                    }
                } else {
                    self.register(arm.id, owner, arm.at);
                    finish_arm(&self.teardown, &self.v8, arm, Ok(())).map_err(|e| e.to_string())?;
                }
            }
            Action::FireRefused(by) => {
                let arm = match by {
                    Who::Host => self.host_refused.take(),
                    Who::Thread => self.thread_refused.take(),
                }
                .ok_or("no refused arm")?;
                let refused = finish_arm(
                    &self.teardown,
                    &self.v8,
                    arm,
                    Err(LoadIsolateError::SupervisorCapacity),
                );
                ensure(refused.is_err(), || "the refusal is returned".into())?;
            }
            Action::Finish | Action::FinishClear => {
                let interrupt = if self.seed == Seed::FinishKeepsRequest {
                    self.teardown.lock().unwrap().state.finish_execution()
                } else {
                    finish_interruptible_execution(&mut self.v8, &self.teardown)
                };
                let cut = events.iter().find_map(|e| match e {
                    Event::FinishCut { consumed } => Some(*consumed),
                    _ => None,
                });
                ensure(interrupt.map(|i| i.consumed) == cut, || {
                    format!("finish returned {interrupt:?}; the model's cut is {cut:?}")
                })?;
            }
            Action::StopCommand | Action::ScriptStop | Action::StopSend | Action::DropSend => {}
            Action::EnterHook => {
                let entered = enter_teardown_hook(&self.teardown, now);
                ensure(entered == had(Event::HookEntered), || {
                    format!("hook entry: code entered={entered}")
                })?;
            }
            Action::HookGuard => {
                self.hook_at = hook_deadline(&self.teardown);
                ensure(self.hook_at.is_some() == !had(Event::HookSkipped), || {
                    "the hook deadline's phase guard: code and model disagree".into()
                })?;
            }
            Action::HookCancel | Action::CancelAfter => self.v8.cancel_terminate(),
            Action::HookRegister { refuse } => {
                if !refuse {
                    let at = self.hook_at.ok_or("no hook deadline to register")?;
                    let entry = model::Entry::Hook { at: self.units(at) };
                    self.insert(entry, DeadlineKind::Hook);
                    self.hook_runs += 1;
                }
            }
            Action::HookDone => {}
            Action::Complete => finish_teardown_hook(&self.teardown),
            Action::Poll | Action::Pause => {
                self.look = watchdog_look(&self.teardown, true, now);
                let want = match next.host {
                    Host::Look { within, .. } => Some(within),
                    _ => None,
                };
                let got = self.look.map(|look| matches!(look, WatchdogLook::Within));
                ensure(got == want, || {
                    format!("watchdog look: code {:?}, model within={want:?}", self.look)
                })?;
            }
            Action::Look => match self.look.take().ok_or("no look to act on")? {
                WatchdogLook::Within => {
                    self.host_arm = begin_arm(&self.teardown, ExecutionInterrupt::Watchdog)
                        .map(|arm| (arm, ExecutionInterrupt::Watchdog));
                    ensure(
                        self.host_arm.is_some() == had(Event::Armed(Owner::Watchdog)),
                        || "the watchdog's arm: code and model disagree".into(),
                    )?;
                }
                WatchdogLook::Past { id, .. } => {
                    let fired = fire_execution_interrupt(
                        &self.teardown,
                        Some(&self.v8),
                        id,
                        ExecutionInterrupt::Watchdog,
                    );
                    ensure(fired == had(Event::PollFired), || {
                        format!("the watchdog's cut: code fired={fired}")
                    })?;
                }
            },
            Action::PauseSet => self.teardown.lock().unwrap().state.request_pause(),
            Action::PauseArm => {
                self.host_arm = begin_arm(&self.teardown, ExecutionInterrupt::Pause)
                    .map(|arm| (arm, ExecutionInterrupt::Pause));
                ensure(
                    self.host_arm.is_some() == had(Event::Armed(Owner::Pause)),
                    || "Pause's arm: code and model disagree".into(),
                )?;
            }
            Action::Resume => self.teardown.lock().unwrap().state.clear_pause(),
            Action::Reset => {
                self.host_arm = begin_arm(&self.teardown, ExecutionInterrupt::SessionReset)
                    .map(|arm| (arm, ExecutionInterrupt::SessionReset));
                ensure(
                    self.host_arm.is_some() == had(Event::Armed(Owner::Reset)),
                    || "reset's arm: code and model disagree".into(),
                )?;
            }
            Action::StopLock => {
                begin_stop(&self.teardown, Some(&self.v8));
                if self.seed == Seed::StopTerminatesInHook
                    && matches!(
                        self.teardown.lock().unwrap().state,
                        TeardownState::Hook { .. }
                    )
                {
                    self.v8.terminate();
                }
            }
            Action::Drop => {
                if self.seed == Seed::DropOverwritesHook {
                    let mut st = self.teardown.lock().unwrap();
                    let execution = st.state.execution().copied();
                    if !matches!(st.state, TeardownState::Done { .. }) {
                        st.state = TeardownState::Done { execution };
                    }
                } else {
                    revoke(&self.teardown);
                }
            }
            Action::DropFire => self.v8.terminate(),
            Action::Fire(i) => {
                let (_, kind) = self
                    .entries
                    .get(i)
                    .copied()
                    .ok_or("no registration to decide")?;
                self.entries.remove(i);
                run_deadline(&self.teardown, kind, &self.v8);
            }
        }
        Ok(())
    }

    /// Compare the real state with the model's.
    fn check(&self, m: &model::State) -> Check {
        let st = self.teardown.lock().unwrap();
        let phase = match st.state {
            TeardownState::Running { .. } => Phase::Running,
            TeardownState::UnwindingTick { .. } => Phase::Unwinding,
            TeardownState::Hook { .. } => Phase::Hook,
            TeardownState::Done { .. } => Phase::Done,
        };
        ensure(phase == m.phase, || {
            format!("phase: code {phase:?}, model {:?}", m.phase)
        })?;
        if phase == Phase::Running {
            ensure(st.state.pause_requested() == m.pause, || {
                format!(
                    "Pause intent: code {}, model {}",
                    st.state.pause_requested(),
                    m.pause
                )
            })?;
        }
        match (st.state.execution(), &m.exec) {
            (None, None) => {}
            (Some(real), Some(exec)) => {
                ensure(real.id == u64::from(exec.id), || {
                    format!("eval id: code {}, model {}", real.id, exec.id)
                })?;
                ensure(self.units(real.started) == exec.started, || {
                    "eval start differs".into()
                })?;
                ensure(real.deadline.map(model_owner) == exec.armed, || {
                    format!(
                        "deadline owner: code {:?}, model {:?}",
                        real.deadline, exec.armed
                    )
                })?;
                let interrupt = real.interrupt.map(|i| (model_owner(i.owner), i.consumed));
                ensure(
                    interrupt == exec.interrupt.map(|i| (i.owner, i.consumed)),
                    || format!("interrupt: code {interrupt:?}, model {:?}", exec.interrupt),
                )?;
            }
            (real, exec) => {
                return Err(format!("active eval: code {real:?}, model {exec:?}"));
            }
        }
        ensure(st.executions == u64::from(m.last_id), || {
            format!(
                "eval ids issued: code {}, model {}",
                st.executions, m.last_id
            )
        })?;
        ensure(self.v8.pending.get() == (m.flag != 0), || {
            format!(
                "V8 holds a request: code {}, model flag {:#09b}",
                self.v8.pending.get(),
                m.flag
            )
        })?;
        let entries: Vec<_> = self.entries.iter().map(|(entry, _)| *entry).collect();
        ensure(entries == m.entries, || {
            format!("registrations: code {entries:?}, model {:?}", m.entries)
        })?;
        if let TeardownState::Hook { fired, .. } = st.state {
            ensure(fired == m.hook_fired, || {
                format!("hook deadline fired: code {fired}, model {}", m.hook_fired)
            })?;
        }
        ensure(self.hook_runs == m.hook_runs, || {
            format!("hook runs: code {}, model {}", self.hook_runs, m.hook_runs)
        })?;
        let halted = m.phase != Phase::Running || m.exec.is_some_and(|e| e.interrupt.is_some());
        ensure(st.state.halted() == halted, || {
            format!(
                "machines_halted: code {}, model {halted}",
                st.state.halted()
            )
        })?;
        Ok(())
    }
}

/// Replay `trace` of the head model on the real functions.
fn replay(bounds: model::Bounds, trace: &[Action], seed: Seed) -> Result<Replay, String> {
    let m = model::Teardown::new(bounds, Mutation::None);
    let mut state = m.initial_state();
    let mut real = Replay::new(seed);
    for (step, action) in trace.iter().enumerate() {
        ensure(m.enabled(&state, action), || {
            format!("step {}: {action:?} is not offered by the model", step + 1)
        })?;
        let (next, events) = m
            .step_detail(&state, action)
            .map_err(|v| format!("step {}: the head model violates {}", step + 1, v.name))?;
        real.apply(*action, &events, &next)
            .and_then(|()| real.check(&next))
            .map_err(|why| {
                format!(
                    "step {} ({}): {why}\n{}",
                    step + 1,
                    action.text(),
                    model::describe_trace(&m, &trace[..=step])
                )
            })?;
        state = next;
    }
    Ok(real)
}

fn bounds(name: &str) -> model::Bounds {
    teardown_profile(name).expect("a named profile").bounds
}

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// The shortest traces that reach each rule of the protocol, with the
/// frame and lifecycle entries on.
fn goal_traces(cap: usize) -> Vec<(Event, Vec<Vec<Action>>)> {
    let profile = bounds("frames");
    let (run, goals) = teardown_goal_traces(
        profile,
        Mutation::None,
        profile.horizon,
        Limits::default(),
        cap,
    );
    assert!(run.complete && run.cap.is_none() && run.failures.is_empty());
    goals
}

#[test]
fn the_real_functions_follow_the_shortest_trace_of_every_rule() {
    let goals = goal_traces(env_count("TEARDOWN_CONFORMANCE_TRACES", 25));
    let missing: Vec<_> = TEARDOWN_GOALS
        .iter()
        .filter(|goal| !goals.iter().any(|(event, _)| event == *goal))
        .collect();
    assert!(
        missing.is_empty(),
        "the model reaches no trace of {missing:?}"
    );
    let mut replayed = 0usize;
    for (event, traces) in &goals {
        for trace in traces {
            if let Err(why) = replay(bounds("frames"), trace, Seed::Head) {
                panic!("a trace to {event:?} diverges:\n{why}");
            }
            replayed += 1;
        }
    }
    assert!(replayed >= TEARDOWN_GOALS.len());
}

/// A deterministic xorshift walk over the enabled actions of the full
/// profile: long interleavings the shortest traces do not reach.
#[test]
fn the_real_functions_follow_random_walks_of_the_full_profile() {
    let profile = bounds("full");
    let m = model::Teardown::new(profile, Mutation::None);
    let walks = env_count("TEARDOWN_CONFORMANCE_WALKS", 3000);
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut steps = 0usize;
    for walk in 0..walks {
        let mut state = m.initial_state();
        let mut trace = Vec::new();
        for _ in 0..60 {
            let mut offered = Vec::new();
            js_teardown_model::Machine::actions(&m, &state, &mut offered);
            if offered.is_empty() {
                break;
            }
            let action = offered[usize::try_from(next() % offered.len() as u64).unwrap_or(0)];
            state = m
                .step_detail(&state, &action)
                .expect("head model is clean")
                .0;
            trace.push(action);
        }
        steps += trace.len();
        if let Err(why) = replay(profile, &trace, Seed::Head) {
            panic!("walk {walk} diverges:\n{why}");
        }
    }
    assert!(steps > walks);
}

/// A conformance test that cannot see an earlier behaviour of the code is
/// worthless: each seeded divergence must fail on some trace.
#[test]
fn each_seeded_divergence_fails_conformance() {
    let goals = goal_traces(25);
    for seed in SEEDS {
        let found = goals
            .iter()
            .flat_map(|(_, traces)| traces)
            .find_map(|trace| replay(bounds("frames"), trace, seed).err());
        let why = found.unwrap_or_else(|| panic!("{seed:?} passed every trace"));
        eprintln!("{seed:?} is caught: {}", why.lines().next().unwrap_or(""));
    }
}

/// Finding D1 on the real code. Raw Drop between the hook's phase check and
/// its cancel: Drop's termination is eaten by the cancel, and the
/// supervisor's hook deadline must still terminate the body.
#[test]
fn raw_drop_in_the_hook_window_leaves_the_hook_deadline_armed() {
    let trace = [
        Action::ScriptStop,
        Action::EnterHook,
        Action::HookGuard,
        Action::Drop,
        Action::DropSend,
        Action::DropFire,
        Action::HookCancel,
        Action::HookRegister { refuse: false },
        Action::Advance,
        Action::Fire(0),
    ];
    let real = replay(bounds("stop"), &trace, Seed::Head).expect("the head follows the model");
    assert!(
        real.v8.pending.get(),
        "the hook deadline terminated the body"
    );
    assert!(matches!(
        real.teardown.lock().unwrap().state,
        TeardownState::Hook { fired: true, .. }
    ));
    let old = replay(bounds("stop"), &trace, Seed::DropOverwritesHook)
        .err()
        .expect("the earlier Drop diverges");
    assert!(old.contains("phase: code Done, model Hook"), "{old}");
}

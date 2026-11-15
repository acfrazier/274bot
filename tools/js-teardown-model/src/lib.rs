//! Bounded conformance oracle for the JS isolate's teardown protocol. No
//! product crate depends on this crate, except as a dev-dependency of the
//! conformance tests in `crates/script-js`.
//!
//! Two models share one explorer:
//!
//! * [`teardown`]: evals, V8 termination requests, Pause, Stop, raw Drop and
//!   the `onStop` hook (the product's `TeardownState`);
//! * [`retire`]: the deadline supervisor's retirement registry and its
//!   capacity credits.
//!
//! The explorer (`explorer.rs`) searches a model breadth first inside caps
//! and returns the shortest trace of any violation. See README.md.
//!
//! The Luau surface reuses this shape for its own run lifecycle (ADR 0016,
//! L2a): copy `explorer.rs`, write a model beside these two, and keep this
//! crate JS-only.

pub mod explorer;
pub mod retire;
pub mod teardown;

pub use explorer::*;

/// A named set of bounds for one run of the teardown model.
#[derive(Clone, Copy, Debug)]
pub struct TeardownProfile {
    pub name: &'static str,
    pub bounds: teardown::Bounds,
}

/// A named set of bounds for one run of the retirement model.
#[derive(Clone, Debug)]
pub struct RetireProfile {
    pub name: &'static str,
    pub bounds: retire::Bounds,
}

/// The teardown profiles. Every run uses one of them, with a horizon.
pub fn teardown_profiles() -> Vec<TeardownProfile> {
    let std = teardown::Bounds::standard();
    vec![
        TeardownProfile {
            name: "smoke",
            bounds: teardown::Bounds {
                horizon: 24,
                max_exec: 1,
                max_now: 2,
                ..std
            },
        },
        // Ticks with Pause, Resume, reset, Stop, Drop and the script's own stop.
        TeardownProfile {
            name: "ticks",
            bounds: std,
        },
        // Frame and lifecycle entries do not honour Pause.
        TeardownProfile {
            name: "frames",
            bounds: teardown::Bounds {
                frames: true,
                ..std
            },
        },
        // A supervisor that never refuses: the reachable states of the
        // hook protocol with a registered deadline only.
        TeardownProfile {
            name: "no-refusal",
            bounds: teardown::Bounds {
                refusals: false,
                horizon: 32,
                ..std
            },
        },
        // The hook protocol alone: Stop, Drop and the script's stop, with no
        // Pause or reset to widen the search.
        TeardownProfile {
            name: "stop",
            bounds: teardown::Bounds {
                pause: false,
                reset: false,
                horizon: 30,
                max_now: 4,
                ..std
            },
        },
        // A longer clock: a Pause deadline can pass and a later eval start.
        TeardownProfile {
            name: "clock",
            bounds: teardown::Bounds {
                max_now: 5,
                max_exec: 3,
                horizon: 40,
                script_stop: false,
                raw_drop: false,
                ..std
            },
        },
        // Everything on, three evals, a longer clock. The nightly profile.
        TeardownProfile {
            name: "full",
            bounds: teardown::Bounds {
                frames: true,
                max_exec: 3,
                max_now: 4,
                horizon: 48,
                ..std
            },
        },
    ]
}

pub fn teardown_profile(name: &str) -> Option<TeardownProfile> {
    teardown_profiles().into_iter().find(|p| p.name == name)
}

/// The retirement profiles.
pub fn retire_profiles() -> Vec<RetireProfile> {
    let std = retire::Bounds::standard();
    vec![
        RetireProfile {
            name: "retire-smoke",
            bounds: retire::Bounds {
                horizon: 26,
                rows: vec![false, true],
                ..std.clone()
            },
        },
        RetireProfile {
            name: "retire",
            bounds: std.clone(),
        },
        // Room for two: the first quarantine does not exhaust it.
        RetireProfile {
            name: "retire-cap2",
            bounds: retire::Bounds {
                cap: 2,
                rows: vec![false, false, false, true],
                horizon: 36,
                ..std
            },
        },
    ]
}

pub fn retire_profile(name: &str) -> Option<RetireProfile> {
    retire_profiles().into_iter().find(|p| p.name == name)
}

/// Explore one teardown profile with one mutation. With `all_violations` the
/// run does not stop at the first violation: `Exploration::failures` holds
/// the shortest trace of each violation name.
pub fn explore_teardown(
    profile: &TeardownProfile,
    mutation: teardown::Mutation,
    horizon: Option<usize>,
    limits: Limits,
    all_violations: bool,
) -> Exploration<teardown::State, teardown::Action> {
    let model = teardown::Teardown::new(profile.bounds, mutation);
    explore(
        &model,
        horizon.unwrap_or(profile.bounds.horizon),
        limits,
        &Options {
            all_violations,
            ..Options::default()
        },
    )
}

/// Explore one retirement profile with one mutation.
pub fn explore_retire(
    profile: &RetireProfile,
    mutation: retire::Mutation,
    horizon: Option<usize>,
    limits: Limits,
    all_violations: bool,
) -> Exploration<retire::State, retire::Action> {
    let model = retire::Retire::new(profile.bounds.clone(), mutation);
    explore(
        &model,
        horizon.unwrap_or(profile.bounds.horizon),
        limits,
        &Options {
            all_violations,
            ..Options::default()
        },
    )
}

/// Every shortest trace to each event of [`TEARDOWN_GOALS`], for the
/// conformance tests.
pub type GoalTraces = Vec<(teardown::Event, Vec<Vec<teardown::Action>>)>;

pub fn teardown_goal_traces(
    bounds: teardown::Bounds,
    mutation: teardown::Mutation,
    horizon: usize,
    limits: Limits,
    cap: usize,
) -> (Exploration<teardown::State, teardown::Action>, GoalTraces) {
    let model = teardown::Teardown::new(bounds, mutation);
    let goal = |pre: &teardown::State,
                action: &teardown::Action,
                _post: &teardown::State,
                out: &mut Vec<u32>| {
        if let Ok((_, events)) = model.step_detail(pre, action) {
            for event in events {
                if let Some(id) = TEARDOWN_GOALS.iter().position(|g| *g == event) {
                    out.push(id as u32);
                }
            }
        }
    };
    let run = explore(
        &model,
        horizon,
        limits,
        &Options {
            retain_dag: true,
            all_violations: true,
            goal: Some(&goal),
        },
    );
    let mut found = Vec::new();
    for (id, event) in TEARDOWN_GOALS.iter().enumerate() {
        let traces = run.goal_traces(id as u32, cap);
        if !traces.is_empty() {
            found.push((*event, traces));
        }
    }
    (run, found)
}

/// The events whose shortest traces the conformance tests drive: each rule of
/// the protocol, at the step where it decides.
pub const TEARDOWN_GOALS: [teardown::Event; 24] = [
    teardown::Event::EntryAccepted(teardown::Kind::Tick),
    teardown::Event::EntryRefusedPause(teardown::Kind::Tick),
    teardown::Event::EntryRefusedPhase,
    teardown::Event::EntryRefusedActive,
    teardown::Event::Armed(teardown::Owner::Watchdog),
    teardown::Event::Armed(teardown::Owner::Pause),
    teardown::Event::Armed(teardown::Owner::Reset),
    teardown::Event::Refused,
    teardown::Event::NoDeadlineCut,
    teardown::Event::PollFired,
    teardown::Event::DeadlineCut(teardown::Owner::Watchdog),
    teardown::Event::DeadlineCut(teardown::Owner::Pause),
    teardown::Event::DeadlineStale,
    teardown::Event::DeadlinePauseResumed,
    teardown::Event::FinishCut { consumed: true },
    teardown::Event::FinishPending,
    teardown::Event::StopTerminates,
    teardown::Event::StopLate,
    teardown::Event::HookRan,
    teardown::Event::HookDeadlineFired,
    teardown::Event::HookDeadlineStale,
    teardown::Event::HookRefusedEntry,
    teardown::Event::HookRefusedActive,
    teardown::Event::DropKeptHook,
];

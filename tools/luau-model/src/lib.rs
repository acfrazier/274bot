//! Bounded conformance oracle for the Luau run lifecycle (ADR 0017, LD1
//! `design-r3.md` section 6, with the runner side of LD2). No product crate
//! depends on this crate, except as a dev-dependency of the conformance tests
//! of `crates/script-luau` (slice L2b).
//!
//! One model, [`lifecycle`]: the slot, the run and its VM, and the actors that
//! meet at the slot mutex. The explorer (`explorer.rs`) searches it breadth
//! first inside caps and returns the shortest trace of any violation. See
//! README.md.

pub mod explorer;
pub mod lifecycle;

pub use explorer::*;

use lifecycle::{Action, Bounds, Event, Lifecycle, Mutation, Shapes, State};

/// A named set of bounds for one run of the model.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub name: &'static str,
    pub bounds: Bounds,
    /// The unit test lane explores every profile that is not `nightly`. The
    /// nightly profiles run in `nightly.sh`, one per 16 GB scope, and with
    /// `LUAU_MODEL_FULL=1 cargo test -p luau-model`.
    pub nightly: bool,
}

/// The profiles. Every run uses one of them, with a horizon.
pub fn profiles() -> Vec<Profile> {
    let min = Bounds::minimal();
    let waits = Bounds {
        seat_waits: true,
        typed_waits: true,
        walk_waits: true,
        tick_waits: true,
        ..min
    };
    vec![
        // One run: Start, Stop, the watchdog and `api.stop`.
        Profile {
            name: "smoke",
            bounds: Bounds {
                horizon: 28,
                ctl: 2,
                clock: true,
                self_stop: true,
                ..min
            },
            nightly: false,
        },
        // The control thread: Start, Start while Stopping, Pause, Resume,
        // Stop, removal, the progress watchdog's restart, the client gate.
        Profile {
            name: "control",
            bounds: Bounds {
                horizon: 40,
                starts: 2,
                ctl: 3,
                ticks: 2,
                restarts: 1,
                gates: 1,
                self_stop: true,
                hook: Shapes::Either,
                paint: Shapes::Either,
                removal: true,
                ..min
            },
            nightly: false,
        },
        // Waits and completions: seat, family, walk and tick waits; Pause, a
        // hold, a session reset and the client gate between them.
        Profile {
            name: "waits",
            bounds: Bounds {
                horizon: 44,
                ctl: 2,
                ticks: 3,
                held: 1,
                wakes: 2,
                waits: 2,
                resets: 1,
                holds: 1,
                gates: 1,
                ..waits
            },
            nightly: false,
        },
        // The watchdog: one deadline for the Tick and the Paint entry, the
        // self-stop exceptions, trips and panics in every entry kind.
        Profile {
            name: "watchdog",
            bounds: Bounds {
                horizon: 44,
                ctl: 2,
                ticks: 2,
                panics: 1,
                clock: true,
                self_stop: true,
                ..min
            },
            nightly: false,
        },
        // The memory cap: E1, E2 at every site, E3.
        Profile {
            name: "memory",
            bounds: Bounds {
                horizon: 40,
                ticks: 2,
                caps: 2,
                self_stop: true,
                ..min
            },
            nightly: false,
        },
        // The scenario dock: extension calls, the runner's take and
        // complete, revocation, a hold and a session reset.
        Profile {
            name: "dock",
            bounds: Bounds {
                horizon: 44,
                ctl: 3,
                ticks: 3,
                held: 1,
                wakes: 1,
                waits: 1,
                resets: 1,
                holds: 1,
                self_stop: true,
                dock: true,
                hook: Shapes::Either,
                ..min
            },
            nightly: false,
        },
        // The dock with trips and internal failures.
        Profile {
            name: "dock-trips",
            bounds: Bounds {
                horizon: 44,
                ctl: 2,
                ticks: 2,
                waits: 1,
                panics: 1,
                clock: true,
                self_stop: true,
                dock: true,
                ..min
            },
            nightly: false,
        },
        // The dock across a restart: a retained Start gets a fresh dock.
        Profile {
            name: "dock-restart",
            bounds: Bounds {
                horizon: 44,
                starts: 2,
                ctl: 2,
                ticks: 1,
                waits: 1,
                restarts: 1,
                dock: true,
                ..min
            },
            nightly: false,
        },
        // The entry fence and the revocation of the dock.
        Profile {
            name: "fence",
            bounds: Bounds {
                horizon: 40,
                ctl: 2,
                ticks: 2,
                waits: 1,
                self_stop: true,
                dock: true,
                fence: true,
                ..min
            },
            nightly: false,
        },
        // The execution ledger, the retirement tickets and the final gate.
        Profile {
            name: "runner",
            bounds: Bounds {
                horizon: 44,
                starts: 2,
                ctl: 3,
                ticks: 1,
                self_stop: true,
                fence: true,
                runner: true,
                ..min
            },
            nightly: false,
        },
        // The natural self-stop of LD2 4.1 and T30: the plan requires the
        // script's own `api.stop`, retired by one housekeeping Stop of the
        // idle slot. A Stop that the runner forces is not that witness.
        Profile {
            name: "runner-natural",
            bounds: Bounds {
                horizon: 44,
                starts: 2,
                ctl: 3,
                ticks: 1,
                self_stop: true,
                fence: true,
                runner: true,
                natural: true,
                ..min
            },
            nightly: false,
        },
        // The chain of LD2 3.2: two planned executions, each retired by the
        // runner's own Stop, with the next Start linked to the earlier ticket.
        // A Stop of the idle slot between them breaks the link.
        Profile {
            name: "runner-chain",
            bounds: Bounds {
                horizon: 44,
                starts: 2,
                ctl: 3,
                ticks: 0,
                fence: true,
                runner: true,
                plan: 2,
                ..min
            },
            nightly: false,
        },
        // The ledger across a restart and a third admission, with a plan that
        // owns all three starts, a self-stop and four control operations.
        Profile {
            name: "runner-restart",
            bounds: Bounds {
                horizon: 52,
                starts: 3,
                ctl: 4,
                ticks: 2,
                plan: 3,
                restarts: 1,
                self_stop: true,
                fence: true,
                runner: true,
                ..min
            },
            nightly: true,
        },
        // A longer clock: two units for the Load and the Hook, three for the
        // dispatch, with three ticks, a memory event and a panic.
        Profile {
            name: "clock",
            bounds: Bounds {
                horizon: 52,
                ctl: 3,
                ticks: 3,
                caps: 1,
                panics: 1,
                clock: true,
                self_stop: true,
                b_dispatch: 3,
                b_load: 2,
                b_hook: 2,
                hook: Shapes::Either,
                paint: Shapes::Either,
                ..min
            },
            nightly: false,
        },
        // Everything on in small counts, with the longer clock: two runs and
        // a restart, held and evidence ticks, a wait, three control
        // operations, one memory event and one panic, with the watchdog.
        Profile {
            name: "full",
            bounds: Bounds {
                horizon: 64,
                starts: 2,
                ctl: 3,
                ticks: 2,
                held: 1,
                wakes: 1,
                waits: 1,
                restarts: 1,
                gates: 1,
                holds: 1,
                resets: 1,
                caps: 1,
                panics: 1,
                clock: true,
                self_stop: true,
                b_dispatch: 3,
                b_load: 2,
                b_hook: 2,
                hook: Shapes::Either,
                paint: Shapes::Either,
                removal: true,
                ..waits
            },
            nightly: true,
        },
        // The suggested start of LD1 6.5: two runs, four game ticks of which
        // one is held, two waits, three control operations, and one trip of
        // each kind (the watchdog, one memory event, one panic) in every
        // entry kind. The seat pages, the reset, the hold and the client gate
        // of the `waits` profile stay on.
        Profile {
            name: "suggested",
            bounds: Bounds {
                horizon: 64,
                starts: 2,
                ctl: 3,
                ticks: 4,
                held: 1,
                wakes: 2,
                waits: 2,
                gates: 1,
                holds: 1,
                resets: 1,
                caps: 1,
                panics: 1,
                clock: true,
                self_stop: true,
                ..waits
            },
            nightly: true,
        },
        // Everything on: LD1 and the runner side of LD2 together, with one
        // dispatch. The holds, resets and seat waits of the `waits` profile
        // are left out, and so is a second dispatch: with two the search
        // passed 62 million states and the 12 GiB byte cap without ending.
        Profile {
            name: "scenario",
            bounds: Bounds {
                horizon: 64,
                starts: 2,
                ctl: 3,
                ticks: 1,
                waits: 1,
                restarts: 1,
                panics: 1,
                clock: true,
                self_stop: true,
                dock: true,
                fence: true,
                runner: true,
                ..min
            },
            nightly: true,
        },
    ]
}

pub fn profile(name: &str) -> Option<Profile> {
    profiles().into_iter().find(|p| p.name == name)
}

/// Explore one profile with one mutation. With `all_violations` the run does
/// not stop at the first violation: `Exploration::failures` holds the
/// shortest trace of each violation name.
pub fn explore_lifecycle(
    profile: &Profile,
    mutation: Mutation,
    horizon: Option<usize>,
    limits: Limits,
    all_violations: bool,
) -> Exploration<State, Action> {
    let model = Lifecycle::new(profile.bounds, mutation);
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

/// Every shortest trace to each event of [`GOALS`], for the conformance
/// tests.
pub type GoalTraces = Vec<(Event, Vec<Vec<Action>>)>;

pub fn goal_traces(
    bounds: Bounds,
    mutation: Mutation,
    horizon: usize,
    limits: Limits,
    cap: usize,
) -> (Exploration<State, Action>, GoalTraces) {
    let model = Lifecycle::new(bounds, mutation);
    let goal = |pre: &State, action: &Action, _post: &State, out: &mut Vec<u32>| {
        if let Ok((_, events)) = model.step_detail(pre, action) {
            for event in events {
                if let Some(id) = GOALS.iter().position(|g| *g == event) {
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
    for (id, event) in GOALS.iter().enumerate() {
        let traces = run.goal_traces(id as u32, cap);
        if !traces.is_empty() {
            found.push((*event, traces));
        }
    }
    (run, found)
}

/// The events whose shortest traces the conformance tests drive: each rule of
/// the lifecycle, at the step where it decides.
pub const GOALS: [Event; 30] = [
    Event::Spawned,
    Event::SpawnFailed,
    Event::StartQueued,
    Event::InvocationStarted,
    Event::InvocationResumed,
    Event::InvocationYielded,
    Event::PaintRan,
    Event::NothingDue,
    Event::CompletionStored,
    Event::CompletionAcked,
    Event::DispatchAccepted,
    Event::DispatchSelfStopped,
    Event::DispatchFailed(lifecycle::Fail::Watchdog),
    Event::DispatchFailed(lifecycle::Fail::Memory),
    Event::DispatchFailed(lifecycle::Fail::Internal),
    Event::SelfStop,
    Event::InternalAfterSelfStop,
    Event::HookRan,
    Event::HookSkipped,
    Event::HookTripped(lifecycle::Fail::Watchdog),
    Event::HookTripped(lifecycle::Fail::Internal),
    Event::Stopped(lifecycle::StopReason::Operator),
    Event::Stopped(lifecycle::StopReason::ScriptStop),
    Event::Stopped(lifecycle::StopReason::Failure),
    Event::Stopped(lifecycle::StopReason::Restart),
    Event::Retired(lifecycle::After::Idle),
    Event::QueuedStartSpawned,
    Event::QueuedStartDropped,
    Event::Restarted,
    Event::Refused(lifecycle::Refusal::Latch),
];

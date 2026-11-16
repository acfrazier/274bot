//! The Luau run lifecycle (LD1 `design-r3.md` section 6, with the runner side
//! of LD2): one slot, one run at a time, the VM that runs script code only
//! inside an entry, and the actors that meet there.
//!
//! Actors, and what one atomic step is (LD1 6.1, rule X2: the slot mutex
//! serialises every step):
//!
//! * the control thread: Start (with its Load entry), Start while the slot is
//!   `Stopping`, Pause, Resume, Stop, removal and raw Drop;
//! * the slot thread: the dispatch of one wake (a game tick, held or not, an
//!   evidence wake, a seat page), the observe step (`poll_ready`, the log
//!   drain and the stop completion) and the progress watchdog's restart;
//! * the script, inside an entry: one operation at a time, chosen from the
//!   table of LD1 6.1;
//! * the host and the seat: the completion of the awaited request, a session
//!   reset, a hold, the client-up gate, the clock;
//! * the scenario runner (LD2): the entry fence, the dock, the execution
//!   ledger and the retirement tickets.
//!
//! A call that enters the VM (Start, a dispatch, a Stop with a hook) is a
//! *section*: it holds the slot mutex, so while it runs only script operations
//! and the clock are offered. The model states TA, the termination assumption
//! of LD1 6.2: every section ends, and no wall clock is modelled. See
//! README.md for every abstraction.

mod actions;
pub mod checks;
mod dispatch;
mod invariants;
mod layers;
mod mutation;
mod slot;
mod text;
mod types;
mod vm;

pub use mutation::Mutation;
pub use text::describe_trace;
pub use types::*;

use crate::explorer::{Machine, Violation};

/// Which script shapes `Load` may return (the entries the file defines).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Shapes {
    Without,
    With,
    Either,
}

/// What a profile offers. Every count is a budget of the whole run of the
/// model, not of one slot call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub horizon: usize,
    /// Admitted Starts: fresh, queued behind a Stop, and restarts.
    pub starts: u8,
    /// Game-tick wakes delivered to a run.
    pub ticks: u8,
    /// Of those, the ones that the host holds.
    pub held: u8,
    /// Evidence and seat-page wakes.
    pub wakes: u8,
    /// Requests that waiting bindings start.
    pub waits: u8,
    /// Pause, Resume, Stop and removal.
    pub ctl: u8,
    pub restarts: u8,
    /// Client-up gate changes.
    pub gates: u8,
    pub holds: u8,
    pub resets: u8,
    /// Memory events: E1, E2 and E3 of rule C4.
    pub caps: u8,
    pub panics: u8,
    /// `R_gap` and `R_prop` of rule C2. LD1 6.5 requires `R_gap` of at least
    /// two: several operations can run between the deadline and the next
    /// check point, not one (LD1 3.4, the withdrawn one-operation rule).
    pub r_gap: u8,
    pub r_prop: u8,
    /// Rows that a run can queue before the slot admits them (rule C6).
    pub rows: u8,
    /// Budgets in time units. The dispatch has one for its Tick entry and its
    /// Paint entry together (rule C3).
    pub b_dispatch: u8,
    pub b_load: u8,
    pub b_hook: u8,
    pub hook: Shapes,
    pub paint: Shapes,
    pub clock: bool,
    pub self_stop: bool,
    pub seat_waits: bool,
    pub typed_waits: bool,
    pub walk_waits: bool,
    pub tick_waits: bool,
    pub removal: bool,
    /// LD2 layers.
    pub dock: bool,
    pub fence: bool,
    pub runner: bool,
    /// Starts that the runner's plan owns.
    pub plan: u8,
    /// The plan requires the natural self-stop of each planned execution
    /// (LD2 `wait_script_stop`, T30): a clean `api.stop` that the runner did
    /// not force, retired by one housekeeping Stop of the idle slot.
    pub natural: bool,
}

impl Bounds {
    /// Every layer and every event off: a model of Start, Stop and one tick.
    pub const fn minimal() -> Self {
        Bounds {
            horizon: 24,
            starts: 1,
            ticks: 1,
            held: 0,
            wakes: 0,
            waits: 0,
            ctl: 1,
            restarts: 0,
            gates: 0,
            holds: 0,
            resets: 0,
            caps: 0,
            panics: 0,
            r_gap: 2,
            r_prop: 1,
            rows: 1,
            b_dispatch: 2,
            b_load: 1,
            b_hook: 1,
            hook: Shapes::With,
            paint: Shapes::With,
            clock: false,
            self_stop: false,
            seat_waits: false,
            typed_waits: false,
            walk_waits: false,
            tick_waits: false,
            removal: false,
            dock: false,
            fence: false,
            runner: false,
            plan: 1,
            natural: false,
        }
    }
}

/// One script operation inside an entry (LD1 6.1, the table of what a script
/// can do).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Op {
    /// Operations with no check point: straight-line code, bounded by
    /// `R_gap` (rule C2).
    Gap,
    /// The interrupt: before a call, before a return, at a loop back-edge.
    Point,
    /// A binding, through the registration function that runs the check.
    Call(Binding),
    /// The entry function returns, after its own interrupt.
    Return(Ret),
    /// The error leaves the VM: an uncaught script error, or the unwinding of
    /// a trip, a self-stop or a refusal.
    Fail,
    /// The script's `pcall` swallows the error that propagates.
    Catch,
    Alloc(Alloc),
    /// The script drops its large values: used bytes fall below `S`.
    Free,
    /// An internal failure of the run (rule X11).
    Panic,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Binding {
    /// An action binding: it queues one typed row.
    Row,
    /// A log line or a paint command: an effect with no state in the model.
    Log,
    /// A waiting binding: one native call, then one coroutine yield.
    Wait(Wait),
    /// `api.stop` (4.3).
    Stop,
    /// An extension call of the scenario dock (LD2 2.3).
    Ext { awaited: bool },
}

/// What the entry function returns. The Load entry returns the table of the
/// file.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Ret {
    Plain,
    Table {
        hook: bool,
        paint: bool,
    },
    /// The file returns something that is not a table with `tick`.
    Other,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Alloc {
    /// Used bytes reach `S` (event E1 waits for a check point).
    OverS,
    /// The allocator refuses a request (event E2) at the site that sees it.
    Refuse(Site),
    /// An `mlua` call inside a binding returns the replaced error (event E3).
    Lost,
}

/// The three kinds of site that can see a refused allocation (rule C4).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Site {
    /// The resume of the entry returns `Error::MemoryError` to the run.
    Host,
    /// An `mlua` call inside a binding returns it.
    Binding,
    /// A guarded protected call catches it.
    Guard,
}

/// One atomic step.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    // The control thread.
    Start(Who),
    Pause,
    Resume,
    Stop,
    Remove,
    DropSlot,
    // The slot thread.
    Dispatch(Wake),
    Observe,
    Restart,
    /// The seat drains the rows that a run queued (a design that does not
    /// admit them in the dispatch; only mutations offer it).
    Admit,
    // The host, the seat and the client.
    ClientDown,
    ClientUp,
    HoldOn,
    HoldOff,
    Arrive,
    SessionReset,
    // The clock.
    Advance,
    /// The dispatch goes from its Tick entry to its Paint entry.
    Next,
    // The script, inside an entry.
    Vm(Op),
    // The runner and the dock (LD2).
    Fence,
    FenceWait,
    Revoke,
    Post(Intent),
    Pump,
    Take,
    Complete,
    FinalGate,
}

/// What a step did, for the conformance tests and the printed traces.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Event {
    Spawned,
    SpawnFailed,
    StartQueued,
    StartRefused,
    /// A dispatch that the run refused, and why.
    Refused(Refusal),
    /// A dispatch with nothing due: the run stored the facts and did not
    /// enter the VM.
    NothingDue,
    InvocationStarted,
    InvocationResumed,
    InvocationYielded,
    PaintRan,
    /// A completion fact was stored by an accepted delivery and the host
    /// acknowledged it.
    CompletionStored,
    CompletionAcked,
    CompletionKept,
    DispatchAccepted,
    DispatchFailed(Fail),
    DispatchSelfStopped,
    SelfStop,
    Tripped(Fail),
    InternalAfterSelfStop,
    HookRan,
    HookSkipped,
    HookTripped(Fail),
    /// `stop` dropped the VM and sent the completion.
    Stopped(StopReason),
    /// The observe step read the completion and ran `complete_stop`.
    Retired(After),
    QueuedStartSpawned,
    QueuedStartDropped,
    Restarted,
    Paused,
    Resumed,
    Gated,
    Denied,
    Fenced,
    Revoked,
    Ticketed,
    Verdict(Verdict),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Refusal {
    Paused,
    Latch,
    Fence,
}

/// The model: bounds and one mutation.
#[derive(Clone, Copy, Debug)]
pub struct Lifecycle {
    pub bounds: Bounds,
    pub mutation: Mutation,
}

impl Lifecycle {
    pub fn new(bounds: Bounds, mutation: Mutation) -> Self {
        Lifecycle { bounds, mutation }
    }

    pub fn initial_state(&self) -> State {
        State {
            slot: Slot {
                state: RunState::Idle,
                want_run: false,
                up: true,
                after: After::Idle,
                generation: 0,
                base: None,
                ready: false,
                chan: None,
                receipt: Receipt::None,
                card: Card {
                    reason: Reason::None,
                    hook: false,
                },
                hook_logs: 0,
                diag: false,
                serial: 0,
                stops: 0,
            },
            run: None,
            sec: None,
            host: Host {
                hold: false,
                comp: None,
            },
            clock: Clock {
                armed: None,
                truth: None,
            },
            n: Counts::default(),
            gone: false,
            leaked: 0,
            orphan_rows: 0,
            row_after_stop: false,
            closed_dock: None,
            runner: Runner {
                phase: RunnerPhase::Running,
                ledger: [None; 3],
                intent: None,
                plan: if self.bounds.runner {
                    self.bounds.plan
                } else {
                    0
                },
                foreign_start: false,
                foreign_end: false,
                pending_planned: false,
                issuing: None,
                failed: false,
            },
        }
    }

    pub(crate) fn is(&self, mutation: Mutation) -> bool {
        self.mutation == mutation
    }

    pub(crate) fn violation(name: &'static str, detail: impl Into<String>) -> Violation {
        debug_assert!(
            name == "model" || checks::find(name).is_some(),
            "the check {name} is not in checks::CHECKS"
        );
        Violation {
            name,
            detail: detail.into(),
        }
    }

    /// Whether `action` is offered in `state`.
    pub fn enabled(&self, state: &State, action: &Action) -> bool {
        let mut offered = Vec::new();
        self.actions(state, &mut offered);
        offered.contains(action)
    }

    /// One step: the next state and its events, or the violation.
    pub fn step_detail(
        &self,
        state: &State,
        action: &Action,
    ) -> Result<(State, Vec<Event>), Violation> {
        let mut s = state.clone();
        let mut ev: Vec<Event> = Vec::new();
        match *action {
            Action::Start(who) => self.start(&mut s, who, &mut ev)?,
            Action::Pause => self.pause(&mut s, &mut ev),
            Action::Resume => self.resume(&mut s, &mut ev),
            Action::Stop => self.operator_stop(&mut s, &mut ev)?,
            Action::Remove => self.remove(&mut s, &mut ev)?,
            Action::DropSlot => self.drop_slot(&mut s, &mut ev)?,
            Action::Dispatch(wake) => self.dispatch(&mut s, wake, &mut ev)?,
            Action::Observe => self.observe(&mut s, &mut ev)?,
            Action::Restart => self.restart(&mut s, &mut ev)?,
            Action::Admit => self.admit_rows(&mut s)?,
            Action::ClientDown => self.client_gate(&mut s, false, &mut ev),
            Action::ClientUp => self.client_gate(&mut s, true, &mut ev),
            Action::HoldOn => {
                s.host.hold = true;
                s.n.holds += 1;
            }
            Action::HoldOff => {
                s.host.hold = false;
                s.n.holds += 1;
            }
            Action::Arrive => self.arrive(&mut s),
            Action::SessionReset => self.session_reset(&mut s)?,
            Action::Advance => self.advance(&mut s),
            Action::Next => self.next_entry(&mut s, &mut ev)?,
            Action::Vm(op) => self.vm_op(&mut s, op, &mut ev)?,
            Action::Fence => self.fence(&mut s, &mut ev)?,
            Action::FenceWait => return Err(self.fence_wait(&s)),
            Action::Revoke => self.revoke(&mut s, &mut ev),
            Action::Post(intent) => self.post(&mut s, intent),
            Action::Pump => self.pump(&mut s, &mut ev)?,
            Action::Take => self.take(&mut s),
            Action::Complete => self.complete(&mut s)?,
            Action::FinalGate => self.final_gate(&mut s, &mut ev)?,
        }
        self.check_state(&s)?;
        Ok((s, ev))
    }
}

impl Machine for Lifecycle {
    type State = State;
    type Action = Action;

    fn initial(&self) -> State {
        self.initial_state()
    }

    fn actions(&self, state: &State, out: &mut Vec<Action>) {
        Lifecycle::actions(self, state, out);
    }

    fn step(&self, state: &State, action: &Action) -> Result<State, Violation> {
        self.step_detail(state, action).map(|(next, _)| next)
    }
}

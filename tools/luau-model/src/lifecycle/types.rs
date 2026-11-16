//! The data of the model: the slot, the run, the sections that run script
//! code, the host, the clock and the counters. Every type is small and
//! `Copy` where it can be, because the explorer hashes and clones a state for
//! each transition.

use std::fmt::Write as _;

/// `RunState` of `crates/script/src/slot.rs`. The `Error` state is the result
/// of the JS cut limit only; a Luau run never reports a cut (LD1 section 6.2,
/// P1), so it is not here.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RunState {
    Idle,
    Starting,
    Running,
    Paused,
    Stopping,
}

/// `AfterStop` of the slot: what `complete_stop` does next. `Fail` and
/// `CutLimit` belong to the JS surface (a setup that failed after its spawn,
/// and the cut limit).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum After {
    Idle,
    Start,
    Restart,
}

/// The three failure codes of rule C5: `luau-watchdog`, `luau-memory` and
/// `luau-internal`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Fail {
    Watchdog,
    Memory,
    Internal,
}

impl Fail {
    pub fn code(self) -> &'static str {
        match self {
            Fail::Watchdog => "luau-watchdog",
            Fail::Memory => "luau-memory",
            Fail::Internal => "luau-internal",
        }
    }
}

/// The terminal cause latch of a run (rule C2). It is written at most once:
/// a write to a set latch changes nothing. A trip is a write of a failure
/// code; a self-stop is the write that `api.stop` makes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Latch {
    Empty,
    SelfStop,
    Trip(Fail),
}

impl Latch {
    pub fn is_trip(self) -> bool {
        matches!(self, Latch::Trip(_))
    }
}

/// The four entry kinds (LD1 6.1): one call from Rust that lets the VM run
/// script code. Each entry is one resume of a coroutine (rule X12).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Kind {
    Load,
    Tick,
    Paint,
    Hook,
}

/// What a suspended invocation of `tick` waits for (rule B4 and LD2 2.3).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Wait {
    /// A seat request. `typed` is a request of an API family: its completion
    /// is the typed outcome of the family (4.2 rule 5), not the bare seat end.
    Seat { typed: bool },
    /// A walk. Its completion is the walk outcome fields.
    Walk,
    /// A wait with no request: it ends at the next game tick that is not held.
    Tick,
    /// An extension call of the scenario dock (LD2 2.3). Its completion is a
    /// payload in the dock, not a host delivery.
    Ext,
}

/// The one invocation of `tick` (rule X4).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Inv {
    Idle,
    Running,
    Suspended(Wait),
}

/// The completion fact of a host delivery (4.2 rule 5).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Comp {
    /// The terminal of a seat request. `typed` says that the typed outcome of
    /// the family came with it.
    Terminal { typed: bool },
    /// The walk outcome fields.
    Walk,
    /// The typed abort of a session reset (4.4 item 7).
    Abort,
}

/// The slot wakes that can enter the VM (rule X5). A game tick is held when
/// the host holds the frame (rule X6).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Wake {
    Tick,
    Evidence,
    SeatPage,
}

/// Who a control operation or a Start belongs to (LD2): the operator, or the
/// scenario runner that owns the execution.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Who {
    Operator,
    Runner,
}

/// `ScriptTerminalState` of the slot's lifecycle receipt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Receipt {
    None,
    Stopped,
    Failed,
}

/// What the card text (`last_error`) says first.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Reason {
    None,
    SelfStop,
    Trip,
}

/// The card text: the reason that ended the run, and the failure of `onStop`
/// that the slot adds after it (4.3).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Card {
    pub reason: Reason,
    pub hook: bool,
}

/// What `stop` sends on `done` (4.3 `StopCompletion`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Msg {
    pub hook_fail: Option<Fail>,
}

/// Why a run is stopped: the eight ways of I9, with the failure of a dispatch
/// and the script's own request kept apart.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StopReason {
    Operator,
    Removed,
    /// `api.stop`: the slot's log drain reads `stopped()` (4.3).
    ScriptStop,
    /// A trip or an internal failure that a dispatch returned (`stop_blocked`).
    Failure,
    /// The progress watchdog's restart.
    Restart,
    /// Raw `Drop` of the slot.
    Drop,
}

impl StopReason {
    /// The completion has no receiver: the slot is gone.
    pub fn receiver_gone(self) -> bool {
        matches!(self, StopReason::Removed | StopReason::Drop)
    }
}

/// The slot: `SlotScript` as far as the run lifecycle reads it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Slot {
    pub state: RunState,
    /// Operator intent (`want_run`).
    pub want_run: bool,
    /// The client is up (`on_is_up`).
    pub up: bool,
    pub after: After,
    /// `runtime_generation`.
    pub generation: u8,
    /// `setup_generation_base`: the generation that a failed setup gives back.
    pub base: Option<u8>,
    /// `surface_ready`: the first `poll_ready` of the run returned `Ready`.
    pub ready: bool,
    /// `stop_rx`: the completion that `stop` sent and the observe step has not
    /// read.
    pub chan: Option<Msg>,
    pub receipt: Receipt,
    pub card: Card,
    /// Log lines that report a failure of `onStop`.
    pub hook_logs: u8,
    /// The diagnostic of a Start whose spawn failed (`last_error`).
    pub diag: bool,
    /// The monotonic Start serial of LD2 3.2. It never goes back.
    pub serial: u8,
    /// Stop ordinal for the retirement tickets of LD2 3.2.
    pub stops: u8,
}

/// The call states of the scenario dock (LD2 2.3).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Call {
    None,
    /// The current Tick dispatch owns it. `awaited` is the awaited operation.
    Staged {
        awaited: bool,
    },
    Queued {
        awaited: bool,
    },
    Taken {
        awaited: bool,
    },
    Completed,
    Consumed,
    Done,
    Discarded,
    Cancelled,
}

impl Call {
    /// A call that no longer holds the one call slot.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Call::None | Call::Consumed | Call::Done | Call::Discarded | Call::Cancelled
        )
    }
}

/// The end record that `close` retains (LD2 2.4).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct End {
    pub hook_fail: Option<Fail>,
    pub fault: bool,
}

/// The scenario dock of one run (LD2 2.2 to 2.4): one awaited call, one
/// completion slot, one durable first-fault latch and one end record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Dock {
    /// Identity of the dock. A fresh admitted Start gets another dock.
    pub id: u8,
    pub claimed: bool,
    pub closed: bool,
    pub revoked: bool,
    pub call: Call,
    /// The one completion payload is in the dock.
    pub payload: bool,
    /// `first_fault`: written before an error returns to Luau, never cleared.
    pub fault: bool,
    /// Ghost: the retained history of the faults of this run (LD2 2.4). Each
    /// refused call adds one before any code of the dock acts, and the count
    /// saturates at two ("the count of later faults may saturate"). No step
    /// clears it and no transition reads it: the invariants compare the latch
    /// and the end record with it.
    pub faults: u8,
    /// Ghost: the payload was delivered to a resume.
    pub delivered: bool,
    pub end: Option<End>,
}

impl Dock {
    pub const fn none() -> Self {
        Dock {
            id: 0,
            claimed: false,
            closed: false,
            revoked: false,
            call: Call::None,
            payload: false,
            fault: false,
            faults: 0,
            delivered: false,
            end: None,
        }
    }
}

/// `LuauSurfaceRun` as far as the lifecycle reads it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Run {
    /// Which run this is, from 1.
    pub id: u8,
    /// The Start serial of the admission that made this run (LD2 3.2).
    pub serial: u8,
    /// The VM exists (I1).
    pub vm: bool,
    /// The script returned an `onStop` entry, and an `onPaint` entry.
    pub hook: bool,
    pub paint: bool,
    pub latch: Latch,
    /// The failure of `onStop`, or an internal failure after `api.stop`.
    pub hook_fail: Option<Fail>,
    pub paused: bool,
    pub inv: Inv,
    /// Invocations beyond the first. Only a mutation makes one (I6).
    pub extra: u8,
    /// The completion that an accepted delivery stored (4.2 rule 5).
    pub stored: Option<Comp>,
    /// Ghost: the completion of the awaited request has arrived at the host.
    pub arrived: bool,
    /// Ghost: the arrived completion has resumed the invocation.
    pub consumed: bool,
    /// Rows that entries queued and the slot has not admitted to the seat.
    pub rows: u8,
    /// Used bytes of the VM are at or above `S` (rule C4, event E1).
    pub over: bool,
    /// Ghost: a refusal or a lost error happened in this run (events E2, E3).
    pub cap_event: bool,
    /// `stop` began: no later entry (rule X8 step 1).
    pub stopping: bool,
    pub hook_runs: u8,
    /// Messages that `stop` sent on `done`.
    pub sends: u8,
    /// The entry fence of LD2 5.1 phase A is published for this run.
    pub fenced: bool,
    pub dock: Dock,
}

impl Run {
    pub fn new(id: u8, serial: u8) -> Self {
        Run {
            id,
            serial,
            vm: true,
            hook: false,
            paint: false,
            latch: Latch::Empty,
            hook_fail: None,
            paused: false,
            inv: Inv::Idle,
            extra: 0,
            stored: None,
            arrived: false,
            consumed: false,
            rows: 0,
            over: false,
            cap_event: false,
            stopping: false,
            hook_runs: 0,
            sends: 0,
            fenced: false,
            dock: Dock::none(),
        }
    }
}

/// One script entry that is in the VM (6.1). `gap` counts script operations
/// since the last check point (the residual `R_gap`), `caught` counts trip
/// errors that the script swallowed (`R_prop`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Vm {
    pub kind: Kind,
    pub gap: u8,
    /// An error propagates through the script. It is set by a failed check,
    /// by `api.stop` and by a refused allocation. The run ends the entry at
    /// once, unless the script can catch the error (a mutation).
    pub err: bool,
    pub caught: u8,
    /// A trip that the check found and did not write to the latch. Only a
    /// mutation has one: the verdict then waits for the error to reach the
    /// host.
    pub pending: Option<Fail>,
}

impl Vm {
    pub fn new(kind: Kind) -> Self {
        Vm {
            kind,
            gap: 0,
            err: false,
            caught: 0,
            pending: None,
        }
    }
}

/// How a spawn was asked for.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LoadHow {
    /// `start_surface` on an Idle slot: bumps the generation.
    Operator { who: Who },
    /// `complete_stop` with `AfterStop::Start`: the generation moved when the
    /// Start was queued.
    Queued,
    /// `complete_stop` with `AfterStop::Restart`: bumps the generation.
    Restart,
}

/// The Load entry inside `spawn` (rule X9). The run under construction holds
/// the VM.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LoadSec {
    pub vm: Vm,
    pub run: Run,
    pub how: LoadHow,
}

/// Phases of one dispatch (rule C2: one deadline for the Tick entry and the
/// Paint entry).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DPhase {
    InTick,
    /// The Tick entry ended; the Paint entry has not started. Time can pass.
    Between,
    InPaint,
}

/// One dispatch in progress (4.2). The slot mutex is held (rule X2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Disp {
    pub wake: Wake,
    pub held: bool,
    pub phase: DPhase,
    pub vm: Option<Vm>,
    /// The Paint entry is planned after the Tick entry.
    pub paint: bool,
    /// At least one entry ran: a deadline was armed.
    pub ran: bool,
    /// The completion that this delivery carries, acknowledged by the host
    /// only when the dispatch is accepted (4.2 rule 6).
    pub carries: Option<Comp>,
    /// This dispatch resumed the invocation with a completion.
    pub consumed: bool,
}

/// The Hook entry inside `stop` (rule X8 step 2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HookSec {
    pub vm: Vm,
    pub reason: StopReason,
}

/// The call that holds the slot mutex and has script code in the VM.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Sec {
    Load(LoadSec),
    Disp(Disp),
    Hook(HookSec),
}

/// The host side: the hold, and the completion that the host has in its
/// facts and the run has not acknowledged (4.2 rule 6).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Host {
    pub hold: bool,
    pub comp: Option<Comp>,
}

/// Which deadline is armed (rule C2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Owner {
    Load,
    Dispatch,
    Hook,
}

/// The armed deadline, and the true deadline of the owner that runs. Time is
/// a unit counter. One unit is a part of the budget; the model keeps the
/// order of the events and drops the values (README, abstraction 1).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Clock {
    /// What the run armed: the owner and the units left before it passes.
    pub armed: Option<(Owner, u8)>,
    /// Ghost: the true deadline of the owner that runs now. It does not
    /// depend on what a mutation does with `armed`.
    pub truth: Option<(Owner, u8)>,
}

/// Budgets of one exploration (the bounds of a profile).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
pub struct Counts {
    pub starts: u8,
    pub ticks: u8,
    pub held: u8,
    pub wakes: u8,
    pub waits: u8,
    pub ctl: u8,
    pub restarts: u8,
    pub gates: u8,
    pub resets: u8,
    pub caps: u8,
    pub panics: u8,
    pub holds: u8,
    /// Runs made so far: the identity of the next run.
    pub runs: u8,
}

/// A scenario runner's execution ledger entry (LD2 3.2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Exec {
    /// The Start serial of the admission.
    pub serial: u8,
    /// The runner's plan authorised the admission (a planned Start).
    pub planned: bool,
    pub state: ExecState,
    /// The run ended with no trip, no self-stop, no failure of `onStop` and
    /// no first fault.
    pub clean: bool,
    /// The run ended by its own `api.stop`, with no failure of `onStop` and
    /// no first fault: the natural self-stop witness of LD2 4.1 and T30. Only
    /// a profile whose plan requires it (`Bounds::natural`) records it.
    pub natural: bool,
    /// The slot's generation when the run was spawned.
    pub generation: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExecState {
    /// Admitted: its spawn is running, or its run is live, or it is queued.
    Live,
    /// A runner's Stop retired it: the ticket is issued.
    Ticketed(Ticket),
    /// It ended by itself, or by a foreign Stop, or it failed.
    Ended,
    /// Its setup never made a run.
    FailedStart,
    /// Its queued Start was dropped before it ran.
    Dropped,
}

/// The retirement ticket of LD2 3.2: slot-issued, names the execution, the
/// stop ordinal, the actual generations and the Start serial at the Stop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Ticket {
    pub serial: u8,
    pub ordinal: u8,
    pub gen_before: u8,
    pub gen_after: u8,
    pub start_serial: u8,
}

/// The runner's phase (LD2 5.1, as far as the lifecycle needs it).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RunnerPhase {
    Running,
    Fenced,
    Done(Verdict),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Verdict {
    Pass,
    Fail,
}

/// A posted owner intent (LD2 5.1 phases B and D): the role pump executes it
/// outside the runner lock.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Intent {
    Start,
    Stop { serial: u8 },
}

/// The runner and its ledger (LD2). Empty and inert when the profile has no
/// runner.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Runner {
    pub phase: RunnerPhase,
    pub ledger: [Option<Exec>; 3],
    pub intent: Option<Intent>,
    /// Planned Starts the runner still owns.
    pub plan: u8,
    /// Ghost: a Start that the plan did not authorise was admitted.
    pub foreign_start: bool,
    /// Ghost: a Stop that the runner did not issue ended a run of the ledger.
    pub foreign_end: bool,
    /// A planned Start is being admitted by the pump.
    pub pending_planned: bool,
    /// The runner's Stop in flight: the serial it was issued for.
    pub issuing: Option<u8>,
    /// The runner fixed its fixed non-pass outcome F: it revoked the run
    /// (LD2 5.3, row V08). A revoked run never reaches PASS.
    pub failed: bool,
}

/// The whole state.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    pub slot: Slot,
    pub run: Option<Run>,
    pub sec: Option<Sec>,
    pub host: Host,
    pub clock: Clock,
    pub n: Counts,
    /// The slot is removed or dropped: nothing happens after it.
    pub gone: bool,
    /// VMs that a failed spawn kept (mutation `FailedLoadLeak`).
    pub leaked: u8,
    /// Rows that a stop left in a queue the slot drains later (mutation).
    pub orphan_rows: u8,
    /// Ghost: a row reached the seat after the Stop began (I4).
    pub row_after_stop: bool,
    /// The dock of the last run, closed: the end record that an observer
    /// reads (LD2 2.4).
    pub closed_dock: Option<Dock>,
    pub runner: Runner,
}

impl State {
    /// VMs that exist now: the run's, the one a Load entry is building, and
    /// the ones that a failed spawn kept (I1).
    pub fn vms(&self) -> u8 {
        let run = self.run.as_ref().is_some_and(|run| run.vm);
        let load = matches!(self.sec, Some(Sec::Load(_)));
        u8::from(run) + u8::from(load) + self.leaked
    }

    pub fn summary(&self) -> String {
        let mut out = String::new();
        let slot = &self.slot;
        let _ = write!(
            out,
            "slot={:?}{}{} after={:?} generation={} serial={} receipt={:?} card={:?}{}",
            slot.state,
            if slot.want_run { "" } else { " !want" },
            if slot.up { "" } else { " !up" },
            slot.after,
            slot.generation,
            slot.serial,
            slot.receipt,
            slot.card.reason,
            if slot.card.hook { "+hook" } else { "" },
        );
        if let Some(chan) = slot.chan {
            let _ = write!(out, " chan={chan:?}");
        }
        if let Some(run) = &self.run {
            let _ = write!(
                out,
                " | run#{} vm={} latch={:?} paused={} inv={:?} rows={} hook_runs={}",
                run.id, run.vm, run.latch, run.paused, run.inv, run.rows, run.hook_runs
            );
            if let Some(fail) = run.hook_fail {
                let _ = write!(out, " hook_fail={fail:?}");
            }
            if let Some(comp) = run.stored {
                let _ = write!(out, " stored={comp:?}");
            }
            if run.fenced {
                out.push_str(" fenced");
            }
        }
        if let Some(comp) = self.host.comp {
            let _ = write!(out, " | host owes {comp:?}");
        }
        if self.host.hold {
            out.push_str(" | hold");
        }
        if let Some((owner, left)) = self.clock.armed {
            let _ = write!(out, " | deadline {owner:?} left={left}");
        }
        if let Some(sec) = &self.sec {
            let _ = write!(out, " | in {}", sec_name(sec));
        }
        out
    }
}

pub fn sec_name(sec: &Sec) -> String {
    match sec {
        Sec::Load(load) => format!("Load({:?})", load.how),
        Sec::Disp(disp) => format!("Dispatch({:?}, {:?})", disp.wake, disp.phase),
        Sec::Hook(hook) => format!("Hook({:?})", hook.reason),
    }
}

//! The JS isolate's eval and `onStop` protocol: one isolate, three actors, one
//! lock. The product is `crates/script-js/src/runtime/isolate/teardown.rs`.
//!
//! Actors:
//!
//! * the isolate thread (`T`): evals, the V8-entry fence, the `onStop` hook;
//! * the host (`H`), one sequential actor: the dispatch watchdog, Pause,
//!   Resume, session reset, Stop and raw Drop;
//! * the deadline supervisor (`S`): expired execution and hook deadlines.
//!
//! Every action is one critical section of the teardown mutex, or one step
//! the product takes outside it (a registration with the supervisor, a V8
//! cancel after the lock was released, Drop's terminate). A V8 termination
//! request is a flag whose *provenance* is the set of requests that armed it;
//! the invariants use the provenance to say which eval a request was aimed at.
//! Time is one counter; every comparison in the product is `<=` or `>=` between
//! a start and a bound.
//!
//! The four invariants of the isolate teardown design (README.md) are
//! the `a-*`, `b-*`, `c-*` (see `retire.rs`) and `d-*` names. The `e-*` names
//! are two more safety properties the same protocol needs. README.md lists
//! them, the product function behind each action, and the abstractions.

use crate::explorer::{Machine, Violation};
use std::fmt::Write as _;

/// `EXECUTION_WATCHDOG_HORIZON`, in model time units.
pub const HORIZON: u8 = 1;
/// `ON_STOP_DEADLINE`, in model time units.
pub const HOOK_BUDGET: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Phase {
    Running,
    Unwinding,
    Hook,
    Done,
}

/// `ExecutionInterrupt`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Owner {
    Watchdog,
    Pause,
    Reset,
    /// The supervisor refused the deadline: terminal, already consumed.
    NoDeadline,
}

/// The three entries into an interruptible eval. Only a tick honours Pause.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    Tick,
    Frame,
    Lifecycle,
}

impl Kind {
    pub fn honors_pause(self) -> bool {
        self == Kind::Tick
    }
}

/// `InterruptedExecution`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Interrupt {
    pub owner: Owner,
    pub consumed: bool,
}

/// The active eval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Exec {
    pub id: u8,
    pub started: u8,
    pub kind: Kind,
    /// `Execution::deadline`: registered with the supervisor, and its owner.
    pub armed: Option<Owner>,
    pub interrupt: Option<Interrupt>,
    /// V8 delivered a pending termination to this eval's code.
    pub delivered: bool,
}

/// Provenance bits of V8's pending termination request.
pub mod bit {
    /// A request aimed at eval `id`: its deadline, the watchdog or a refusal.
    pub fn exec(id: u8) -> u8 {
        1 << id.clamp(1, 3)
    }
    /// `begin_stop` while the tick unwinds.
    pub const STOP: u8 = 1 << 4;
    /// The hook's own deadline.
    pub const HOOK: u8 = 1 << 5;
    /// Raw Drop's termination, which the product issues outside the lock.
    pub const REVOKE: u8 = 1 << 6;
}

/// A registration with the supervisor.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Entry {
    Exec { id: u8, owner: Owner, at: u8 },
    Hook { at: u8 },
}

impl Entry {
    fn at(self) -> u8 {
        match self {
            Entry::Exec { at, .. } | Entry::Hook { at } => at,
        }
    }
}

/// What a host operation does after its watchdog look.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Then {
    Free,
    PauseSet,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Host {
    Free,
    /// `watchdog_look` returned; its second lock section arms or fires.
    Look {
        id: u8,
        within: bool,
        then: Then,
    },
    /// `arm_deadline` returned `Some`: the registration is next.
    Registering {
        id: u8,
        at: u8,
        owner: Owner,
        then: Then,
    },
    /// The supervisor refused: the cut with no deadline is next.
    Refused {
        id: u8,
        then: Then,
    },
    PauseSet,
    PauseArm,
    StopLock,
    DropSend,
    DropFire,
    Gone,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Thread {
    Idle,
    /// Mutation R2 only: the Pause check of an entry was made outside the lock.
    EnterCommit(Kind),
    InExec,
    Registering {
        id: u8,
        at: u8,
    },
    Refused {
        id: u8,
    },
    /// Mutation R3 only: the cancel ran before the eval was cleared.
    FinishClear,
    Stopping {
        invoke: bool,
    },
    HookGuard,
    HookCancel,
    HookRegister,
    HookBody,
    Complete,
    CancelAfter,
    Exited,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    pub phase: Phase,
    pub pause: bool,
    pub exec: Option<Exec>,
    pub last_id: u8,
    /// Provenance of V8's pending termination request; zero means none.
    pub flag: u8,
    pub now: u8,
    pub thread: Thread,
    pub host: Host,
    /// Queued `Stop { invoke_hook }` commands, oldest first.
    pub stop_q: Vec<bool>,
    pub entries: Vec<Entry>,
    pub hook_fired: bool,
    /// The hook's absolute deadline, fixed when the hook is entered
    /// (`TeardownState::Hook::deadline`).
    pub hook_at: u8,
    pub hook_runs: u8,
    pub script_stopped: bool,
}

impl State {
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "phase={:?} pause={} now={} flag={:#09b} thread={:?} host={:?}",
            self.phase, self.pause, self.now, self.flag, self.thread, self.host
        );
        if let Some(e) = &self.exec {
            let _ = write!(
                out,
                " exec#{}({:?}, started {}, armed {:?}, interrupt {:?}, delivered {})",
                e.id, e.kind, e.started, e.armed, e.interrupt, e.delivered
            );
        }
        if !self.entries.is_empty() {
            let _ = write!(out, " entries={:?}", self.entries);
        }
        if !self.stop_q.is_empty() {
            let _ = write!(out, " stop_q={:?}", self.stop_q);
        }
        let _ = write!(out, " hook_runs={}", self.hook_runs);
        out
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Who {
    Host,
    Thread,
}

/// One atomic step.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    Advance,
    // The isolate thread.
    Enter(Kind),
    EnterCheck(Kind),
    EnterCommit,
    /// An entry attempted while an eval is active (`begin_execution`).
    EnterBusy(Kind),
    /// The Stop path attempts to enter the hook while an eval is active.
    TryHook,
    Deliver,
    ArmWatchdog,
    Register {
        by: Who,
        refuse: bool,
    },
    FireRefused(Who),
    Finish,
    FinishClear,
    StopCommand,
    ScriptStop,
    EnterHook,
    HookGuard,
    HookCancel,
    HookRegister {
        refuse: bool,
    },
    HookDone,
    Complete,
    CancelAfter,
    // The host.
    Poll,
    Pause,
    Look,
    PauseSet,
    PauseArm,
    Resume,
    Reset,
    StopSend,
    StopLock,
    Drop,
    DropSend,
    DropFire,
    // The supervisor.
    Fire(usize),
}

impl Action {
    pub fn text(self) -> String {
        match self {
            Action::Advance => "time passes: one unit".into(),
            Action::Enter(k) => format!("isolate thread: begin a {k:?} eval"),
            Action::EnterCheck(k) => {
                format!("isolate thread: reads Pause for a {k:?} entry, outside the lock")
            }
            Action::EnterCommit => "isolate thread: commits the entry it checked".into(),
            Action::EnterBusy(k) => format!("isolate thread: tries a {k:?} entry during an eval"),
            Action::TryHook => "isolate thread: tries enter_teardown_hook during an eval".into(),
            Action::Deliver => "V8: delivers the pending termination to the running code".into(),
            Action::ArmWatchdog => "isolate thread: arm_execution_watchdog (lifecycle hook)".into(),
            Action::Register { by, refuse } => format!(
                "{by:?}: registers the execution deadline with the supervisor{}",
                if refuse { " (refused)" } else { "" }
            ),
            Action::FireRefused(by) => {
                format!("{by:?}: cuts the eval after the refusal, with no deadline")
            }
            Action::Finish => "isolate thread: finish_interruptible_execution".into(),
            Action::FinishClear => "isolate thread: clears the eval after its cancel".into(),
            Action::StopCommand => "isolate thread: takes a queued Stop command".into(),
            Action::ScriptStop => "isolate thread: the script requests stop".into(),
            Action::EnterHook => "isolate thread: enter_teardown_hook".into(),
            Action::HookGuard => "isolate thread: hook_deadline reads the phase".into(),
            Action::HookCancel => "isolate thread: cancels the pending termination".into(),
            Action::HookRegister { refuse } => format!(
                "isolate thread: registers the hook deadline{}",
                if refuse { " (refused)" } else { "" }
            ),
            Action::HookDone => "isolate thread: the onStop body returns".into(),
            Action::Complete => "isolate thread: finish_teardown_hook".into(),
            Action::CancelAfter => "isolate thread: cancels termination after the hook".into(),
            Action::Poll => "host: the dispatch watchdog looks at the active eval".into(),
            Action::Pause => "host: Pause starts with the watchdog's look".into(),
            Action::Look => "host: the watchdog arms the deadline or cuts".into(),
            Action::PauseSet => "host: Pause publishes its intent".into(),
            Action::PauseArm => "host: Pause arms the Pause deadline".into(),
            Action::Resume => "host: Resume".into(),
            Action::Reset => "host: session reset arms the reset deadline".into(),
            Action::StopSend => "host: queues Stop for the isolate thread".into(),
            Action::StopLock => "host: begin_stop unwinds the tick and terminates".into(),
            Action::Drop => "host: raw Drop revokes the handle".into(),
            Action::DropSend => "host: Drop queues Stop { invoke_hook: false }".into(),
            Action::DropFire => "host: Drop terminates the isolate".into(),
            Action::Fire(i) => format!("supervisor: decides the due deadline #{i}"),
        }
    }
}

/// Seeded earlier designs, found defects and self-checks. The head is `None`.
///
/// `R*`: the design without a rule that the product's comments say the lock
/// discipline enforces. `D*`: a defect this model found at the head of
/// `a3a50d098`. `Check*`: a builder's check that an invariant can fail.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Mutation {
    #[default]
    None,
    /// R1: Stop terminates whatever phase it finds, so it can cut `onStop`
    /// ("join cannot terminate after the hook starts", `teardown.rs`).
    R1StopTerminatesInHook,
    /// R2: the Pause check of an entry is outside the lock ("Pause sees no
    /// active eval immediately before the isolate thread enters a queued
    /// one").
    R2PauseCheckedOutsideLock,
    /// R3: finish cancels V8's request before it clears the eval, so a
    /// deadline can arm it in between ("cannot fire after this cancel and
    /// leave termination armed").
    R3FinishCancelsFirst,
    /// R4: the hook deadline fires whatever the phase ("the hook's 50 ms
    /// one-shot cannot fire into Done").
    R4HookDeadlineIntoDone,
    /// R5: an execution deadline applies to whatever eval is active ("deadlines
    /// capture the identity so they cannot terminate later work").
    R5DeadlineWithoutIdentity,
    /// R6: teardown is not exactly-once: `enter_teardown_hook` does not refuse
    /// a second entry, and the isolate thread keeps taking commands after the
    /// hook instead of leaving its loop.
    R6HookEnterTwice,
    /// R7: Stop enters the hook from the host while an eval is active.
    R7StopEntersHook,
    /// D1: `Drop` sets Done even when the hook is running. The supervisor's
    /// hook deadline fires only in `Hook`, so it no longer bounds the body,
    /// and Drop's own termination can be cancelled by the isolate thread's
    /// pre-hook cancel. Found by this model at `a3a50d098`; LP1 fixes it
    /// (`TeardownState::revoke` keeps `Hook`).
    D1DropOverwritesHook,
    /// Check: finish leaves V8's request pending.
    CheckFinishNoCancel,
    /// Check: Pause cuts the active eval at once.
    CheckPauseCutsActive,
    /// Check: the machine pass halts on Pause intent.
    CheckHaltedOnPause,
    /// Check: finish marks every cut consumed, whether or not V8 delivered it.
    CheckFenceMarksAlways,
}

impl Mutation {
    pub const ALL: [Mutation; 13] = [
        Mutation::None,
        Mutation::R1StopTerminatesInHook,
        Mutation::R2PauseCheckedOutsideLock,
        Mutation::R3FinishCancelsFirst,
        Mutation::R4HookDeadlineIntoDone,
        Mutation::R5DeadlineWithoutIdentity,
        Mutation::R6HookEnterTwice,
        Mutation::R7StopEntersHook,
        Mutation::D1DropOverwritesHook,
        Mutation::CheckFinishNoCancel,
        Mutation::CheckPauseCutsActive,
        Mutation::CheckHaltedOnPause,
        Mutation::CheckFenceMarksAlways,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mutation::None => "none",
            Mutation::R1StopTerminatesInHook => "r1-stop-terminates-in-hook",
            Mutation::R2PauseCheckedOutsideLock => "r2-pause-checked-outside-lock",
            Mutation::R3FinishCancelsFirst => "r3-finish-cancels-first",
            Mutation::R4HookDeadlineIntoDone => "r4-hook-deadline-into-done",
            Mutation::R5DeadlineWithoutIdentity => "r5-deadline-without-identity",
            Mutation::R6HookEnterTwice => "r6-hook-enter-twice",
            Mutation::R7StopEntersHook => "r7-stop-enters-hook",
            Mutation::D1DropOverwritesHook => "d1-drop-overwrites-hook",
            Mutation::CheckFinishNoCancel => "check-finish-no-cancel",
            Mutation::CheckPauseCutsActive => "check-pause-cuts-active",
            Mutation::CheckHaltedOnPause => "check-halted-on-pause",
            Mutation::CheckFenceMarksAlways => "check-fence-marks-always",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The invariant that must catch this mutation.
    pub fn caught_by(self) -> Option<&'static str> {
        Some(match self {
            Mutation::None => return None,
            Mutation::R1StopTerminatesInHook => "a-on-stop-sees-terminate",
            Mutation::R2PauseCheckedOutsideLock => "d-pause-did-not-block-entry",
            Mutation::R3FinishCancelsFirst => "a-terminate-outlives-its-eval",
            Mutation::R4HookDeadlineIntoDone => "a-terminate-after-the-hook",
            Mutation::R5DeadlineWithoutIdentity => "d-cut-before-horizon",
            Mutation::R6HookEnterTwice => "b-on-stop-twice",
            Mutation::R7StopEntersHook => "b-on-stop-with-active-eval",
            Mutation::D1DropOverwritesHook => "e-hook-unbounded",
            Mutation::CheckFinishNoCancel => "a-terminate-outlives-its-eval",
            Mutation::CheckPauseCutsActive => "d-cut-before-horizon",
            Mutation::CheckHaltedOnPause => "d-pause-halts-owning-eval",
            Mutation::CheckFenceMarksAlways => "a-cut-reported-without-delivery",
        })
    }
}

/// What a profile offers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Bounds {
    pub horizon: usize,
    /// Evals per run.
    pub max_exec: u8,
    pub max_now: u8,
    /// Frame and lifecycle entries beside the tick.
    pub frames: bool,
    /// The supervisor may refuse a registration.
    pub refusals: bool,
    pub pause: bool,
    pub reset: bool,
    pub raw_drop: bool,
    /// The script may request its own stop.
    pub script_stop: bool,
}

impl Bounds {
    pub const fn standard() -> Self {
        Bounds {
            horizon: 32,
            max_exec: 2,
            max_now: 3,
            frames: false,
            refusals: true,
            pause: true,
            reset: true,
            raw_drop: true,
            script_stop: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Teardown {
    pub bounds: Bounds,
    pub mutation: Mutation,
}

/// What a step did, for the conformance tests and the printed traces.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Event {
    EntryAccepted(Kind),
    EntryRefusedPause(Kind),
    EntryRefusedPhase,
    /// An entry while an eval is active.
    EntryRefusedActive,
    Armed(Owner),
    Refused,
    NoDeadlineCut,
    PollFired,
    DeadlineCut(Owner),
    DeadlineStale,
    DeadlinePauseResumed,
    PauseSet,
    Delivered {
        consumed: bool,
    },
    FinishCut {
        consumed: bool,
    },
    FinishPending,
    FinishPlain,
    StopUnwinds,
    StopTerminates,
    StopLate,
    HookEntered,
    HookRefusedEntry,
    /// The hook entry while an eval is active.
    HookRefusedActive,
    HookSkipped,
    HookRan,
    HookDeadlineFired,
    HookDeadlineStale,
    HookDone,
    DropRevoked,
    DropKeptHook,
}

impl Teardown {
    pub fn new(bounds: Bounds, mutation: Mutation) -> Self {
        Teardown { bounds, mutation }
    }

    pub fn initial_state(&self) -> State {
        State {
            phase: Phase::Running,
            pause: false,
            exec: None,
            last_id: 0,
            flag: 0,
            now: 0,
            thread: Thread::Idle,
            host: Host::Free,
            stop_q: Vec::new(),
            entries: Vec::new(),
            hook_fired: false,
            hook_at: 0,
            hook_runs: 0,
            script_stopped: false,
        }
    }

    fn violation(name: &'static str, detail: impl Into<String>) -> Violation {
        Violation {
            name,
            detail: detail.into(),
        }
    }

    /// `machines_halted`.
    fn halted(&self, s: &State) -> bool {
        s.phase != Phase::Running
            || s.exec.is_some_and(|e| e.interrupt.is_some())
            || (self.mutation == Mutation::CheckHaltedOnPause && s.pause)
    }

    /// `TeardownState::arm_deadline`: `Some((id, at))` when the caller must
    /// register a deadline. The owner is replaced unless a Watchdog arm finds
    /// one; the time never moves.
    fn arm(s: &mut State, owner: Owner) -> Option<(u8, u8)> {
        if s.phase != Phase::Running {
            return None;
        }
        let exec = s.exec.as_mut().filter(|e| e.interrupt.is_none())?;
        let armed = exec.armed.is_some();
        if owner != Owner::Watchdog || exec.armed.is_none() {
            exec.armed = Some(owner);
        }
        (!armed).then_some((exec.id, exec.started + HORIZON))
    }

    fn can_arm(s: &State) -> bool {
        s.phase == Phase::Running && s.exec.is_some_and(|e| e.interrupt.is_none())
    }

    /// `TeardownState::fire_interrupt`: a cut aimed at eval `id`.
    fn fire(s: &mut State, id: u8, owner: Owner) -> bool {
        if s.phase != Phase::Running {
            return false;
        }
        let Some(exec) = s.exec.as_mut() else {
            return false;
        };
        if exec.id != id || exec.interrupt.is_some() {
            return false;
        }
        exec.interrupt = Some(Interrupt {
            owner,
            consumed: owner == Owner::NoDeadline,
        });
        s.flag |= bit::exec(id);
        true
    }

    fn then_host(then: Then) -> Host {
        match then {
            Then::Free => Host::Free,
            Then::PauseSet => Host::PauseSet,
        }
    }

    fn insert(entries: &mut Vec<Entry>, entry: Entry) {
        entries.push(entry);
        entries.sort();
    }

    /// The post-state invariants.
    fn check_state(&self, s: &State) -> Result<(), Violation> {
        if s.phase == Phase::Hook && s.exec.is_some() {
            return Err(Self::violation(
                "b-on-stop-with-active-eval",
                "the phase is Hook while an eval is active",
            ));
        }
        if s.hook_runs > 1 {
            return Err(Self::violation(
                "b-on-stop-twice",
                format!("onStop started {} times", s.hook_runs),
            ));
        }
        for id in 1..=3u8 {
            if s.flag & bit::exec(id) != 0 && s.exec.is_none_or(|e| e.id != id) {
                return Err(Self::violation(
                    "a-terminate-outlives-its-eval",
                    format!(
                        "V8 holds a termination request armed for eval {id}, but that eval is no longer active"
                    ),
                ));
            }
        }
        if s.thread == Thread::Exited && s.flag & !bit::REVOKE != 0 {
            return Err(Self::violation(
                "a-terminate-after-the-hook",
                format!(
                    "the isolate thread left the hook, and V8 holds request {:#09b} that is not Drop's",
                    s.flag
                ),
            ));
        }
        if s.flag & bit::HOOK != 0
            && !matches!(
                s.thread,
                Thread::HookBody | Thread::Complete | Thread::CancelAfter
            )
        {
            return Err(Self::violation(
                "a-terminate-after-the-hook",
                format!(
                    "the hook deadline's request is pending in phase {:?} with the thread at {:?}",
                    s.phase, s.thread
                ),
            ));
        }
        if s.flag & bit::STOP != 0
            && !(s.phase == Phase::Unwinding
                || matches!(
                    s.thread,
                    Thread::HookGuard | Thread::HookCancel | Thread::Complete | Thread::CancelAfter
                ))
        {
            return Err(Self::violation(
                "a-on-stop-sees-terminate",
                format!(
                    "Stop's request is pending in phase {:?} with the isolate thread at {:?}",
                    s.phase, s.thread
                ),
            ));
        }
        if let Some(exec) = &s.exec {
            if let Some(interrupt) = exec.interrupt
                && interrupt.owner != Owner::NoDeadline
                && s.now < exec.started + HORIZON
            {
                return Err(Self::violation(
                    "d-cut-before-horizon",
                    format!(
                        "eval {} was cut ({:?}) at time {} before its horizon {}",
                        exec.id,
                        interrupt.owner,
                        s.now,
                        exec.started + HORIZON
                    ),
                ));
            }
            if s.phase == Phase::Running && exec.interrupt.is_none() && self.halted(s) {
                return Err(Self::violation(
                    "d-pause-halts-owning-eval",
                    format!(
                        "the machine pass halts the in-budget eval {} that owns execution",
                        exec.id
                    ),
                ));
            }
        }
        if s.thread == Thread::HookBody
            && s.flag & (bit::HOOK | bit::REVOKE) == 0
            && !matches!(s.host, Host::DropSend | Host::DropFire)
            && !s.hook_fired
            && !(s.phase == Phase::Hook
                && s.entries.iter().any(|e| matches!(e, Entry::Hook { .. })))
        {
            return Err(Self::violation(
                "e-hook-unbounded",
                format!(
                    "the onStop body runs in phase {:?} and no hook deadline can still terminate it",
                    s.phase
                ),
            ));
        }
        Ok(())
    }

    /// Whether `action` is offered in `state`. The explorer offers only
    /// enabled actions; a test or a conformance trace must too.
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
        let m = self.mutation;
        match *action {
            Action::Advance => s.now += 1,
            Action::Enter(kind) => {
                if s.phase != Phase::Running {
                    ev.push(Event::EntryRefusedPhase);
                } else if s.exec.is_some() {
                    ev.push(Event::EntryRefusedActive);
                } else if kind.honors_pause() && s.pause {
                    ev.push(Event::EntryRefusedPause(kind));
                } else {
                    self.accept_entry(&mut s, kind, &mut ev)?;
                }
            }
            Action::EnterBusy(_) => {
                if s.exec.is_some() {
                    ev.push(Event::EntryRefusedActive);
                }
            }
            Action::TryHook => {
                if s.exec.is_some() {
                    ev.push(Event::HookRefusedActive);
                }
            }
            Action::EnterCheck(kind) => {
                if s.phase != Phase::Running {
                    ev.push(Event::EntryRefusedPhase);
                } else if kind.honors_pause() && s.pause {
                    ev.push(Event::EntryRefusedPause(kind));
                } else {
                    s.thread = Thread::EnterCommit(kind);
                }
            }
            Action::EnterCommit => {
                let Thread::EnterCommit(kind) = s.thread else {
                    return Ok((s, ev));
                };
                if s.phase != Phase::Running {
                    ev.push(Event::EntryRefusedPhase);
                    s.thread = Thread::Idle;
                } else {
                    self.accept_entry(&mut s, kind, &mut ev)?;
                }
            }
            Action::Deliver => self.deliver(&mut s, &mut ev)?,
            Action::ArmWatchdog => {
                if let Some((id, at)) = Self::arm(&mut s, Owner::Watchdog) {
                    ev.push(Event::Armed(Owner::Watchdog));
                    s.thread = Thread::Registering { id, at };
                }
            }
            Action::Register { by, refuse } => {
                let (id, at, owner, then) = match (by, s.host, s.thread) {
                    (
                        Who::Host,
                        Host::Registering {
                            id,
                            at,
                            owner,
                            then,
                        },
                        _,
                    ) => (id, at, owner, Some(then)),
                    (Who::Thread, _, Thread::Registering { id, at }) => {
                        (id, at, Owner::Watchdog, None)
                    }
                    _ => return Ok((s, ev)),
                };
                if refuse {
                    ev.push(Event::Refused);
                    match then {
                        Some(then) => s.host = Host::Refused { id, then },
                        None => s.thread = Thread::Refused { id },
                    }
                } else {
                    Self::insert(&mut s.entries, Entry::Exec { id, owner, at });
                    match then {
                        Some(then) => s.host = Self::then_host(then),
                        None => s.thread = Thread::InExec,
                    }
                }
            }
            Action::FireRefused(by) => {
                let (id, then) = match (by, s.host, s.thread) {
                    (Who::Host, Host::Refused { id, then }, _) => (id, Some(then)),
                    (Who::Thread, _, Thread::Refused { id }) => (id, None),
                    _ => return Ok((s, ev)),
                };
                if Self::fire(&mut s, id, Owner::NoDeadline) {
                    ev.push(Event::NoDeadlineCut);
                }
                match then {
                    Some(then) => s.host = Self::then_host(then),
                    None => s.thread = Thread::InExec,
                }
            }
            Action::Finish => {
                if m == Mutation::R3FinishCancelsFirst {
                    s.flag = 0;
                    s.thread = Thread::FinishClear;
                } else {
                    self.finish(&mut s, &mut ev)?;
                }
            }
            Action::FinishClear => self.finish(&mut s, &mut ev)?,
            Action::StopCommand => {
                if !s.stop_q.is_empty() {
                    let invoke = s.stop_q.remove(0);
                    s.thread = Thread::Stopping { invoke };
                }
            }
            Action::ScriptStop => {
                s.script_stopped = true;
                s.thread = Thread::Stopping { invoke: true };
            }
            Action::EnterHook => {
                let Thread::Stopping { invoke } = s.thread else {
                    return Ok((s, ev));
                };
                let entered =
                    matches!(s.phase, Phase::Hook | Phase::Done) && m != Mutation::R6HookEnterTwice;
                if entered || (s.exec.is_some() && m != Mutation::R7StopEntersHook) {
                    ev.push(Event::HookRefusedEntry);
                    s.thread = Thread::Exited;
                } else {
                    s.phase = Phase::Hook;
                    s.hook_fired = false;
                    s.hook_at = s.now + HOOK_BUDGET;
                    ev.push(Event::HookEntered);
                    s.thread = if invoke {
                        Thread::HookGuard
                    } else {
                        Thread::Complete
                    };
                }
            }
            Action::HookGuard => {
                if s.phase == Phase::Hook {
                    s.thread = Thread::HookCancel;
                } else {
                    ev.push(Event::HookSkipped);
                    s.thread = Thread::Complete;
                }
            }
            Action::HookCancel => {
                s.flag = 0;
                s.thread = Thread::HookRegister;
            }
            Action::HookRegister { refuse } => {
                if refuse {
                    ev.push(Event::HookSkipped);
                    s.thread = Thread::Complete;
                } else {
                    Self::insert(&mut s.entries, Entry::Hook { at: s.hook_at });
                    self.start_hook_body(&mut s, &mut ev)?;
                }
            }
            Action::HookDone => {
                ev.push(Event::HookDone);
                s.thread = Thread::Complete;
            }
            Action::Complete => {
                s.phase = Phase::Done;
                s.thread = Thread::CancelAfter;
            }
            Action::CancelAfter => {
                s.flag = 0;
                s.thread = if m == Mutation::R6HookEnterTwice {
                    Thread::Idle
                } else {
                    Thread::Exited
                };
            }
            Action::Poll | Action::Pause => {
                let then = if *action == Action::Pause {
                    Then::PauseSet
                } else {
                    Then::Free
                };
                s.host = match s
                    .exec
                    .as_ref()
                    .filter(|e| s.phase == Phase::Running && e.interrupt.is_none())
                {
                    Some(e) => Host::Look {
                        id: e.id,
                        within: s.now - e.started <= HORIZON,
                        then,
                    },
                    None => Self::then_host(then),
                };
            }
            Action::Look => {
                let Host::Look { id, within, then } = s.host else {
                    return Ok((s, ev));
                };
                if within {
                    s.host = match Self::arm(&mut s, Owner::Watchdog) {
                        Some((id, at)) => {
                            ev.push(Event::Armed(Owner::Watchdog));
                            Host::Registering {
                                id,
                                at,
                                owner: Owner::Watchdog,
                                then,
                            }
                        }
                        None => Self::then_host(then),
                    };
                } else {
                    if Self::fire(&mut s, id, Owner::Watchdog) {
                        ev.push(Event::PollFired);
                    }
                    s.host = Self::then_host(then);
                }
            }
            Action::PauseSet => {
                if s.phase == Phase::Running {
                    s.pause = true;
                    ev.push(Event::PauseSet);
                    if m == Mutation::CheckPauseCutsActive
                        && let Some(id) = s
                            .exec
                            .as_ref()
                            .filter(|e| e.interrupt.is_none())
                            .map(|e| e.id)
                    {
                        Self::fire(&mut s, id, Owner::Pause);
                    }
                }
                s.host = Host::PauseArm;
            }
            Action::PauseArm => {
                s.host = match Self::arm(&mut s, Owner::Pause) {
                    Some((id, at)) => {
                        ev.push(Event::Armed(Owner::Pause));
                        Host::Registering {
                            id,
                            at,
                            owner: Owner::Pause,
                            then: Then::Free,
                        }
                    }
                    None => Host::Free,
                };
            }
            Action::Resume => {
                if s.phase == Phase::Running {
                    s.pause = false;
                }
            }
            Action::Reset => {
                if let Some((id, at)) = Self::arm(&mut s, Owner::Reset) {
                    ev.push(Event::Armed(Owner::Reset));
                    s.host = Host::Registering {
                        id,
                        at,
                        owner: Owner::Reset,
                        then: Then::Free,
                    };
                }
            }
            Action::StopSend => {
                s.stop_q.push(true);
                s.host = Host::StopLock;
            }
            Action::StopLock => {
                if m == Mutation::R7StopEntersHook && s.phase == Phase::Running {
                    s.phase = Phase::Hook;
                    s.flag |= bit::STOP;
                } else {
                    if s.phase == Phase::Running {
                        s.phase = Phase::Unwinding;
                        ev.push(Event::StopUnwinds);
                    }
                    if s.phase == Phase::Unwinding {
                        s.flag |= bit::STOP;
                        ev.push(Event::StopTerminates);
                    } else if m == Mutation::R1StopTerminatesInHook {
                        s.flag |= bit::STOP;
                    } else {
                        ev.push(Event::StopLate);
                    }
                }
                s.host = Host::Gone;
            }
            Action::Drop => {
                let overwrite = m == Mutation::D1DropOverwritesHook;
                if matches!(s.phase, Phase::Running | Phase::Unwinding)
                    || (overwrite && s.phase == Phase::Hook)
                {
                    s.phase = Phase::Done;
                    ev.push(Event::DropRevoked);
                } else if s.phase == Phase::Hook {
                    ev.push(Event::DropKeptHook);
                }
                s.host = Host::DropSend;
            }
            Action::DropSend => {
                s.stop_q.push(false);
                s.host = Host::DropFire;
            }
            Action::DropFire => {
                s.flag |= bit::REVOKE;
                s.host = Host::Gone;
            }
            Action::Fire(i) => self.fire_entry(&mut s, i, &mut ev),
        }
        self.check_state(&s)?;
        Ok((s, ev))
    }

    fn accept_entry(
        &self,
        s: &mut State,
        kind: Kind,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if kind.honors_pause() && s.pause {
            return Err(Self::violation(
                "d-pause-did-not-block-entry",
                format!("a {kind:?} eval entered V8 while Pause intent was published"),
            ));
        }
        if s.flag != 0 {
            return Err(Self::violation(
                "a-later-eval-sees-terminate",
                format!(
                    "a new eval enters V8 while termination request {:#09b} is still pending",
                    s.flag
                ),
            ));
        }
        s.last_id += 1;
        s.exec = Some(Exec {
            id: s.last_id,
            started: s.now,
            kind,
            armed: None,
            interrupt: None,
            delivered: false,
        });
        s.thread = Thread::InExec;
        ev.push(Event::EntryAccepted(kind));
        Ok(())
    }

    /// V8 delivers the pending request to the code that runs, and the fence
    /// marks the cut consumed. The request stays pending until a cancel.
    fn deliver(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let flag = s.flag;
        match s.thread {
            Thread::InExec => {
                let id = s.exec.map_or(0, |e| e.id);
                let own = bit::exec(id) | bit::STOP | bit::REVOKE;
                if flag & !own != 0 {
                    return Err(Self::violation(
                        "a-later-eval-sees-terminate",
                        format!(
                            "eval {id} receives termination request {:#09b} that was armed for another eval or for onStop",
                            flag & !own
                        ),
                    ));
                }
                let mut consumed = false;
                if let Some(e) = s.exec.as_mut() {
                    e.delivered = true;
                    if let Some(i) = e.interrupt.as_mut() {
                        i.consumed = true;
                        consumed = true;
                    }
                }
                ev.push(Event::Delivered { consumed });
            }
            Thread::HookBody => {
                let own = bit::HOOK | bit::REVOKE;
                if flag & !own != 0 {
                    return Err(Self::violation(
                        "a-on-stop-sees-terminate",
                        format!(
                            "the onStop body receives termination request {:#09b} that was armed for an eval or for Stop",
                            flag & !own
                        ),
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let pending = s.flag != 0;
        if let Some(e) = s.exec.take() {
            match e.interrupt {
                Some(mut i) => {
                    if self.mutation == Mutation::CheckFenceMarksAlways {
                        i.consumed = true;
                    }
                    if i.consumed && i.owner != Owner::NoDeadline && !e.delivered {
                        return Err(Self::violation(
                            "a-cut-reported-without-delivery",
                            format!(
                                "eval {} is reported cut, but V8 delivered no termination to it",
                                e.id
                            ),
                        ));
                    }
                    ev.push(Event::FinishCut {
                        consumed: i.consumed,
                    });
                }
                None if pending => ev.push(Event::FinishPending),
                None => ev.push(Event::FinishPlain),
            }
        }
        if !matches!(
            self.mutation,
            Mutation::CheckFinishNoCancel | Mutation::R3FinishCancelsFirst
        ) {
            s.flag = 0;
        }
        s.thread = Thread::Idle;
        Ok(())
    }

    fn start_hook_body(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        s.hook_runs += 1;
        ev.push(Event::HookRan);
        if s.exec.is_some() {
            return Err(Self::violation(
                "b-on-stop-with-active-eval",
                "the onStop body starts while an eval is active",
            ));
        }
        let own = bit::HOOK | bit::REVOKE;
        if s.flag & !own != 0 {
            return Err(Self::violation(
                "a-on-stop-sees-terminate",
                format!(
                    "the onStop body starts with termination request {:#09b} armed for an eval or for Stop",
                    s.flag & !own
                ),
            ));
        }
        s.thread = Thread::HookBody;
        Ok(())
    }

    /// `run_deadline`: the supervisor decides one due registration.
    fn fire_entry(&self, s: &mut State, i: usize, ev: &mut Vec<Event>) {
        if i >= s.entries.len() {
            return;
        }
        match s.entries.remove(i) {
            Entry::Exec { id, owner, .. } => {
                let identity = self.mutation != Mutation::R5DeadlineWithoutIdentity;
                let running = s.phase == Phase::Running;
                let pause = s.pause;
                let mut cut = None;
                let mut target = id;
                if let Some(e) = s.exec.as_mut()
                    && running
                    && (!identity || e.id == id)
                    && e.interrupt.is_none()
                {
                    target = e.id;
                    let owner = e.armed.take().unwrap_or(owner);
                    if owner != Owner::Pause || pause {
                        e.interrupt = Some(Interrupt {
                            owner,
                            consumed: false,
                        });
                        cut = Some(owner);
                    } else {
                        ev.push(Event::DeadlinePauseResumed);
                    }
                } else {
                    ev.push(Event::DeadlineStale);
                }
                if let Some(owner) = cut {
                    s.flag |= bit::exec(target);
                    ev.push(Event::DeadlineCut(owner));
                }
                if let Some(e) = s.exec.as_mut()
                    && e.id == id
                {
                    e.armed = None;
                }
            }
            Entry::Hook { .. } => {
                let phase_ok =
                    s.phase == Phase::Hook || self.mutation == Mutation::R4HookDeadlineIntoDone;
                if phase_ok && !s.hook_fired {
                    s.hook_fired = true;
                    s.flag |= bit::HOOK;
                    ev.push(Event::HookDeadlineFired);
                } else {
                    ev.push(Event::HookDeadlineStale);
                }
            }
        }
    }

    fn thread_actions(&self, s: &State, out: &mut Vec<Action>) {
        let b = &self.bounds;
        match s.thread {
            Thread::Idle => {
                if s.last_id < b.max_exec {
                    let kinds: &[Kind] = if b.frames {
                        &[Kind::Tick, Kind::Frame, Kind::Lifecycle]
                    } else {
                        &[Kind::Tick]
                    };
                    for kind in kinds {
                        out.push(if self.mutation == Mutation::R2PauseCheckedOutsideLock {
                            Action::EnterCheck(*kind)
                        } else {
                            Action::Enter(*kind)
                        });
                    }
                }
                if !s.stop_q.is_empty() {
                    out.push(Action::StopCommand);
                }
                if b.script_stop && !s.script_stopped {
                    out.push(Action::ScriptStop);
                }
            }
            Thread::EnterCommit(_) => out.push(Action::EnterCommit),
            Thread::InExec => {
                // Attempts the thread's own loop never makes while it runs
                // an eval: the guards of `begin_execution` and `enter_hook`
                // must still refuse.
                let kinds: &[Kind] = if b.frames {
                    &[Kind::Tick, Kind::Frame, Kind::Lifecycle]
                } else {
                    &[Kind::Tick]
                };
                for kind in kinds {
                    out.push(Action::EnterBusy(*kind));
                }
                if b.script_stop {
                    out.push(Action::TryHook);
                }
                if s.flag != 0 {
                    out.push(Action::Deliver);
                }
                if s.exec.is_some_and(|e| e.kind == Kind::Lifecycle) && Self::can_arm(s) {
                    out.push(Action::ArmWatchdog);
                }
                out.push(Action::Finish);
            }
            Thread::Registering { .. } => {
                out.push(Action::Register {
                    by: Who::Thread,
                    refuse: false,
                });
                if b.refusals {
                    out.push(Action::Register {
                        by: Who::Thread,
                        refuse: true,
                    });
                }
            }
            Thread::Refused { .. } => out.push(Action::FireRefused(Who::Thread)),
            Thread::FinishClear => out.push(Action::FinishClear),
            Thread::Stopping { .. } => out.push(Action::EnterHook),
            Thread::HookGuard => out.push(Action::HookGuard),
            Thread::HookCancel => out.push(Action::HookCancel),
            Thread::HookRegister => {
                out.push(Action::HookRegister { refuse: false });
                if b.refusals {
                    out.push(Action::HookRegister { refuse: true });
                }
            }
            Thread::HookBody => {
                if s.flag != 0 {
                    out.push(Action::Deliver);
                }
                out.push(Action::HookDone);
            }
            Thread::Complete => out.push(Action::Complete),
            Thread::CancelAfter => out.push(Action::CancelAfter),
            Thread::Exited => {}
        }
    }

    fn host_actions(&self, s: &State, out: &mut Vec<Action>) {
        let b = &self.bounds;
        match s.host {
            Host::Free => {
                if Self::can_arm(s) {
                    out.push(Action::Poll);
                }
                if b.pause {
                    out.push(Action::Pause);
                    out.push(Action::Resume);
                }
                if b.reset && Self::can_arm(s) {
                    out.push(Action::Reset);
                }
                out.push(Action::StopSend);
                if b.raw_drop {
                    out.push(Action::Drop);
                }
            }
            Host::Look { .. } => out.push(Action::Look),
            Host::Registering { .. } => {
                out.push(Action::Register {
                    by: Who::Host,
                    refuse: false,
                });
                if b.refusals {
                    out.push(Action::Register {
                        by: Who::Host,
                        refuse: true,
                    });
                }
            }
            Host::Refused { .. } => out.push(Action::FireRefused(Who::Host)),
            Host::PauseSet => out.push(Action::PauseSet),
            Host::PauseArm => out.push(Action::PauseArm),
            Host::StopLock => out.push(Action::StopLock),
            Host::DropSend => out.push(Action::DropSend),
            Host::DropFire => out.push(Action::DropFire),
            Host::Gone => {}
        }
    }
}

impl Machine for Teardown {
    type State = State;
    type Action = Action;

    fn initial(&self) -> State {
        self.initial_state()
    }

    fn actions(&self, s: &State, out: &mut Vec<Action>) {
        if s.now < self.bounds.max_now {
            out.push(Action::Advance);
        }
        self.thread_actions(s, out);
        self.host_actions(s, out);
        for (i, entry) in s.entries.iter().enumerate() {
            if entry.at() <= s.now {
                out.push(Action::Fire(i));
            }
        }
    }

    fn step(&self, state: &State, action: &Action) -> Result<State, Violation> {
        self.step_detail(state, action).map(|(next, _)| next)
    }
}

/// Replay `trace` and print each step with its events, in the product's terms.
pub fn describe_trace(model: &Teardown, trace: &[Action]) -> String {
    let mut out = String::new();
    let mut state = model.initial_state();
    let _ = writeln!(out, "  start: {}", state.summary());
    for (i, action) in trace.iter().enumerate() {
        let _ = writeln!(out, "  {:>2}. {}", i + 1, action.text());
        match model.step_detail(&state, action) {
            Ok((next, events)) => {
                for event in events {
                    let _ = writeln!(out, "        event: {event:?}");
                }
                let _ = writeln!(out, "        {}", next.summary());
                state = next;
            }
            Err(violation) => {
                let _ = writeln!(
                    out,
                    "        VIOLATION {}: {}",
                    violation.name, violation.detail
                );
                break;
            }
        }
    }
    out
}

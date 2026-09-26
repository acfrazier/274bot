//! Per-isolate step-machine host (fence F03).
//!
//! A **step machine** owns one multi-tick behavior in Rust. JS starts it
//! with one native call and awaits one completion; Rust owns the loop,
//! the sequencing, the game ops and the retries.
//!
//! # Family registration
//!
//! A family is a Rust type implementing [`Family`] and listed once in
//! [`FAMILIES`] (its [`Family::NAME`] is the string JS passes to
//! `runMachine`). The family supplies:
//!
//! - [`Family::begin`]: validate the typed [`Family::Args`] against the
//!   isolate scene ([`crate::observed`]) and selected game data, emit the
//!   first ops through [`Cx::emit`], and return [`Begin::Run`] (a live
//!   row), [`Begin::Done`] (settled at once, e.g. a refused precondition)
//!   or [`Begin::Refuse`] (nothing started; ops emitted by a refusing
//!   begin are discarded). Begin never calls script callbacks, and a
//!   machine start from inside a begin is refused.
//! - [`Family::step`]: read the scene, emit ops, arm/read deadlines on
//!   [`Cx::clock`], read the last callback's [`Cx::reply`], and return
//!   [`Step::Wait`] (done for this tick), [`Step::Call`] (call one script
//!   callback, then step again), [`Step::Done`] or [`Step::Fail`].
//! - [`Family::CALLBACKS`]: the script callbacks the family may call, by
//!   key on the `hooks` object passed to `runMachine`; [`Call::hook`]
//!   indexes this list. Each key is read once, at start (getters run
//!   then), with receiver `hooks`. [`Cx::has`] says whether it was
//!   present (not `undefined`/`null`), so a family keeps the frozen
//!   `hook?.()` semantics by skipping an absent hook; calling an absent
//!   hook throws the frozen `<name> is not a function`. A script that
//!   needs call-time reads passes a wrapper
//!   (`{ walkBack: () => opts.walkBack?.() }`).
//! - [`Family::abort`]: optional cleanup when the host drops a live row
//!   (ResetSession, a superseding start). It emits nothing.
//!
//! # Lifecycle
//!
//! - **Start** (`runMachine(family, args, hooks)` in `_kernel.js`, native
//!   `__rs2b0t_machine_start`): holds `hooks[name]` for every
//!   [`Family::CALLBACKS`] name (receiver `hooks`), then runs `begin`
//!   synchronously inside the caller's JS. Ops it emits join this tick's
//!   InteractReq batch at the caller's position in the JS queue. A
//!   [`Family::EXCLUSIVE`] family aborts its older live row as
//!   `superseded` when a new one runs; families that share an
//!   [`Family::EXCLUSIVE_GROUP`] supersede each other. A
//!   [`Family::KICK_ON_START`] row is then driven once ([`kick`]) inside
//!   the same call, so its first
//!   callbacks and ops run in the caller's own synchronous turn, as the
//!   frozen driver's first stretch did. The kick ignores pause and
//!   guardian hold on purpose: the frozen driver ran whenever the caller
//!   ran (a held tick still runs paint, native events and the event-loop
//!   drain), and ops emitted on a held tick are dropped with the JS queue
//!   there too ([`drop_ops`]). Only later steps are held back.
//! - **Step**: the isolate thread records the tick number, then calls
//!   [`step`] once per eligible tick, before any other of that tick's JS
//!   (the scene was applied by the Snapshot command before the Tick). A
//!   row started during tick N is first stepped on tick N+1. Ops a step
//!   emits join the batch in emit order, after any JS rows already queued
//!   or queued by its callbacks.
//! - **Callbacks**: a [`Step::Call`] invokes the held script function
//!   through the one callback path (`load/callback_v8.rs`), in a fresh
//!   handle scope per call, followed by a microtask checkpoint. A plain
//!   return value is the [`Reply`] of the very next step, in the same
//!   tick; so is a promise that settles through microtasks alone. A
//!   promise still pending is held; the row does not step until it
//!   settles. After the tick's pump (and its microtasks) the isolate runs
//!   [`resume`], which drives only the rows whose promise was pending, so
//!   a callback that waited on an Execution wait resumes its row in the
//!   tick the wait settled, as a frozen `await` would; a row that ends
//!   there settles its await in the same tick ([`any_settled`]). A throw
//!   or rejection is [`Reply::Threw`] carrying the thrown value itself; a
//!   family that fails with it ([`Step::Fail`]) rejects the script's
//!   `runMachine` await with that same value. At most [`CALLS_PER_TICK`]
//!   callbacks run per row per tick, across both passes; the rest
//!   continue next tick. A row whose callback superseded it stops at once.
//!   A callback ended by termination (join's or the watchdog's) is never
//!   shown to the family: the row is aborted `terminated` and no further
//!   callback runs in that pass or kick. A hook in [`Family::SYNC_HOOKS`]
//!   keeps frozen synchronous-call semantics instead: a returned promise is
//!   not awaited, and its reply is the promise as a value (`{}`: an object
//!   with no own properties).
//! - **Synchronous asks** ([`Cx::ask`]): a step may call a held hook
//!   inline, through the same path, for a script getter or predicate its
//!   decision reads where it stands (frozen `host.hpFraction()`,
//!   `site.inArea(t)`) or a synchronous notification (`host.log(m)`).
//!   Join's claim is checked before and after each ask. An ask never
//!   waits: a returned promise is `not impl`. It cannot be absorbed: a
//!   throw fails the row with the thrown value, a terminate or claim
//!   aborts it `terminated` (the flag above is set), and more than
//!   [`ASKS_PER_STEP`] asks in one step fail it; the host enforces that
//!   after the step returns and drops that step's ops. Asks do not count
//!   toward [`CALLS_PER_TICK`]. Begin has no script side.
//! - **Completion**: a `Done`/`Fail` row ends at once (no JS `end` op). Its
//!   outcome waits in the host until the JS await helper — a wait parked
//!   on the `Execution` park list — takes it, so it settles exactly once:
//!   in the tick's pump, or, for an outcome reached after the pump (the
//!   resume pass, a supersede by the tick's own JS), in the machine-only
//!   settle that follows the resume pass in the same tick.
//! - **ResetSession** ([`on_reset`]): every live row is aborted and its
//!   await settles `{ kind: 'aborted', reason: 'reset' }`; held callbacks
//!   and pending promises are released; pending ops are dropped with the
//!   JS queue, including ops of machines started during the reset drain
//!   ([`drop_ops`]).
//! - **Pause / guardian hold** ([`on_pause`], [`on_resume`], [`on_hold`]):
//!   rows are not stepped nor promises polled, and every row's
//!   [`InstantTaskClock`] freezes, so deadlines resume where they stopped.
//!   Ops emitted on a held tick are dropped with the JS queue
//!   ([`drop_ops`]).
//! - **Stop** ([`on_stop`]): every row, outcome and op is dropped before
//!   the isolate is; nothing settles.
//!
//! JS sees one envelope per start: `{ kind: 'done', value }`,
//! `{ kind: 'refused', reason }` or `{ kind: 'aborted', reason }`; a
//! `Fail` outcome rejects the `runMachine` promise with its [`Thrown`].

use crate::load::callback_v8::HeldCallback;
use crate::shim::{InteractReq, MaybeInteractReq};
use crate::task_clock::InstantTaskClock;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cell::{Cell, RefCell};

/// A started machine's id, unique for the isolate thread's life.
pub(crate) type Handle = u64;

/// Callbacks one row may run in one tick before it yields to the next.
///
/// The bound exists so a family that keeps calling back cannot hold the
/// tick forever; it must not split frozen-sized work across ticks. The
/// largest frozen driver iteration is bounded by the 28-slot pack: the
/// partner-trade receiver counts their offer twice (2 × 28
/// `theirProductMatch`) plus under ten fixed hooks, and `Trade` `pick`
/// sees at most 28 rows. 256 leaves about 4× headroom over that while
/// still yielding a runaway family within one tick.
pub(crate) const CALLS_PER_TICK: usize = 256;

/// One machine family: a Rust type the host begins, steps and aborts.
pub(crate) trait Family: Sized + 'static {
    /// The name JS passes to `runMachine`.
    const NAME: &'static str;
    /// At most one live row: a newer running start supersedes the older.
    const EXCLUSIVE: bool = false;
    /// The exclusive set this family belongs to: by default itself. Two
    /// families naming the same group supersede each other's rows (for
    /// example every bank open, whichever surface started it).
    const EXCLUSIVE_GROUP: &'static str = Self::NAME;
    /// Script callbacks by `hooks` key; [`Call::hook`] indexes this.
    const CALLBACKS: &'static [&'static str] = &[];
    /// Drive this row once inside `start`, so the first callbacks and
    /// verbs join the starting tick. Default is the host's next-tick
    /// first step (teleport clicks in begin). Clue's first next is a
    /// callback, so it opts in.
    const KICK_ON_START: bool = false;
    /// [`Family::CALLBACKS`] indexes called synchronously (frozen calls
    /// these hooks without `await`). A family whose callbacks are all
    /// synchronous lists every callback index here.
    const SYNC_HOOKS: &'static [usize] = &[];
    /// Typed start arguments, decoded from the JS value.
    type Args: DeserializeOwned;
    /// The completion value JS receives as `value`.
    type Output: Into<Value>;

    fn begin(args: Self::Args, cx: &mut Cx<'_>) -> Begin<Self>;

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output>;

    fn abort(&mut self, _why: AbortReason) {}
}

/// What `begin` decided.
pub(crate) enum Begin<F: Family> {
    /// A live row; it is stepped from the next eligible tick.
    Run(F),
    /// Settled without a row.
    Done(F::Output),
    /// Nothing started (missing facts or control, bad arguments).
    Refuse(String),
}

/// What one step decided.
pub(crate) enum Step<T> {
    /// Nothing more this tick.
    Wait,
    /// Call one script callback; its [`Reply`] reaches the next step.
    Call(Call),
    Done(T),
    /// End the row; the `runMachine` promise rejects with this value.
    Fail(Thrown),
}

/// One script-callback request: `hooks[CALLBACKS[hook]](...args)`.
pub(crate) struct Call {
    pub(crate) hook: usize,
    pub(crate) args: Vec<Value>,
}

/// A settled script callback.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Reply {
    /// The returned (or promise-fulfilled) value.
    Value(Value),
    /// The throw or rejection.
    Threw(Thrown),
}

/// A thrown JS value (kept as is) or a family's own failure message.
#[derive(Clone)]
pub(crate) struct Thrown {
    message: String,
    value: Option<v8::Global<v8::Value>>,
}

impl Thrown {
    /// A failure the family raises itself; JS sees `new Error(message)`.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            value: None,
        }
    }

    /// What script code threw or rejected with, and its message.
    pub(crate) fn js(message: String, value: v8::Global<v8::Value>) -> Self {
        Self {
            message,
            value: Some(value),
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// The thrown value itself, when script code threw it.
    pub(crate) fn value(&self) -> Option<&v8::Global<v8::Value>> {
        self.value.as_ref()
    }
}

impl std::fmt::Debug for Thrown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Thrown").field(&self.message).finish()
    }
}

/// By message: V8 handles have no value equality.
impl PartialEq for Thrown {
    fn eq(&self, other: &Self) -> bool {
        self.message == other.message
    }
}

/// Why the host ended a live row without its own completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AbortReason {
    /// ResetSession (reconnect / new session).
    Reset,
    /// A newer start of the same exclusive family.
    Superseded,
    /// A callback of this row was terminated (join or the watchdog).
    Terminated,
    /// The handle was never started here or was already taken.
    Unknown,
}

impl AbortReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Superseded => "superseded",
            Self::Terminated => "terminated",
            Self::Unknown => "unknown",
        }
    }
}

/// A machine's single settlement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Done(Value),
    Failed(Thrown),
    Aborted(AbortReason),
}

/// The synchronous result of a start.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Started {
    Running(Handle),
    Settled(Outcome),
    Refused(String),
}

/// One `take` by the JS await helper.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Take {
    Pending,
    Settled(Outcome),
}

/// What a family sees while it begins or steps.
pub(crate) struct Cx<'a> {
    ops: &'a mut Vec<InteractReq>,
    clock: &'a mut InstantTaskClock,
    reply: Option<Reply>,
    hooks: &'a [Hook],
    /// The script side while a row steps; `None` in begin.
    js: Option<&'a mut (dyn Js + 'a)>,
    /// Asks this step made, and how the first failing one ended the row.
    asks: usize,
    ending: Option<Ending>,
}

impl<'a> Cx<'a> {
    /// A bare context for a family's own unit tests: no hooks held.
    #[cfg(test)]
    pub(crate) fn test(
        ops: &'a mut Vec<InteractReq>,
        clock: &'a mut InstantTaskClock,
        reply: Option<Reply>,
    ) -> Self {
        Self {
            ops,
            clock,
            reply,
            hooks: &[],
            js: None,
            asks: 0,
            ending: None,
        }
    }
}

impl Cx<'_> {
    /// Append one game op to this tick's InteractReq batch.
    pub(crate) fn emit(&mut self, op: InteractReq) {
        self.ops.push(op);
    }

    /// This row's pause/hold-aware deadline clock.
    pub(crate) fn clock(&mut self) -> &mut InstantTaskClock {
        self.clock
    }

    /// The settlement of the callback the previous step asked for; `None`
    /// on any other step.
    pub(crate) fn reply(&mut self) -> Option<Reply> {
        self.reply.take()
    }

    /// Whether `hooks[CALLBACKS[hook]]` was present (not `undefined` or
    /// `null`) when the machine started.
    pub(crate) fn has(&self, hook: usize) -> bool {
        self.hooks.get(hook).is_some_and(|hook| hook.present)
    }

    /// Call `hooks[CALLBACKS[hook]](...args)` now, inside this step: a
    /// script getter or predicate a decision reads where it stands
    /// (frozen `host.hpFraction()`, `site.inArea(t)`), or a synchronous
    /// notification (`host.log(m)`). Same callback path as [`Step::Call`]
    /// (microtask checkpoint included), but synchronous only and outside
    /// [`CALLS_PER_TICK`]; async hooks go through [`Step::Call`].
    ///
    /// `Err(Ended)` ends the row whatever the step then returns, and the
    /// family should return at once: join claimed the tick for Stop
    /// (before or during the call) or the call was terminated (the row is
    /// aborted `terminated`, as a terminated [`Step::Call`] is), the hook
    /// threw (the `runMachine` await rejects with that value), it returned
    /// a promise (never awaited here: `not impl`), or the step made more
    /// than [`ASKS_PER_STEP`] asks. Begin has no script side, so an ask
    /// there is `Err` and ends nothing.
    pub(crate) fn ask(&mut self, hook: usize, args: &[Value]) -> Result<Value, Ended> {
        if self.ending.is_some() {
            return Err(Ended);
        }
        let Some(js) = self.js.as_deref_mut() else {
            return Err(Ended);
        };
        if js.claimed() {
            self.ending = Some(Ending::Stopped);
            return Err(Ended);
        }
        self.asks += 1;
        if self.asks > ASKS_PER_STEP {
            self.ending = Some(Ending::Failed(Thrown::new(format!(
                "not impl: more than {ASKS_PER_STEP} synchronous callbacks in one step"
            ))));
            return Err(Ended);
        }
        let callback = self.hooks.get(hook).map(|hook| &hook.callback);
        let called = js.call(callback, args);
        if matches!(called, Called::Terminated) {
            TERMINATED.with(|flag| flag.set(true));
            self.ending = Some(Ending::Stopped);
            return Err(Ended);
        }
        if js.claimed() {
            self.ending = Some(Ending::Stopped);
            return Err(Ended);
        }
        match called {
            Called::Settled(Reply::Value(value)) => Ok(value),
            Called::Settled(Reply::Threw(thrown)) => {
                self.ending = Some(Ending::Failed(thrown));
                Err(Ended)
            }
            Called::Pending(_) | Called::Terminated => {
                self.ending = Some(Ending::Failed(Thrown::new(
                    "not impl: a synchronous machine callback returned a promise",
                )));
                Err(Ended)
            }
        }
    }
}

/// An [`Cx::ask`] that ended its row; the family returns at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ended;

/// Synchronous callbacks one step may make ([`Cx::ask`]).
pub(crate) const ASKS_PER_STEP: usize = 512;

/// How an ask ended its row.
enum Ending {
    /// Join claimed the tick or terminated the call: the row is aborted
    /// `terminated`; the family has already seen a missing answer.
    Stopped,
    /// The row fails with this value.
    Failed(Thrown),
}

/// One declared script callback, held from start.
pub(crate) struct Hook {
    pub(crate) callback: HeldCallback,
    /// Not `undefined`/`null` when read at start.
    pub(crate) present: bool,
}

/// The isolate side of a step: script callbacks and the JS queue.
pub(crate) trait Js {
    /// `__rs2b0t_host.interact.length` now.
    fn queue_len(&mut self) -> usize;
    /// Call `hook` (`None`: the family asked for an undeclared hook), then
    /// run a microtask checkpoint, so a promise that settles through
    /// microtasks alone comes back settled.
    fn call(&mut self, hook: Option<&HeldCallback>, args: &[Value]) -> Called;
    /// Poll a promise a callback returned; `None` while pending.
    fn poll(&mut self, pending: &Pending) -> Option<Reply>;
    /// Join has claimed this tick for Stop, or the slow-tick watchdog armed
    /// a terminate: no more script may run. A callback may have absorbed
    /// the terminate, so a later one would otherwise spin past it.
    fn claimed(&mut self) -> bool;
}

/// A promise a script callback returned, held until it settles.
pub(crate) type Pending = v8::Global<v8::Promise>;

pub(crate) enum Called {
    Settled(Reply),
    Pending(Pending),
    /// Execution was terminated inside the callback; nothing else may run.
    Terminated,
}

/// One registered family.
struct Entry {
    name: &'static str,
    callbacks: &'static [&'static str],
    kick_on_start: bool,
    begin: fn(Value, Vec<Hook>, usize) -> Started,
}

const fn entry<F: Family>() -> Entry {
    Entry {
        name: F::NAME,
        callbacks: F::CALLBACKS,
        kick_on_start: F::KICK_ON_START,
        begin: begin_row::<F>,
    }
}

/// Every registered family, by name. The only machine dispatch table.
const FAMILIES: &[Entry] = &[
    entry::<crate::teleport::Teleport>(),
    entry::<crate::prayer::Prayer>(),
    entry::<crate::autocast::Autocast>(),
    entry::<crate::special::Special>(),
    entry::<crate::modals::Modals>(),
    entry::<crate::bank_open::BankOpen>(),
    entry::<crate::banking_open::BankingOpen>(),
    entry::<crate::bank_select::SelectBank>(),
    entry::<crate::fire::LightFire>(),
    entry::<crate::shop::Shop>(),
    entry::<crate::cake_stall::CakeStall>(),
    entry::<crate::production::ChatDialog>(),
    entry::<crate::reach::NpcDialog>(),
    entry::<crate::dialog::Dialog>(),
    entry::<crate::trade::Trade>(),
    entry::<crate::drive_partner_trade::PartnerTrade>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::FightKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::HoldKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::RetreatKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::WalkSpotKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_lair::Enter>>(),
    entry::<crate::hunt::Hunt<crate::hunt_leave::Leave>>(),
    entry::<crate::hunt::Hunt<crate::hunt_key::Key>>(),
    entry::<crate::hunt::Hunt<crate::hunt_cell::Cell>>(),
    entry::<crate::hunt::Hunt<crate::hunt_bank::Bank>>(),
    entry::<crate::hunt::WaitFed>(),
    entry::<crate::hunt::TeleportOut>(),
    entry::<crate::hunt::Acquire>(),
    entry::<crate::clue::Clue>(),
    entry::<crate::quest_journal::QuestJournal>(),
    entry::<crate::reach_entity::EntityOp>(),
    entry::<crate::reach_entity::WalkHops>(),
    entry::<crate::walk::WalkResilient>(),
    entry::<crate::walk::WalkOpening>(),
    entry::<crate::bank_op::BankOp>(),
    entry::<crate::bank_deposit::BankDeposit>(),
    entry::<crate::bank_withdraw::WithdrawTo>(),
    entry::<crate::bank_withdraw::CloseConfirm>(),
    entry::<crate::bank_access::BankAccess>(),
    entry::<crate::bank_access::NpcAccess>(),
    entry::<crate::periodic_bank::BankNearest>(),
    entry::<crate::periodic_bank::PeriodicBank>(),
    entry::<crate::death_recovery::DeathRecovery>(),
    #[cfg(test)]
    entry::<tests::Probe>(),
    #[cfg(test)]
    entry::<tests::Solo>(),
    #[cfg(test)]
    entry::<tests::Hooked>(),
    #[cfg(test)]
    entry::<tests::Burst>(),
    #[cfg(test)]
    entry::<tests::Present>(),
    #[cfg(test)]
    entry::<tests::Nested>(),
    #[cfg(test)]
    entry::<tests::SoloHooked>(),
    #[cfg(test)]
    entry::<tests::Kicked>(),
    #[cfg(test)]
    entry::<tests::Ask>(),
    #[cfg(test)]
    entry::<tests::Inline>(),
    #[cfg(test)]
    entry::<tests::SoloAsker>(),
];

/// The callback names `family` holds at start, or `None` if unregistered.
pub(crate) fn callbacks_of(family: &str) -> Option<&'static [&'static str]> {
    FAMILIES
        .iter()
        .find(|entry| entry.name == family)
        .map(|entry| entry.callbacks)
}

/// Whether a live row of `family` is running (its begin returned
/// [`Begin::Run`] and it has not settled).
///
/// A family whose frozen surface admits one operation at a time asks
/// inside its own begin, before it emits anything; a start that refuses
/// this way supersedes nothing, so the admitted row keeps its await.
/// A row a [`pass`] is stepping counts: its callback runs inside that
/// pass, and script code started from it must see the row it belongs to.
pub(crate) fn live(family: &str) -> bool {
    HOST.with(|host| {
        let host = host.borrow();
        host.rows.iter().any(|row| row.family == family)
            || host.stepping.iter().any(|(name, _)| *name == family)
    })
}

/// Test seam: pull a live row's deadline `millis` closer to now, so a
/// family's own window is exercised without sleeping the bound out. A
/// deadline the row never armed stays unarmed.
#[cfg(test)]
pub(crate) fn age(handle: Handle, millis: u64) {
    use std::time::Duration;
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if let Some(row) = host.rows.iter_mut().find(|row| row.handle == handle) {
            row.clock.deadline = row
                .clock
                .deadline
                .and_then(|deadline| deadline.checked_sub(Duration::from_millis(millis)));
        }
    });
}

/// Whether `start` should drive `family` once before returning Running.
pub(crate) fn kick_on_start(family: &str) -> bool {
    FAMILIES
        .iter()
        .find(|entry| entry.name == family)
        .is_some_and(|entry| entry.kick_on_start)
}

/// The type-erased row the host steps.
trait Machine {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value>;
    fn abort(&mut self, why: AbortReason);
}

impl<F: Family> Machine for F {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match Family::step(self, cx) {
            Step::Wait => Step::Wait,
            Step::Call(call) => Step::Call(call),
            Step::Done(out) => Step::Done(out.into()),
            Step::Fail(reason) => Step::Fail(reason),
        }
    }

    fn abort(&mut self, why: AbortReason) {
        Family::abort(self, why);
    }
}

struct Row {
    handle: Handle,
    family: &'static str,
    /// [`Family::EXCLUSIVE_GROUP`] of an exclusive family.
    exclusive: Option<&'static str>,
    /// [`Family::SYNC_HOOKS`].
    sync_hooks: &'static [usize],
    clock: InstantTaskClock,
    hooks: Vec<Hook>,
    /// Callbacks run this tick, across both passes.
    calls: usize,
    /// The settled callback the next step reads.
    reply: Option<Reply>,
    /// The callback promise the row waits on.
    pending: Option<Pending>,
    machine: Box<dyn Machine>,
}

/// Emitted ops keyed by the JS queue length at emit time, so they merge
/// into the tick batch where they happened.
type PlacedOp = (usize, InteractReq);

struct Host {
    next: Handle,
    rows: Vec<Row>,
    /// The rows a [`pass`] is stepping right now. They are out of `rows`
    /// while a callback runs, and [`live`] must still see them.
    stepping: Vec<(&'static str, Handle)>,
    /// The newest running row of each exclusive group.
    newest: Vec<(&'static str, Handle)>,
    settled: Vec<(Handle, Outcome)>,
    ops: Vec<PlacedOp>,
    paused: bool,
    held: bool,
}

thread_local! {
    static HOST: RefCell<Host> = const { RefCell::new(Host::new()) };
    /// A family `begin` is running: a nested start is refused.
    static IN_BEGIN: Cell<bool> = const { Cell::new(false) };
    /// A callback was terminated in this pass or kick: drive no further.
    static TERMINATED: Cell<bool> = const { Cell::new(false) };
}

impl Host {
    const fn new() -> Self {
        Self {
            next: 1,
            rows: Vec::new(),
            stepping: Vec::new(),
            newest: Vec::new(),
            settled: Vec::new(),
            ops: Vec::new(),
            paused: false,
            held: false,
        }
    }

    fn fresh_clock(&self) -> InstantTaskClock {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(self.paused, self.held);
        clock
    }

    fn place(&mut self, at: usize, ops: Vec<InteractReq>) {
        self.ops.extend(ops.into_iter().map(|op| (at, op)));
    }

    /// Drop `handle` from the rows a pass is stepping: it ended here, so
    /// it is no longer live.
    fn unstep(&mut self, family: &'static str, handle: Handle) {
        self.stepping
            .retain(|(name, held)| !(*name == family && *held == handle));
    }

    fn superseded(&self, row: &Row) -> bool {
        row.exclusive.is_some_and(|group| {
            self.newest
                .iter()
                .any(|(newest, handle)| *newest == group && *handle != row.handle)
        })
    }

    fn abort_superseded(&mut self) {
        let mut i = 0;
        while i < self.rows.len() {
            if self.superseded(&self.rows[i]) {
                let mut row = self.rows.remove(i);
                row.machine.abort(AbortReason::Superseded);
                self.settled
                    .push((row.handle, Outcome::Aborted(AbortReason::Superseded)));
            } else {
                i += 1;
            }
        }
    }

    fn take(&mut self, handle: Handle) -> Take {
        if let Some(i) = self.settled.iter().position(|(h, _)| *h == handle) {
            return Take::Settled(self.settled.remove(i).1);
        }
        if self.rows.iter().any(|row| row.handle == handle) {
            return Take::Pending;
        }
        Take::Settled(Outcome::Aborted(AbortReason::Unknown))
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.paused = paused;
        self.held = held;
        for row in &mut self.rows {
            row.clock.set_freeze(paused, held);
        }
    }

    fn reset(&mut self) {
        for mut row in self.rows.drain(..) {
            row.machine.abort(AbortReason::Reset);
            self.settled
                .push((row.handle, Outcome::Aborted(AbortReason::Reset)));
        }
        self.newest.clear();
        self.stepping.clear();
        self.ops.clear();
    }

    fn stop(&mut self) {
        self.rows.clear();
        self.newest.clear();
        self.stepping.clear();
        self.settled.clear();
        self.ops.clear();
    }
}

fn begin_row<F: Family>(args: Value, hooks: Vec<Hook>, at: usize) -> Started {
    let args: F::Args = match serde_json::from_value(args) {
        Ok(args) => args,
        Err(e) => return Started::Refused(format!("{} arguments: {e}", F::NAME)),
    };
    let mut clock = HOST.with(|host| host.borrow().fresh_clock());
    let mut ops = Vec::new();
    // The host is not borrowed while the family begins.
    IN_BEGIN.with(|flag| flag.set(true));
    let _begun = BeginGuard;
    let begun = F::begin(
        args,
        &mut Cx {
            ops: &mut ops,
            clock: &mut clock,
            reply: None,
            hooks: &hooks,
            js: None,
            asks: 0,
            ending: None,
        },
    );
    drop(_begun);
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match begun {
            Begin::Refuse(reason) => Started::Refused(reason),
            Begin::Done(out) => {
                host.place(at, ops);
                Started::Settled(Outcome::Done(out.into()))
            }
            Begin::Run(machine) => {
                let handle = host.next;
                host.next += 1;
                if F::EXCLUSIVE {
                    host.newest
                        .retain(|(group, _)| *group != F::EXCLUSIVE_GROUP);
                    host.newest.push((F::EXCLUSIVE_GROUP, handle));
                    host.abort_superseded();
                }
                host.place(at, ops);
                host.rows.push(Row {
                    handle,
                    family: F::NAME,
                    exclusive: F::EXCLUSIVE.then_some(F::EXCLUSIVE_GROUP),
                    sync_hooks: F::SYNC_HOOKS,
                    clock,
                    hooks,
                    calls: 0,
                    reply: None,
                    pending: None,
                    machine: Box::new(machine),
                });
                Started::Running(handle)
            }
        }
    })
}

/// Clears [`IN_BEGIN`] however the begin returns.
struct BeginGuard;

impl Drop for BeginGuard {
    fn drop(&mut self) {
        IN_BEGIN.with(|flag| flag.set(false));
    }
}

/// Start `family` with JS `args` and its held `hooks` (one per
/// [`Family::CALLBACKS`] name); `at` is the JS interact queue length.
pub(crate) fn start(family: &str, args: Value, hooks: Vec<Hook>, at: usize) -> Started {
    if IN_BEGIN.with(Cell::get) {
        return Started::Refused("a machine begin may not start a machine".into());
    }
    let Some(entry) = FAMILIES.iter().find(|entry| entry.name == family) else {
        return Started::Refused(format!("unknown machine family {family:?}"));
    };
    (entry.begin)(args, hooks, at)
}

/// Drive one live row once (the starting tick for [`Family::KICK_ON_START`]).
pub(crate) fn kick(handle: Handle, js: &mut impl Js) {
    TERMINATED.with(|flag| flag.set(false));
    let mut at = js.queue_len();
    let Some(mut row) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.rows
            .iter()
            .position(|row| row.handle == handle)
            .map(|i| host.rows.remove(i))
    }) else {
        return;
    };
    let outcome = drive(&mut row, js, &mut at);
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match outcome {
            // The termination unwinds the caller's `runMachine` before it
            // parks an await: nothing would ever take this outcome.
            Some(Outcome::Aborted(AbortReason::Terminated)) => {}
            Some(outcome) => host.settled.push((handle, outcome)),
            None => host.rows.push(row),
        }
    });
}

/// Step every live row once. Called at the start of each eligible tick,
/// after the tick number is recorded.
pub(crate) fn step(js: &mut impl Js) {
    pass(js, Pass::Step);
}

/// Drive the rows waiting on a callback promise, if it settled. Called
/// once per eligible tick, after the pump and its microtasks.
pub(crate) fn resume(js: &mut impl Js) {
    pass(js, Pass::Resume);
}

/// Whether an outcome waits for its JS await.
pub(crate) fn any_settled() -> bool {
    HOST.with(|host| !host.borrow().settled.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Every row; starts the tick's callback budget.
    Step,
    /// Rows waiting on a promise; the budget carries over.
    Resume,
}

/// Rows are taken out of the host while they step, so a callback may
/// start another machine; the host is borrowed only between JS calls.
/// [`Host::stepping`] keeps those rows visible to [`live`].
fn pass(js: &mut impl Js, pass: Pass) {
    let Some(mut rows) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.paused || host.held {
            return None;
        }
        let rows = std::mem::take(&mut host.rows);
        host.stepping = rows.iter().map(|row| (row.family, row.handle)).collect();
        Some(rows)
    }) else {
        return;
    };
    TERMINATED.with(|flag| flag.set(false));
    // Script code may have queued rows before this pass (a probe, the
    // recovery anchor, this tick's own JS); step ops land after them.
    let mut at = js.queue_len();
    rows.retain_mut(|row| {
        if pass == Pass::Step {
            row.calls = 0;
        } else if row.pending.is_none() {
            return true;
        }
        if HOST.with(|host| host.borrow().superseded(row)) {
            row.machine.abort(AbortReason::Superseded);
            HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
            settle(row.handle, Outcome::Aborted(AbortReason::Superseded));
            return false;
        }
        match drive(row, js, &mut at) {
            Some(outcome) => {
                HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
                settle(row.handle, outcome);
                false
            }
            None => true,
        }
    });
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.stepping.clear();
        let started = std::mem::replace(&mut host.rows, rows);
        host.rows.extend(started);
        host.abort_superseded();
    });
}

fn settle(handle: Handle, outcome: Outcome) {
    HOST.with(|host| host.borrow_mut().settled.push((handle, outcome)));
}

/// One row's steps for this pass; `Some` when it ended. Stops, leaving
/// the row as it is, once join has claimed the tick.
fn drive(row: &mut Row, js: &mut impl Js, at: &mut usize) -> Option<Outcome> {
    loop {
        if js.claimed() || TERMINATED.with(Cell::get) {
            return None;
        }
        if let Some(pending) = &row.pending {
            row.reply = Some(js.poll(pending)?);
            row.pending = None;
        }
        if row.calls == CALLS_PER_TICK {
            return None;
        }
        let mut ops = Vec::new();
        let mut cx = Cx {
            ops: &mut ops,
            clock: &mut row.clock,
            reply: row.reply.take(),
            hooks: &row.hooks,
            js: Some(&mut *js),
            asks: 0,
            ending: None,
        };
        let step = row.machine.step(&mut cx);
        let asked = cx.asks > 0;
        // An ask that failed ends the row whatever the step returned; the
        // ops of that step are dropped with it.
        match cx.ending.take() {
            Some(Ending::Stopped) => {
                row.machine.abort(AbortReason::Terminated);
                return Some(Outcome::Aborted(AbortReason::Terminated));
            }
            Some(Ending::Failed(thrown)) => return Some(Outcome::Failed(thrown)),
            None => {}
        }
        // An ask started a newer row of this exclusive family: that row
        // owns the tick, so this step's decision and ops are dropped.
        if asked && HOST.with(|host| host.borrow().superseded(row)) {
            row.machine.abort(AbortReason::Superseded);
            return Some(Outcome::Aborted(AbortReason::Superseded));
        }
        HOST.with(|host| host.borrow_mut().place(*at, ops));
        match step {
            Step::Wait => return None,
            Step::Done(value) => return Some(Outcome::Done(value)),
            Step::Fail(thrown) => return Some(Outcome::Failed(thrown)),
            Step::Call(call) => {
                let hook = row.hooks.get(call.hook).map(|hook| &hook.callback);
                let called = js.call(hook, &call.args);
                row.calls += 1;
                *at = js.queue_len();
                match called {
                    Called::Settled(reply) => row.reply = Some(reply),
                    Called::Pending(pending) if !row.sync_hooks.contains(&call.hook) => {
                        row.pending = Some(pending)
                    }
                    // A frozen synchronous call sees the promise object.
                    Called::Pending(_) => {
                        row.reply = Some(Reply::Value(Value::Object(Default::default())))
                    }
                    Called::Terminated => {
                        TERMINATED.with(|flag| flag.set(true));
                        row.machine.abort(AbortReason::Terminated);
                        return Some(Outcome::Aborted(AbortReason::Terminated));
                    }
                }
                // The callback started a newer row of this exclusive family.
                if HOST.with(|host| host.borrow().superseded(row)) {
                    row.machine.abort(AbortReason::Superseded);
                    return Some(Outcome::Aborted(AbortReason::Superseded));
                }
            }
        }
    }
}

/// The JS await helper's poll: an outcome is handed out exactly once.
pub(crate) fn take(handle: Handle) -> Take {
    HOST.with(|host| host.borrow_mut().take(handle))
}

/// Merge this tick's machine ops into the drained JS rows in emit order.
pub(crate) fn merge_ops(rows: Vec<MaybeInteractReq>) -> Vec<InteractReq> {
    let mut ops = HOST.with(|host| std::mem::take(&mut host.borrow_mut().ops));
    merge(rows, &mut ops)
}

fn merge(rows: Vec<MaybeInteractReq>, ops: &mut Vec<PlacedOp>) -> Vec<InteractReq> {
    ops.sort_by_key(|(at, _)| *at);
    let mut out = Vec::with_capacity(rows.len() + ops.len());
    let mut ops = ops.drain(..).peekable();
    for (i, row) in rows.into_iter().enumerate() {
        while let Some((_, op)) = ops.next_if(|(at, _)| *at <= i) {
            out.push(op);
        }
        if let MaybeInteractReq::Req(req) = row {
            out.push(req);
        }
    }
    out.extend(ops.map(|(_, op)| op));
    out
}

/// A held tick drops its public actions; machine ops go with them.
pub(crate) fn drop_ops() {
    HOST.with(|host| host.borrow_mut().ops.clear());
}

pub(crate) fn on_pause() {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let held = host.held;
        host.set_freeze(true, held);
    });
}

pub(crate) fn on_resume() {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let held = host.held;
        host.set_freeze(false, held);
    });
}

pub(crate) fn on_hold(held: bool) {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let paused = host.paused;
        host.set_freeze(paused, held);
    });
}

pub(crate) fn on_reset() {
    HOST.with(|host| host.borrow_mut().reset());
}

pub(crate) fn on_stop() {
    HOST.with(|host| host.borrow_mut().stop());
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::time::Duration;

    /// Test family: emits `if-button {button}` at begin, one `if-button`
    /// per step for `steps` steps, then completes with the step count.
    /// `deadline_ms` arms the clock at begin; a reached bound completes
    /// with `"timeout"`.
    pub(crate) struct Probe {
        left: u32,
        done: u32,
        button: i32,
        deadline: bool,
    }

    #[derive(Deserialize)]
    pub(crate) struct ProbeArgs {
        button: i32,
        steps: u32,
        #[serde(default)]
        refuse: bool,
        #[serde(default)]
        deadline_ms: Option<u64>,
    }

    impl Family for Probe {
        const NAME: &'static str = "probe";
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            cx.emit(InteractReq::IfButton {
                component_id: args.button,
            });
            if args.refuse {
                return Begin::Refuse("probe refused".into());
            }
            if args.steps == 0 {
                return Begin::Done(json!(0));
            }
            if let Some(ms) = args.deadline_ms {
                cx.clock().arm(ms);
            }
            Begin::Run(Self {
                left: args.steps,
                done: 0,
                button: args.button,
                deadline: args.deadline_ms.is_some(),
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if self.deadline {
                return if cx.clock().bound_reached() {
                    Step::Done(json!("timeout"))
                } else {
                    Step::Wait
                };
            }
            if self.left == 0 {
                return Step::Done(json!(self.done));
            }
            self.left -= 1;
            self.done += 1;
            cx.emit(InteractReq::IfButton {
                component_id: self.button + self.done as i32,
            });
            Step::Wait
        }
    }

    /// [`Probe`] as an exclusive family; records its abort reason.
    pub(crate) struct Solo(Probe);

    thread_local! {
        static SOLO_ABORTS: std::cell::Cell<Option<AbortReason>> =
            const { std::cell::Cell::new(None) };
    }

    impl Family for Solo {
        const NAME: &'static str = "solo";
        const EXCLUSIVE: bool = true;
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            match Probe::begin(args, cx) {
                Begin::Run(probe) => Begin::Run(Self(probe)),
                Begin::Done(out) => Begin::Done(out),
                Begin::Refuse(reason) => Begin::Refuse(reason),
            }
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            Family::step(&mut self.0, cx)
        }

        fn abort(&mut self, why: AbortReason) {
            SOLO_ABORTS.with(|cell| cell.set(Some(why)));
        }
    }

    /// Test family over two script callbacks: `sync(1, 'a')`, then
    /// `later(<sync reply>)` (typically async), then `mark()` once
    /// `after_ticks` more steps passed, completing with every reply. A
    /// throwing callback fails the machine with its message.
    pub(crate) struct Hooked {
        replies: Vec<Value>,
        after_ticks: u32,
        emit: i32,
    }

    #[derive(Deserialize)]
    pub(crate) struct HookedArgs {
        #[serde(default)]
        after_ticks: u32,
        #[serde(default)]
        emit: i32,
    }

    impl Family for Hooked {
        const NAME: &'static str = "hooked";
        const CALLBACKS: &'static [&'static str] = &["sync", "later", "mark"];
        type Args = HookedArgs;
        type Output = Value;

        fn begin(args: HookedArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                replies: Vec::new(),
                after_ticks: args.after_ticks,
                emit: args.emit,
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            match cx.reply() {
                Some(Reply::Threw(message)) => return Step::Fail(message),
                Some(Reply::Value(value)) => {
                    self.replies.push(value);
                    if self.emit != 0 {
                        cx.emit(InteractReq::IfButton {
                            component_id: self.emit + self.replies.len() as i32,
                        });
                    }
                }
                None => {}
            }
            match self.replies.len() {
                0 => Step::Call(Call {
                    hook: 0,
                    args: vec![json!(1), json!("a")],
                }),
                1 => Step::Call(Call {
                    hook: 1,
                    args: vec![self.replies[0].clone()],
                }),
                2 if self.after_ticks > 0 => {
                    self.after_ticks -= 1;
                    Step::Wait
                }
                2 => Step::Call(Call {
                    hook: 2,
                    args: Vec::new(),
                }),
                _ => Step::Done(Value::Array(std::mem::take(&mut self.replies))),
            }
        }
    }

    /// Calls `each(i)` for i in 0..calls, then completes with the replies.
    /// A throw fails the machine, unless `keep` records its message.
    pub(crate) struct Burst {
        calls: usize,
        keep: bool,
        replies: Vec<Value>,
    }

    #[derive(Deserialize)]
    pub(crate) struct BurstArgs {
        calls: usize,
        #[serde(default)]
        keep: bool,
    }

    impl Family for Burst {
        const NAME: &'static str = "burst";
        const CALLBACKS: &'static [&'static str] = &["each"];
        type Args = BurstArgs;
        type Output = Value;

        fn begin(args: BurstArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                calls: args.calls,
                keep: args.keep,
                replies: Vec::new(),
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            match cx.reply() {
                Some(Reply::Threw(thrown)) if self.keep => {
                    self.replies.push(json!(thrown.message()));
                }
                Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
                Some(Reply::Value(value)) => self.replies.push(value),
                None => {}
            }
            if self.replies.len() == self.calls {
                return Step::Done(Value::Array(std::mem::take(&mut self.replies)));
            }
            Step::Call(Call {
                hook: 0,
                args: vec![json!(self.replies.len())],
            })
        }
    }

    /// Settles at begin with which of `a`, `b`, `c` were present.
    pub(crate) struct Present;

    impl Family for Present {
        const NAME: &'static str = "present";
        const CALLBACKS: &'static [&'static str] = &["a", "b", "c"];
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Done(json!([cx.has(0), cx.has(1), cx.has(2), cx.has(3)]))
        }

        fn step(&mut self, _cx: &mut Cx<'_>) -> Step<Value> {
            Step::Wait
        }
    }

    /// A begin that (wrongly) starts another machine; settles with what
    /// that start returned.
    pub(crate) struct Nested;

    impl Family for Nested {
        const NAME: &'static str = "nested";
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            let inner = start("probe", json!({ "button": 1, "steps": 1 }), Vec::new(), 0);
            Begin::Done(json!(format!("{inner:?}")))
        }

        fn step(&mut self, _cx: &mut Cx<'_>) -> Step<Value> {
            Step::Wait
        }
    }

    /// Exclusive: calls `again()` once, then emits `if-button 999` each
    /// step; `quiet` rows only wait.
    pub(crate) struct SoloHooked {
        quiet: bool,
        called: bool,
    }

    #[derive(Deserialize)]
    pub(crate) struct SoloHookedArgs {
        #[serde(default)]
        quiet: bool,
    }

    impl Family for SoloHooked {
        const NAME: &'static str = "solo-hooked";
        const EXCLUSIVE: bool = true;
        const CALLBACKS: &'static [&'static str] = &["again"];
        type Args = SoloHookedArgs;
        type Output = Value;

        fn begin(args: SoloHookedArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                quiet: args.quiet,
                called: false,
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if self.quiet {
                return Step::Wait;
            }
            if !self.called {
                self.called = true;
                return Step::Call(Call {
                    hook: 0,
                    args: Vec::new(),
                });
            }
            cx.emit(InteractReq::IfButton { component_id: 999 });
            Step::Wait
        }
    }

    /// Asks `hooks.get(i)` inline `asks` times in one step, emitting
    /// `if-button i` after each answer, then completes with the answers.
    /// Keeps stepping after a failed ask, to prove the host ends the row.
    pub(crate) struct Inline {
        asks: usize,
    }

    #[derive(Deserialize)]
    pub(crate) struct InlineArgs {
        asks: usize,
    }

    impl Family for Inline {
        const NAME: &'static str = "inline";
        const CALLBACKS: &'static [&'static str] = &["get"];
        type Args = InlineArgs;
        type Output = Value;

        fn begin(args: InlineArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            assert_eq!(
                cx.ask(0, &[json!(0)]),
                Err(Ended),
                "begin has no script side"
            );
            Begin::Run(Self { asks: args.asks })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            let mut answers = Vec::new();
            for i in 0..self.asks {
                if let Ok(value) = cx.ask(0, &[json!(i)]) {
                    answers.push(value);
                }
                cx.emit(InteractReq::IfButton {
                    component_id: i as i32,
                });
            }
            Step::Done(Value::Array(answers))
        }
    }

    /// Exclusive: asks `get()` once per step, then emits `if-button 5`.
    pub(crate) struct SoloAsker;

    impl Family for SoloAsker {
        const NAME: &'static str = "solo-asker";
        const EXCLUSIVE: bool = true;
        const CALLBACKS: &'static [&'static str] = &["get"];
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self)
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if cx.ask(0, &[json!(0)]).is_err() {
                return Step::Wait;
            }
            cx.emit(InteractReq::IfButton { component_id: 5 });
            Step::Wait
        }
    }

    /// Calls `ask()` once and completes with its reply. Used to start
    /// another machine from inside a callback.
    pub(crate) struct Ask;

    impl Family for Ask {
        const NAME: &'static str = "ask";
        const CALLBACKS: &'static [&'static str] = &["ask"];
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self)
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            match cx.reply() {
                Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                Some(Reply::Value(value)) => Step::Done(value),
                None => Step::Call(Call {
                    hook: 0,
                    args: Vec::new(),
                }),
            }
        }
    }

    /// KICK_ON_START: calls `each(0)`, `each(1)`, emits `if-button 60`,
    /// then completes with the replies (at once when `done`, else on the
    /// next step).
    pub(crate) struct Kicked {
        done: bool,
        replies: Vec<Value>,
    }

    #[derive(Deserialize)]
    pub(crate) struct KickedArgs {
        done: bool,
    }

    impl Family for Kicked {
        const NAME: &'static str = "kicked";
        const CALLBACKS: &'static [&'static str] = &["each"];
        const KICK_ON_START: bool = true;
        type Args = KickedArgs;
        type Output = Value;

        fn begin(args: KickedArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                done: args.done,
                replies: Vec::new(),
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if let Some(Reply::Value(value)) = cx.reply() {
                self.replies.push(value);
                if self.replies.len() == 2 {
                    cx.emit(InteractReq::IfButton { component_id: 60 });
                    if !self.done {
                        return Step::Wait;
                    }
                }
            }
            if self.replies.len() < 2 {
                return Step::Call(Call {
                    hook: 0,
                    args: vec![json!(self.replies.len())],
                });
            }
            Step::Done(Value::Array(self.replies.clone()))
        }
    }

    /// Answers every callback at once with its first argument.
    struct Echo {
        calls: usize,
    }

    impl Js for Echo {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, args: &[Value]) -> Called {
            self.calls += 1;
            Called::Settled(Reply::Value(args[0].clone()))
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            unreachable!("Echo never returns a promise");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    /// No script callbacks: a machine that asks for one is a test bug.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no script callbacks in this test");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no script callbacks in this test");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn tick() {
        step(&mut NoJs);
    }

    fn begin(family: &str, args: Value) -> Started {
        start(family, args, Vec::new(), 0)
    }

    fn running(started: Started) -> Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    fn button(id: i32) -> InteractReq {
        InteractReq::IfButton { component_id: id }
    }

    fn drain() -> Vec<InteractReq> {
        merge_ops(Vec::new())
    }

    fn live_rows() -> usize {
        HOST.with(|host| host.borrow().rows.len())
    }

    fn freeze(paused: bool, held: bool) {
        HOST.with(|host| host.borrow_mut().set_freeze(paused, held));
    }

    /// Every live row's armed deadline has passed (family unit tests).
    pub(crate) fn expire_deadlines() {
        HOST.with(|host| {
            for row in &mut host.borrow_mut().rows {
                if row.clock.deadline.is_some() {
                    row.clock.deadline = Some(std::time::Instant::now() - Duration::from_millis(1));
                }
            }
        });
    }

    #[test]
    fn a_row_steps_emits_in_order_and_settles_exactly_once() {
        let h = running(begin("probe", json!({ "button": 10, "steps": 2 })));
        assert_eq!(drain(), vec![button(10)]);
        assert_eq!(take(h), Take::Pending);
        tick();
        tick();
        assert_eq!(drain(), vec![button(11), button(12)]);
        assert_eq!(take(h), Take::Pending);
        tick();
        assert_eq!(live_rows(), 0, "completion ends the row");
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!(2))));
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Unknown)),
            "an outcome is handed out once"
        );
    }

    #[test]
    fn begin_settles_or_refuses_without_a_row() {
        assert_eq!(
            begin("probe", json!({ "button": 5, "steps": 0 })),
            Started::Settled(Outcome::Done(json!(0)))
        );
        assert_eq!(drain(), vec![button(5)]);
        assert_eq!(
            begin("probe", json!({ "button": 6, "steps": 1, "refuse": true })),
            Started::Refused("probe refused".into())
        );
        assert!(drain().is_empty(), "a refused begin sends nothing");
        assert!(matches!(
            begin("probe", json!({ "steps": 1 })),
            Started::Refused(reason) if reason.starts_with("probe arguments:")
        ));
        assert!(matches!(begin("nowhere", json!({})), Started::Refused(_)));
        assert_eq!(live_rows(), 0);
    }

    #[test]
    fn reset_aborts_every_row_and_drops_pending_ops() {
        let a = running(begin("probe", json!({ "button": 1, "steps": 5 })));
        let b = running(begin("probe", json!({ "button": 2, "steps": 5 })));
        on_reset();
        assert!(drain().is_empty());
        assert_eq!(live_rows(), 0);
        tick();
        assert!(drain().is_empty(), "an aborted row never steps");
        for h in [a, b] {
            assert_eq!(take(h), Take::Settled(Outcome::Aborted(AbortReason::Reset)));
        }
    }

    #[test]
    fn stop_drops_rows_and_outcomes() {
        let a = running(begin("probe", json!({ "button": 1, "steps": 1 })));
        tick();
        tick();
        let b = running(begin("probe", json!({ "button": 2, "steps": 5 })));
        on_stop();
        assert_eq!(live_rows(), 0);
        assert!(drain().is_empty());
        for h in [a, b] {
            assert_eq!(
                take(h),
                Take::Settled(Outcome::Aborted(AbortReason::Unknown)),
                "Stop settles nothing"
            );
        }
    }

    #[test]
    fn pause_and_hold_skip_steps_and_freeze_deadlines() {
        let h = running(begin(
            "probe",
            json!({ "button": 1, "steps": 1, "deadline_ms": 40 }),
        ));
        on_pause();
        std::thread::sleep(Duration::from_millis(60));
        tick();
        assert_eq!(take(h), Take::Pending, "a paused row is not stepped");
        on_hold(true);
        on_resume();
        tick();
        assert_eq!(take(h), Take::Pending, "a held row is not stepped");
        on_hold(false);
        tick();
        assert_eq!(
            take(h),
            Take::Pending,
            "the frozen span does not count toward the deadline"
        );
        std::thread::sleep(Duration::from_millis(60));
        tick();
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!("timeout"))));
    }

    #[test]
    fn a_row_started_while_frozen_starts_frozen() {
        freeze(false, true);
        let h = running(begin(
            "probe",
            json!({ "button": 1, "steps": 1, "deadline_ms": 30 }),
        ));
        std::thread::sleep(Duration::from_millis(50));
        freeze(false, false);
        tick();
        assert_eq!(take(h), Take::Pending);
    }

    #[test]
    fn two_concurrent_rows_step_in_start_order_and_settle_independently() {
        let a = running(begin("probe", json!({ "button": 100, "steps": 1 })));
        let b = running(begin("probe", json!({ "button": 200, "steps": 2 })));
        assert_eq!(drain(), vec![button(100), button(200)]);
        tick();
        assert_eq!(drain(), vec![button(101), button(201)]);
        tick();
        assert_eq!(drain(), vec![button(202)]);
        assert_eq!(take(a), Take::Settled(Outcome::Done(json!(1))));
        assert_eq!(take(b), Take::Pending);
        tick();
        assert_eq!(take(b), Take::Settled(Outcome::Done(json!(2))));
    }

    #[test]
    fn an_exclusive_start_supersedes_only_its_own_family() {
        let other = running(begin("probe", json!({ "button": 1, "steps": 5 })));
        let old = running(begin("solo", json!({ "button": 2, "steps": 5 })));
        assert!(matches!(
            begin("solo", json!({ "button": 3, "steps": 1, "refuse": true })),
            Started::Refused(_)
        ));
        assert_eq!(
            take(old),
            Take::Pending,
            "a refused start supersedes nothing"
        );
        let new = running(begin("solo", json!({ "button": 4, "steps": 5 })));
        assert_eq!(
            take(old),
            Take::Settled(Outcome::Aborted(AbortReason::Superseded))
        );
        assert_eq!(SOLO_ABORTS.with(|c| c.get()), Some(AbortReason::Superseded));
        assert_eq!(take(new), Take::Pending);
        assert_eq!(take(other), Take::Pending);
    }

    /// Answers each call with its argument; call `throw_at` throws, call
    /// `terminate_at` is terminated, and from call `claim_at` on join has
    /// claimed the tick.
    struct Inliner {
        calls: usize,
        throw_at: Option<usize>,
        terminate_at: Option<usize>,
        claim_at: Option<usize>,
    }

    impl Inliner {
        fn new() -> Self {
            Self {
                calls: 0,
                throw_at: None,
                terminate_at: None,
                claim_at: None,
            }
        }
    }

    impl Js for Inliner {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, args: &[Value]) -> Called {
            self.calls += 1;
            if self.terminate_at == Some(self.calls) {
                return Called::Terminated;
            }
            if self.throw_at == Some(self.calls) {
                return Called::Settled(Reply::Threw(Thrown::new("boom")));
            }
            Called::Settled(Reply::Value(args[0].clone()))
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            unreachable!("Inliner never returns a promise");
        }

        fn claimed(&mut self) -> bool {
            self.claim_at.is_some_and(|at| self.calls >= at)
        }
    }

    #[test]
    fn asks_answer_inside_one_step_outside_the_callback_budget() {
        let asks = CALLS_PER_TICK + 3;
        let h = running(begin("inline", json!({ "asks": asks })));
        let mut js = Inliner::new();
        step(&mut js);
        assert_eq!(js.calls, asks);
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Done(json!((0..asks).collect::<Vec<_>>())))
        );
        assert_eq!(drain().len(), asks, "the step's ops are kept");
    }

    #[test]
    fn a_throwing_ask_fails_the_row_with_the_throw_and_drops_its_ops() {
        let h = running(begin("inline", json!({ "asks": 4 })));
        let mut js = Inliner::new();
        js.throw_at = Some(2);
        step(&mut js);
        assert_eq!(js.calls, 2, "no ask runs after the throw");
        assert_eq!(take(h), Take::Settled(Outcome::Failed(Thrown::new("boom"))));
        assert!(drain().is_empty());
    }

    #[test]
    fn a_terminated_or_claimed_ask_aborts_the_row() {
        let h = running(begin("inline", json!({ "asks": 4 })));
        let mut js = Inliner::new();
        js.terminate_at = Some(1);
        step(&mut js);
        assert_eq!(js.calls, 1);
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Terminated))
        );
        assert!(drain().is_empty());

        let h = running(begin("inline", json!({ "asks": 4 })));
        let mut js = Inliner::new();
        js.claim_at = Some(1);
        // The pass sees no claim before the step; the first ask's call
        // claims the tick, and the pass settles the row `terminated`.
        step(&mut js);
        assert_eq!(js.calls, 1, "no ask runs once the tick is claimed");
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Terminated))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn an_ask_that_starts_a_newer_exclusive_row_supersedes_the_asker() {
        /// The first call starts a newer `solo-asker` row.
        struct Starter {
            calls: usize,
        }
        impl Js for Starter {
            fn queue_len(&mut self) -> usize {
                0
            }
            fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
                self.calls += 1;
                if self.calls == 1 {
                    running(begin("solo-asker", json!({})));
                }
                Called::Settled(Reply::Value(json!(true)))
            }
            fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
                unreachable!("Starter never returns a promise");
            }
            fn claimed(&mut self) -> bool {
                false
            }
        }
        let old = running(begin("solo-asker", json!({})));
        let mut js = Starter { calls: 0 };
        step(&mut js);
        assert_eq!(
            take(old),
            Take::Settled(Outcome::Aborted(AbortReason::Superseded))
        );
        assert!(
            drain().is_empty(),
            "the superseded step's ops are dropped; the new row has not stepped"
        );
    }

    #[test]
    fn a_runaway_step_is_bounded() {
        let h = running(begin("inline", json!({ "asks": ASKS_PER_STEP + 10 })));
        let mut js = Inliner::new();
        step(&mut js);
        assert_eq!(js.calls, ASKS_PER_STEP);
        let Take::Settled(Outcome::Failed(thrown)) = take(h) else {
            panic!("the bound fails the row");
        };
        assert!(
            thrown.message().contains("synchronous callbacks"),
            "{thrown:?}"
        );
    }

    #[test]
    fn a_row_runs_at_most_the_callback_budget_per_tick() {
        let total = CALLS_PER_TICK + 8;
        let h = running(begin("burst", json!({ "calls": total })));
        let mut js = Echo { calls: 0 };
        step(&mut js);
        assert_eq!(js.calls, CALLS_PER_TICK);
        resume(&mut js);
        assert_eq!(
            js.calls, CALLS_PER_TICK,
            "a row not waiting on a promise does not resume"
        );
        assert_eq!(take(h), Take::Pending);
        step(&mut js);
        assert_eq!(js.calls, total);
        let Take::Settled(Outcome::Done(Value::Array(replies))) = take(h) else {
            panic!("the burst completes on the second tick");
        };
        assert_eq!(replies, (0..total).map(|i| json!(i)).collect::<Vec<_>>());
    }

    #[test]
    fn the_budget_does_not_split_a_frozen_sized_iteration() {
        // The partner-trade receiver's worst frozen iteration: two passes
        // over 28 offer rows plus its fixed hooks.
        let h = running(begin("burst", json!({ "calls": 2 * 28 + 10 })));
        step(&mut Echo { calls: 0 });
        assert!(matches!(take(h), Take::Settled(Outcome::Done(_))));
    }

    /// Answers like [`Echo`], and moves the JS queue one row per call (a
    /// callback that queued a JS op), until call `terminate_at`, which is
    /// terminated.
    struct Scripted {
        calls: usize,
        queued: usize,
        terminate_at: Option<usize>,
    }

    impl Js for Scripted {
        fn queue_len(&mut self) -> usize {
            self.queued
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, args: &[Value]) -> Called {
            self.calls += 1;
            if self.terminate_at == Some(self.calls) {
                return Called::Terminated;
            }
            self.queued += 1;
            Called::Settled(Reply::Value(args[0].clone()))
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            unreachable!("Scripted never returns a promise");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn scripted(terminate_at: Option<usize>) -> Scripted {
        Scripted {
            calls: 0,
            queued: 0,
            terminate_at,
        }
    }

    #[test]
    fn a_kick_runs_the_first_callbacks_and_ops_in_the_starting_turn() {
        let h = running(begin("kicked", json!({ "done": false })));
        let mut js = scripted(None);
        kick(h, &mut js);
        assert_eq!(js.calls, 2, "both callbacks ran inside the start");
        let js_row = |id: i32| MaybeInteractReq::Req(button(id));
        assert_eq!(
            merge_ops(vec![js_row(1), js_row(2)]),
            vec![button(1), button(2), button(60)],
            "the kick's op lands after the rows its callbacks queued"
        );
        assert_eq!(take(h), Take::Pending);
        resume(&mut js);
        assert_eq!(js.calls, 2, "a kicked row not on a promise does not resume");
        step(&mut js);
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!([0, 1]))));
    }

    #[test]
    fn a_row_that_ends_inside_its_kick_settles_at_once() {
        let h = running(begin("kicked", json!({ "done": true })));
        kick(h, &mut scripted(None));
        assert_eq!(live_rows(), 0);
        assert!(any_settled());
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!([0, 1]))));
    }

    #[test]
    fn a_terminated_callback_in_a_kick_aborts_the_row_unseen() {
        // `keep` would record a throw and call on: the family must never
        // see the termination.
        let h = running(begin("burst", json!({ "calls": 3, "keep": true })));
        let mut js = scripted(Some(1));
        kick(h, &mut js);
        assert_eq!(js.calls, 1, "no callback runs after the terminated one");
        assert_eq!(live_rows(), 0);
        assert!(
            !any_settled(),
            "the unwound caller never awaits it: no orphaned settlement"
        );
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Unknown))
        );
    }

    #[test]
    fn a_terminated_callback_stops_the_whole_pass() {
        let a = running(begin("burst", json!({ "calls": 3, "keep": true })));
        let b = running(begin("burst", json!({ "calls": 1, "keep": true })));
        let mut js = scripted(Some(1));
        step(&mut js);
        assert_eq!(js.calls, 1, "the second row is not driven this pass");
        assert_eq!(
            take(a),
            Take::Settled(Outcome::Aborted(AbortReason::Terminated))
        );
        assert_eq!(take(b), Take::Pending);
        step(&mut Echo { calls: 0 });
        assert_eq!(take(b), Take::Settled(Outcome::Done(json!([0]))));
    }

    #[test]
    fn a_begin_that_starts_a_machine_is_refused_not_a_panic() {
        assert_eq!(
            begin("nested", json!({})),
            Started::Settled(Outcome::Done(json!(format!(
                "{:?}",
                Started::Refused("a machine begin may not start a machine".into())
            ))))
        );
        assert_eq!(live_rows(), 0);
        running(begin("probe", json!({ "button": 1, "steps": 1 })));
    }

    #[test]
    fn a_row_stepping_a_callback_stays_live() {
        let handle = running(begin("ask", json!({})));
        assert!(live("ask"));
        // Starts a `solo` row from inside the callback, as script code
        // does, and answers with what `live` saw during that call.
        struct StartsInside;
        impl Js for StartsInside {
            fn queue_len(&mut self) -> usize {
                0
            }

            fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
                running(begin("solo", json!({ "button": 7, "steps": 5 })));
                Called::Settled(Reply::Value(json!({
                    "asking": live("ask"),
                    "started": live("solo"),
                })))
            }

            fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
                unreachable!("the ask callback returns a value")
            }

            fn claimed(&mut self) -> bool {
                false
            }
        }
        step(&mut StartsInside);
        assert_eq!(
            take(handle),
            Take::Settled(Outcome::Done(json!({ "asking": true, "started": true }))),
            "a callback must see this pass's rows, not an empty map"
        );
        assert!(live("solo"), "the callback's own row is live");
        assert!(!live("ask"), "a settled row is not live");
        assert_eq!(drain(), vec![button(7)], "the inner row kept its ops");
    }

    #[test]
    fn machine_ops_merge_at_their_js_queue_position() {
        let js = |id: i32| MaybeInteractReq::Req(button(id));
        let mut placed = vec![
            (0, button(1)),
            (2, button(3)),
            (2, button(4)),
            (9, button(6)),
        ];
        let skipped = MaybeInteractReq::Skip(crate::shim::RejectedRow("a number".into()));
        let rows = vec![js(2), skipped, js(5)];
        assert_eq!(
            merge(rows, &mut placed),
            vec![
                button(1),
                button(2),
                button(3),
                button(4),
                button(5),
                button(6)
            ]
        );
    }
}

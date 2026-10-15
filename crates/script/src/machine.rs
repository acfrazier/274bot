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
//! - [`Family::release`]: optional op that stops host work the row armed
//!   (its walk follow). A superseding start sends it at the new start's
//!   queue position; a reset does not (the host drops that work itself).
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
//!   frozen driver's first stretch did. The kick does not drain the
//!   isolate microtask queue: unrelated tasks queued before the call run
//!   after the caller continues, and an already-settled callback promise
//!   is observed by polling it. The kick ignores pause and
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
//! - **Changed evidence**: [`snapshot_step`] drives opted-in families once
//!   per distinct `(tick, evidence sequence)` without recording another tick.
//!   It carries the tick's callback count through callback continuations;
//!   families retain their existing interaction/batch dispatch guards.
//!   This pass can settle an already-observed result and its await without
//!   running `loop`, tick listeners, or explicit tick delays.
//! - **Callbacks**: a [`Step::Call`] invokes the held script function
//!   through the one callback path (`load/callback_v8.rs`), in a fresh
//!   handle scope per call. On a tick step a microtask checkpoint
//!   follows, so a plain return value, or a promise that settles through
//!   microtasks alone, is the [`Reply`] of the very next step in the same
//!   tick. A kick runs no checkpoint (see Kick above): its promise settles
//!   when the caller's own microtasks run, and the row takes it on its
//!   next step. A promise still pending is held; the row does not step
//!   until it settles. After the tick's pump (and its microtasks) the isolate runs
//!   [`resume`], which drives only the rows whose promise was pending, so
//!   a callback that waited on an Execution wait resumes its row in the
//!   tick the wait settled, as a frozen `await` would; a row that ends
//!   there settles its await in the same tick ([`any_settled`]). A throw
//!   or rejection is [`Reply::Threw`] carrying the thrown value itself; a
//!   family that fails with it ([`Step::Fail`]) rejects the script's
//!   `runMachine` await with that same value. At most [`CALLS_PER_TICK`]
//!   callbacks run per row per tick, across all passes; the rest
//!   continue next tick. A row whose callback superseded it stops at once.
//!   A callback ended by termination (join's or the watchdog's) is never
//!   shown to the family: the row is aborted `terminated` and no further
//!   callback runs in that pass or kick. Termination inside a pending
//!   callback promise's continuation (it runs in the caller's or the
//!   pump's microtasks) does not abort the row; the promise stays pending
//!   until Stop or Reset. A hook in [`Family::SYNC_HOOKS`]
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
//! - **ResetSession, session ended** ([`on_reset`]): every live row is
//!   aborted and its await settles `{ kind: 'aborted', reason: 'reset' }`;
//!   held callbacks and pending promises are released; pending ops are
//!   dropped with the JS queue, including ops of machines started during
//!   the reset drain ([`drop_ops`]).
//! - **ResetSession, reconnect**: nothing here is reset. Every row, held
//!   callback and pending promise stays, frozen like a Pause, so the whole
//!   chain the script had in flight (a walk started inside another row's
//!   callback included) resumes together on the relogged session's first
//!   tick, as frozen AutoRelogin resumes the paused script
//!   (`AutoRelogin.ts:180-190`, `159-163`). Pending ops are dropped with
//!   the JS queue; the host re-arms the script walk it was following.
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
use crate::{isolate_fb, native};
use api::identity::{ExecutionId, RequestId};
use api::script_contract::{CallbackCancelReason, RequestStamp};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
/// A started machine's id, unique for the isolate thread's life.
pub(crate) type Handle = u64;

/// Handles are returned to JavaScript as Number and encoded in isolate request ids.
const MAX_MACHINE_HANDLE: Handle = 9_007_199_254_740_991;

const MAX_PENDING_SEAT_STARTS: usize = api::script_contract::MAX_PENDING_STARTS;

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
    /// Whether changed evidence may drive this family without a game tick.
    /// Opt in only when `snapshot_step` preserves the family's per-tick
    /// interaction/batch quota and any server-required tick gates.
    const SNAPSHOT_SENSITIVE: bool = false;
    /// This family owns a walking operation while its row is live.
    const WALKING_OPERATION: bool = false;
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
    /// Drive a changed-evidence pass when opted in above.
    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        if Self::SNAPSHOT_SENSITIVE {
            self.step(cx)
        } else {
            Step::Wait
        }
    }

    fn abort(&mut self, _why: AbortReason) {}

    /// The op that stops the host work this row armed (a host walk follow),
    /// sent when a newer exclusive start supersedes the row and when the
    /// row fails or is terminated. A reset needs none: the host drops its
    /// session work itself.
    fn release(&self) -> Option<InteractReq> {
        None
    }
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
    /// A child of a revoked in-isolate parent.
    ParentAborted,
    /// Manual Combat takeover.
    UserInput,
    /// The handle was never started here or was already taken.
    Unknown,
}

impl AbortReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Superseded => "superseded",
            Self::Terminated => "terminated",
            Self::ParentAborted => "parent-aborted",
            Self::UserInput => "user-input",
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
    Refused(String),
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
    handle: Handle,
    /// The script side while a row steps; `None` in begin.
    js: Option<&'a mut (dyn Js + 'a)>,
    /// The outer operation's user-movement intent baseline.
    intent_baseline: u64,
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
            handle: 0,
            js: None,
            intent_baseline: user_move_intent_seq(),
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

    /// This row's isolate request identity, when it has a wire-safe handle.
    pub(crate) fn request_id(&self) -> Option<RequestId> {
        RequestId::isolate(self.handle).ok()
    }

    /// The raw seat token shared with this row's machine handle.
    pub(crate) fn seat_token(&self) -> Option<u64> {
        self.request_id().map(RequestId::get)
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
    /// The outer operation's user-movement baseline, inherited by nested rows.
    pub(crate) fn user_move_intent_baseline(&self) -> u64 {
        self.intent_baseline
    }

    /// Whether a user movement intent has taken over this operation.
    pub(crate) fn user_move_intent_interrupted(&self) -> bool {
        user_move_intent_seq() > self.intent_baseline
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
        let called = js.call_for(self.handle, callback, args);
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
    /// Call `hook` (`None`: the family asked for an undeclared hook). A
    /// tick step then drains microtasks so a promise that settles through
    /// them alone comes back settled. A kick is nested in the caller's JS
    /// and must not drain that queue; it polls the callback promise
    /// instead.
    fn call(&mut self, hook: Option<&HeldCallback>, args: &[Value]) -> Called;
    /// Call a row-owned hook. Implementations that need row attribution
    /// override this method; test hosts can keep the context-free default.
    fn call_for(&mut self, _handle: Handle, hook: Option<&HeldCallback>, args: &[Value]) -> Called {
        self.call(hook, args)
    }
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
    begin: fn(Handle, Value, Vec<Hook>, usize) -> Started,
    /// [`Family::SYNC_HOOKS`]. The ABI coverage test reads this field.
    #[allow(dead_code)]
    sync_hooks: &'static [usize],
}

const fn entry<F: Family>() -> Entry {
    Entry {
        name: F::NAME,
        callbacks: F::CALLBACKS,
        kick_on_start: F::KICK_ON_START,
        begin: begin_row::<F>,
        sync_hooks: F::SYNC_HOOKS,
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
    entry::<crate::fight_upkeep::BuryInFight>(),
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
    entry::<crate::duel::Handshake>(),
    entry::<crate::duel::Helper>(),
    entry::<crate::duel::Travel>(),
    entry::<crate::duel::Close>(),
    entry::<crate::api_session::GatherSession>(),
    entry::<crate::api_session::GatherStop>(),
    entry::<crate::api_session::CombatSession>(),
    entry::<crate::api_session::CombatStop>(),
    entry::<crate::api_session::ProgressQuery>(),
    entry::<crate::quest_journal::QuestJournal>(),
    entry::<crate::reach_entity::EntityOp>(),
    entry::<crate::reach_entity::WalkHops>(),
    entry::<crate::walk::WalkResilient>(),
    entry::<crate::walk::WalkOpening>(),
    entry::<crate::walk::DirectWalk>(),
    entry::<crate::walk::DirectClick>(),
    entry::<crate::boat_fare::WalkTo>(),
    entry::<crate::anchor_return::ReturnToAnchor>(),
    entry::<crate::bank_op::BankOp>(),
    entry::<crate::bank_op::BankWithdrawLoad>(),
    entry::<crate::bank_op::BankClose>(),
    entry::<crate::bank_op::BankNoteMode>(),
    entry::<crate::bank_deposit::BankDeposit>(),
    entry::<crate::bank_withdraw::WithdrawTo>(),
    entry::<crate::bank_withdraw::CloseConfirm>(),
    entry::<crate::bank_access::BankAccess>(),
    entry::<crate::bank_access::NpcAccess>(),
    entry::<crate::bank_access::BankOpenAccess>(),
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
    entry::<tests::NotificationOrder>(),
    #[cfg(test)]
    entry::<tests::Inline>(),
    #[cfg(test)]
    entry::<tests::SoloAsker>(),
    #[cfg(test)]
    entry::<tests::Walker>(),
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
            || host.stepping.iter().any(|row| row.family == family)
    })
}

/// Live walking ownership copied from the existing machine rows. The takeover
/// watermark is shared with the host so an in-flight callback cannot republish
/// an already interrupted operation before its queued notice is processed.
#[derive(Default)]
pub(crate) struct WalkingOwnership {
    live: std::sync::atomic::AtomicBool,
    baseline: std::sync::atomic::AtomicU64,
    takeover: std::sync::atomic::AtomicU64,
}

impl WalkingOwnership {
    pub(crate) fn live(&self) -> bool {
        self.live.load(std::sync::atomic::Ordering::Acquire)
            && self.baseline.load(std::sync::atomic::Ordering::Acquire)
                >= self.takeover.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn note_takeover(&self, seq: u64) {
        self.takeover
            .fetch_max(seq, std::sync::atomic::Ordering::AcqRel);
    }
}

pub(crate) fn set_walking_publication(publication: std::sync::Arc<WalkingOwnership>) {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.walking_publication = Some(publication);
        host.publish_walking();
    });
}

/// Guardian hold freezes stepping, not walking ownership. Reconnect-held rows
/// are carried work instead; operator Pause is gated by the host's run state.
#[cfg(test)]
pub(crate) fn walking_live() -> bool {
    HOST.with(|host| host.borrow().walking_baseline().is_some())
}
/// The type-erased row the host steps.
trait Machine {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value>;
    fn snapshot_sensitive(&self) -> bool;
    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Value>;
    fn abort(&mut self, why: AbortReason);
    fn release(&self) -> Option<InteractReq>;
    fn walking_operation(&self) -> bool;
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

    fn snapshot_sensitive(&self) -> bool {
        F::SNAPSHOT_SENSITIVE
    }

    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match Family::snapshot_step(self, cx) {
            Step::Wait => Step::Wait,
            Step::Call(call) => Step::Call(call),
            Step::Done(out) => Step::Done(out.into()),
            Step::Fail(reason) => Step::Fail(reason),
        }
    }

    fn abort(&mut self, why: AbortReason) {
        Family::abort(self, why);
    }

    fn release(&self) -> Option<InteractReq> {
        Family::release(self)
    }

    fn walking_operation(&self) -> bool {
        F::WALKING_OPERATION
    }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapturedLineage {
    pub(crate) lineage: Option<RequestId>,
    pub(crate) mirror_owner: Option<RequestId>,
    pub(crate) dropped: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MachineEventKind {
    Phase(crate::native::NativePhase),
    Done,
    Failed,
    Aborted(api::script_contract::AbortReason),
    ManualWalk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MachineEvent {
    pub(crate) id: RequestId,
    pub(crate) kind: MachineEventKind,
    pub(crate) admitted: bool,
}

#[derive(Clone, Copy, Default)]
struct StartContext {
    parent_row: Option<Handle>,
    seat_parent: Option<RequestId>,
    local_parent: Option<RequestId>,
    parent_revoked: bool,
    history_expired: bool,
}
struct StartContextGuard(Option<StartContext>);
impl Drop for StartContextGuard {
    fn drop(&mut self) {
        START_CONTEXT.with(|slot| slot.set(self.0));
    }
}

fn with_start_context<T>(context: StartContext, f: impl FnOnce() -> T) -> T {
    let previous = START_CONTEXT.with(|slot| slot.replace(Some(context)));
    let _restore = StartContextGuard(previous);
    f()
}

#[derive(Clone)]
pub(crate) enum SeatStartPayload {
    Empty,
    Gather(std::sync::Arc<crate::native::SettingsBag>),
    Combat(std::sync::Arc<crate::api_combat::CombatSessionRequest>),
    QuestProgress(std::sync::Arc<str>),
}

impl SeatStartPayload {
    fn as_ref(&self) -> isolate_fb::SeatStartArgsRef<'_> {
        match self {
            Self::Empty => isolate_fb::SeatStartArgsRef::Empty,
            Self::Gather(settings) => isolate_fb::SeatStartArgsRef::Gather(settings),
            Self::Combat(request) => isolate_fb::SeatStartArgsRef::Combat(request),
            Self::QuestProgress(name) => isolate_fb::SeatStartArgsRef::QuestProgress(name),
        }
    }
}

#[derive(Clone)]
pub(crate) struct PendingSeatStart {
    id: RequestId,
    parent: Option<RequestId>,
    local_parent: Option<RequestId>,
    kind: native::request::NativeKind,
    args: SeatStartPayload,
}

impl PendingSeatStart {
    pub(crate) fn as_ref(
        &self,
        execution: ExecutionId,
    ) -> Result<isolate_fb::SeatStartRef<'_>, String> {
        let stamp = RequestStamp::new(self.id, self.parent, self.local_parent, execution)
            .map_err(|error| error.to_string())?;
        Ok(isolate_fb::SeatStartRef {
            stamp,
            kind: self.kind,
            args: self.args.as_ref(),
        })
    }
}

#[derive(Clone, Copy)]
struct PendingSeatCancel {
    of: RequestId,
    reason: CallbackCancelReason,
}

pub(crate) struct SeatBatchCarry<'a> {
    pub(crate) consumed_terminal_seq: u64,
    pub(crate) terminal_ack_pending: bool,
    pub(crate) starts: Result<Vec<isolate_fb::SeatStartRef<'a>>, String>,
    pub(crate) cancels: Vec<isolate_fb::SeatCancelInput>,
    pub(crate) mirror_revokes: Vec<isolate_fb::SeatMirrorRevoke>,
    pub(crate) machine_events: Vec<MachineEvent>,
}

impl SeatBatchCarry<'_> {
    pub(crate) fn has_work(&self) -> bool {
        self.terminal_ack_pending
            || self.starts.as_ref().is_ok_and(|starts| !starts.is_empty())
            || self.starts.is_err()
            || !self.cancels.is_empty()
            || !self.mirror_revokes.is_empty()
            || !self.machine_events.is_empty()
    }
}

#[derive(Clone, Copy)]
struct SteppingRow {
    family: &'static str,
    handle: Handle,
    walking: bool,
    baseline: u64,
    lineage: CapturedLineage,
    parent_row: Option<Handle>,
    seat_parent: Option<RequestId>,
    native_kind: Option<native::request::NativeKind>,
    admitted: bool,
    phase: native::NativePhase,
    manual_walk_event_sent: bool,
    authority_revoked: bool,
}

impl SteppingRow {
    fn capture(self) -> CapturedLineage {
        if self.native_kind.is_some() {
            CapturedLineage {
                lineage: RequestId::isolate(self.handle).ok(),
                mirror_owner: None,
                dropped: self.authority_revoked,
            }
        } else {
            CapturedLineage {
                dropped: self.authority_revoked,
                ..self.lineage
            }
        }
    }

    fn start_context(self) -> StartContext {
        StartContext {
            parent_row: Some(self.handle),
            seat_parent: if self.native_kind.is_some() {
                RequestId::isolate(self.handle).ok()
            } else {
                self.seat_parent
            },
            local_parent: if self.native_kind.is_none() {
                RequestId::isolate(self.handle).ok()
            } else {
                None
            },
            parent_revoked: self.authority_revoked,
            history_expired: false,
        }
    }
    fn revoked_record(self) -> LocalRevocation {
        let mut context = self.start_context();
        context.parent_revoked = true;
        LocalRevocation {
            handle: self.handle,
            lineage: self.capture(),
            context,
        }
    }
}

#[derive(Clone)]
struct PlacedOp {
    at: usize,
    request: InteractReq,
    lineage: CapturedLineage,
}

#[derive(Clone, Copy)]
struct WalkReleaseReservation {
    id: Handle,
    queued: bool,
}

const MAX_WALK_RELEASES: usize = api::script_contract::MAX_WALK_RELEASES;

#[derive(Clone, Copy)]
struct MirrorRevokeCarry {
    wire: isolate_fb::SeatMirrorRevoke,
}
#[derive(Clone, Copy)]
struct LocalRevocation {
    handle: Handle,
    lineage: CapturedLineage,
    context: StartContext,
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
    /// Last `(tick, evidence sequence)` pass this row consumed.
    last_snapshot: Option<(u64, u64)>,
    /// The settled callback the next step reads.
    reply: Option<Reply>,
    /// The callback promise the row waits on.
    pending: Option<Pending>,
    machine: Box<dyn Machine>,
    /// The nearest seat entry this local row mirrors.
    seat_parent: Option<RequestId>,
    /// The local row that started this row, including across seat cutover.
    parent_row: Option<Handle>,
    /// A native seat owner remains distinct from an in-isolate mirror.
    native_kind: Option<native::request::NativeKind>,
    /// The seat acknowledged this start as live.
    admitted: bool,
    /// A revoked owner retains its token so late pushes fail closed.
    authority_revoked: bool,
    /// Current truthful phase reported for this request.
    phase: native::NativePhase,
    /// ManualWalk publishes a Done terminal before this local row settles.
    manual_walk_event_sent: bool,
    /// The outer walking intent baseline carried through nested rows.
    intent_baseline: u64,
    /// This row belongs to an outer walking operation or owns one.
    carries_walk_baseline: bool,
    /// This family's live row represents a walking operation.
    walking_operation: bool,
    /// Rebaseline only an uncancelled operator-paused walk on resume.
    rebaseline_on_resume: bool,
}
impl Row {
    fn capture(&self) -> CapturedLineage {
        if self.native_kind.is_some() {
            CapturedLineage {
                lineage: RequestId::isolate(self.handle).ok(),
                mirror_owner: None,
                dropped: self.authority_revoked,
            }
        } else {
            CapturedLineage {
                lineage: self.seat_parent,
                mirror_owner: RequestId::isolate(self.handle).ok(),
                dropped: self.authority_revoked,
            }
        }
    }

    fn start_context(&self) -> StartContext {
        StartContext {
            parent_row: Some(self.handle),
            seat_parent: if self.native_kind.is_some() {
                RequestId::isolate(self.handle).ok()
            } else {
                self.seat_parent
            },
            local_parent: if self.native_kind.is_none() {
                RequestId::isolate(self.handle).ok()
            } else {
                None
            },
            parent_revoked: self.authority_revoked,
            history_expired: false,
        }
    }

    fn revoked_record(&self) -> LocalRevocation {
        let mut context = self.start_context();
        context.parent_revoked = true;
        LocalRevocation {
            handle: self.handle,
            lineage: self.capture(),
            context,
        }
    }
    fn stepping_record(&self) -> SteppingRow {
        SteppingRow {
            family: self.family,
            handle: self.handle,
            walking: self.walking_operation,
            baseline: self.intent_baseline,
            lineage: self.capture(),
            parent_row: self.parent_row,
            seat_parent: self.seat_parent,
            native_kind: self.native_kind,
            admitted: self.admitted,
            phase: self.phase,
            manual_walk_event_sent: self.manual_walk_event_sent,
            authority_revoked: self.authority_revoked,
        }
    }
}

struct Host {
    next: Handle,
    rows: Vec<Row>,
    /// Rows a pass is stepping while user callbacks may start children.
    stepping: Vec<SteppingRow>,
    /// The newest running row of each exclusive group.
    newest: Vec<(&'static str, Handle)>,
    settled: Vec<(Handle, Outcome)>,
    ops: Vec<PlacedOp>,
    pending_starts: Vec<PendingSeatStart>,
    pending_cancels: Vec<PendingSeatCancel>,
    mirror_revokes: Vec<MirrorRevokeCarry>,
    machine_events: Vec<MachineEvent>,
    consumed_terminal_seq: u64,
    terminal_ack_pending: bool,
    mirror_revokes_consumed: u64,
    revoked_local: VecDeque<LocalRevocation>,
    revoked_floor: Handle,
    walk_releases: Vec<WalkReleaseReservation>,
    next_mirror_revoke_seq: u64,
    execution: Option<ExecutionId>,
    paused: bool,
    held: bool,
    walking_publication: Option<std::sync::Arc<WalkingOwnership>>,
}

thread_local! {
    static HOST: RefCell<Host> = const { RefCell::new(Host::new()) };
    /// A family `begin` is running: a nested start is refused.
    static IN_BEGIN: Cell<bool> = const { Cell::new(false) };
    /// The push-time local/seat lineage for the next machine start.
    static START_CONTEXT: Cell<Option<StartContext>> = const { Cell::new(None) };
    /// A reconnect holds the session's work (`ResetSession { keep_work }`):
    /// the host carries the held walk, so no row sends a release op.
    static SESSION_HELD: Cell<bool> = const { Cell::new(false) };
    /// A callback was terminated in this pass or kick: drive no further.
    static TERMINATED: Cell<bool> = const { Cell::new(false) };
    /// The active walking-operation lineage for synchronous nested starts.
    static WALK_BASELINE: Cell<Option<u64>> = const { Cell::new(None) };
    /// Latest takeover sequence delivered through the ordered host queue.
    static TAKEOVER_SEQ: Cell<u64> = const { Cell::new(0) };
}

struct WalkBaselineGuard(Option<u64>);

impl Drop for WalkBaselineGuard {
    fn drop(&mut self) {
        WALK_BASELINE.with(|baseline| baseline.set(self.0));
    }
}

fn with_walk_baseline<T>(baseline: Option<u64>, f: impl FnOnce() -> T) -> T {
    let previous = WALK_BASELINE.with(|active| active.replace(baseline));
    let _restore = WalkBaselineGuard(previous);
    f()
}

fn observed_user_move_intent_seq() -> u64 {
    crate::observed::with(|scene| scene.since_login().user_move_intent_seq().unwrap_or(0))
}

pub(crate) fn user_move_intent_seq() -> u64 {
    observed_user_move_intent_seq().max(HOST.with(|host| host.borrow().takeover_seq()))
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
            pending_starts: Vec::new(),
            pending_cancels: Vec::new(),
            mirror_revokes: Vec::new(),
            machine_events: Vec::new(),
            consumed_terminal_seq: 0,
            terminal_ack_pending: false,
            mirror_revokes_consumed: 0,
            revoked_local: VecDeque::new(),
            revoked_floor: 1,
            walk_releases: Vec::new(),
            next_mirror_revoke_seq: 1,
            execution: None,
            paused: false,
            held: false,
            walking_publication: None,
        }
    }

    fn takeover_seq(&self) -> u64 {
        self.walking_publication
            .as_ref()
            .map_or(0, |publication| {
                publication
                    .takeover
                    .load(std::sync::atomic::Ordering::Acquire)
            })
            .max(TAKEOVER_SEQ.with(Cell::get))
    }

    fn walking_baseline(&self) -> Option<u64> {
        if SESSION_HELD.with(Cell::get) {
            return None;
        }
        let takeover = self.takeover_seq();
        self.rows
            .iter()
            .filter(|row| row.walking_operation)
            .map(|row| row.intent_baseline)
            .chain(
                self.stepping
                    .iter()
                    .filter_map(|row| row.walking.then_some(row.baseline)),
            )
            .filter(|baseline| *baseline >= takeover)
            .max()
    }

    fn publish_walking(&self) {
        let Some(publication) = &self.walking_publication else {
            return;
        };
        if let Some(baseline) = self.walking_baseline() {
            publication
                .baseline
                .store(baseline, std::sync::atomic::Ordering::Release);
            publication
                .live
                .store(true, std::sync::atomic::Ordering::Release);
        } else {
            publication
                .live
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    fn fresh_clock(&self) -> InstantTaskClock {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(self.paused, self.held);
        clock
    }

    fn allocate_handle(&mut self) -> Option<Handle> {
        if self.next == 0 || self.next > MAX_MACHINE_HANDLE {
            return None;
        }
        let handle = self.next;
        self.next = handle.checked_add(1)?;
        Some(handle)
    }

    fn start_context_for(&self, parent: Option<Handle>) -> StartContext {
        let Some(handle) = parent else {
            return StartContext::default();
        };
        if let Some(row) = self.rows.iter().find(|row| row.handle == handle) {
            return row.start_context();
        }
        if let Some(row) = self.stepping.iter().find(|row| row.handle == handle) {
            return row.start_context();
        }
        if let Some(revoked) = self
            .revoked_local
            .iter()
            .find(|revoked| revoked.handle == handle)
        {
            return revoked.context;
        }
        if handle < self.revoked_floor {
            return StartContext {
                history_expired: true,
                ..StartContext::default()
            };
        }
        if handle < self.next {
            // Ordinary completion is not a continuation authority.
            StartContext::default()
        } else {
            StartContext {
                parent_revoked: true,
                ..StartContext::default()
            }
        }
    }

    fn capture_push(&self, parent: Option<Handle>) -> CapturedLineage {
        if self.execution.is_none() {
            return CapturedLineage::default();
        }
        let Some(handle) = parent else {
            return CapturedLineage::default();
        };
        if let Some(row) = self.rows.iter().find(|row| row.handle == handle) {
            return row.capture();
        }
        if let Some(row) = self.stepping.iter().find(|row| row.handle == handle) {
            return row.capture();
        }
        if let Some(revoked) = self
            .revoked_local
            .iter()
            .find(|revoked| revoked.handle == handle)
        {
            return CapturedLineage {
                dropped: true,
                ..revoked.lineage
            };
        }
        if handle < self.revoked_floor || handle >= self.next {
            return CapturedLineage {
                dropped: true,
                ..CapturedLineage::default()
            };
        }
        CapturedLineage::default()
    }

    fn remember_revoked(&mut self, row: &Row) {
        if row.handle < self.revoked_floor
            || self
                .revoked_local
                .iter()
                .any(|revoked| revoked.handle == row.handle)
        {
            return;
        }
        if self.revoked_local.len() == api::script_contract::REVOKED_RING {
            if let Some(oldest) = self.revoked_local.pop_front() {
                self.revoked_floor = self.revoked_floor.max(oldest.handle.saturating_add(1));
            }
        }
        self.revoked_local.push_back(row.revoked_record());
    }

    fn queue_machine_event(&mut self, id: RequestId, kind: MachineEventKind, admitted: bool) {
        if self.execution.is_none() {
            return;
        }
        if matches!(kind, MachineEventKind::Phase(_)) {
            if let Some(event) = self
                .machine_events
                .iter_mut()
                .find(|event| event.id == id && matches!(event.kind, MachineEventKind::Phase(_)))
            {
                event.kind = kind;
                event.admitted |= admitted;
                return;
            }
        } else {
            self.machine_events.retain(|event| {
                event.id != id || !matches!(event.kind, MachineEventKind::Phase(_))
            });
            if let Some(event) = self
                .machine_events
                .iter_mut()
                .find(|event| event.id == id && event.kind == kind)
            {
                event.admitted |= admitted;
                return;
            }
        }
        self.machine_events
            .push(MachineEvent { id, kind, admitted });
    }

    fn set_phase(&mut self, row: &mut Row, phase: native::NativePhase) {
        if row.phase == phase {
            return;
        }
        row.phase = phase;
        if let Some(id) = RequestId::isolate(row.handle)
            .ok()
            .filter(|_| row.native_kind.is_some())
        {
            self.queue_machine_event(id, MachineEventKind::Phase(phase), row.admitted);
        }
    }

    fn carry_typed_start(&mut self, request: &InteractReq) -> bool {
        if self.execution.is_none() {
            return false;
        }
        let (raw_id, args) = match request {
            InteractReq::GatherRun {
                request_id,
                settings,
            } => (
                *request_id,
                SeatStartPayload::Gather(std::sync::Arc::clone(settings)),
            ),
            InteractReq::CombatFight {
                request_id,
                request,
            } => (
                *request_id,
                SeatStartPayload::Combat(std::sync::Arc::clone(request)),
            ),
            InteractReq::ProgressRead { request_id, name } => (
                *request_id,
                SeatStartPayload::QuestProgress(std::sync::Arc::from(name.as_str())),
            ),
            _ => return false,
        };
        if let Ok(id) = RequestId::isolate(raw_id) {
            if let Some(start) = self.pending_starts.iter_mut().find(|start| start.id == id) {
                start.args = args;
            }
        }
        true
    }

    fn place(&mut self, at: usize, ops: Vec<InteractReq>, lineage: CapturedLineage) {
        for request in ops {
            if self.carry_typed_start(&request) {
                continue;
            }
            if let InteractReq::AbortWalk { request_id } = &request {
                if *request_id > 0 {
                    if let Some(slot) = self
                        .walk_releases
                        .iter_mut()
                        .find(|slot| slot.id == *request_id)
                    {
                        if slot.queued {
                            continue;
                        }
                        slot.queued = true;
                    } else if self.walk_releases.len() < MAX_WALK_RELEASES {
                        self.walk_releases.push(WalkReleaseReservation {
                            id: *request_id,
                            queued: true,
                        });
                    }
                }
            }
            self.ops.push(PlacedOp {
                at,
                request,
                lineage,
            });
        }
    }

    fn reserve_walk_release(&mut self, id: Handle) -> bool {
        if self.walk_releases.iter().any(|slot| slot.id == id) {
            return true;
        }
        if self.walk_releases.len() >= MAX_WALK_RELEASES {
            return false;
        }
        self.walk_releases
            .push(WalkReleaseReservation { id, queued: false });
        true
    }

    fn free_completed_walk(&mut self, id: Handle) {
        if self
            .walk_releases
            .iter()
            .any(|slot| slot.id == id && slot.queued)
        {
            return;
        }
        self.walk_releases.retain(|slot| slot.id != id);
    }

    fn mark_walk_releases_encoded(&mut self, ids: &[Handle]) {
        self.walk_releases
            .retain(|slot| !slot.queued || !ids.contains(&slot.id));
    }

    fn queue_cancel(&mut self, of: RequestId, reason: CallbackCancelReason) {
        if self.execution.is_none() {
            return;
        }
        if let Some(existing) = self
            .pending_cancels
            .iter_mut()
            .find(|cancel| cancel.of == of)
        {
            if reason == CallbackCancelReason::ParentAborted {
                existing.reason = reason;
            }
            return;
        }
        self.pending_cancels.push(PendingSeatCancel { of, reason });
    }

    fn queue_mirror_revoke(&mut self, owner: RequestId) {
        if self.execution.is_none()
            || self
                .mirror_revokes
                .iter()
                .any(|revoke| revoke.wire.owner == owner)
        {
            return;
        }
        let seq = self.next_mirror_revoke_seq;
        let Some(next) = seq.checked_add(1) else {
            return;
        };
        self.next_mirror_revoke_seq = next;
        self.mirror_revokes.push(MirrorRevokeCarry {
            wire: isolate_fb::SeatMirrorRevoke { owner, seq },
        });
    }

    fn unstep(&mut self, family: &'static str, handle: Handle) {
        self.stepping
            .retain(|row| !(row.family == family && row.handle == handle));
        self.publish_walking();
    }
    fn is_revoked(&self, handle: Handle) -> bool {
        self.revoked_local
            .iter()
            .any(|revoked| revoked.handle == handle)
    }

    fn revoke_descendants(&mut self, root: Handle, at: usize) {
        let mut frontier = vec![root];
        let mut descendants = Vec::new();
        let mut index = 0;
        while index < frontier.len() {
            let parent = frontier[index];
            for row in &self.rows {
                if row.parent_row == Some(parent) && !descendants.contains(&row.handle) {
                    descendants.push(row.handle);
                    frontier.push(row.handle);
                }
            }
            index += 1;
        }

        while let Some(handle) = descendants
            .iter()
            .copied()
            .filter(|handle| {
                !self.rows.iter().any(|child| {
                    child.parent_row == Some(*handle) && descendants.contains(&child.handle)
                })
            })
            .min()
        {
            descendants.retain(|candidate| *candidate != handle);
            let Some(position) = self.rows.iter().position(|row| row.handle == handle) else {
                continue;
            };
            let mut row = self.rows.remove(position);
            self.remember_revoked(&row);
            if let Some(id) = RequestId::isolate(row.handle).ok() {
                if row.native_kind.is_some() {
                    self.queue_cancel(id, CallbackCancelReason::ParentAborted);
                } else {
                    self.queue_mirror_revoke(id);
                }
            }
            if let Some(op) = release_op(&*row.machine) {
                self.place(at, vec![op], row.capture());
            }
            row.machine.abort(AbortReason::ParentAborted);
            self.free_completed_walk(row.handle);
            self.settled
                .push((row.handle, Outcome::Aborted(AbortReason::ParentAborted)));
        }
    }

    fn revoke_row(&mut self, row: &Row, reason: AbortReason, at: usize) {
        self.remember_revoked(row);
        if let Some(id) = RequestId::isolate(row.handle).ok() {
            if row.native_kind.is_some() {
                if !row.admitted || reason == AbortReason::ParentAborted {
                    self.queue_cancel(id, CallbackCancelReason::ParentAborted);
                } else if let Some(reason) = match reason {
                    AbortReason::Superseded => Some(api::script_contract::AbortReason::Superseded),
                    AbortReason::UserInput => Some(api::script_contract::AbortReason::UserInput),
                    AbortReason::Reset | AbortReason::Terminated | AbortReason::Unknown => None,
                    AbortReason::ParentAborted => None,
                } {
                    self.queue_machine_event(id, MachineEventKind::Aborted(reason), row.admitted);
                }
            } else {
                self.queue_mirror_revoke(id);
            }
        }
        self.revoke_descendants(row.handle, at);
    }

    fn finish_row(&mut self, row: &Row, outcome: Outcome, at: usize) {
        match &outcome {
            Outcome::Done(_) if row.manual_walk_event_sent => {
                self.remember_revoked(row);
                if let Some(id) = RequestId::isolate(row.handle).ok() {
                    if row.native_kind.is_some() {
                        self.queue_machine_event(id, MachineEventKind::ManualWalk, row.admitted);
                    } else {
                        self.queue_mirror_revoke(id);
                    }
                }
                self.revoke_descendants(row.handle, at);
            }
            Outcome::Done(_) => {
                if let Some(id) = RequestId::isolate(row.handle)
                    .ok()
                    .filter(|_| row.native_kind.is_some())
                {
                    self.queue_machine_event(id, MachineEventKind::Done, row.admitted);
                }
            }
            Outcome::Failed(_) => {
                if let Some(id) = RequestId::isolate(row.handle)
                    .ok()
                    .filter(|_| row.native_kind.is_some())
                {
                    self.queue_machine_event(id, MachineEventKind::Failed, row.admitted);
                }
            }
            Outcome::Aborted(reason) => self.revoke_row(row, *reason, at),
            Outcome::Refused(_) => {}
        }
        self.free_completed_walk(row.handle);
        self.settled.push((row.handle, outcome));
    }

    fn superseded(&self, row: &Row) -> bool {
        row.exclusive.is_some_and(|group| {
            self.newest
                .iter()
                .any(|(newest, handle)| *newest == group && *handle != row.handle)
        })
    }

    /// Abort every superseded row; its release operation stays at `at`.
    fn abort_superseded(&mut self, at: usize) {
        let mut i = 0;
        while i < self.rows.len() {
            if self.superseded(&self.rows[i]) {
                let mut row = self.rows.remove(i);
                if let Some(op) = release_op(&*row.machine) {
                    self.place(at, vec![op], row.capture());
                }
                self.revoke_row(&row, AbortReason::Superseded, at);
                row.machine.abort(AbortReason::Superseded);
                self.free_completed_walk(row.handle);
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
        self.publish_walking();
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
        self.pending_starts.clear();
        self.pending_cancels.clear();
        self.mirror_revokes.clear();
        self.machine_events.clear();
        self.consumed_terminal_seq = 0;
        self.terminal_ack_pending = false;
        self.mirror_revokes_consumed = 0;
        self.publish_walking();
    }

    fn stop(&mut self) {
        self.rows.clear();
        self.newest.clear();
        self.stepping.clear();
        self.settled.clear();
        self.ops.clear();
        self.publish_walking();
        self.walking_publication = None;
    }
}

fn seat_phase_from_wire(phase: isolate_fb::SeatPhase) -> Result<native::NativePhase, String> {
    match phase {
        isolate_fb::SeatPhase::Preparing => Ok(native::NativePhase::Preparing),
        isolate_fb::SeatPhase::Working => Ok(native::NativePhase::Working),
        isolate_fb::SeatPhase::Waiting => Ok(native::NativePhase::Waiting),
        isolate_fb::SeatPhase::Blocked => Ok(native::NativePhase::Blocked),
        isolate_fb::SeatPhase::Complete => Ok(native::NativePhase::Complete),
        isolate_fb::SeatPhase::None => Err("seat live row has no phase".into()),
        _ => Err("seat live row has unknown phase enum".into()),
    }
}

fn seat_result_outcome(
    row: isolate_fb::SeatResult<'_>,
) -> Result<(RequestId, u64, Outcome), String> {
    let id = RequestId::from_raw(row.id())
        .ok_or_else(|| format!("seat result has invalid identity {}", row.id()))?;
    let outcome = row
        .outcome()
        .ok_or_else(|| format!("seat result {} has no outcome", id.get()))?;
    let value = outcome.value();
    let result = match outcome.kind() {
        isolate_fb::SeatOutcomeKind::Done => {
            let value = value
                .map(|bytes| serde_json::from_slice(bytes.bytes()))
                .transpose()
                .map_err(|error| format!("seat result {} value: {error}", id.get()))?
                .unwrap_or(Value::Null);
            Outcome::Done(value)
        }
        isolate_fb::SeatOutcomeKind::Failed => {
            let message = value
                .map(|bytes| serde_json::from_slice::<Value>(bytes.bytes()))
                .transpose()
                .map_err(|error| format!("seat result {} failure value: {error}", id.get()))?
                .map(|value| match value {
                    Value::String(message) => message,
                    value => value.to_string(),
                })
                .unwrap_or_else(|| "native seat failed".into());
            Outcome::Failed(Thrown::new(message))
        }
        isolate_fb::SeatOutcomeKind::Refused => {
            let reason = match outcome.refusal() {
                isolate_fb::SeatRefusal::Busy => "busy",
                isolate_fb::SeatRefusal::ParentBusy => "parent-busy",
                isolate_fb::SeatRefusal::ParentNotLive => "parent-not-live",
                isolate_fb::SeatRefusal::LineageDepth => "lineage-depth",
                isolate_fb::SeatRefusal::HistoryExpired => "history-expired",
                isolate_fb::SeatRefusal::Unknown => "unknown",
                isolate_fb::SeatRefusal::AwaitingDecision => "awaiting-decision",
                isolate_fb::SeatRefusal::None | isolate_fb::SeatRefusal::ArgsTooLarge => {
                    return Err(format!("seat result {} has invalid refusal", id.get()));
                }
                _ => return Err(format!("seat result {} has unknown refusal", id.get())),
            };
            Outcome::Refused(reason.into())
        }
        isolate_fb::SeatOutcomeKind::Aborted => {
            let reason = match outcome.abort_reason() {
                isolate_fb::SeatAbortReason::Superseded => AbortReason::Superseded,
                isolate_fb::SeatAbortReason::UserInput => AbortReason::UserInput,
                isolate_fb::SeatAbortReason::ParentAborted => AbortReason::ParentAborted,
                isolate_fb::SeatAbortReason::None
                | isolate_fb::SeatAbortReason::Reset
                | isolate_fb::SeatAbortReason::Terminated => {
                    return Err(format!("seat result {} has invalid abort reason", id.get()));
                }
                _ => return Err(format!("seat result {} has unknown abort reason", id.get())),
            };
            Outcome::Aborted(reason)
        }
        isolate_fb::SeatOutcomeKind::None => {
            return Err(format!("seat result {} has no outcome kind", id.get()));
        }
        _ => return Err(format!("seat result {} has unknown outcome kind", id.get())),
    };
    Ok((id, row.terminal_seq(), result))
}

fn settle_seat_result(host: &mut Host, id: RequestId, outcome: Outcome) {
    let Ok(handle) = RequestId::isolate(id.get()) else {
        return;
    };
    let handle = handle.get();
    let Some(position) = host
        .rows
        .iter()
        .position(|row| row.handle == handle && row.native_kind.is_some())
    else {
        return;
    };
    let mut row = host.rows.remove(position);
    let abort_reason = match &outcome {
        Outcome::Aborted(reason) => Some(*reason),
        _ => None,
    };
    if abort_reason.is_some() || matches!(&outcome, Outcome::Refused(_)) {
        row.authority_revoked = true;
        host.remember_revoked(&row);
        host.revoke_descendants(row.handle, 0);
    }
    let cleanup_reason = abort_reason.unwrap_or(AbortReason::Reset);
    row.machine.abort(cleanup_reason);
    host.ops
        .retain(|op| op.lineage.lineage != Some(id) && op.lineage.mirror_owner != Some(id));
    host.free_completed_walk(row.handle);
    host.settled.push((row.handle, outcome));
}

pub(crate) fn set_execution(execution: Option<ExecutionId>) {
    HOST.with(|host| host.borrow_mut().execution = execution);
}

pub(crate) fn has_pending_seat_batch() -> bool {
    HOST.with(|host| {
        let host = host.borrow();
        host.execution.is_some()
            && (!host.pending_starts.is_empty()
                || !host.pending_cancels.is_empty()
                || !host.mirror_revokes.is_empty()
                || host.machine_events.iter().any(|event| event.admitted)
                || host.terminal_ack_pending)
    })
}

pub(crate) fn with_seat_batch<R>(f: impl FnOnce(Option<SeatBatchCarry<'_>>) -> R) -> R {
    HOST.with(|host| {
        let host = host.borrow();
        let Some(execution) = host.execution else {
            return f(None);
        };
        let starts = host
            .pending_starts
            .iter()
            .map(|start| start.as_ref(execution))
            .collect();
        let cancels = host
            .pending_cancels
            .iter()
            .map(|cancel| isolate_fb::SeatCancelInput {
                of: cancel.of,
                reason: cancel.reason,
            })
            .collect();
        let mirror_revokes = host
            .mirror_revokes
            .iter()
            .map(|revoke| revoke.wire)
            .collect();
        f(Some(SeatBatchCarry {
            consumed_terminal_seq: host.consumed_terminal_seq,
            terminal_ack_pending: host.terminal_ack_pending,
            starts,
            cancels,
            mirror_revokes,
            machine_events: machine_events_for_batch(&host),
        }))
    })
}

fn machine_events_for_batch(host: &Host) -> Vec<MachineEvent> {
    host.machine_events
        .iter()
        .filter(|event| event.admitted)
        .copied()
        .collect()
}

fn reconcile_machine_events(
    host: &mut Host,
    live_rows: &[(RequestId, native::NativePhase)],
    result_ids: &[RequestId],
) {
    for event in &mut host.machine_events {
        if live_rows.iter().any(|(id, _)| *id == event.id) {
            event.admitted = true;
        }
    }
    host.machine_events.retain(|event| {
        !result_ids.contains(&event.id)
            && !matches!(
                event.kind,
                MachineEventKind::Phase(phase)
                    if live_rows.iter().any(|(id, live_phase)| {
                        *id == event.id && *live_phase == phase
                    })
            )
    });
}
pub(crate) fn on_snapshot(snapshot: &isolate_fb::Snapshot<'_>) -> Result<(), String> {
    let live = snapshot.seat_live();
    let results = snapshot.seat_results();
    let mut live_rows = Vec::with_capacity(live.as_ref().map_or(0, |rows| rows.len()));
    if let Some(rows) = live.as_ref() {
        for row in rows.iter() {
            let id = RequestId::from_raw(row.id())
                .ok_or_else(|| format!("seat live row has invalid identity {}", row.id()))?;
            live_rows.push((id, seat_phase_from_wire(row.phase())?));
        }
    }
    let has_results_page = results.is_some();
    let mut result_rows = Vec::with_capacity(results.as_ref().map_or(0, |rows| rows.len()));
    if let Some(rows) = results.as_ref() {
        for row in rows.iter() {
            result_rows.push(seat_result_outcome(row)?);
        }
    }
    let mirror_revokes_consumed = snapshot.mirror_revokes_consumed();
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for (id, phase) in &live_rows {
            if let Some(row) = host.rows.iter_mut().find(|row| {
                row.native_kind.is_some() && RequestId::isolate(row.handle).ok() == Some(*id)
            }) {
                row.admitted = true;
                row.phase = *phase;
            }
        }
        let result_ids: Vec<_> = result_rows.iter().map(|(id, _, _)| *id).collect();
        host.pending_starts.retain(|start| {
            !live_rows.iter().any(|(id, _)| *id == start.id) && !result_ids.contains(&start.id)
        });
        host.pending_cancels
            .retain(|cancel| !result_ids.contains(&cancel.of));
        reconcile_machine_events(&mut host, &live_rows, &result_ids);
        if let Some(consumed) = mirror_revokes_consumed {
            host.mirror_revokes_consumed = host.mirror_revokes_consumed.max(consumed);
            host.mirror_revokes
                .retain(|revoke| revoke.wire.seq > consumed);
        }
        if has_results_page {
            for (_, seq, _) in &result_rows {
                host.consumed_terminal_seq = host.consumed_terminal_seq.max(*seq);
            }
            host.terminal_ack_pending = result_rows
                .iter()
                .any(|(_, seq, _)| *seq <= host.consumed_terminal_seq);
        }
        for (id, _, outcome) in result_rows {
            settle_seat_result(&mut host, id, outcome);
        }
    });
    Ok(())
}

pub(crate) fn capture_push(parent: Option<Handle>) -> CapturedLineage {
    HOST.with(|host| host.borrow().capture_push(parent))
}

fn native_kind_for_family(family: &str) -> Option<native::request::NativeKind> {
    use native::request::NativeKind;
    Some(match family {
        "gather" => NativeKind::Gather,
        "combat" => NativeKind::Combat,
        "quest-progress" => NativeKind::QuestProgress,
        "hunt-fight" | "hunt-hold" | "hunt-retreat" | "hunt-walkspot" | "hunt-enter" => {
            NativeKind::Hunt
        }
        "bank_nearest" => NativeKind::BankNearest,
        "periodic_bank" => NativeKind::PeriodicBank,
        _ => return None,
    })
}

fn seat_start_payload(
    kind: native::request::NativeKind,
    id: RequestId,
    ops: &mut Vec<InteractReq>,
) -> Result<SeatStartPayload, String> {
    let position = ops.iter().position(|request| match (kind, request) {
        (native::request::NativeKind::Gather, InteractReq::GatherRun { request_id, .. })
        | (native::request::NativeKind::Combat, InteractReq::CombatFight { request_id, .. })
        | (
            native::request::NativeKind::QuestProgress,
            InteractReq::ProgressRead { request_id, .. },
        ) => *request_id == id.get(),
        _ => false,
    });
    match kind {
        native::request::NativeKind::Gather => {
            let Some(position) = position else {
                return Err("native Gather start is missing its typed settings".into());
            };
            let InteractReq::GatherRun { settings, .. } = ops.remove(position) else {
                unreachable!("matched GatherRun")
            };
            Ok(SeatStartPayload::Gather(settings))
        }
        native::request::NativeKind::Combat => {
            let Some(position) = position else {
                return Err("native Combat start is missing its typed request".into());
            };
            let InteractReq::CombatFight { request, .. } = ops.remove(position) else {
                unreachable!("matched CombatFight")
            };
            Ok(SeatStartPayload::Combat(request))
        }
        native::request::NativeKind::QuestProgress => {
            let Some(position) = position else {
                return Err("native QuestProgress start is missing its quest name".into());
            };
            let InteractReq::ProgressRead { name, .. } = ops.remove(position) else {
                unreachable!("matched ProgressRead")
            };
            Ok(SeatStartPayload::QuestProgress(std::sync::Arc::from(name)))
        }
        _ => Ok(SeatStartPayload::Empty),
    }
}

fn begin_row<F: Family>(handle: Handle, args: Value, hooks: Vec<Hook>, at: usize) -> Started {
    let args: F::Args = match serde_json::from_value(args) {
        Ok(args) => args,
        Err(e) => return Started::Refused(format!("{} arguments: {e}", F::NAME)),
    };
    let context = START_CONTEXT.with(Cell::get).unwrap_or_default();
    let inherited_baseline = WALK_BASELINE.with(Cell::get);
    let intent_baseline = inherited_baseline.unwrap_or_else(observed_user_move_intent_seq);
    let carries_walk_baseline = inherited_baseline.is_some() || F::WALKING_OPERATION;
    let mut clock = HOST.with(|host| host.borrow().fresh_clock());
    let mut ops = Vec::new();
    IN_BEGIN.with(|flag| flag.set(true));
    let _begun = BeginGuard;
    let begun = with_walk_baseline(carries_walk_baseline.then_some(intent_baseline), || {
        F::begin(
            args,
            &mut Cx {
                ops: &mut ops,
                clock: &mut clock,
                reply: None,
                hooks: &hooks,
                handle,
                js: None,
                intent_baseline,
                asks: 0,
                ending: None,
            },
        )
    });
    drop(_begun);
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match begun {
            Begin::Refuse(reason) => Started::Refused(reason),
            Begin::Done(out) => {
                host.place(
                    at,
                    ops,
                    CapturedLineage {
                        lineage: context.seat_parent,
                        mirror_owner: context.local_parent,
                        dropped: false,
                    },
                );
                Started::Settled(Outcome::Done(out.into()))
            }
            Begin::Run(mut machine) => {
                let native_kind = host
                    .execution
                    .is_some()
                    .then(|| native_kind_for_family(F::NAME))
                    .flatten();
                let owner_id = native_kind.and_then(|_| RequestId::isolate(handle).ok());
                if native_kind.is_some() && host.pending_starts.len() >= MAX_PENDING_SEAT_STARTS {
                    machine.abort(AbortReason::Reset);
                    return Started::Refused("native seat start carry is full".into());
                }
                let lineage = if native_kind.is_some() {
                    let Some(id) = owner_id else {
                        machine.abort(AbortReason::Reset);
                        return Started::Refused("native seat request identity exhausted".into());
                    };
                    let payload =
                        match seat_start_payload(native_kind.expect("checked"), id, &mut ops) {
                            Ok(payload) => payload,
                            Err(reason) => {
                                machine.abort(AbortReason::Reset);
                                return Started::Refused(reason);
                            }
                        };
                    host.pending_starts.push(PendingSeatStart {
                        id,
                        parent: context.seat_parent,
                        local_parent: context.local_parent,
                        kind: native_kind.expect("checked"),
                        args: payload,
                    });
                    CapturedLineage {
                        lineage: Some(id),
                        mirror_owner: None,
                        dropped: false,
                    }
                } else {
                    CapturedLineage {
                        lineage: context.seat_parent,
                        mirror_owner: context.local_parent,
                        dropped: false,
                    }
                };
                if F::EXCLUSIVE {
                    host.newest
                        .retain(|(group, _)| *group != F::EXCLUSIVE_GROUP);
                    host.newest.push((F::EXCLUSIVE_GROUP, handle));
                    host.abort_superseded(at);
                }
                host.place(at, ops, lineage);
                let walking_operation = machine.walking_operation();
                host.rows.push(Row {
                    handle,
                    family: F::NAME,
                    exclusive: F::EXCLUSIVE.then_some(F::EXCLUSIVE_GROUP),
                    sync_hooks: F::SYNC_HOOKS,
                    clock,
                    hooks,
                    calls: 0,
                    last_snapshot: None,
                    reply: None,
                    pending: None,
                    machine: Box::new(machine),
                    seat_parent: context.seat_parent,
                    parent_row: context.parent_row,
                    native_kind,
                    admitted: native_kind.is_none(),
                    authority_revoked: false,
                    phase: native::NativePhase::Preparing,
                    manual_walk_event_sent: false,
                    intent_baseline,
                    carries_walk_baseline,
                    walking_operation,
                    rebaseline_on_resume: false,
                });
                host.publish_walking();
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
    let parent = START_CONTEXT
        .with(Cell::get)
        .and_then(|context| context.parent_row);
    start_with_parent(family, args, hooks, at, parent)
}

pub(crate) fn start_with_parent(
    family: &str,
    args: Value,
    hooks: Vec<Hook>,
    at: usize,
    parent: Option<Handle>,
) -> Started {
    if IN_BEGIN.with(Cell::get) {
        return Started::Refused("a machine begin may not start a machine".into());
    }
    let Some(entry) = FAMILIES.iter().find(|entry| entry.name == family) else {
        return Started::Refused(format!("unknown machine family {family:?}"));
    };
    let context = HOST.with(|host| host.borrow().start_context_for(parent));
    if context.history_expired {
        return Started::Refused("history expired".into());
    }
    if context.parent_revoked {
        return Started::Refused("parent not live".into());
    }
    let needs_seat = HOST.with(|host| {
        let host = host.borrow();
        host.execution.is_some() && native_kind_for_family(entry.name).is_some()
    });
    if needs_seat && HOST.with(|host| host.borrow().pending_starts.len() >= MAX_PENDING_SEAT_STARTS)
    {
        return Started::Refused("native seat start carry is full".into());
    }
    let Some(handle) = HOST.with(|host| host.borrow_mut().allocate_handle()) else {
        return Started::Refused("machine handle exhausted".into());
    };
    if needs_seat && RequestId::isolate(handle).is_err() {
        return Started::Refused("native seat request identity exhausted".into());
    }
    with_start_context(context, || (entry.begin)(handle, args, hooks, at))
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
            .map(|index| host.rows.remove(index))
    }) else {
        return;
    };
    if row.native_kind.is_some() && !row.admitted {
        HOST.with(|host| host.borrow_mut().rows.push(row));
        return;
    }
    let stepping = row.stepping_record();
    HOST.with(|host| host.borrow_mut().stepping.push(stepping));
    let outcome = drive(&mut row, js, &mut at);
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match outcome {
            // Termination unwinds runMachine before it can park this result.
            Some(Outcome::Aborted(AbortReason::Terminated)) => {}
            Some(outcome) => host.finish_row(&row, outcome, at),
            None => host.rows.push(row),
        }
        host.unstep(stepping.family, stepping.handle);
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

/// Recheck opted-in rows against one changed snapshot without starting a
/// new tick's callback or per-family event budget.
pub(crate) fn snapshot_step(js: &mut impl Js, tick: u64, evidence_sequence: u64) {
    pass(
        js,
        Pass::Snapshot {
            tick,
            evidence_sequence,
        },
    );
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
    /// Opted-in rows, once per distinct changed snapshot; carries the budget.
    Snapshot { tick: u64, evidence_sequence: u64 },
}

/// Rows are taken out of the host while they step, so a callback may
/// start another machine; the host is borrowed only between JS calls.
/// [`Host::stepping`] keeps those rows visible to [`live`].
fn pass(js: &mut impl Js, pass: Pass) {
    let entered_scene = crate::observed::with(crate::observed::Scene::entered_scene);
    let Some(mut rows) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.paused || host.held {
            return None;
        }
        let rows = std::mem::take(&mut host.rows);
        host.stepping = rows.iter().map(Row::stepping_record).collect();
        Some(rows)
    }) else {
        return;
    };
    TERMINATED.with(|flag| flag.set(false));
    // Script code may have queued rows before this pass (a probe, the
    // recovery anchor, this tick's own JS); step ops land after them.
    let mut at = js.queue_len();
    rows.retain_mut(|row| {
        if row.native_kind.is_some() && !row.admitted {
            return true;
        }
        match pass {
            Pass::Step => row.calls = 0,
            Pass::Resume if row.pending.is_none() => return true,
            Pass::Resume => {}
            Pass::Snapshot {
                tick,
                evidence_sequence,
            } => {
                if !row.machine.snapshot_sensitive() || row.pending.is_some() {
                    return true;
                }
                let snapshot = (tick, evidence_sequence);
                if row.last_snapshot == Some(snapshot) {
                    return true;
                }
                row.last_snapshot = Some(snapshot);
            }
        }
        if HOST.with(|host| host.borrow().superseded(row)) {
            if let Some(op) = release_op(&*row.machine) {
                HOST.with(|host| host.borrow_mut().place(at, vec![op], row.capture()));
            }
            row.machine.abort(AbortReason::Superseded);
            HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
            settle(row, Outcome::Aborted(AbortReason::Superseded), at);
            return false;
        }
        if HOST.with(|host| host.borrow().is_revoked(row.handle)) {
            row.machine.abort(AbortReason::ParentAborted);
            HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
            settle(row, Outcome::Aborted(AbortReason::ParentAborted), at);
            return false;
        }
        // PLAYER_INFO can expose the new level before that scene's loc
        // changes. Opted-in rows must keep that fence on every pass, including
        // promise resumes and receipt re-observes, without delaying revocation.
        if entered_scene && row.machine.snapshot_sensitive() {
            return true;
        }
        let outcome = match pass {
            Pass::Snapshot { .. } => drive_snapshot(row, js, &mut at),
            Pass::Step | Pass::Resume => drive(row, js, &mut at),
        };
        match outcome {
            Some(outcome) => {
                HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
                settle(row, outcome, at);
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
        host.abort_superseded(at);
        host.publish_walking();
    });
}

fn settle(row: &Row, outcome: Outcome, at: usize) {
    HOST.with(|host| host.borrow_mut().finish_row(row, outcome, at));
}

/// One row's steps for this pass; `Some` when it ended. Stops, leaving
/// the row as it is, once join has claimed the tick.
fn drive(row: &mut Row, js: &mut impl Js, at: &mut usize) -> Option<Outcome> {
    drive_in_pass(row, js, at, false)
}

fn drive_snapshot(row: &mut Row, js: &mut impl Js, at: &mut usize) -> Option<Outcome> {
    drive_in_pass(row, js, at, true)
}

fn drive_in_pass(
    row: &mut Row,
    js: &mut impl Js,
    at: &mut usize,
    snapshot: bool,
) -> Option<Outcome> {
    if row.rebaseline_on_resume {
        // Pause and reconnect holds produce no runnable observations. Refresh
        // on the first eligible resumed step, after its Running snapshot.
        // An actual post-hold takeover (including its early atomic notice)
        // must never be absorbed into the carried operation's baseline.
        if !SESSION_HELD.with(Cell::get)
            && HOST.with(|host| host.borrow().takeover_seq()) <= row.intent_baseline
        {
            row.intent_baseline = observed_user_move_intent_seq();
        }
        row.rebaseline_on_resume = false;
    }
    let baseline = row.carries_walk_baseline.then_some(row.intent_baseline);
    with_walk_baseline(baseline, || drive_inner(row, js, at, snapshot))
}

fn drive_inner(row: &mut Row, js: &mut impl Js, at: &mut usize, snapshot: bool) -> Option<Outcome> {
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
            handle: row.handle,
            js: Some(&mut *js),
            asks: 0,
            intent_baseline: row.intent_baseline,
            ending: None,
        };
        // Callback continuations belong to the same pass: a snapshot-only
        // pass must not fall back to the normal tick step after a JS call.
        let step = if snapshot {
            row.machine.snapshot_step(&mut cx)
        } else {
            row.machine.step(&mut cx)
        };
        let asked = cx.asks > 0;
        // An ask that failed ends the row whatever the step returned; the
        // ops of that step are dropped with it.
        // Every ending stops the host work the row armed.
        match cx.ending.take() {
            Some(Ending::Stopped) => {
                if let Some(op) = release_op(&*row.machine) {
                    HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
                }
                row.machine.abort(AbortReason::Terminated);
                return Some(Outcome::Aborted(AbortReason::Terminated));
            }
            Some(Ending::Failed(thrown)) => {
                if let Some(op) = release_op(&*row.machine) {
                    HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
                }
                return Some(Outcome::Failed(thrown));
            }
            None => {}
        }
        // An ask started a newer row of this exclusive family: that row
        // owns the tick, so this step's decision and ops are dropped.
        if asked && HOST.with(|host| host.borrow().superseded(row)) {
            if let Some(op) = release_op(&*row.machine) {
                HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
            }
            row.machine.abort(AbortReason::Superseded);
            return Some(Outcome::Aborted(AbortReason::Superseded));
        }
        HOST.with(|host| host.borrow_mut().place(*at, ops, row.capture()));
        match step {
            Step::Wait => return None,
            Step::Done(value) => return Some(Outcome::Done(value)),
            Step::Fail(thrown) => {
                if let Some(op) = release_op(&*row.machine) {
                    HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
                }
                return Some(Outcome::Failed(thrown));
            }
            Step::Call(call) => {
                let hook = row.hooks.get(call.hook).map(|hook| &hook.callback);
                let called = js.call_for(row.handle, hook, &call.args);
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
    ops.sort_by_key(|op| op.at);
    let mut out = Vec::with_capacity(rows.len() + ops.len());
    let mut ops = ops.into_iter().peekable();
    for (index, row) in rows.into_iter().enumerate() {
        while ops.peek().is_some_and(|op| op.at <= index) {
            out.push(ops.next().expect("peeked placed op").request);
        }
        if let MaybeInteractReq::Req(request) = row {
            out.push(request);
        }
    }
    out.extend(ops.map(|op| op.request));
    out
}

pub(crate) fn merge_ops_with_lineage(
    rows: Vec<MaybeInteractReq>,
    lineages: Vec<CapturedLineage>,
) -> Result<Vec<(InteractReq, CapturedLineage)>, String> {
    if rows.len() != lineages.len() {
        return Err(format!(
            "interact lineage count {} does not match row count {}",
            lineages.len(),
            rows.len()
        ));
    }
    let mut ops = HOST.with(|host| std::mem::take(&mut host.borrow_mut().ops));
    ops.sort_by_key(|op| op.at);
    let mut out = Vec::with_capacity(rows.len() + ops.len());
    let mut ops = ops.into_iter().peekable();
    for (index, (row, lineage)) in rows.into_iter().zip(lineages).enumerate() {
        while ops.peek().is_some_and(|op| op.at <= index) {
            let op = ops.next().expect("peeked placed op");
            if !op.lineage.dropped {
                out.push((op.request, op.lineage));
            }
        }
        if let MaybeInteractReq::Req(request) = row {
            if !lineage.dropped {
                out.push((request, lineage));
            }
        }
    }
    out.extend(
        ops.filter(|op| !op.lineage.dropped)
            .map(|op| (op.request, op.lineage)),
    );
    Ok(out)
}

/// A held tick drops its public actions; machine ops go with them.
pub(crate) fn drop_ops() {
    HOST.with(|host| host.borrow_mut().ops.clear());
}

pub(crate) fn on_pause() {
    let intent = user_move_intent_seq();
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for row in &mut host.rows {
            row.rebaseline_on_resume = row.carries_walk_baseline && intent <= row.intent_baseline;
        }
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

/// Record the queued intent before the host applies the corresponding pause.
pub(crate) fn on_manual_walk_takeover(seq: u64) {
    TAKEOVER_SEQ.with(|watermark| watermark.set(watermark.get().max(seq)));
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for row in &mut host.rows {
            if row.carries_walk_baseline && row.intent_baseline < seq {
                row.rebaseline_on_resume = false;
            }
        }
        host.publish_walking();
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
    SESSION_HELD.with(|flag| flag.set(false));
    TAKEOVER_SEQ.with(|seq| seq.set(0));
    HOST.with(|host| host.borrow_mut().reset());
}

/// A reconnect began (`true`) or the relogged session's first tick ended
/// it (`false`). While held, the connection's queue was reset and the host
/// carry owns the held walk: [`Family::release`] ops are not sent.
pub(crate) fn on_session_hold(held: bool) {
    SESSION_HELD.with(|flag| flag.set(held));
    HOST.with(|host| host.borrow().publish_walking());
}

/// The row's [`Family::release`] op, unless a reconnect holds the session.
fn release_op(machine: &dyn Machine) -> Option<InteractReq> {
    if SESSION_HELD.with(Cell::get) {
        return None;
    }
    machine.release()
}

pub(crate) fn on_stop() {
    SESSION_HELD.with(|flag| flag.set(false));
    TAKEOVER_SEQ.with(|seq| seq.set(0));
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
        const SNAPSHOT_SENSITIVE: bool = true;
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

    /// An exclusive probe whose host work is a walk: its release is the
    /// walk's AbortWalk, request id = its button.
    pub(crate) struct Walker(Probe, u64);

    impl Family for Walker {
        const NAME: &'static str = "walker";
        const EXCLUSIVE: bool = true;
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            let id = u64::try_from(args.button).unwrap_or(0);
            match Probe::begin(args, cx) {
                Begin::Run(probe) => Begin::Run(Self(probe, id)),
                Begin::Done(out) => Begin::Done(out),
                Begin::Refuse(reason) => Begin::Refuse(reason),
            }
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            Family::step(&mut self.0, cx)
        }

        fn release(&self) -> Option<InteractReq> {
            Some(InteractReq::AbortWalk { request_id: self.1 })
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
        const SYNC_HOOKS: &'static [usize] = &[0];
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
        const SNAPSHOT_SENSITIVE: bool = true;
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

    /// Feeds a fixture effect through Hunt's real notification/decision
    /// consumer, without duplicating its notes loop or callback mapping.
    pub(crate) struct NotificationOrder {
        started: bool,
    }

    impl Family for NotificationOrder {
        const NAME: &'static str = "notification-order";
        const CALLBACKS: &'static [&'static str] = crate::hunt::HOOKS;
        const KICK_ON_START: bool = true;
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self { started: false })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if self.started {
                return match cx.reply() {
                    Some(Reply::Value(value)) => Step::Done(value),
                    Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                    None => Step::Wait,
                };
            }
            self.started = true;
            let effect = json!({
                "kind": "eat",
                "notes": [{"kind": "count-kill"}, {"kind": "count-kill"}],
            });
            match crate::hunt::characterize_effect(&effect, cx) {
                Ok(Some(step)) => step,
                Ok(None) | Err(_) => Step::Wait,
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

    #[test]
    fn a_reconnect_hold_sends_no_release_op() {
        on_reset();
        let aborts = |ops: Vec<InteractReq>| -> Vec<u64> {
            ops.into_iter()
                .filter_map(|op| match op {
                    InteractReq::AbortWalk { request_id } => Some(request_id),
                    _ => None,
                })
                .collect()
        };
        running(begin("walker", json!({ "button": 7, "steps": 5 })));
        drain();
        running(begin("walker", json!({ "button": 8, "steps": 5 })));
        assert_eq!(aborts(drain()), vec![7], "a superseded row stops its walk");
        on_session_hold(true);
        running(begin("walker", json!({ "button": 9, "steps": 5 })));
        assert!(
            aborts(drain()).is_empty(),
            "the reconnect's host carry owns the held walk"
        );
        on_session_hold(false);
        on_reset();
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
    fn snapshot_pass_deduplicates_the_same_tick_and_evidence_sequence() {
        let handle = running(begin("probe", json!({ "button": 10, "steps": 3 })));
        merge_ops(Vec::new());
        let mut js = Echo { calls: 0 };
        snapshot_step(&mut js, 1, 1);
        assert_eq!(
            merge_ops(Vec::new()),
            vec![InteractReq::IfButton { component_id: 11 }]
        );
        snapshot_step(&mut js, 1, 1);
        assert!(merge_ops(Vec::new()).is_empty());
        snapshot_step(&mut js, 1, 2);
        assert_eq!(
            merge_ops(Vec::new()),
            vec![InteractReq::IfButton { component_id: 12 }]
        );
        snapshot_step(&mut js, 2, 2);
        assert_eq!(
            merge_ops(Vec::new()),
            vec![InteractReq::IfButton { component_id: 13 }]
        );
        assert_eq!(take(handle), Take::Pending);
    }

    #[test]
    fn snapshot_passes_do_not_replenish_the_callback_budget() {
        let total = CALLS_PER_TICK + 8;
        let handle = running(begin("burst", json!({ "calls": total })));
        let mut js = Echo { calls: 0 };
        step(&mut js);
        for evidence_sequence in 1..=4 {
            snapshot_step(&mut js, 1, evidence_sequence);
        }
        assert_eq!(js.calls, CALLS_PER_TICK);
        assert_eq!(take(handle), Take::Pending);
        step(&mut js);
        assert_eq!(js.calls, total);
        assert!(matches!(take(handle), Take::Settled(Outcome::Done(_))));
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
        let placed = vec![
            PlacedOp {
                at: 0,
                request: button(1),
                lineage: CapturedLineage::default(),
            },
            PlacedOp {
                at: 2,
                request: button(3),
                lineage: CapturedLineage::default(),
            },
            PlacedOp {
                at: 2,
                request: button(4),
                lineage: CapturedLineage::default(),
            },
            PlacedOp {
                at: 9,
                request: button(6),
                lineage: CapturedLineage::default(),
            },
        ];
        HOST.with(|host| host.borrow_mut().ops = placed);
        let skipped = MaybeInteractReq::Skip(crate::shim::RejectedRow("a number".into()));
        let rows = vec![js(2), skipped, js(5)];
        assert_eq!(
            merge_ops(rows),
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

    #[test]
    fn unadmitted_machine_events_wait_for_admission_and_replay_until_ack() {
        let phase_id = RequestId::isolate(1).expect("isolate id");
        let done_id = RequestId::isolate(2).expect("isolate id");
        let failed_id = RequestId::isolate(3).expect("isolate id");
        let manual_walk_id = RequestId::isolate(4).expect("isolate id");
        let phase = MachineEventKind::Phase(native::NativePhase::Working);
        let done = MachineEventKind::Done;
        let failed = MachineEventKind::Failed;
        let manual_walk = MachineEventKind::ManualWalk;
        let mut host = Host::new();
        host.machine_events.extend([
            MachineEvent {
                id: phase_id,
                kind: phase,
                admitted: false,
            },
            MachineEvent {
                id: done_id,
                kind: done,
                admitted: false,
            },
            MachineEvent {
                id: failed_id,
                kind: failed,
                admitted: false,
            },
            MachineEvent {
                id: manual_walk_id,
                kind: manual_walk,
                admitted: false,
            },
        ]);

        // A lost page or deferred start leaves every event in carrier custody.
        reconcile_machine_events(&mut host, &[], &[]);
        assert!(machine_events_for_batch(&host).is_empty());

        reconcile_machine_events(
            &mut host,
            &[
                (phase_id, native::NativePhase::Preparing),
                (done_id, native::NativePhase::Waiting),
                (failed_id, native::NativePhase::Waiting),
                (manual_walk_id, native::NativePhase::Waiting),
            ],
            &[],
        );
        let replayed = machine_events_for_batch(&host);
        assert_eq!(replayed.len(), 4);
        assert_eq!(
            replayed.iter().map(|event| event.kind).collect::<Vec<_>>(),
            vec![phase, done, failed, manual_walk]
        );
        assert!(replayed.iter().all(|event| event.admitted));

        reconcile_machine_events(&mut host, &[(phase_id, native::NativePhase::Working)], &[]);
        assert_eq!(
            machine_events_for_batch(&host),
            vec![
                MachineEvent {
                    id: done_id,
                    kind: done,
                    admitted: true,
                },
                MachineEvent {
                    id: failed_id,
                    kind: failed,
                    admitted: true,
                },
                MachineEvent {
                    id: manual_walk_id,
                    kind: manual_walk,
                    admitted: true,
                },
            ]
        );

        reconcile_machine_events(&mut host, &[], &[done_id, failed_id, manual_walk_id]);
        assert!(machine_events_for_batch(&host).is_empty());
    }

    #[test]
    fn every_production_family_callback_has_an_abi_row_matching_sync_hooks() {
        use crate::native::abi::{FrozenPathStatus, HookPath, HOOK_CHARACTERIZATIONS};
        use crate::native::divergence::{DivergenceStatus, DIVERGENCES};

        const TEST_FAMILY_NAMES: &[&str] = &[
            "probe",
            "solo",
            "walker",
            "hooked",
            "burst",
            "present",
            "nested",
            "solo-hooked",
            "inline",
            "solo-asker",
            "ask",
            "notification-order",
            "kicked",
        ];
        for family in super::FAMILIES {
            if TEST_FAMILY_NAMES.contains(&family.name) {
                continue;
            }
            for (index, member) in family.callbacks.iter().enumerate() {
                let rows: Vec<_> = HOOK_CHARACTERIZATIONS
                    .iter()
                    .filter(|row| row.families().contains(&family.name) && row.member() == *member)
                    .collect();
                assert!(
                    !rows.is_empty(),
                    "no ABI row for registered {} / {}",
                    family.name,
                    member
                );
                let sync = family.sync_hooks.contains(&index);
                for row in rows {
                    assert_eq!(
                        row.path() == HookPath::CallSync,
                        sync,
                        "{} / {} path {:?} vs SYNC_HOOKS {:?}",
                        family.name,
                        member,
                        row.path(),
                        family.sync_hooks
                    );
                }
            }
        }
        for row in HOOK_CHARACTERIZATIONS {
            for name in row.families() {
                let family = super::FAMILIES
                    .iter()
                    .find(|family| {
                        family.name == *name && !TEST_FAMILY_NAMES.contains(&family.name)
                    })
                    .unwrap_or_else(|| panic!("ABI row names unregistered family {name}"));
                assert!(
                    family.callbacks.contains(&row.member()),
                    "ABI row {name} / {} is not a registered callback",
                    row.member()
                );
            }
        }
        for row in HOOK_CHARACTERIZATIONS {
            let FrozenPathStatus::ExistingDiscrepancy { q5_row, .. } = row.frozen_path() else {
                continue;
            };
            let id = q5_row.to_string();
            let found = DIVERGENCES
                .iter()
                .find(|divergence| divergence.id() == id)
                .unwrap_or_else(|| panic!("q5_row {q5_row} is not in DIVERGENCES"));
            assert_eq!(
                found.status(),
                DivergenceStatus::Existing,
                "q5_row {q5_row} must be an existing discrepancy"
            );
        }
    }
}

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
//!   begin are discarded).
//! - [`Family::step`]: one step per eligible tick. Read the scene, emit
//!   ops, arm/read deadlines on [`Cx::clock`], and return [`Step::Wait`]
//!   or [`Step::Done`].
//! - [`Family::abort`]: optional cleanup when the host drops a live row
//!   (ResetSession, a superseding start). It emits nothing.
//!
//! # Lifecycle
//!
//! - **Start** (`runMachine(family, args)` in `_kernel.js`, native
//!   `__rs2b0t_machine_start`): runs `begin` synchronously inside the
//!   caller's JS. Ops it emits join this tick's InteractReq batch at the
//!   caller's position in the JS queue. A [`Family::EXCLUSIVE`] family
//!   aborts its older live row as `superseded` when a new one runs.
//! - **Step**: the isolate thread calls [`step`] once per eligible tick,
//!   before any of that tick's JS runs (the scene was applied by the
//!   Snapshot command before the Tick). A row started during tick N is
//!   first stepped on tick N+1. Step ops lead the tick's batch.
//! - **Completion**: a `Done` row ends at once (no JS `end` op). Its
//!   outcome waits in the host until the JS await helper — a wait parked
//!   on the `Execution` park list — takes it in that tick's pump, so it
//!   settles exactly once, in the Execution phase order.
//! - **ResetSession** ([`on_reset`]): every live row is aborted and its
//!   await settles `{ kind: 'aborted', reason: 'reset' }`; pending ops are
//!   dropped with the JS queue.
//! - **Pause / guardian hold** ([`on_pause`], [`on_resume`], [`on_hold`]):
//!   rows are not stepped and every row's [`InstantTaskClock`] freezes, so
//!   deadlines resume where they stopped. Ops emitted on a held tick are
//!   dropped with the JS queue ([`drop_ops`]).
//! - **Stop** ([`on_stop`]): every row, outcome and op is dropped; the
//!   isolate is going away and nothing settles.
//!
//! JS sees one envelope per start: `{ kind: 'done', value }`,
//! `{ kind: 'refused', reason }` or `{ kind: 'aborted', reason }`.

use crate::shim::{InteractReq, MaybeInteractReq};
use crate::task_clock::InstantTaskClock;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cell::RefCell;

/// A started machine's id, unique for the isolate thread's life.
pub(crate) type Handle = u64;

/// One machine family: a Rust type the host begins, steps and aborts.
pub(crate) trait Family: Sized + 'static {
    /// The name JS passes to `runMachine`.
    const NAME: &'static str;
    /// At most one live row: a newer running start supersedes the older.
    const EXCLUSIVE: bool = false;
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
    Wait,
    Done(T),
}

/// Why the host ended a live row without its own completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AbortReason {
    /// ResetSession (reconnect / new session).
    Reset,
    /// A newer start of the same exclusive family.
    Superseded,
    /// The handle was never started here or was already taken.
    Unknown,
}

impl AbortReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Superseded => "superseded",
            Self::Unknown => "unknown",
        }
    }
}

/// A machine's single settlement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Done(Value),
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "first deadline family lands in F05")
    )]
    clock: &'a mut InstantTaskClock,
}

impl Cx<'_> {
    /// Append one game op to this tick's InteractReq batch.
    pub(crate) fn emit(&mut self, op: InteractReq) {
        self.ops.push(op);
    }

    /// This row's pause/hold-aware deadline clock.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "first deadline family lands in F05")
    )]
    pub(crate) fn clock(&mut self) -> &mut InstantTaskClock {
        self.clock
    }
}

/// Every registered family, by name. The only machine dispatch table.
const FAMILIES: &[(&str, StartFn)] = &[
    (
        crate::teleport::Teleport::NAME,
        begin_row::<crate::teleport::Teleport>,
    ),
    #[cfg(test)]
    (tests::Probe::NAME, begin_row::<tests::Probe>),
    #[cfg(test)]
    (tests::Solo::NAME, begin_row::<tests::Solo>),
];

type StartFn = fn(&mut Host, Value, usize) -> Started;

/// The type-erased row the host steps.
trait Machine {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value>;
    fn abort(&mut self, why: AbortReason);
}

impl<F: Family> Machine for F {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match Family::step(self, cx) {
            Step::Wait => Step::Wait,
            Step::Done(out) => Step::Done(out.into()),
        }
    }

    fn abort(&mut self, why: AbortReason) {
        Family::abort(self, why);
    }
}

struct Row {
    handle: Handle,
    family: &'static str,
    exclusive: bool,
    clock: InstantTaskClock,
    machine: Box<dyn Machine>,
}

/// Emitted ops keyed by the JS queue length at emit time, so they merge
/// into the tick batch where they happened.
type PlacedOp = (usize, InteractReq);

pub(crate) struct Host {
    next: Handle,
    rows: Vec<Row>,
    settled: Vec<(Handle, Outcome)>,
    ops: Vec<PlacedOp>,
    paused: bool,
    held: bool,
}

thread_local! {
    static HOST: RefCell<Host> = const { RefCell::new(Host::new()) };
}

impl Host {
    const fn new() -> Self {
        Self {
            next: 1,
            rows: Vec::new(),
            settled: Vec::new(),
            ops: Vec::new(),
            paused: false,
            held: false,
        }
    }

    fn start(&mut self, family: &str, args: Value, at: usize) -> Started {
        match FAMILIES.iter().find(|(name, _)| *name == family) {
            Some((_, begin)) => begin(self, args, at),
            None => Started::Refused(format!("unknown machine family {family:?}")),
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

    fn step(&mut self, at: usize) {
        if self.paused || self.held {
            return;
        }
        let mut ops = Vec::new();
        let mut i = 0;
        while i < self.rows.len() {
            let row = &mut self.rows[i];
            let step = row.machine.step(&mut Cx {
                ops: &mut ops,
                clock: &mut row.clock,
            });
            match step {
                Step::Wait => i += 1,
                Step::Done(value) => {
                    let row = self.rows.remove(i);
                    self.settled.push((row.handle, Outcome::Done(value)));
                }
            }
        }
        self.place(at, ops);
    }

    fn abort_where(&mut self, why: AbortReason, doomed: impl Fn(&Row) -> bool) {
        let mut i = 0;
        while i < self.rows.len() {
            if doomed(&self.rows[i]) {
                let mut row = self.rows.remove(i);
                row.machine.abort(why);
                self.settled.push((row.handle, Outcome::Aborted(why)));
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
        self.abort_where(AbortReason::Reset, |_| true);
        self.ops.clear();
    }

    fn stop(&mut self) {
        self.rows.clear();
        self.settled.clear();
        self.ops.clear();
    }
}

fn begin_row<F: Family>(host: &mut Host, args: Value, at: usize) -> Started {
    let args: F::Args = match serde_json::from_value(args) {
        Ok(args) => args,
        Err(e) => return Started::Refused(format!("{} arguments: {e}", F::NAME)),
    };
    let mut clock = host.fresh_clock();
    let mut ops = Vec::new();
    let begun = F::begin(
        args,
        &mut Cx {
            ops: &mut ops,
            clock: &mut clock,
        },
    );
    match begun {
        Begin::Refuse(reason) => Started::Refused(reason),
        Begin::Done(out) => {
            host.place(at, ops);
            Started::Settled(Outcome::Done(out.into()))
        }
        Begin::Run(machine) => {
            if F::EXCLUSIVE {
                host.abort_where(AbortReason::Superseded, |row| {
                    row.exclusive && row.family == F::NAME
                });
            }
            host.place(at, ops);
            let handle = host.next;
            host.next += 1;
            host.rows.push(Row {
                handle,
                family: F::NAME,
                exclusive: F::EXCLUSIVE,
                clock,
                machine: Box::new(machine),
            });
            Started::Running(handle)
        }
    }
}

/// Start `family` with JS `args`; `at` is the JS interact queue length.
pub(crate) fn start(family: &str, args: Value, at: usize) -> Started {
    HOST.with(|host| host.borrow_mut().start(family, args, at))
}

/// Step every live row once. Called at the start of each eligible tick.
pub(crate) fn step() {
    HOST.with(|host| host.borrow_mut().step(0));
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

    /// Test family: emits `if-button {first}` at begin, one `if-button`
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
    pub(crate) struct Solo(Probe, std::rc::Rc<std::cell::Cell<Option<AbortReason>>>);

    thread_local! {
        static SOLO_ABORTS: std::rc::Rc<std::cell::Cell<Option<AbortReason>>> =
            std::rc::Rc::default();
    }

    impl Family for Solo {
        const NAME: &'static str = "solo";
        const EXCLUSIVE: bool = true;
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            match Probe::begin(args, cx) {
                Begin::Run(probe) => Begin::Run(Self(probe, SOLO_ABORTS.with(Clone::clone))),
                Begin::Done(out) => Begin::Done(out),
                Begin::Refuse(reason) => Begin::Refuse(reason),
            }
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            Family::step(&mut self.0, cx)
        }

        fn abort(&mut self, why: AbortReason) {
            self.1.set(Some(why));
        }
    }

    fn button(id: i32) -> InteractReq {
        InteractReq::IfButton { component_id: id }
    }

    fn drain(host: &mut Host) -> Vec<InteractReq> {
        merge(Vec::new(), &mut host.ops)
    }

    fn running(started: Started) -> Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    #[test]
    fn a_row_steps_emits_in_order_and_settles_exactly_once() {
        let mut host = Host::new();
        let h = running(host.start("probe", json!({ "button": 10, "steps": 2 }), 0));
        assert_eq!(drain(&mut host), vec![button(10)]);
        assert_eq!(host.take(h), Take::Pending);
        host.step(0);
        host.step(0);
        assert_eq!(drain(&mut host), vec![button(11), button(12)]);
        assert_eq!(host.take(h), Take::Pending);
        host.step(0);
        assert!(host.rows.is_empty(), "completion ends the row");
        assert_eq!(host.take(h), Take::Settled(Outcome::Done(json!(2))));
        assert_eq!(
            host.take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Unknown)),
            "an outcome is handed out once"
        );
    }

    #[test]
    fn begin_settles_or_refuses_without_a_row() {
        let mut host = Host::new();
        assert_eq!(
            host.start("probe", json!({ "button": 5, "steps": 0 }), 0),
            Started::Settled(Outcome::Done(json!(0)))
        );
        assert_eq!(drain(&mut host), vec![button(5)]);
        assert_eq!(
            host.start(
                "probe",
                json!({ "button": 6, "steps": 1, "refuse": true }),
                0
            ),
            Started::Refused("probe refused".into())
        );
        assert!(drain(&mut host).is_empty(), "a refused begin sends nothing");
        assert!(matches!(
            host.start("probe", json!({ "steps": 1 }), 0),
            Started::Refused(reason) if reason.starts_with("probe arguments:")
        ));
        assert!(matches!(
            host.start("nowhere", json!({}), 0),
            Started::Refused(_)
        ));
        assert!(host.rows.is_empty());
    }

    #[test]
    fn reset_aborts_every_row_and_drops_pending_ops() {
        let mut host = Host::new();
        let a = running(host.start("probe", json!({ "button": 1, "steps": 5 }), 0));
        let b = running(host.start("probe", json!({ "button": 2, "steps": 5 }), 0));
        host.reset();
        assert!(drain(&mut host).is_empty());
        assert!(host.rows.is_empty());
        host.step(0);
        assert!(drain(&mut host).is_empty(), "an aborted row never steps");
        for h in [a, b] {
            assert_eq!(
                host.take(h),
                Take::Settled(Outcome::Aborted(AbortReason::Reset))
            );
        }
    }

    #[test]
    fn stop_drops_rows_and_outcomes() {
        let mut host = Host::new();
        let a = running(host.start("probe", json!({ "button": 1, "steps": 1 }), 0));
        host.step(0);
        host.step(0);
        assert!(host.rows.is_empty() && !host.settled.is_empty());
        let b = running(host.start("probe", json!({ "button": 2, "steps": 5 }), 0));
        host.stop();
        assert!(host.rows.is_empty() && host.settled.is_empty() && host.ops.is_empty());
        for h in [a, b] {
            assert_eq!(
                host.take(h),
                Take::Settled(Outcome::Aborted(AbortReason::Unknown))
            );
        }
    }

    #[test]
    fn pause_and_hold_skip_steps_and_freeze_deadlines() {
        let mut host = Host::new();
        let h = running(host.start(
            "probe",
            json!({ "button": 1, "steps": 1, "deadline_ms": 40 }),
            0,
        ));
        host.set_freeze(true, false);
        std::thread::sleep(Duration::from_millis(60));
        host.step(0);
        assert_eq!(host.take(h), Take::Pending, "a paused row is not stepped");
        host.set_freeze(true, true);
        host.set_freeze(false, true);
        host.step(0);
        assert_eq!(host.take(h), Take::Pending, "a held row is not stepped");
        host.set_freeze(false, false);
        host.step(0);
        assert_eq!(
            host.take(h),
            Take::Pending,
            "the frozen span does not count toward the deadline"
        );
        std::thread::sleep(Duration::from_millis(60));
        host.step(0);
        assert_eq!(host.take(h), Take::Settled(Outcome::Done(json!("timeout"))));
    }

    #[test]
    fn a_row_started_while_frozen_starts_frozen() {
        let mut host = Host::new();
        host.set_freeze(false, true);
        let h = running(host.start(
            "probe",
            json!({ "button": 1, "steps": 1, "deadline_ms": 30 }),
            0,
        ));
        std::thread::sleep(Duration::from_millis(50));
        host.set_freeze(false, false);
        host.step(0);
        assert_eq!(host.take(h), Take::Pending);
    }

    #[test]
    fn two_concurrent_rows_step_in_start_order_and_settle_independently() {
        let mut host = Host::new();
        let a = running(host.start("probe", json!({ "button": 100, "steps": 1 }), 0));
        let b = running(host.start("probe", json!({ "button": 200, "steps": 2 }), 0));
        assert_eq!(drain(&mut host), vec![button(100), button(200)]);
        host.step(0);
        assert_eq!(drain(&mut host), vec![button(101), button(201)]);
        host.step(0);
        assert_eq!(drain(&mut host), vec![button(202)]);
        assert_eq!(host.take(a), Take::Settled(Outcome::Done(json!(1))));
        assert_eq!(host.take(b), Take::Pending);
        host.step(0);
        assert_eq!(host.take(b), Take::Settled(Outcome::Done(json!(2))));
    }

    #[test]
    fn an_exclusive_start_supersedes_only_its_own_family() {
        let mut host = Host::new();
        let other = running(host.start("probe", json!({ "button": 1, "steps": 5 }), 0));
        let old = running(host.start("solo", json!({ "button": 2, "steps": 5 }), 0));
        assert!(matches!(
            host.start(
                "solo",
                json!({ "button": 3, "steps": 1, "refuse": true }),
                0
            ),
            Started::Refused(_)
        ));
        assert_eq!(
            host.take(old),
            Take::Pending,
            "a refused start supersedes nothing"
        );
        let new = running(host.start("solo", json!({ "button": 4, "steps": 5 }), 0));
        assert_eq!(
            host.take(old),
            Take::Settled(Outcome::Aborted(AbortReason::Superseded))
        );
        assert_eq!(SOLO_ABORTS.with(|c| c.get()), Some(AbortReason::Superseded));
        assert_eq!(host.take(new), Take::Pending);
        assert_eq!(host.take(other), Take::Pending);
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
        let rows = vec![js(2), MaybeInteractReq::Skip(serde::de::IgnoredAny), js(5)];
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

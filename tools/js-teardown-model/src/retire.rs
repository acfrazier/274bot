//! The deadline supervisor's retirement registry: when a capacity credit may
//! be released. The product is `crates/script-js/src/runtime/isolate/supervisor.rs`
//! (`Registry`, `Pass`, `settle`, `Registry::reap`).
//!
//! Invariant 3 of the C23 slice: "the supervisor releases a capacity credit
//! only after the OS thread joins". A credit is released by a receipt: a
//! gameplay row's J charge, a validation row's V charge, and a quarantine's
//! A credit by the join that `reap` does. One exception is designed: a
//! gameplay row whose join deadline passed with its thread alive moves its
//! charge to a credited quarantine slot, and its receipt is sent only after
//! the quarantine is published in the census. A validation row keeps its
//! charge until the real join.
//!
//! Actors: the host (`retire` pushes a row), the isolate threads (exit), and
//! the one supervisor worker, whose pass is four steps: take the rows
//! (`begin_pass`), decide each (`settle_next`), publish (`finish_pass`), send
//! receipts (`send_receipts`). `reap` runs in the worker's loop and in every
//! `admit`.

use crate::explorer::{Machine, Violation};
use std::fmt::Write as _;

/// Units of time from the retirement to its join deadline (`JOIN_TIMEOUT`).
pub const JOIN: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Thread {
    Alive,
    /// The OS thread returned; `JoinHandle::is_finished` is true.
    Finished,
    Joined,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Custody {
    /// The host still owns the isolate.
    Host,
    Retiring,
    /// The worker holds the row for a pass; the census counts it by number.
    Processing,
    Quarantined,
    Gone,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Decision {
    Undecided,
    Joined,
    Quarantine,
    Keep,
    /// Mutation only: the row was dropped with its thread alive.
    Dropped,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Receipt {
    None,
    Queued,
    Sent(u8),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Row {
    pub validation: bool,
    pub thread: Thread,
    pub custody: Custody,
    pub deadline: u8,
    pub decision: Decision,
    pub receipt: Receipt,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Worker {
    Idle,
    /// Deciding the rows in order.
    Settling,
    /// The pass is published; the receipts are next.
    Published,
    /// Mutation only: the receipts went out; the publication is next.
    Sent,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    pub rows: Vec<Row>,
    pub worker: Worker,
    pub room: u8,
    pub now: u8,
    /// The census's processing rows, as `begin_pass` published them.
    pub processing_gameplay: u8,
    pub processing_validation: u8,
    /// `Registry::retiring`, in order.
    pub retiring: Vec<usize>,
    /// The rows of the pass still to decide, in order (`Pass::retiring`).
    pub pass: Vec<usize>,
    /// The rows the pass keeps (`Pass::keep`), in order.
    pub keep: Vec<usize>,
}

impl State {
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "now={} worker={:?} room={} processing=({},{})",
            self.now, self.worker, self.room, self.processing_gameplay, self.processing_validation
        );
        let _ = write!(
            out,
            " retiring={:?} pass={:?} keep={:?}",
            self.retiring, self.pass, self.keep
        );
        for (i, r) in self.rows.iter().enumerate() {
            let _ = write!(
                out,
                " | row{i}{} {:?}/{:?}/{:?}/{:?}",
                if r.validation { "V" } else { "J" },
                r.thread,
                r.custody,
                r.decision,
                r.receipt
            );
        }
        out
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    Advance,
    Exit(usize),
    Retire(usize),
    Begin,
    Settle,
    Finish,
    Send,
    Reap,
}

impl Action {
    pub fn text(self) -> String {
        match self {
            Action::Advance => "time passes: one unit".into(),
            Action::Exit(i) => format!("isolate thread {i}: exits (the OS thread returns)"),
            Action::Retire(i) => format!("host: retire hands isolate {i} to the supervisor"),
            Action::Begin => "worker: begin_pass takes the retiring rows".into(),
            Action::Settle => "worker: settle_next decides the next row".into(),
            Action::Finish => "worker: finish_pass publishes the decisions".into(),
            Action::Send => "worker: send_receipts releases the credits".into(),
            Action::Reap => {
                "reap (worker loop or admit): joins finished quarantined threads".into()
            }
        }
    }
}

/// Seeded designs and self-checks. The head is `None`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Mutation {
    #[default]
    None,
    /// The worker sends receipts before it publishes the pass ("a receipt is
    /// authoritative only after capacity and quarantine state reflect it",
    /// `supervisor.rs`).
    ReceiptsBeforePublish,
    /// A validation row's receipt goes out when its thread is quarantined
    /// ("validation keeps its permit until real join").
    ValidationCreditAtQuarantine,
    /// `settle` joins whatever thread a row has, without `is_finished`
    /// ("only finished OS threads are joined").
    JoinWithoutFinished,
    /// `begin_pass` does not count the rows it takes ("rows temporarily owned
    /// by the worker still count as retiring").
    ProcessingUncounted,
    /// `settle` quarantines without room.
    QuarantineIgnoresRoom,
    /// A row whose deadline passed with no quarantine room is dropped, and its
    /// receipt sent ("a timeout never releases a live thread").
    TimeoutReleasesLiveThread,
    /// `reap` sends a gameplay receipt again for a quarantined thread it joins.
    ReapSendsSecondReceipt,
    /// `reap` joins a quarantined thread that has not finished.
    ReapJoinsLive,
}

impl Mutation {
    pub const ALL: [Mutation; 9] = [
        Mutation::None,
        Mutation::ReceiptsBeforePublish,
        Mutation::ValidationCreditAtQuarantine,
        Mutation::JoinWithoutFinished,
        Mutation::ProcessingUncounted,
        Mutation::QuarantineIgnoresRoom,
        Mutation::TimeoutReleasesLiveThread,
        Mutation::ReapSendsSecondReceipt,
        Mutation::ReapJoinsLive,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mutation::None => "none",
            Mutation::ReceiptsBeforePublish => "receipts-before-publish",
            Mutation::ValidationCreditAtQuarantine => "validation-credit-at-quarantine",
            Mutation::JoinWithoutFinished => "join-without-finished",
            Mutation::ProcessingUncounted => "processing-uncounted",
            Mutation::QuarantineIgnoresRoom => "quarantine-ignores-room",
            Mutation::TimeoutReleasesLiveThread => "timeout-releases-live-thread",
            Mutation::ReapSendsSecondReceipt => "reap-sends-second-receipt",
            Mutation::ReapJoinsLive => "reap-joins-live",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    pub fn caught_by(self) -> Option<&'static str> {
        Some(match self {
            Mutation::None => return None,
            Mutation::ReceiptsBeforePublish => "c-credit-before-join",
            Mutation::ValidationCreditAtQuarantine => "c-validation-credit-before-join",
            Mutation::JoinWithoutFinished => "c-join-of-live-thread",
            Mutation::ProcessingUncounted => "c-census-loses-row",
            Mutation::QuarantineIgnoresRoom => "c-quarantine-over-cap",
            Mutation::TimeoutReleasesLiveThread => "c-live-thread-released",
            Mutation::ReapSendsSecondReceipt => "c-receipt-twice",
            Mutation::ReapJoinsLive => "c-join-of-live-thread",
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Bounds {
    pub horizon: usize,
    /// The rows, by validation flag.
    pub rows: Vec<bool>,
    /// `QUARANTINE_CAP`.
    pub cap: u8,
    pub max_now: u8,
}

impl Bounds {
    pub fn standard() -> Self {
        Bounds {
            horizon: 32,
            rows: vec![false, false, true],
            cap: 1,
            max_now: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Retire {
    pub bounds: Bounds,
    pub mutation: Mutation,
}

impl Retire {
    pub fn new(bounds: Bounds, mutation: Mutation) -> Self {
        Retire { bounds, mutation }
    }

    pub fn initial_state(&self) -> State {
        State {
            rows: self
                .bounds
                .rows
                .iter()
                .map(|validation| Row {
                    validation: *validation,
                    thread: Thread::Alive,
                    custody: Custody::Host,
                    deadline: 0,
                    decision: Decision::Undecided,
                    receipt: Receipt::None,
                })
                .collect(),
            worker: Worker::Idle,
            room: 0,
            now: 0,
            processing_gameplay: 0,
            processing_validation: 0,
            retiring: Vec::new(),
            pass: Vec::new(),
            keep: Vec::new(),
        }
    }

    fn violation(name: &'static str, detail: impl Into<String>) -> Violation {
        Violation {
            name,
            detail: detail.into(),
        }
    }

    fn quarantined(s: &State) -> usize {
        s.rows
            .iter()
            .filter(|r| r.custody == Custody::Quarantined)
            .count()
    }

    /// Send one row's receipt: the credit is released.
    fn send(s: &mut State, i: usize) -> Result<(), Violation> {
        let row = s.rows[i];
        let Receipt::Queued = row.receipt else {
            return Ok(());
        };
        let joined = row.thread == Thread::Joined;
        let credited = !row.validation && row.custody == Custody::Quarantined;
        if row.validation && !joined {
            return Err(Self::violation(
                "c-validation-credit-before-join",
                format!(
                    "row {i}: the validation charge is released while its thread has not joined"
                ),
            ));
        }
        if !joined && !credited {
            return Err(Self::violation(
                "c-credit-before-join",
                format!(
                    "row {i}: the credit is released with the thread {:?} and the row in custody {:?}, not in a published quarantine",
                    row.thread, row.custody
                ),
            ));
        }
        s.rows[i].receipt = Receipt::Sent(1);
        Ok(())
    }

    fn check_state(&self, s: &State) -> Result<(), Violation> {
        if Self::quarantined(s) > usize::from(self.bounds.cap) {
            return Err(Self::violation(
                "c-quarantine-over-cap",
                format!(
                    "{} rows are quarantined; the cap is {}",
                    Self::quarantined(s),
                    self.bounds.cap
                ),
            ));
        }
        for (i, r) in s.rows.iter().enumerate() {
            if r.custody == Custody::Gone && r.thread != Thread::Joined {
                return Err(Self::violation(
                    "c-live-thread-released",
                    format!("row {i} left custody with its thread {:?}", r.thread),
                ));
            }
        }
        let live = s
            .rows
            .iter()
            .filter(|r| {
                r.thread != Thread::Joined
                    && matches!(
                        r.custody,
                        Custody::Retiring | Custody::Processing | Custody::Quarantined
                    )
            })
            .count();
        let counted = s
            .rows
            .iter()
            .filter(|r| matches!(r.custody, Custody::Retiring | Custody::Quarantined))
            .count()
            + usize::from(s.processing_gameplay)
            + usize::from(s.processing_validation);
        if counted < live {
            return Err(Self::violation(
                "c-census-loses-row",
                format!(
                    "the census counts {counted} rows, and {live} rows hold an unjoined thread"
                ),
            ));
        }
        Ok(())
    }

    /// Whether `action` is offered in `state`.
    pub fn enabled(&self, state: &State, action: &Action) -> bool {
        let mut offered = Vec::new();
        self.actions(state, &mut offered);
        offered.contains(action)
    }

    pub fn step_detail(&self, state: &State, action: &Action) -> Result<State, Violation> {
        let mut s = state.clone();
        let m = self.mutation;
        match *action {
            Action::Advance => s.now += 1,
            Action::Exit(i) => s.rows[i].thread = Thread::Finished,
            Action::Retire(i) => {
                s.rows[i].custody = Custody::Retiring;
                s.rows[i].deadline = s.now + JOIN;
                s.retiring.push(i);
            }
            Action::Begin => {
                let mut g = 0u8;
                let mut v = 0u8;
                s.pass = std::mem::take(&mut s.retiring);
                for i in s.pass.clone() {
                    let r = &mut s.rows[i];
                    r.custody = Custody::Processing;
                    r.decision = Decision::Undecided;
                    if r.validation {
                        v += 1;
                    } else {
                        g += 1;
                    }
                }
                if m != Mutation::ProcessingUncounted {
                    s.processing_gameplay = g;
                    s.processing_validation = v;
                }
                let cap = usize::from(self.bounds.cap);
                s.room = u8::try_from(cap.saturating_sub(Self::quarantined(&s))).unwrap_or(0);
                s.worker = Worker::Settling;
            }
            Action::Settle => {
                if s.pass.is_empty() {
                    return Ok(s);
                }
                let i = s.pass.remove(0);
                let row = s.rows[i];
                let finished = row.thread == Thread::Finished
                    || (m == Mutation::JoinWithoutFinished && row.thread == Thread::Alive);
                let due = s.now >= row.deadline;
                if finished {
                    if row.thread == Thread::Alive {
                        return Err(Self::violation(
                            "c-join-of-live-thread",
                            format!("row {i}: a join of a thread that has not finished"),
                        ));
                    }
                    s.rows[i].thread = Thread::Joined;
                    s.rows[i].decision = Decision::Joined;
                    s.rows[i].receipt = Receipt::Queued;
                } else if due && (s.room != 0 || m == Mutation::QuarantineIgnoresRoom) {
                    s.room = s.room.saturating_sub(1);
                    s.rows[i].decision = Decision::Quarantine;
                    if !row.validation || m == Mutation::ValidationCreditAtQuarantine {
                        s.rows[i].receipt = Receipt::Queued;
                    }
                } else if due && m == Mutation::TimeoutReleasesLiveThread {
                    s.rows[i].decision = Decision::Dropped;
                    s.rows[i].receipt = Receipt::Queued;
                } else {
                    s.rows[i].decision = Decision::Keep;
                    s.keep.push(i);
                }
            }
            Action::Finish => {
                Self::publish(&mut s);
                s.worker = if s.worker == Worker::Sent {
                    Worker::Idle
                } else {
                    Worker::Published
                };
            }
            Action::Send => {
                for i in 0..s.rows.len() {
                    Self::send(&mut s, i)?;
                }
                s.worker = if s.worker == Worker::Settling {
                    Worker::Sent
                } else {
                    Worker::Idle
                };
            }
            Action::Reap => {
                for i in 0..s.rows.len() {
                    let r = s.rows[i];
                    if r.custody != Custody::Quarantined {
                        continue;
                    }
                    let joins = r.thread == Thread::Finished
                        || (m == Mutation::ReapJoinsLive && r.thread == Thread::Alive);
                    if !joins {
                        continue;
                    }
                    if r.thread == Thread::Alive {
                        return Err(Self::violation(
                            "c-join-of-live-thread",
                            format!(
                                "row {i}: reap joins a quarantined thread that has not finished"
                            ),
                        ));
                    }
                    s.rows[i].thread = Thread::Joined;
                    s.rows[i].custody = Custody::Gone;
                    if r.validation {
                        s.rows[i].receipt = Receipt::Queued;
                        Self::send(&mut s, i)?;
                    } else if m == Mutation::ReapSendsSecondReceipt
                        && let Receipt::Sent(n) = r.receipt
                    {
                        return Err(Self::violation(
                            "c-receipt-twice",
                            format!(
                                "row {i}: reap sends a second receipt (the first was number {n})"
                            ),
                        ));
                    }
                }
            }
        }
        self.check_state(&s)?;
        Ok(s)
    }

    /// `finish_pass`: the decisions become custody, and the census stops
    /// counting the processing rows.
    fn publish(s: &mut State) {
        for r in s
            .rows
            .iter_mut()
            .filter(|r| r.custody == Custody::Processing)
        {
            match r.decision {
                Decision::Joined | Decision::Dropped => r.custody = Custody::Gone,
                Decision::Quarantine => r.custody = Custody::Quarantined,
                Decision::Keep | Decision::Undecided => r.custody = Custody::Retiring,
            }
            r.decision = Decision::Undecided;
        }
        let keep = std::mem::take(&mut s.keep);
        s.retiring.extend(keep);
        s.processing_gameplay = 0;
        s.processing_validation = 0;
    }
}

impl Machine for Retire {
    type State = State;
    type Action = Action;

    fn initial(&self) -> State {
        self.initial_state()
    }

    fn actions(&self, s: &State, out: &mut Vec<Action>) {
        if s.now < self.bounds.max_now {
            out.push(Action::Advance);
        }
        for (i, r) in s.rows.iter().enumerate() {
            if r.thread == Thread::Alive {
                out.push(Action::Exit(i));
            }
            if r.custody == Custody::Host {
                out.push(Action::Retire(i));
            }
        }
        let undecided = !s.pass.is_empty();
        match s.worker {
            Worker::Idle => out.push(Action::Begin),
            Worker::Settling if undecided => out.push(Action::Settle),
            Worker::Settling => out.push(if self.mutation == Mutation::ReceiptsBeforePublish {
                Action::Send
            } else {
                Action::Finish
            }),
            Worker::Published => out.push(Action::Send),
            Worker::Sent => out.push(Action::Finish),
        }
        if s.rows.iter().any(|r| r.custody == Custody::Quarantined) {
            out.push(Action::Reap);
        }
    }

    fn step(&self, state: &State, action: &Action) -> Result<State, Violation> {
        self.step_detail(state, action)
    }
}

/// Replay `trace` and print each step.
pub fn describe_trace(model: &Retire, trace: &[Action]) -> String {
    let mut out = String::new();
    let mut state = model.initial_state();
    let _ = writeln!(out, "  start: {}", state.summary());
    for (i, action) in trace.iter().enumerate() {
        let _ = writeln!(out, "  {:>2}. {}", i + 1, action.text());
        match model.step_detail(&state, action) {
            Ok(next) => {
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

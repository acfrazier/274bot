//! Bounded model of the host stderr queue (ADR 0017): the reservation
//! counter, concurrent producers, the receiver and the shutdown sender
//! clones of `crates/api/src/hostlog.rs`.
//!
//! No product crate depends on this crate, except as a dev-dependency of the
//! conformance tests in `crates/api`. The explorer (`explorer.rs`) searches
//! the model breadth first inside caps and returns the shortest trace of any
//! violation. See README.md.
//!
//! The model keeps the product's own steps. A producer is the phase machine
//! of `Enqueue` (`Start`, `Cloned`, `Measured`, `Reserved`, `Held`); the
//! writer is `Recv`, then a sink write, then `Finish`; shutdown is `Close`
//! followed by either `Settle` (the writer ended) or `GiveUp` (the wait
//! ended), and the process `Exit`s at any later point. Producers keep acting
//! between `GiveUp` and `Exit`. Each action is one atomic step of the product:
//! one try-lock, one compare-and-swap of the state word (lines, pending and
//! the closed flag together), one send, one receive; or a pass that touches no
//! shared state.

pub mod explorer;

pub use explorer::*;

/// Most producers a state holds.
pub const MAX_PRODUCERS: usize = 3;

/// What the sink does with a write.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Sink {
    /// Every write succeeds.
    Writable,
    /// A write succeeds or fails, as the full disk of a redirected stderr
    /// does.
    Flaky,
    /// The first write never returns: a pipe nobody reads.
    Stalled,
}

/// Defects that the invariants must keep catching: each is an earlier design
/// or a plausible slip of the product (ADR 0017 point 2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mutation {
    None,
    /// The lines are reserved without looking at the cap.
    ReserveWithoutCheck,
    /// A reservation takes the lines and, in a later step, the pending count
    /// (the first design: two atomic operations).
    HalfReserve,
    /// `give_up` does not close admission: a producer that reserves after its
    /// snapshot is accepted and lost uncounted.
    GiveUpKeepsAdmission,
    /// A formatting failure keeps the reservation.
    FormatFailKeepsReservation,
    /// The writer forgets to take the entry off the pending count.
    FinishKeepsPending,
    /// The writer releases an entry's lines when it takes the entry, not when
    /// the write ends (the first design of the port).
    ReleaseAtRecv,
    /// A refused entry is not added to the drop counter.
    RefusalNotCounted,
    /// Process exit does not wait for the writer (the first design).
    ShutdownNoWait,
    /// Shutdown waits for the writer with no deadline.
    ShutdownNoDeadline,
    /// The deadline gives up on the writer but does not count the entries.
    GiveUpNotCounted,
}

impl Mutation {
    pub const ALL: [Mutation; 11] = [
        Mutation::None,
        Mutation::ReserveWithoutCheck,
        Mutation::HalfReserve,
        Mutation::GiveUpKeepsAdmission,
        Mutation::FormatFailKeepsReservation,
        Mutation::FinishKeepsPending,
        Mutation::ReleaseAtRecv,
        Mutation::RefusalNotCounted,
        Mutation::ShutdownNoWait,
        Mutation::ShutdownNoDeadline,
        Mutation::GiveUpNotCounted,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mutation::None => "none",
            Mutation::ReserveWithoutCheck => "reserve-without-check",
            Mutation::HalfReserve => "half-reserve",
            Mutation::GiveUpKeepsAdmission => "give-up-keeps-admission",
            Mutation::FormatFailKeepsReservation => "format-fail-keeps-reservation",
            Mutation::FinishKeepsPending => "finish-keeps-pending",
            Mutation::ReleaseAtRecv => "release-at-recv",
            Mutation::RefusalNotCounted => "refusal-not-counted",
            Mutation::ShutdownNoWait => "shutdown-no-wait",
            Mutation::ShutdownNoDeadline => "shutdown-no-deadline",
            Mutation::GiveUpNotCounted => "give-up-not-counted",
        }
    }

    pub fn parse(name: &str) -> Option<Mutation> {
        Mutation::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The invariant that must catch this mutation.
    pub fn caught_by(self) -> Option<&'static str> {
        match self {
            Mutation::None => None,
            Mutation::ReserveWithoutCheck => Some("bound"),
            Mutation::HalfReserve | Mutation::GiveUpKeepsAdmission => Some("exit-silent-loss"),
            Mutation::FormatFailKeepsReservation => Some("lines-conserved"),
            Mutation::FinishKeepsPending => Some("pending-conserved"),
            Mutation::ReleaseAtRecv => Some("lines-conserved"),
            Mutation::RefusalNotCounted | Mutation::GiveUpNotCounted => Some("drop-uncounted"),
            Mutation::ShutdownNoWait => Some("exit-silent-loss"),
            Mutation::ShutdownNoDeadline => Some("stuck"),
        }
    }
}

/// The bounds of one run.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    /// The line cap (`STDERR_QUEUE_LINES` in the product).
    pub cap: u8,
    /// Producer threads, at most [`MAX_PRODUCERS`].
    pub producers: usize,
    /// Entries each producer submits.
    pub messages: u8,
    pub sink: Sink,
    /// An entry may need two lines.
    pub two_line: bool,
    /// An entry may be over the entry limit and refused whole.
    pub over: bool,
    /// A try-lock of the sender may fail.
    pub busy: bool,
    /// The second formatting pass may differ from the first.
    pub format_fails: bool,
    pub horizon: usize,
}

/// The phases of one entry, as `Enqueue` in the product.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Phase {
    /// Nothing held.
    Start,
    /// A sender clone is held.
    Cloned,
    /// The entry's size is known: it needs this many lines.
    Measured(u8),
    /// Only the lines are reserved (mutation `HalfReserve`).
    HalfReserved(u8),
    /// All the lines and the entry are reserved.
    Reserved(u8),
    /// The entry is formatted; it waits to be sent.
    Held(u8),
    /// No entries left.
    Done,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Prod {
    pub phase: Phase,
    /// Entries not yet finished, the current one included.
    pub left: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Shutdown {
    NotStarted,
    Closed,
    Done,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    pub prods: [Prod; MAX_PRODUCERS],
    /// The state word of the product: lines reserved, queued or being written,
    /// entries pending, and the closed flag. Every operation on it is one step.
    pub queued: u8,
    pub pending: u8,
    pub closed: bool,
    /// The product's drop counter.
    pub dropped: u8,
    /// Line counts of the entries in the channel, oldest first.
    pub chan: Vec<u8>,
    /// The entry the writer holds, with its lines.
    pub inflight: Option<u8>,
    pub worker_exited: bool,
    /// The output still holds the original sender.
    pub original: bool,
    pub shutdown: Shutdown,
    /// The process has exited: nothing runs any more.
    pub exited: bool,
    // Ghosts: the model's own books, never read by the product.
    pub accepted: u8,
    pub written: u8,
    pub failed: u8,
    pub refused: u8,
    pub lost: u8,
}

impl State {
    /// Sender clones the producers hold.
    pub fn clones(&self) -> u8 {
        self.prods
            .iter()
            .filter(|p| !matches!(p.phase, Phase::Start | Phase::Done))
            .count() as u8
    }

    fn held_lines(&self) -> u8 {
        self.prods
            .iter()
            .map(|p| match p.phase {
                Phase::HalfReserved(k) | Phase::Reserved(k) | Phase::Held(k) => k,
                _ => 0,
            })
            .sum()
    }

    fn held_entries(&self) -> u8 {
        self.prods
            .iter()
            .filter(|p| matches!(p.phase, Phase::Reserved(_) | Phase::Held(_)))
            .count() as u8
    }
}

/// One step. `i` is a producer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    /// `StderrOutput::sender`. `busy`: the try-lock failed.
    Acquire { i: u8, busy: bool },
    /// `measure_entry`; `lines` 0 is an entry that needs too many lines.
    Measure { i: u8, lines: u8 },
    /// `Counters::reserve`: all the lines and the entry in one operation.
    Reserve { i: u8 },
    /// The pending count of a `HalfReserve` reservation.
    Pend { i: u8 },
    /// `format_entry`; `ok` false: the second pass differed from the first.
    Format { i: u8, ok: bool },
    /// `SyncSender::try_send`, then `Reservation::commit`.
    Send { i: u8 },
    /// The writer takes an entry (`Receiver::recv`).
    Recv,
    /// The sink write ends and `Counters::finish` runs.
    Finish { fail: bool },
    /// `recv` sees every sender gone and the channel empty.
    WorkerExit,
    /// `StderrOutput::close`.
    Close,
    /// The wait sees the writer ended, and joins it.
    Settle,
    /// The wait ends with the writer alive: `StderrOutput::give_up`.
    GiveUp,
    /// The process exits.
    Exit,
    /// No action is enabled in a state where the process has not exited.
    Stuck,
}

pub struct Model {
    pub bounds: Bounds,
    pub mutation: Mutation,
}

impl Model {
    pub fn new(bounds: Bounds, mutation: Mutation) -> Self {
        assert!(bounds.producers >= 1 && bounds.producers <= MAX_PRODUCERS);
        Self { bounds, mutation }
    }

    fn violation(name: &'static str, detail: String) -> Violation {
        Violation { name, detail }
    }

    /// A refused entry: counted, and the producer moves on.
    fn refuse(&self, s: &mut State, i: usize) {
        if self.mutation != Mutation::RefusalNotCounted {
            s.dropped += 1;
        }
        s.refused += 1;
        Self::finish_entry(s, i);
    }

    fn finish_entry(s: &mut State, i: usize) {
        let p = &mut s.prods[i];
        p.left -= 1;
        p.phase = if p.left > 0 {
            Phase::Start
        } else {
            Phase::Done
        };
    }

    /// The invariants, after every step.
    fn check(&self, s: &State, action: Action) -> Result<(), Violation> {
        if s.queued > self.bounds.cap {
            return Err(Self::violation(
                "bound",
                format!(
                    "{} lines queued over the cap of {}",
                    s.queued, self.bounds.cap
                ),
            ));
        }
        let lines = s.held_lines() + s.chan.iter().sum::<u8>() + s.inflight.unwrap_or(0);
        if s.queued != lines {
            return Err(Self::violation(
                "lines-conserved",
                format!(
                    "counter {} but producers, channel and writer hold {lines}",
                    s.queued
                ),
            ));
        }
        let entries = s.held_entries() + s.chan.len() as u8 + u8::from(s.inflight.is_some());
        if s.pending != entries {
            return Err(Self::violation(
                "pending-conserved",
                format!("counter {} but {entries} entries are in flight", s.pending),
            ));
        }
        if s.dropped != s.refused + s.failed + s.lost {
            return Err(Self::violation(
                "drop-uncounted",
                format!(
                    "counter {} but {} refused, {} failed, {} lost",
                    s.dropped, s.refused, s.failed, s.lost
                ),
            ));
        }
        if matches!(action, Action::Exit) || (s.exited && matches!(action, Action::Close)) {
            let unwritten = s.accepted - s.written - s.failed;
            if unwritten > s.lost {
                return Err(Self::violation(
                    "exit-silent-loss",
                    format!(
                        "{unwritten} accepted entries unwritten, {} counted lost",
                        s.lost
                    ),
                ));
            }
        }
        if matches!(action, Action::Settle)
            && (s.pending != 0 || s.accepted != s.written + s.failed)
        {
            return Err(Self::violation(
                "exit-silent-loss",
                "a clean shutdown left entries pending".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Machine for Model {
    type State = State;
    type Action = Action;

    fn initial(&self) -> State {
        let mut prods = [Prod {
            phase: Phase::Done,
            left: 0,
        }; MAX_PRODUCERS];
        for p in prods.iter_mut().take(self.bounds.producers) {
            *p = Prod {
                phase: Phase::Start,
                left: self.bounds.messages,
            };
        }
        State {
            prods,
            queued: 0,
            pending: 0,
            closed: false,
            dropped: 0,
            chan: Vec::new(),
            inflight: None,
            worker_exited: false,
            original: true,
            shutdown: Shutdown::NotStarted,
            exited: false,
            accepted: 0,
            written: 0,
            failed: 0,
            refused: 0,
            lost: 0,
        }
    }

    fn actions(&self, s: &State, out: &mut Vec<Action>) {
        if s.exited {
            return;
        }
        let first = out.len();
        for (index, p) in s.prods.iter().enumerate() {
            let i = index as u8;
            match p.phase {
                Phase::Start => {
                    out.push(Action::Acquire { i, busy: false });
                    if self.bounds.busy {
                        out.push(Action::Acquire { i, busy: true });
                    }
                }
                Phase::Cloned => {
                    out.push(Action::Measure { i, lines: 1 });
                    if self.bounds.two_line {
                        out.push(Action::Measure { i, lines: 2 });
                    }
                    if self.bounds.over {
                        out.push(Action::Measure { i, lines: 0 });
                    }
                }
                Phase::Measured(_) => out.push(Action::Reserve { i }),
                Phase::HalfReserved(_) => out.push(Action::Pend { i }),
                Phase::Reserved(_) => {
                    out.push(Action::Format { i, ok: true });
                    if self.bounds.format_fails {
                        out.push(Action::Format { i, ok: false });
                    }
                }
                Phase::Held(_) => out.push(Action::Send { i }),
                Phase::Done => {}
            }
        }
        if !s.worker_exited {
            match s.inflight {
                None if !s.chan.is_empty() => out.push(Action::Recv),
                None if !s.original && s.clones() == 0 => out.push(Action::WorkerExit),
                None => {}
                Some(_) => match self.bounds.sink {
                    Sink::Writable => out.push(Action::Finish { fail: false }),
                    Sink::Flaky => {
                        out.push(Action::Finish { fail: false });
                        out.push(Action::Finish { fail: true });
                    }
                    Sink::Stalled => {}
                },
            }
        }
        match s.shutdown {
            Shutdown::NotStarted => out.push(Action::Close),
            Shutdown::Closed => {
                if s.worker_exited {
                    out.push(Action::Settle);
                } else if self.mutation != Mutation::ShutdownNoDeadline {
                    out.push(Action::GiveUp);
                }
            }
            Shutdown::Done => out.push(Action::Exit),
        }
        if out.len() == first {
            out.push(Action::Stuck);
        }
    }

    fn step(&self, state: &State, action: &Action) -> Result<State, Violation> {
        let mut s = state.clone();
        let cap = self.bounds.cap;
        match *action {
            Action::Acquire { i, busy } => {
                let i = i as usize;
                if busy || !s.original {
                    self.refuse(&mut s, i);
                } else {
                    s.prods[i].phase = Phase::Cloned;
                }
            }
            Action::Measure { i, lines } => {
                let i = i as usize;
                if lines == 0 {
                    self.refuse(&mut s, i);
                } else {
                    s.prods[i].phase = Phase::Measured(lines);
                }
            }
            Action::Reserve { i } => {
                let i = i as usize;
                let Phase::Measured(n) = s.prods[i].phase else {
                    return Err(Self::violation(
                        "model",
                        "Reserve outside Measured".to_owned(),
                    ));
                };
                let room = s.queued + n <= cap || self.mutation == Mutation::ReserveWithoutCheck;
                if !room || s.closed {
                    self.refuse(&mut s, i);
                } else if self.mutation == Mutation::HalfReserve {
                    s.queued += n;
                    s.prods[i].phase = Phase::HalfReserved(n);
                } else {
                    s.queued += n;
                    s.pending += 1;
                    s.prods[i].phase = Phase::Reserved(n);
                }
            }
            Action::Pend { i } => {
                let i = i as usize;
                let Phase::HalfReserved(n) = s.prods[i].phase else {
                    return Err(Self::violation(
                        "model",
                        "Pend outside HalfReserved".to_owned(),
                    ));
                };
                s.pending += 1;
                s.prods[i].phase = Phase::Reserved(n);
            }
            Action::Format { i, ok } => {
                let i = i as usize;
                let Phase::Reserved(n) = s.prods[i].phase else {
                    return Err(Self::violation(
                        "model",
                        "Format outside Reserved".to_owned(),
                    ));
                };
                if ok {
                    s.prods[i].phase = Phase::Held(n);
                } else {
                    if self.mutation != Mutation::FormatFailKeepsReservation {
                        s.queued = s.queued.saturating_sub(n);
                        s.pending = s.pending.saturating_sub(1);
                    }
                    self.refuse(&mut s, i);
                }
            }
            Action::Send { i } => {
                let i = i as usize;
                let Phase::Held(k) = s.prods[i].phase else {
                    return Err(Self::violation("model", "Send outside Held".to_owned()));
                };
                if s.chan.len() >= usize::from(cap) {
                    return Err(Self::violation(
                        "send-found-full",
                        "try_send met a full channel despite the reservation".to_owned(),
                    ));
                }
                s.chan.push(k);
                s.accepted += 1;
                Self::finish_entry(&mut s, i);
            }
            Action::Recv => {
                let k = s.chan.remove(0);
                s.inflight = Some(k);
                if self.mutation == Mutation::ReleaseAtRecv {
                    s.queued = s.queued.saturating_sub(k);
                }
            }
            Action::Finish { fail } => {
                let k = s.inflight.take().expect("Finish needs an entry");
                if fail {
                    s.dropped += 1;
                    s.failed += 1;
                } else {
                    s.written += 1;
                }
                if self.mutation != Mutation::ReleaseAtRecv {
                    s.queued = s.queued.saturating_sub(k);
                }
                if self.mutation != Mutation::FinishKeepsPending {
                    s.pending = s.pending.saturating_sub(1);
                }
            }
            Action::WorkerExit => s.worker_exited = true,
            Action::Close => {
                s.original = false;
                s.shutdown = Shutdown::Closed;
                if self.mutation == Mutation::ShutdownNoWait {
                    s.shutdown = Shutdown::Done;
                    s.exited = true;
                }
            }
            Action::Settle => s.shutdown = Shutdown::Done,
            Action::GiveUp => {
                s.lost += s.pending;
                if self.mutation != Mutation::GiveUpNotCounted {
                    s.dropped += s.pending;
                }
                if self.mutation != Mutation::GiveUpKeepsAdmission {
                    s.closed = true;
                }
                s.shutdown = Shutdown::Done;
            }
            Action::Exit => s.exited = true,
            Action::Stuck => {
                return Err(Self::violation(
                    "stuck",
                    "no action is enabled and the process has not exited".to_owned(),
                ));
            }
        }
        self.check(&s, *action)?;
        Ok(s)
    }
}

/// A named set of bounds.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub name: &'static str,
    pub bounds: Bounds,
}

/// The profiles. Each ends exhausted well inside the default caps.
pub fn profiles() -> Vec<Profile> {
    let base = Bounds {
        cap: 3,
        producers: 2,
        messages: 1,
        sink: Sink::Writable,
        two_line: true,
        over: true,
        busy: true,
        format_fails: true,
        horizon: 80,
    };
    vec![
        Profile {
            name: "writable",
            bounds: base,
        },
        Profile {
            name: "flaky",
            bounds: Bounds {
                sink: Sink::Flaky,
                ..base
            },
        },
        // A sink that never returns: the queue fills, and shutdown must give
        // up while producers keep acting until the process exits.
        Profile {
            name: "stalled",
            bounds: Bounds {
                sink: Sink::Stalled,
                messages: 2,
                ..base
            },
        },
        // The cap binds: two producers, two entries each, two lines of room.
        Profile {
            name: "tight",
            bounds: Bounds {
                cap: 2,
                messages: 2,
                ..base
            },
        },
        Profile {
            name: "three",
            bounds: Bounds {
                cap: 2,
                producers: 3,
                busy: false,
                over: false,
                format_fails: false,
                ..base
            },
        },
    ]
}

pub fn profile(name: &str) -> Option<Profile> {
    profiles().into_iter().find(|p| p.name == name)
}

/// Explore one profile with one mutation.
pub fn explore_profile(
    profile: &Profile,
    mutation: Mutation,
    horizon: Option<usize>,
    limits: Limits,
    all_violations: bool,
) -> Exploration<State, Action> {
    let model = Model::new(profile.bounds, mutation);
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

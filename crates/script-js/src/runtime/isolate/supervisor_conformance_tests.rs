//! Conformance of the retirement registry to `tools/js-teardown-model`
//! (`retire.rs`): invariant 3, "the supervisor releases a capacity credit
//! only after the OS thread joins".
//!
//! Each trace is a path of the head model. [`Replay`] drives the real
//! `Registry`, `Pass` and `settle` through it. A row's isolate is a double
//! that owns a real OS thread: the model's `Exit` lets that thread return,
//! and the real `JoinHandle::is_finished` and `join` decide what the worker
//! does. After every step the real census (`Registry::status`), the order of
//! the registry's and the pass's rows, where each row's handle is held, the
//! quarantine room, and the receipts each row's slot has received are
//! compared with the model.
//!
//! Three seeded divergences ([`Seed`]) put an earlier behaviour back into
//! the driver; each must fail.

use super::*;
use js_teardown_model::retire as model;
use js_teardown_model::{explore_retire, retire_profile, Limits, Machine};
use model::{Action, Custody, Mutation, Receipt, Thread};
use std::collections::BTreeSet;

/// One model time unit.
const UNIT: Duration = Duration::from_millis(100);

/// The isolate double: a real OS thread that returns when its row's release
/// sender is dropped.
struct Double {
    handle: Option<JoinHandle<()>>,
}

impl Retiree for Double {
    fn poll_setup(&mut self) -> bool {
        true
    }

    fn thread(&mut self) -> &mut Option<JoinHandle<()>> {
        &mut self.handle
    }

    fn take_logs(&mut self) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seed {
    Head,
    /// The worker sends the receipts before it publishes the pass.
    ReceiptsBeforePublish,
    /// `settle` always has quarantine room.
    QuarantineIgnoresRoom,
    /// `begin_pass` does not count the rows it takes.
    ProcessingUncounted,
}

const SEEDS: [Seed; 3] = [
    Seed::ReceiptsBeforePublish,
    Seed::QuarantineIgnoresRoom,
    Seed::ProcessingUncounted,
];

struct Ctl {
    validation: bool,
    release: Option<mpsc::Sender<()>>,
    retirement: Option<Retirement<Double>>,
    receipts: mpsc::Receiver<Vec<String>>,
    sent: u8,
}

struct Replay {
    registry: Registry<Double>,
    pass: Pass<Double>,
    rows: Vec<Ctl>,
    base: Instant,
    now: u8,
    /// Rows the pass joined: out of every container, still processing.
    joined_in_pass: BTreeSet<usize>,
    seed: Seed,
}

type Check = Result<(), String>;

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Check {
    if ok {
        Ok(())
    } else {
        Err(what())
    }
}

fn handle_index(handle: &JoinHandle<()>) -> Option<usize> {
    handle.thread().name()?.strip_prefix("row")?.parse().ok()
}

fn retirement_index(row: &Retirement<Double>) -> Option<usize> {
    row.isolate.handle.as_ref().and_then(handle_index)
}

impl Replay {
    fn new(rows: &[bool], cap: u8, seed: Seed) -> Self {
        let base = Instant::now();
        let rows = rows
            .iter()
            .enumerate()
            .map(|(i, validation)| {
                let (release, held) = mpsc::channel::<()>();
                let handle = std::thread::Builder::new()
                    .name(format!("row{i}"))
                    .spawn(move || {
                        let _ = held.recv();
                    })
                    .expect("a double thread");
                let (done, receipts) = mpsc::channel();
                Ctl {
                    validation: *validation,
                    release: Some(release),
                    retirement: Some(Retirement {
                        isolate: Double {
                            handle: Some(handle),
                        },
                        done,
                        deadline: base,
                        awaiting_setup: false,
                        validation: *validation,
                    }),
                    receipts,
                    sent: 0,
                }
            })
            .collect();
        Self {
            registry: Registry {
                quarantine_cap: usize::from(cap),
                ..Registry::default()
            },
            pass: Pass::new(),
            rows,
            base,
            now: 0,
            joined_in_pass: BTreeSet::new(),
            seed,
        }
    }

    fn at(&self, units: u8) -> Instant {
        self.base + UNIT * u32::from(units)
    }

    /// The handle of row `i` wherever the registry or the pass holds it.
    fn handle(&self, i: usize) -> Option<&JoinHandle<()>> {
        let held = |index: Option<usize>| index == Some(i);
        let ctl = self.rows[i]
            .retirement
            .as_ref()
            .and_then(|r| r.isolate.handle.as_ref());
        ctl.or_else(|| {
            self.registry
                .retiring
                .iter()
                .chain(&self.pass.retiring)
                .chain(&self.pass.keep)
                .find(|row| held(retirement_index(row)))
                .and_then(|row| row.isolate.handle.as_ref())
        })
        .or_else(|| {
            self.registry
                .quarantined
                .iter()
                .chain(&self.pass.credited)
                .map(|q| &q.handle)
                .find(|handle| held(handle_index(handle)))
        })
    }

    fn custody(&self, i: usize) -> Custody {
        let held = |index: Option<usize>| index == Some(i);
        if self.rows[i].retirement.is_some() {
            Custody::Host
        } else if self
            .registry
            .retiring
            .iter()
            .any(|r| held(retirement_index(r)))
        {
            Custody::Retiring
        } else if self.pass.retiring.iter().any(|r| held(retirement_index(r)))
            || self.pass.keep.iter().any(|r| held(retirement_index(r)))
            || self
                .pass
                .credited
                .iter()
                .any(|q| held(handle_index(&q.handle)))
            || self.joined_in_pass.contains(&i)
        {
            Custody::Processing
        } else if self
            .registry
            .quarantined
            .iter()
            .any(|q| held(handle_index(&q.handle)))
        {
            Custody::Quarantined
        } else {
            Custody::Gone
        }
    }

    fn apply(&mut self, action: Action) -> Check {
        match action {
            Action::Advance => self.now += 1,
            Action::Exit(i) => {
                self.rows[i].release = None;
                let mut spins = 0u32;
                while !self
                    .handle(i)
                    .ok_or_else(|| format!("row {i} has no live handle"))?
                    .is_finished()
                {
                    spins += 1;
                    ensure(spins < 5_000_000, || {
                        format!("row {i}'s thread did not return")
                    })?;
                    std::thread::yield_now();
                }
            }
            Action::Retire(i) => {
                let mut row = self.rows[i]
                    .retirement
                    .take()
                    .ok_or("row already retired")?;
                row.deadline = self.at(self.now + model::JOIN);
                self.registry.retiring.push(row);
            }
            Action::Begin => {
                self.pass.begin(&mut self.registry);
                if self.seed == Seed::ProcessingUncounted {
                    self.registry.processing_gameplay = 0;
                    self.registry.processing_validation = 0;
                }
            }
            Action::Settle => {
                let next = self.pass.retiring.last().and_then(retirement_index);
                if self.seed == Seed::QuarantineIgnoresRoom {
                    self.pass.quarantine_room = usize::MAX / 2;
                }
                ensure(self.pass.settle_next(self.at(self.now)), || {
                    "the pass has no row to settle".to_string()
                })?;
                let i = next.ok_or("the next row has no handle")?;
                if self.custody_after_settle(i) == Custody::Gone {
                    self.joined_in_pass.insert(i);
                }
            }
            Action::Finish => {
                if self.seed == Seed::ReceiptsBeforePublish {
                    self.pass.send_receipts();
                }
                self.pass.finish(&mut self.registry);
                self.joined_in_pass.clear();
            }
            Action::Send => self.pass.send_receipts(),
            Action::Reap => self.registry.reap(),
        }
        Ok(())
    }

    /// Where row `i` is after its settle: nowhere means it was joined.
    fn custody_after_settle(&self, i: usize) -> Custody {
        self.custody(i)
    }

    fn check(&mut self, m: &model::State) -> Check {
        for (i, ctl) in self.rows.iter_mut().enumerate() {
            while ctl.receipts.try_recv().is_ok() {
                ctl.sent += 1;
                ensure(ctl.sent <= 1, || format!("row {i}: a second receipt"))?;
            }
        }
        let status = self.registry.status();
        let count = |custody: Custody, validation: bool| {
            m.rows
                .iter()
                .filter(|r| r.custody == custody && r.validation == validation)
                .count()
        };
        let want = (
            count(Custody::Retiring, false) + usize::from(m.processing_gameplay),
            count(Custody::Retiring, true) + usize::from(m.processing_validation),
            count(Custody::Quarantined, false) + count(Custody::Quarantined, true),
            count(Custody::Quarantined, true),
        );
        let got = (
            status.retiring_gameplay,
            status.retiring_validation,
            status.quarantined,
            status.quarantined_validation,
        );
        ensure(got == want, || {
            format!("census (J, V, A, quarantined V): code {got:?}, model {want:?}")
        })?;
        let order = |rows: &[Retirement<Double>]| -> Vec<usize> {
            rows.iter().filter_map(retirement_index).collect()
        };
        ensure(order(&self.registry.retiring) == m.retiring, || {
            format!(
                "registry.retiring: code {:?}, model {:?}",
                order(&self.registry.retiring),
                m.retiring
            )
        })?;
        // The pass pops from the end; the model decides from the front.
        let mut pass = order(&self.pass.retiring);
        pass.reverse();
        ensure(pass == m.pass, || {
            format!("pass rows: code {pass:?}, model {:?}", m.pass)
        })?;
        ensure(order(&self.pass.keep) == m.keep, || {
            format!(
                "kept rows: code {:?}, model {:?}",
                order(&self.pass.keep),
                m.keep
            )
        })?;
        if m.worker != model::Worker::Idle {
            ensure(self.pass.quarantine_room == usize::from(m.room), || {
                format!(
                    "quarantine room: code {}, model {}",
                    self.pass.quarantine_room, m.room
                )
            })?;
        }
        for (i, row) in m.rows.iter().enumerate() {
            let custody = self.custody(i);
            ensure(custody == row.custody, || {
                format!("row {i} custody: code {custody:?}, model {:?}", row.custody)
            })?;
            match (self.handle(i), row.thread) {
                (Some(handle), Thread::Alive) => {
                    ensure(!handle.is_finished(), || format!("row {i}: finished early"))?;
                }
                (Some(handle), Thread::Finished) => {
                    ensure(handle.is_finished(), || format!("row {i}: not finished"))?;
                }
                (None, Thread::Joined) => {}
                (handle, thread) => {
                    return Err(format!(
                        "row {i}: handle held={}, model thread {thread:?}",
                        handle.is_some()
                    ));
                }
            }
            let sent = match row.receipt {
                Receipt::Sent(n) => n,
                Receipt::None | Receipt::Queued => 0,
            };
            ensure(self.rows[i].sent == sent, || {
                format!(
                    "row {i} receipts: code {}, model {sent} (validation={})",
                    self.rows[i].sent, self.rows[i].validation
                )
            })?;
        }
        Ok(())
    }
}

fn replay(bounds: &model::Bounds, trace: &[Action], seed: Seed) -> Result<(), String> {
    let m = model::Retire::new(bounds.clone(), Mutation::None);
    let mut state = m.initial_state();
    let mut real = Replay::new(&bounds.rows, bounds.cap, seed);
    real.check(&state)?;
    for (step, action) in trace.iter().enumerate() {
        ensure(m.enabled(&state, action), || {
            format!("step {}: {action:?} is not offered by the model", step + 1)
        })?;
        let next = m
            .step_detail(&state, action)
            .map_err(|v| format!("step {}: the head model violates {}", step + 1, v.name))?;
        real.apply(*action)
            .and_then(|()| real.check(&next))
            .map_err(|why| {
                format!(
                    "step {} ({}): {why}\n{}",
                    step + 1,
                    action.text(),
                    model::describe_trace(&m, &trace[..=step])
                )
            })?;
        state = next;
    }
    Ok(())
}

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn walks(bounds: &model::Bounds, count: usize, length: usize) -> Vec<Vec<Action>> {
    let m = model::Retire::new(bounds.clone(), Mutation::None);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut out = Vec::new();
    for _ in 0..count {
        let mut state = m.initial_state();
        let mut trace = Vec::new();
        for _ in 0..length {
            let mut offered = Vec::new();
            Machine::actions(&m, &state, &mut offered);
            if offered.is_empty() {
                break;
            }
            let action = offered[usize::try_from(next() % offered.len() as u64).unwrap_or(0)];
            state = m.step_detail(&state, &action).expect("head model is clean");
            trace.push(action);
        }
        out.push(trace);
    }
    out
}

#[test]
fn the_real_registry_follows_random_walks_of_the_retirement_model() {
    for name in ["retire", "retire-cap2"] {
        let profile = retire_profile(name).expect("a named profile");
        let count = env_count("RETIRE_CONFORMANCE_WALKS", 1500);
        for (walk, trace) in walks(&profile.bounds, count, 40).iter().enumerate() {
            if let Err(why) = replay(&profile.bounds, trace, Seed::Head) {
                panic!("{name}: walk {walk} diverges:\n{why}");
            }
        }
    }
}

/// The model's shortest path to each outcome: a quarantine, a join, a join
/// after quarantine, a kept row, a validation receipt.
#[test]
fn the_real_registry_follows_the_directed_paths() {
    use Action::*;
    let bounds = retire_profile("retire").expect("a named profile").bounds;
    let paths: [&[Action]; 4] = [
        // A gameplay row past its deadline with its thread alive: quarantine,
        // then the receipt, then the thread returns and reap joins it.
        &[
            Retire(0),
            Begin,
            Advance,
            Advance,
            Settle,
            Finish,
            Send,
            Exit(0),
            Reap,
        ],
        // A finished thread joins at once, and its receipt follows the pass.
        &[Retire(0), Exit(0), Begin, Settle, Finish, Send],
        // The validation row keeps its charge until reap joins its thread.
        &[
            Retire(2),
            Begin,
            Advance,
            Advance,
            Settle,
            Finish,
            Send,
            Exit(2),
            Reap,
        ],
        // The room is one: the second due row stays retiring.
        &[
            Retire(0),
            Retire(1),
            Begin,
            Advance,
            Advance,
            Settle,
            Settle,
            Finish,
            Send,
        ],
    ];
    for path in paths {
        if let Err(why) = replay(&bounds, path, Seed::Head) {
            panic!("a directed path diverges:\n{why}");
        }
    }
}

/// Every shortest path to a state where each model row has been retired is
/// walked too: the exploration's first deep states.
#[test]
fn the_real_registry_follows_the_model_to_its_deepest_states() {
    let profile = retire_profile("retire").expect("a named profile");
    let result = explore_retire(&profile, Mutation::None, Some(12), Limits::default(), false);
    assert!(result.failure.is_none());
    let deepest = (0..result.states.min(40_000))
        .step_by(97)
        .map(|node| result.trace_to(node))
        .filter(|trace| !trace.is_empty());
    let mut replayed = 0usize;
    for trace in deepest {
        if let Err(why) = replay(&profile.bounds, &trace, Seed::Head) {
            panic!("an explored path diverges:\n{why}");
        }
        replayed += 1;
    }
    assert!(replayed > 100, "{replayed}");
}

#[test]
fn each_seeded_divergence_fails_conformance() {
    let profile = retire_profile("retire").expect("a named profile");
    let traces = walks(&profile.bounds, 1500, 40);
    for seed in SEEDS {
        let found = traces
            .iter()
            .find_map(|trace| replay(&profile.bounds, trace, seed).err());
        let why = found.unwrap_or_else(|| panic!("{seed:?} passed every walk"));
        eprintln!("{seed:?} is caught: {}", why.lines().next().unwrap_or(""));
    }
}

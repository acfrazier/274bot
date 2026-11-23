//! Conformance of the stderr queue with `tools/hostlog-model`.
//!
//! The model's correct profiles are explored to exhaustion. For every
//! reachable state, the shortest trace to it is replayed on the real queue
//! (`Counters`, `Reservation`, `Enqueue`, the real channel and sender
//! clones), and so is every action enabled there. After each step the real
//! counters, producer phases and channel behaviour must equal the model's. A
//! change to the product that the model does not describe fails here; a
//! change to the model that the product does not follow fails here too.
//!
//! The traces include a producer that holds a sender clone across `give_up`
//! and reserves after it, and producers that keep acting between `give_up`
//! and the process exit.

use super::*;
use hostlog_model::{
    explore, profiles, Action, Limits, Machine, Model, Mutation, Options, Phase, Profile, Shutdown,
    State,
};
use std::cell::Cell;
use std::sync::mpsc::TryRecvError;

/// The text of one producer's entry. `bad`: the second formatting pass yields
/// a longer text than the first.
struct Source {
    text: String,
    bad: Cell<bool>,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)?;
        if self.bad.get() {
            f.write_str(&"x".repeat(STDERR_ENTRY_LINES * STDERR_PIECE_BYTES))?;
        }
        Ok(())
    }
}

/// The text that makes a model entry of `lines` lines (0: refused as too long).
fn text_of(lines: u8) -> String {
    match lines {
        0 => "o".repeat(STDERR_ENTRY_LINES * STDERR_PIECE_BYTES + 1),
        1 => "one line".to_owned(),
        _ => "t".repeat(STDERR_LINE_BYTES + 1),
    }
}

/// The real queue under replay.
struct Replay<'a> {
    output: &'a StderrOutput,
    rx: Receiver<Entry>,
    steps: Vec<Option<Enqueue<'a>>>,
    sources: Vec<Source>,
    inflight: Option<Entry>,
}

impl<'a> Replay<'a> {
    fn new(output: &'a StderrOutput, rx: Receiver<Entry>, producers: usize) -> Self {
        Self {
            output,
            rx,
            steps: (0..producers).map(|_| None).collect(),
            sources: (0..producers)
                .map(|_| Source {
                    text: text_of(1),
                    bad: Cell::new(false),
                })
                .collect(),
            inflight: None,
        }
    }

    fn step_producer(&mut self, i: usize, busy: bool) {
        let output = self.output;
        let slot = &mut self.steps[i];
        let steps = slot.get_or_insert_with(|| Enqueue::new(output));
        // A busy try-lock: another thread holds the sender lock.
        let guard = busy.then(|| output.tx.lock());
        let more = steps.step(format_args!("{}", self.sources[i]));
        drop(guard);
        if !more {
            *slot = None;
        }
    }

    /// The model's phase of producer `i`, read from the real step machine.
    fn phase(&self, i: usize, model: Phase) -> Phase {
        match self.steps[i].as_ref().map(|s| &s.phase) {
            None | Some(EnqueuePhase::Done) => match model {
                Phase::Start | Phase::Done => model,
                _ => Phase::Start,
            },
            Some(EnqueuePhase::Start) => Phase::Start,
            Some(EnqueuePhase::Cloned(_)) => Phase::Cloned,
            Some(EnqueuePhase::Measured(_, plan)) => Phase::Measured(plan.lines as u8),
            Some(EnqueuePhase::Reserved(_, _, plan)) => Phase::Reserved(plan.lines as u8),
            Some(EnqueuePhase::Held(_, _, entry)) => Phase::Held(entry.lines as u8),
        }
    }
}

fn apply(replay: &mut Replay<'_>, action: Action, model: &State) {
    match action {
        Action::Acquire { i, busy } => {
            // A new entry: a fresh text.
            replay.sources[i as usize].bad.set(false);
            replay.step_producer(i as usize, busy);
        }
        Action::Measure { i, lines } => {
            replay.sources[i as usize].text = text_of(lines);
            replay.step_producer(i as usize, false);
        }
        Action::Reserve { i } | Action::Send { i } => replay.step_producer(i as usize, false),
        Action::Format { i, ok } => {
            replay.sources[i as usize].bad.set(!ok);
            replay.step_producer(i as usize, false);
        }
        Action::Pend { .. } => unreachable!("only the HalfReserve mutation has this step"),
        Action::Recv => {
            let entry = replay.rx.try_recv().expect("the model has a queued entry");
            assert_eq!(
                Some(entry.lines as u8),
                model.chan.first().copied(),
                "entry taken"
            );
            replay.inflight = Some(entry);
        }
        Action::Finish { fail } => {
            let entry = replay.inflight.take().expect("an entry in the writer");
            replay.output.counters.finish(entry, fail);
        }
        Action::WorkerExit => {
            assert!(
                matches!(replay.rx.try_recv(), Err(TryRecvError::Disconnected)),
                "the model says every sender is gone"
            );
        }
        Action::Close => replay.output.close(),
        Action::Settle | Action::Exit => {
            // The writer thread ended (it saw `Disconnected` in WorkerExit), or
            // the process exits: nothing runs in the product.
        }
        Action::GiveUp => {
            let lost = replay.output.give_up();
            // `model` is the state before the step: everything pending is given up.
            assert_eq!(lost, u64::from(model.pending), "entries given up");
        }
        Action::Stuck => unreachable!("the correct model never gets stuck"),
    }
}

/// The real queue equals the model's state.
fn compare(replay: &Replay<'_>, model: &State, after: &str) {
    let counters = &replay.output.counters;
    assert_eq!(
        counters.lines(),
        usize::from(model.queued),
        "queued lines after {after}"
    );
    assert_eq!(
        counters.pending(),
        usize::from(model.pending),
        "pending entries after {after}"
    );
    assert_eq!(
        counters.closed(),
        model.closed,
        "admission closed after {after}"
    );
    assert_eq!(
        counters.dropped.load(Ordering::Relaxed),
        u64::from(model.dropped),
        "drop counter after {after}"
    );
    assert_eq!(
        replay.output.tx.lock().is_some(),
        model.original,
        "original sender after {after}"
    );
    for (i, prod) in model.prods.iter().enumerate().take(replay.steps.len()) {
        assert_eq!(
            replay.phase(i, prod.phase),
            prod.phase,
            "producer {i} after {after}"
        );
    }
    if model.chan.is_empty() && model.inflight.is_none() && !model.worker_exited {
        // Nothing to take: the real receiver is empty, or disconnected exactly
        // when the original sender and every clone are gone.
        let expect_disconnected = !model.original && model.clones() == 0;
        match replay.rx.try_recv() {
            Err(TryRecvError::Empty) => assert!(!expect_disconnected, "after {after}"),
            Err(TryRecvError::Disconnected) => assert!(expect_disconnected, "after {after}"),
            Ok(_) => panic!("the model's channel is empty after {after}"),
        }
    }
    assert_eq!(
        model.shutdown == Shutdown::NotStarted,
        replay.output.tx.lock().is_some(),
        "shutdown phase after {after}"
    );
}

/// Replay `trace` from a fresh queue of `cap` lines, comparing after every
/// step.
fn replay_trace(model: &Model, trace: &[Action], cap: usize) {
    let (output, rx) = StderrOutput::channel_with_cap(cap);
    let mut replay = Replay::new(&output, rx, model.bounds.producers);
    let mut state = model.initial();
    compare(&replay, &state, "the start");
    for (n, action) in trace.iter().enumerate() {
        apply(&mut replay, *action, &state);
        state = model
            .step(&state, action)
            .expect("a trace of the correct model");
        compare(&replay, &state, &format!("step {n}: {action:?}"));
    }
}

/// Replay every reachable state and every enabled action of `profile` on a
/// queue of `cap` lines. Returns the number of replays.
fn conform(profile: &Profile, cap: usize) -> usize {
    let model = Model::new(profile.bounds, Mutation::None);
    let result = explore(
        &model,
        profile.bounds.horizon,
        Limits::default(),
        &Options::default(),
    );
    assert!(
        result.complete && result.failure.is_none(),
        "{}",
        profile.name
    );
    let mut replays = 0;
    let mut enabled = Vec::new();
    for node in 0..result.states {
        let prefix = result.trace_to(node);
        let state = result.state_of(node).clone();
        enabled.clear();
        model.actions(&state, &mut enabled);
        replay_trace(&model, &prefix, cap);
        replays += 1;
        for action in &enabled {
            let mut trace = prefix.clone();
            trace.push(*action);
            replay_trace(&model, &trace, cap);
            replays += 1;
        }
    }
    replays
}

#[test]
fn the_real_queue_follows_every_state_and_edge_of_every_model_profile() {
    for profile in profiles() {
        let replays = conform(&profile, usize::from(profile.bounds.cap));
        assert!(replays > 100, "{}: {replays} replays", profile.name);
    }
}

#[test]
fn the_conformance_check_notices_a_product_that_drifts_from_the_model() {
    // A queue one line larger than the model's cap accepts an entry that the
    // model refuses: some replay of the tight profile must fail.
    let profile = profiles().into_iter().find(|p| p.name == "tight").unwrap();
    let drifted = std::panic::catch_unwind(|| {
        conform(&profile, usize::from(profile.bounds.cap) + 1);
    });
    assert!(drifted.is_err(), "a queue with another cap went unnoticed");
}

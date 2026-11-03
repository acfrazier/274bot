//! Conformance of the compatibility journal read with the hold model
//! (`tools/hold-model`).
//!
//! The real journal runtime (`begin`, `click`, `next`, `close`) and the real
//! `CompatJournalLease` run on traces from the model. After each model frame
//! the test compares the lease's hold (`CompatJournalLease::wait`) and its
//! paint-hide state (`CompatJournalLease::live`) with the model's sample. The
//! wall bounds use `backdate`, so no test sleeps. One model wall unit is 2.5 s,
//! so the product's 10 s bounds are four units. The host's policy for this
//! lease is trivial: its reported deadline is the current tick (`slot.rs`
//! `script_interaction_wait`), so a live read holds and the host never ends it.
//! The host side is covered by `host-play` (`script_hold_conformance_tests`).

use super::*;
use crate::scene::observed::{ModalTexts, QuestStatusRow, QuestTab};
use hold_model::{
    release_traces, Action as ModelAction, Begin, Bounds, Env, Lease, Machine as _, Model,
    Mutation, OwnerAct,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One model wall unit.
const UNIT: Duration = Duration::from_millis(2500);
/// Real ticks start here, so that the first post is a later tick than the reset.
const TICK_BASE: u64 = 100;

fn cook_row() -> QuestStatusRow {
    QuestStatusRow {
        name: "Cook's Assistant".into(),
        status: "notStarted".into(),
        component_id: Some(1234),
    }
}

fn post(tick: u64, root: i32, page: u8) {
    let texts = if root == -1 {
        Vec::new()
    } else {
        vec![format!("@dre@page {page}")]
    };
    observed::post(tick, |post| {
        post.quest_statuses(QuestTab::Bound(vec![cook_row()]));
        post.main_modal_texts(ModalTexts { root, texts });
    });
}

fn bounds() -> Bounds {
    Bounds {
        horizon: 9,
        max_tick: 3,
        max_wall: 5,
        max_ids: 4,
        max_pages: 3,
        max_epochs: 1,
        modal: false,
        quiet: false,
        // The journal's 3 s acquisition window runs on the isolate's own task
        // clock, which `backdate` does not move. The model never reaches it.
        compat_acquire: 6,
        env: Env {
            pause: true,
            stop: true,
            session: true,
            disconnect: false,
            hold: true,
            cleanup: false,
            death: true,
            run_off: false,
            page_change: true,
        },
        ..Bounds::standard()
    }
}

/// One `Family::step` of the quest-journal family, without the verbs it emits.
fn real_step() {
    loop {
        let phase = RUNTIME.with(|rt| {
            let rt = rt.borrow();
            (rt.phase != Phase::Idle).then_some((rt.phase, rt.token, rt.generation))
        });
        let Some((phase, token, generation)) = phase else {
            return;
        };
        match phase {
            Phase::Idle => return,
            Phase::Ready => {
                let _ = RUNTIME.with(|rt| rt.borrow_mut().click());
                return;
            }
            Phase::AwaitingAcquire => {
                let step = RUNTIME.with(|rt| rt.borrow_mut().next(token, generation));
                if step.get("kind").and_then(Value::as_str) == Some("done") {
                    continue;
                }
                return;
            }
            Phase::Acquired | Phase::Closing => {
                let _ = RUNTIME.with(|rt| rt.borrow_mut().close(token, generation));
                return;
            }
        }
    }
}

struct Real {
    lease: Arc<CompatJournalLease>,
    generation: u64,
}

impl Real {
    fn new() -> Self {
        on_reset();
        observed::on_reset();
        let lease = Arc::new(CompatJournalLease::default());
        lease.set_generation(7);
        lease.set_host_generation(7);
        bind_compat_runtime(Arc::clone(&lease));
        post(TICK_BASE, -1, 0);
        Self {
            lease,
            generation: 7,
        }
    }

    /// The connection boundary: `LoadIsolate::reset_session`.
    fn boundary(&mut self, tick: u64) {
        on_reset();
        self.lease.revoke();
        self.lease.set_host_generation(u64::MAX);
        self.generation += 1;
        self.lease.set_generation(self.generation);
        self.lease.set_host_generation(self.generation);
        post(TICK_BASE + tick, -1, 0);
    }
}

/// Drive the real runtime and lease with `trace`; report the first frame at
/// which the hold or the paint-hide state differs from the model's.
fn check_trace(model: &Model, trace: &[ModelAction]) -> Result<(usize, usize), String> {
    let mut real = Real::new();
    let mut state = model.initial();
    let mut boundary_pending = false;
    let (mut held_frames, mut free_frames) = (0, 0);
    for (index, action) in trace.iter().enumerate() {
        let (post_state, info, _) = model.step_unchecked(&state, action);
        let tick = TICK_BASE + u64::from(post_state.tick);
        match action {
            ModelAction::Wait => real.lease.backdate(UNIT),
            ModelAction::Pause => on_pause(),
            ModelAction::Resume => on_resume(),
            ModelAction::HoldOn => on_hold(true),
            ModelAction::HoldOff => on_hold(false),
            ModelAction::Stop => on_stop(),
            ModelAction::Start => {
                real = Real::new();
            }
            ModelAction::Relog => {
                boundary_pending = true;
                post(tick, -1, post_state.page);
            }
            ModelAction::ModalOpen | ModelAction::ModalNext => post(tick, 77, post_state.page),
            ModelAction::ModalClose => post(tick, -1, post_state.page),
            ModelAction::Frame(f) => {
                if boundary_pending {
                    real.boundary(u64::from(post_state.tick));
                    boundary_pending = false;
                }
                if info.polled {
                    let died = info
                        .releases
                        .iter()
                        .any(|(_, cause)| *cause == hold_model::Cause::Death);
                    if died {
                        RUNTIME.with(|rt| rt.borrow_mut().abort());
                    } else {
                        match f.act {
                            OwnerAct::Begin(Begin::Compat) => {
                                let begun = dispatch(&json!({
                                    "op": "begin",
                                    "name": "Cook's Assistant",
                                    "generation": real.generation,
                                }));
                                assert_eq!(begun["kind"], "token", "the read begins: {begun}");
                            }
                            OwnerAct::Poll => real_step(),
                            OwnerAct::Drop => RUNTIME.with(|rt| rt.borrow_mut().abort()),
                            _ => {}
                        }
                    }
                }
                if f.drift {
                    real.lease.backdate(UNIT);
                }
                let now = Instant::now();
                let real_wait = real.lease.wait(real.generation, now);
                let real_hidden = real.lease.live(real.generation, now);
                let model_hidden =
                    matches!(post_state.lease, Lease::Compat(c) if post_state.wall < c.paint_until);
                let model_wait = info.wait.is_some();
                if real_wait.is_some() != model_wait || real_hidden != model_hidden {
                    return Err(format!(
                        "frame {index}: real hold {}, hidden {real_hidden}; model hold {model_wait}, hidden {model_hidden}\ntrace:\n{}",
                        real_wait.is_some(),
                        hold_model::describe_trace(model, trace)
                    ));
                }
                if model_wait {
                    held_frames += 1;
                } else {
                    free_frames += 1;
                }
            }
            _ => {}
        }
        state = post_state;
    }
    Ok((held_frames, free_frames))
}

#[test]
fn the_real_compat_lease_matches_the_model_on_the_shortest_trace_of_each_release() {
    let bounds = bounds();
    let cap = std::env::var("HOLD_CONFORMANCE_TRACES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(6);
    let (_, releases) = release_traces(
        bounds,
        Mutation::None,
        bounds.horizon,
        hold_model::Limits::default(),
        cap,
    );
    let model = Model::new(bounds, Mutation::None);
    let (mut checked, mut held, mut free) = (0, 0, 0);
    let mut failures = Vec::new();
    for ((kind, cause), traces) in &releases {
        eprintln!(
            "compat conformance: {kind:?} {cause:?}: {} shortest traces",
            traces.len()
        );
        for trace in traces {
            checked += 1;
            match check_trace(&model, trace) {
                Ok((h, f)) => {
                    held += h;
                    free += f;
                }
                Err(report) => failures.push(format!("{kind:?} {cause:?}: {report}")),
            }
        }
    }
    eprintln!("compat conformance: {checked} traces; {held} held frames, {free} free frames");
    assert!(held > 0 && free > 0, "the traces exercise both decisions");
    assert!(
        failures.is_empty(),
        "{} of {checked} traces differ:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// Phase 2: the counterexample traces of the earlier compat designs (M4, a
/// constant key; M6, no renewal at the click). The real lease must decide like
/// the head model on them. The test fails on a lease that renews nothing at the
/// click (M6): see `arm-r/report.md`.
#[test]
fn the_counterexamples_of_the_earlier_compat_designs_run_on_the_real_lease() {
    // M4 shows only when the owner begins a new read in the frame that revokes
    // the old one (the product's hook resets the session before the poll).
    let bounds = Bounds {
        same_frame_begin: true,
        ..bounds()
    };
    let head = Model::new(bounds, Mutation::None);
    let mut checked = 0;
    for mutation in [Mutation::M6CompatNoRenew, Mutation::M4CompatConstantKey] {
        let run = hold_model::explore_profile(
            &hold_model::Profile {
                name: "compat",
                bounds,
            },
            mutation,
            Some(bounds.horizon),
            hold_model::Limits::default(),
            true,
        );
        assert!(
            !run.failures.is_empty(),
            "{} has a counterexample",
            mutation.name()
        );
        for (violation, trace) in &run.failures {
            checked += 1;
            if let Err(report) = check_trace(&head, trace) {
                panic!("{} {}: {report}", mutation.name(), violation.name);
            }
        }
    }
    eprintln!("compat conformance: {checked} counterexample traces");
}

//! Conformance of the auto-run hold with its model (`tools/hold-model`).
//!
//! The real host policy (`host::ScriptRunPolicy`), the real lease sampling
//! (`SlotScript::script_interaction_wait`, the ledger, the owner's
//! revocation and the active clock) and the real press decision
//! (`SlotLoop::apply_auto_run`) run on the production frame path
//! (`Host::run_client` with `script_observe_cached_with_channels`). The test
//! drives them with event sequences from the model and compares the hold and
//! press decisions of the real code with the model at every frame.
//!
//! The owner is a test double: the model chooses what the owner does at each
//! poll (begin, send, renew, end, drop) and the double does it through the
//! real lease API (`begin_modal_wait`, `renew_modal_wait`,
//! `begin_quiet_read`, `renew_quiet_read`). The real `Dialogue` and
//! `JournalMachine` phase logic is not in this test. One model time unit is
//! `UNIT_MS` of real time.

use super::*;
use hold_model::{
    describe_trace, release_traces, Action as ModelAction, Begin, Bounds, Cause, Env, Kind, Lease,
    Machine as _, Model, Mutation, OwnerAct,
};
use script::native::{
    ActionContext, ActionError, ActionHandle, ModalWaitLease, NativeMachine, NativeTick,
    QuietReadLease, Script, ScriptFailure, ScriptFlow, WaitDeadline,
};
use std::sync::atomic::AtomicU64;
use std::task::Poll;

/// Real time of one model time unit.
const UNIT_MS: u64 = 150;
/// Real time of one model time unit when a journal quiet lease is live: the
/// lease's wall bound is the product's 10 s, four model units.
const WALL_UNIT_MS: u64 = 2500;

/// What the owner double does at one poll, as the model chose it.
#[derive(Clone, Copy, Debug)]
enum Cmd {
    BeginModal {
        page: Duration,
    },
    BeginQuiet {
        deadline_tick: u64,
    },
    Send {
        ack: Duration,
    },
    ToGap {
        deadline_tick: u64,
    },
    Rearm {
        deadline_tick: u64,
    },
    RenewQuiet {
        deadline_tick: u64,
    },
    /// The machine completes: its leases end.
    End,
    /// The script cancels its step: the handle drops.
    Drop,
}

#[derive(Default)]
struct Control {
    /// The command for the next script tick.
    cmd: Mutex<Option<Cmd>>,
    /// The command a live machine runs in its next poll.
    pending: Mutex<Option<Cmd>>,
    /// Real milliseconds that pass at the end of the script tick.
    drift_ms: AtomicU64,
}

struct DrivenMachine {
    control: Arc<Control>,
    modal: Option<ModalWaitLease>,
    quiet: Option<QuietReadLease>,
}

#[derive(Clone)]
struct DrivenArgs {
    control: Arc<Control>,
    begin: Cmd,
}

impl NativeMachine for DrivenMachine {
    type Args = DrivenArgs;
    type Output = ();

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let mut machine = Self {
            control: args.control,
            modal: None,
            quiet: None,
        };
        match args.begin {
            Cmd::BeginModal { page } => {
                machine.modal =
                    Some(cx.begin_modal_wait(WaitDeadline::Active(cx.active_now() + page))?);
            }
            Cmd::BeginQuiet { deadline_tick } => {
                machine.quiet =
                    Some(cx.begin_quiet_read(cx.action_id(), WaitDeadline::Tick(deadline_tick))?);
            }
            other => panic!("not a begin command: {other:?}"),
        }
        Ok(machine)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        let Some(cmd) = self.control.pending.lock().unwrap().take() else {
            return Poll::Pending;
        };
        match cmd {
            Cmd::Send { ack } => {
                // A new send takes a new lease; the old one ends after.
                match cx.begin_modal_wait(WaitDeadline::Active(cx.active_now() + ack)) {
                    Ok(lease) => self.modal = Some(lease),
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            Cmd::ToGap { deadline_tick } => {
                if let Some(lease) = &self.modal {
                    cx.renew_modal_wait(lease, WaitDeadline::Tick(deadline_tick));
                }
            }
            Cmd::Rearm { deadline_tick } => {
                match cx.begin_modal_wait(WaitDeadline::Tick(deadline_tick)) {
                    Ok(lease) => self.modal = Some(lease),
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            Cmd::RenewQuiet { deadline_tick } => {
                if let Some(lease) = &self.quiet {
                    cx.renew_quiet_read(lease, WaitDeadline::Tick(deadline_tick));
                }
            }
            Cmd::End => {
                self.modal = None;
                if let Some(lease) = self.quiet.take() {
                    cx.end_quiet_read(lease);
                }
                return Poll::Ready(Ok(()));
            }
            other => panic!("not a poll command: {other:?}"),
        }
        Poll::Pending
    }
}

struct DrivenScript {
    control: Arc<Control>,
    handle: Option<ActionHandle<DrivenMachine>>,
}

impl Script for DrivenScript {
    fn tick(&mut self, ctx: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let cmd = self.control.cmd.lock().unwrap().take();
        match cmd {
            Some(Cmd::Drop) => self.handle = None,
            Some(begin @ (Cmd::BeginModal { .. } | Cmd::BeginQuiet { .. })) => {
                self.handle = None;
                self.handle = Some(
                    ctx.actions
                        .begin::<DrivenMachine>(
                            DrivenArgs {
                                control: Arc::clone(&self.control),
                                begin,
                            },
                            &mut ctx.cx,
                        )
                        .expect("the owner double begins its wait"),
                );
            }
            Some(other) => *self.control.pending.lock().unwrap() = Some(other),
            None => {}
        }
        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = ctx.actions.poll(handle, &mut ctx.cx) {
                // A revoked owner (Stop, a session boundary) fails its poll.
                match result {
                    Ok(()) | Err(ActionError::Stale | ActionError::Cancelled) => {}
                    Err(error) => panic!("the owner double's command failed: {error:?}"),
                }
                self.handle = None;
            }
        }
        let drift = self.control.drift_ms.load(Ordering::Relaxed);
        if drift > 0 {
            thread::sleep(Duration::from_millis(drift));
        }
        Ok(ScriptFlow::Continue)
    }
}

/// One thing the operator, the connection or time does before a frame.
#[derive(Clone, Copy, Debug)]
enum Op {
    Wait,
    Pause,
    Resume,
    Stop,
    Start,
    Relog,
}

/// One host frame, as the model describes it.
#[derive(Clone, Debug)]
struct RealFrame {
    ops: Vec<Op>,
    /// The drain carries a PLAYER_INFO.
    tick: bool,
    /// Packet families are dirty: a wake polls without a tick.
    wake: bool,
    modal: bool,
    hold: bool,
    cmd: Option<Cmd>,
    drift_ms: u64,
}

/// What the real code decided at one frame, and what the model says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Decision {
    /// The host holds the press after this frame's sample.
    deferred: bool,
    /// The host pressed run in this frame.
    pressed: bool,
}

struct Rig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: EmptyNav,
    names: Arc<api::obj_names::ObjNames>,
    snapshot: Arc<GameSnapshot>,
    client: Client,
    control: Arc<Control>,
}

impl Rig {
    fn new() -> Self {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let mut client = bank_fetch_client();
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        client.main_modal_id = -1;
        client.chat_modal_id = -1;
        client.side_modal_id = -1;
        client.runenergy = 20;
        let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
        let control = Arc::new(Control::default());
        script_slot_or_insert(&scripts, "alice")
            .lock()
            .unwrap()
            .start_test_script(
                Box::new(DrivenScript {
                    control: Arc::clone(&control),
                    handle: None,
                }),
                None,
            )
            .expect("native script starts");
        Self {
            scripts,
            cheats: Arc::new(Mutex::new(HashMap::new())),
            navs: empty_nav(),
            names,
            snapshot: Arc::new(snapshot),
            client,
            control,
        }
    }

    /// Run `frames` through one `Host::run_client` call and report the real
    /// decision at each of them.
    fn run(&mut self, unit_ms: u64, frames: &[RealFrame]) -> Vec<Decision> {
        // One empty frame after the last shows the last frame's press.
        let mut plan = frames.to_vec();
        plan.push(RealFrame {
            ops: Vec::new(),
            tick: false,
            wake: false,
            modal: frames.last().is_some_and(|last| last.modal),
            hold: frames.last().is_some_and(|last| last.hold),
            cmd: None,
            drift_ms: 0,
        });
        let plan = Arc::new(plan);
        let index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen: Arc<Mutex<Vec<(u32, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let tick_edge = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(AtomicBool::new(false));
        let hold = Arc::new(AtomicBool::new(false));
        let boundary = Arc::new(AtomicBool::new(false));
        let script_tick = Arc::new(AtomicU64::new(0));

        let observe_seen = Arc::clone(&seen);
        let observe_index = Arc::clone(&index);
        let observe_edge = Arc::clone(&tick_edge);
        let observe_wake = Arc::clone(&wake);
        let observe_hold = Arc::clone(&hold);
        let observe_boundary = Arc::clone(&boundary);
        let observe_tick = Arc::clone(&script_tick);
        let observe_scripts = Arc::clone(&self.scripts);
        let observe_cheats = Arc::clone(&self.cheats);
        let (observe_navs, observe_world) = (Arc::clone(&self.navs.0), self.navs.1.clone());
        let observe_names = Arc::clone(&self.names);
        let observe_snapshot = Arc::clone(&self.snapshot);
        let observe_cache = Arc::clone(&self.client.cache);

        let probe_plan = Arc::clone(&plan);
        let probe_index = Arc::clone(&index);
        let probe_edge = Arc::clone(&tick_edge);
        let probe_wake = Arc::clone(&wake);
        let probe_hold = Arc::clone(&hold);
        let probe_boundary = Arc::clone(&boundary);
        let probe_scripts = Arc::clone(&self.scripts);
        let probe_control = Arc::clone(&self.control);

        let _final_policy = Host::run_client(
            &mut self.client,
            "alice",
            vault::ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            None,
            None,
            None,
            host::ScriptRunPolicy::default(),
            move |c, _, run_sends, _, run_policy| {
                let edge = observe_edge.load(Ordering::Relaxed);
                let tick = if edge {
                    observe_tick.fetch_add(1, Ordering::Relaxed) + 1
                } else {
                    observe_tick.load(Ordering::Relaxed)
                };
                if observe_boundary.swap(false, Ordering::Relaxed) {
                    // `reset_slot_session_boundary`, before the frame's observe.
                    script_slot(&observe_scripts, "alice")
                        .unwrap()
                        .lock()
                        .unwrap()
                        .reset_session_work();
                }
                let inv: [(i32, i32); 0] = [];
                let held = observe_hold.load(Ordering::Relaxed);
                script_observe_cached_with_channels(
                    c,
                    "alice",
                    true,
                    edge,
                    observe_wake.load(Ordering::Relaxed),
                    tick,
                    Some((3205, 3205, 0)),
                    Some(&inv),
                    None,
                    Some(observe_snapshot.as_ref()),
                    None,
                    None,
                    Some(observe_names.as_ref()),
                    &observe_scripts,
                    &observe_cheats,
                    &observe_navs,
                    &observe_world,
                    held,
                    held,
                    false,
                    None,
                    None,
                    Some(Arc::clone(&observe_cache)),
                    Some(Arc::clone(&observe_names)),
                    None,
                    script_channels::BrokerWorld::Unavailable,
                    Some(&mut *run_policy),
                    None,
                    None,
                    None,
                );
                observe_seen
                    .lock()
                    .unwrap()
                    .push((run_sends, run_policy.should_defer_auto_run()));
                observe_index.fetch_add(1, Ordering::Relaxed);
                true
            },
            move |c| {
                let Some(step) = probe_plan.get(probe_index.load(Ordering::Relaxed)) else {
                    return true;
                };
                for op in &step.ops {
                    match op {
                        Op::Wait => thread::sleep(Duration::from_millis(unit_ms)),
                        Op::Pause => script_slot(&probe_scripts, "alice")
                            .unwrap()
                            .lock()
                            .unwrap()
                            .pause(),
                        Op::Resume => script_slot(&probe_scripts, "alice")
                            .unwrap()
                            .lock()
                            .unwrap()
                            .resume(),
                        Op::Stop => script_slot(&probe_scripts, "alice")
                            .unwrap()
                            .lock()
                            .unwrap()
                            .stop(),
                        Op::Start => script_slot(&probe_scripts, "alice")
                            .unwrap()
                            .lock()
                            .unwrap()
                            .start_test_script(
                                Box::new(DrivenScript {
                                    control: Arc::clone(&probe_control),
                                    handle: None,
                                }),
                                None,
                            )
                            .expect("the script starts again"),
                        Op::Relog => {
                            c.gens.session += 1;
                            c.gens.player += 1;
                            probe_boundary.store(true, Ordering::Relaxed);
                        }
                    }
                }
                c.gens.player_info += u64::from(step.tick);
                if step.tick {
                    c.gens.player += 1;
                }
                c.main_modal_id = if step.modal { 8134 } else { -1 };
                probe_edge.store(step.tick, Ordering::Relaxed);
                probe_wake.store(step.wake, Ordering::Relaxed);
                probe_hold.store(step.hold, Ordering::Relaxed);
                *probe_control.cmd.lock().unwrap() = step.cmd;
                probe_control
                    .drift_ms
                    .store(step.drift_ms, Ordering::Relaxed);
                false
            },
            |_: &api::DetectedRandom| RandomClaim::Host,
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), frames.len() + 1, "every frame was observed");
        // `seen[i + 1].0` is the press count when frame `i + 1` began: frame
        // `i`'s press is its difference to `seen[i].0`.
        (0..frames.len())
            .map(|i| Decision {
                deferred: seen[i].1,
                pressed: seen[i + 1].0 > seen[i].0,
            })
            .collect()
    }
}

/// The frames of a model trace, and the decision that the model expects at
/// each frame.
fn frames_of(
    model: &Model,
    trace: &[ModelAction],
    unit_ms: u64,
) -> (Vec<RealFrame>, Vec<Decision>) {
    let bounds = model.bounds;
    let unit = Duration::from_millis(unit_ms);
    let mut state = model.initial();
    let mut ops = Vec::new();
    let mut frames = Vec::new();
    let mut expected = Vec::new();
    for action in trace {
        // A counterexample trace ends in a violating step: its state and
        // decision still count.
        let (post, info, _) = model.step_unchecked(&state, action);
        match action {
            ModelAction::Wait => ops.push(Op::Wait),
            ModelAction::Pause => ops.push(Op::Pause),
            ModelAction::Resume => ops.push(Op::Resume),
            ModelAction::Stop => ops.push(Op::Stop),
            ModelAction::Start => ops.push(Op::Start),
            ModelAction::Relog => ops.push(Op::Relog),
            ModelAction::Frame(f) => {
                let died = info
                    .releases
                    .iter()
                    .any(|(_, cause)| *cause == Cause::Death);
                let cmd = if died {
                    Some(Cmd::Drop)
                } else if !info.polled {
                    None
                } else {
                    match f.act {
                        OwnerAct::NoPoll | OwnerAct::Stall => None,
                        OwnerAct::Begin(Begin::Modal) => Some(Cmd::BeginModal {
                            page: unit * u32::from(bounds.page_active),
                        }),
                        OwnerAct::Begin(Begin::Quiet) => match post.lease {
                            Lease::Quiet(q) => Some(Cmd::BeginQuiet {
                                deadline_tick: u64::from(q.window),
                            }),
                            _ => None,
                        },
                        OwnerAct::Begin(Begin::Compat) => None,
                        OwnerAct::Poll => match (state.lease, post.lease) {
                            (Lease::None, _) => None,
                            (_, Lease::None) => Some(Cmd::End),
                            (Lease::Quiet(before), Lease::Quiet(after))
                                if before.window != after.window =>
                            {
                                Some(Cmd::RenewQuiet {
                                    deadline_tick: u64::from(after.window),
                                })
                            }
                            _ => None,
                        },
                        OwnerAct::Send => Some(Cmd::Send {
                            ack: unit * u32::from(bounds.ack_active),
                        }),
                        OwnerAct::ToGap(_) => match post.lease {
                            Lease::Modal(m) => match m.ledger {
                                hold_model::LedgerBound::Tick(deadline_tick) => Some(Cmd::ToGap {
                                    deadline_tick: u64::from(deadline_tick),
                                }),
                                hold_model::LedgerBound::Active(_) => None,
                            },
                            _ => None,
                        },
                        OwnerAct::Rearm(_) => match post.lease {
                            Lease::Modal(m) => match m.ledger {
                                hold_model::LedgerBound::Tick(deadline_tick) => Some(Cmd::Rearm {
                                    deadline_tick: u64::from(deadline_tick),
                                }),
                                hold_model::LedgerBound::Active(_) => None,
                            },
                            _ => None,
                        },
                        OwnerAct::Drop => Some(Cmd::Drop),
                    }
                };
                frames.push(RealFrame {
                    ops: std::mem::take(&mut ops),
                    tick: f.tick,
                    wake: f.wake,
                    modal: state.modal,
                    hold: state.hold,
                    cmd,
                    drift_ms: if f.drift { unit_ms } else { 0 },
                });
                expected.push(Decision {
                    deferred: info.deferred,
                    pressed: info.pressed,
                });
            }
            _ => {}
        }
        state = post;
    }
    (frames, expected)
}

/// Whether the model's trace keeps a journal quiet lease for a wall time that
/// reaches its 10 s bound.
fn needs_wall_scale(model: &Model, trace: &[ModelAction]) -> bool {
    let mut state = model.initial();
    for action in trace {
        let (post, _, _) = model.step_unchecked(&state, action);
        if let Lease::Quiet(q) = post.lease {
            if post.wall.saturating_sub(q.since) + 1 >= model.bounds.quiet_wall {
                return true;
            }
        }
        state = post;
    }
    false
}

/// Drive the real code with `trace` and compare it with the model at every
/// frame. Returns the number of frames where the model holds the press and
/// where it presses, or a report of the first frame that differs.
fn check_trace(model: &Model, trace: &[ModelAction]) -> Result<(usize, usize), String> {
    let unit_ms = if needs_wall_scale(model, trace) {
        WALL_UNIT_MS
    } else {
        UNIT_MS
    };
    let (frames, expected) = frames_of(model, trace, unit_ms);
    let real = Rig::new().run(unit_ms, &frames);
    for (i, (real, expected)) in real.iter().zip(&expected).enumerate() {
        if real != expected {
            return Err(format!(
                "frame {i}: real {real:?}, model {expected:?}\ntrace:\n{}",
                describe_trace(model, trace)
            ));
        }
    }
    Ok((
        expected.iter().filter(|d| d.deferred).count(),
        expected.iter().filter(|d| d.pressed).count(),
    ))
}

/// The conformance bounds: the native lease kinds, the events that a test
/// can drive, and drift only where the owner polls.
fn conformance_bounds() -> Bounds {
    Bounds {
        horizon: 8,
        max_tick: 3,
        max_wall: 4,
        max_ids: 4,
        max_pages: 3,
        max_epochs: 1,
        compat: false,
        env: Env {
            pause: true,
            stop: true,
            session: true,
            disconnect: false,
            hold: true,
            cleanup: false,
            death: true,
            run_off: false,
            page_change: false,
        },
        drift_polls_only: true,
        ..Bounds::standard()
    }
}

fn traces_per_release() -> usize {
    std::env::var("HOLD_CONFORMANCE_TRACES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3)
}

#[test]
fn the_real_hold_matches_the_model_on_the_shortest_trace_of_each_release() {
    let bounds = conformance_bounds();
    let (_, releases) = release_traces(
        bounds,
        Mutation::None,
        bounds.horizon,
        hold_model::Limits::default(),
        traces_per_release(),
    );
    let model = Model::new(bounds, Mutation::None);
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut slow = 0;
    let (mut held_frames, mut press_frames) = (0, 0);
    for ((kind, cause), traces) in &releases {
        for trace in traces {
            if needs_wall_scale(&model, trace) {
                slow += 1;
                if slow > 2 {
                    continue;
                }
            }
            checked += 1;
            match check_trace(&model, trace) {
                Ok((held, pressed)) => {
                    held_frames += held;
                    press_frames += pressed;
                }
                Err(report) => failures.push(format!("{kind:?} {cause:?}: {report}")),
            }
        }
    }
    for ((kind, cause), traces) in &releases {
        eprintln!(
            "conformance: {kind:?} {cause:?}: {} shortest traces",
            traces.len()
        );
    }
    eprintln!(
        "conformance: {checked} traces over {} releases; the model held the press on {held_frames} frames and pressed on {press_frames}",
        releases.len()
    );
    assert!(
        held_frames > 0 && press_frames > 0,
        "the traces exercise both decisions"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} traces differ:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
    for kind in [Kind::Modal, Kind::Quiet] {
        assert!(
            releases.iter().any(|((k, _), _)| *k == kind),
            "the model reaches releases of {kind:?}"
        );
    }
}

/// Defect F1 of `HOLD-MODEL-AB`, fixed. An acknowledgement wait that the
/// owner leaves unpolled past its bound is ended by the host, and the end gap
/// that follows is the same lease renewed to a later deadline. The earlier
/// host (`Mutation::F1RenewalKeepsRelease`, the product at `0a3e700f8`) did
/// not hold that renewed wait: its counterexample ends in a press while the
/// gap lease is live. The head host holds it. The real code must decide like
/// the head model at every frame, and the trace must separate the two hosts
/// at its last frame. The test fails on the earlier host code: the real code
/// then presses where the head model holds.
#[test]
fn the_renewed_end_gap_is_held_on_the_real_code_where_the_earlier_host_pressed() {
    let bounds = Bounds {
        quiet: false,
        ..conformance_bounds()
    };
    let run = hold_model::explore_profile(
        &hold_model::Profile {
            name: "conformance-modal",
            bounds,
        },
        Mutation::F1RenewalKeepsRelease,
        Some(bounds.horizon),
        hold_model::Limits::default(),
        true,
    );
    let head = Model::new(bounds, Mutation::None);
    let earlier = Model::new(bounds, Mutation::F1RenewalKeepsRelease);
    for name in ["S1-press-while-live", "S4-released-state-carried"] {
        let (_, trace) = run
            .failures
            .iter()
            .find(|(violation, _)| violation.name == name)
            .unwrap_or_else(|| panic!("the earlier host fails {name}"));
        let (_, head_expected) = frames_of(&head, trace, UNIT_MS);
        let (_, earlier_expected) = frames_of(&earlier, trace, UNIT_MS);
        let last = head_expected.len() - 1;
        assert!(
            earlier_expected[last].pressed && !earlier_expected[last].deferred,
            "{name}: the earlier host presses at the last frame"
        );
        assert!(
            head_expected[last].deferred && !head_expected[last].pressed,
            "{name}: the head host holds the renewed wait at the last frame"
        );
        check_trace(&head, trace).unwrap_or_else(|report| {
            panic!("{name}: the real code differs from the head model: {report}")
        });
    }
}

/// Phase 2: the counterexample traces of the earlier designs. A mutation of
/// the model finds each trace. The real head code must decide like the head
/// model on it, so that the trace is a real event sequence and not a model
/// artifact. The same traces fail on product code that has the earlier
/// design: the demonstration is in `arm-r/report.md`.
#[test]
fn the_counterexamples_of_the_earlier_designs_run_on_the_real_code_like_the_head_model() {
    let bounds = conformance_bounds();
    let head = Model::new(bounds, Mutation::None);
    let mut checked = 0;
    let mut failures = Vec::new();
    for mutation in [
        Mutation::M1HoldTwoTicks,
        Mutation::M2HoldThreeTicks,
        Mutation::M3FixedTerminal,
        Mutation::M5ActiveToWall,
    ] {
        let run = hold_model::explore_profile(
            &hold_model::Profile {
                name: "conformance",
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
            if needs_wall_scale(&head, trace) {
                continue;
            }
            checked += 1;
            if let Err(report) = check_trace(&head, trace) {
                failures.push(format!("{} {}: {report}", mutation.name(), violation.name));
            }
        }
    }
    eprintln!("conformance: {checked} counterexample traces of the earlier designs");
    assert!(
        failures.is_empty(),
        "{} traces differ:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

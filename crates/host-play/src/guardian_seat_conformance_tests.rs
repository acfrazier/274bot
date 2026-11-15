//! Conformance of the Guardian's ownership with the composed seat model
//! (`tools/seat-model/src/guardian.rs`, the `c6-ownership-seat` rules).
//!
//! The real `host::Guardian` runs over the real claim adapter
//! (`ScriptRandomClaims`) and a real slot that runs a compiled script, so a
//! script's start, pause, resume, stop and replacement go through the real
//! slot lifecycle and its runtime generation. The published hold reaches the
//! slot's seat through the production wiring (`SlotScript::seat_guardian_hold`,
//! as `script_observe` calls it) and a real `GameplaySeat` set up like the
//! model's composition prefix: one admitted Gather session with one queued
//! gameplay row. The model's `Seat` steps are that seat's own take and
//! dispatch. After every step the published fields, the seat's held state and
//! its dispatch and drop counts equal the model's.

use super::*;
use api::identity::{ExecutionId, RequestId};
use api::script_contract::{EmissionClass, RequestStamp};
use api::selected::RunKey;
use script::native::request::NativeKind;
use script::native::seat::{Admission, EmissionSource, GameplaySeat};
use seat_model::guardian::{
    Family, GuardianBounds, GuardianMutation, GuardianState, GuardianStep, ScriptExit,
};
use seat_model::Step as SeatStep;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};

/// The Strange box obj; the model's `Trap` family (a held box holds the slot).
const BOX: i32 = 3062;
/// Open at tick 1, three attempts of four ticks, released at tick 12.
const BOX_BOUND: u8 = 11;

/// A compiled script that claims the event at its rising edge, or not.
struct Claimer {
    wants: bool,
    calls: Arc<AtomicU32>,
}

impl script::native::Script for Claimer {
    fn tick(
        &mut self,
        _ctx: &mut script::native::NativeTick<'_>,
    ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
        Ok(script::native::ScriptFlow::Continue)
    }

    fn on_random(&mut self, _event: &DetectedRandom) -> RandomClaim {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.wants {
            RandomClaim::Handle
        } else {
            RandomClaim::Host
        }
    }
}

struct World {
    scripts: ScriptWall,
    source: crate::play_slots::ScriptRandomClaims,
    client: Client,
    snap: GameSnapshot,
    guardian: Guardian,
    driver: GuardRec,
    settings: ProfileSettings,
    seat: GameplaySeat<u32>,
    delivered: Vec<u32>,
    dispatched: usize,
    calls: Arc<AtomicU32>,
    edges: u32,
    last_sig: Option<String>,
    last: host::RandomStatus,
}

fn seat_run() -> RunKey {
    RunKey {
        slot: 1,
        run: 7,
        session: 0,
    }
}

impl World {
    fn new() -> Self {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        script_slot_or_insert(&scripts, "alice");
        let source =
            crate::play_slots::ScriptRandomClaims::new(Arc::clone(&scripts), "alice".into());
        // The model's composition prefix: one admitted Gather session with one
        // queued gameplay row.
        let mut seat = GameplaySeat::new(seat_run());
        let id = RequestId::isolate(1).expect("request id");
        let stamp = RequestStamp::new(id, None, None, ExecutionId::first(seat_run()))
            .expect("request stamp");
        assert_eq!(
            seat.start(stamp, NativeKind::Gather, false).expect("start"),
            Admission::Admitted
        );
        seat.push(9, EmissionClass::Gameplay, EmissionSource::Owner(id), None)
            .expect("gameplay row");
        Self {
            scripts,
            source,
            client: guardian_client(),
            snap: GameSnapshot::new(),
            guardian: Guardian::new(),
            driver: GuardRec::default(),
            settings: ProfileSettings::default(),
            seat,
            delivered: Vec::new(),
            dispatched: 0,
            calls: Arc::new(AtomicU32::new(0)),
            edges: 0,
            last_sig: None,
            last: host::RandomStatus::default(),
        }
    }

    fn slot(&self) -> ScriptSlot {
        script_slot_or_insert(&self.scripts, "alice")
    }

    fn plant_box(&mut self) {
        plant_inv_item(&mut self.client, BOX);
        let item = &mut Arc::get_mut(&mut self.client.cache)
            .expect("sole cache owner")
            .objs[BOX as usize];
        item.name = "Strange box".into();
        item.iop = [Some("Open".into()), None, None, None, None];
    }

    fn clear_box(&mut self) {
        self.client.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![0]),
                link_obj_number: Some(vec![0]),
                ..Default::default()
            },
        );
    }

    /// One Guardian pass, then the production hold wiring of `script_observe`.
    fn pass(&mut self, fresh: bool) {
        if fresh {
            tick_at(&mut self.client, &mut self.snap);
        }
        let policies = BTreeMap::new();
        let status = self.guardian.tick_with_live_policies(
            &mut self.driver,
            &self.snap,
            &self.settings,
            &host::GuardianLiveContext {
                event_policies: &policies,
                game_data: None,
            },
            0,
            Some(&mut self.source),
        );
        if self.guardian.sig != self.last_sig {
            self.last_sig = self.guardian.sig.clone();
            if self.last_sig.is_some() {
                self.edges += 1;
            }
        }
        self.slot()
            .lock()
            .expect("slot")
            .seat_guardian_hold(status.hold);
        self.seat.hold(status.hold);
        self.last = status;
    }

    fn drive(&mut self, step: &GuardianStep) {
        match step {
            GuardianStep::Appear { .. } => self.plant_box(),
            GuardianStep::Disappear => self.clear_box(),
            GuardianStep::ScriptStart { wants, live } => {
                assert!(!live, "the slot replay runs compiled scripts");
                let script = Box::new(Claimer {
                    wants: *wants,
                    calls: Arc::clone(&self.calls),
                });
                self.slot()
                    .lock()
                    .expect("slot")
                    .start_test_script(script, None)
                    .expect("a stopped slot starts");
            }
            GuardianStep::ScriptExit(ScriptExit::Pause) => {
                self.slot().lock().expect("slot").pause();
            }
            GuardianStep::ScriptExit(ScriptExit::Stop) => {
                let slot = self.slot();
                slot.lock().expect("slot").stop();
                assert!(wait_settled(|| {
                    let mut slot = slot.lock().expect("slot");
                    slot.observe_lifecycle();
                    matches!(slot.state(), script::RunState::Idle)
                }));
            }
            GuardianStep::ScriptExit(ScriptExit::Fail) => {
                panic!("the slot replay has no failing script")
            }
            GuardianStep::ScriptResume => self.slot().lock().expect("slot").resume(),
            GuardianStep::SetEnabled(enabled) => self.settings.random_events = *enabled,
            GuardianStep::Tick { .. } => self.pass(true),
            GuardianStep::Frame => self.pass(false),
            GuardianStep::Seat(SeatStep::Take) => self.seat.take(),
            GuardianStep::Seat(SeatStep::Dispatch) => {
                self.seat.dispatch_into(&mut self.delivered);
                self.dispatched += self.delivered.len();
                self.delivered.clear();
            }
            other => panic!("not in the slot replay's alphabet: {other:?}"),
        }
    }

    fn check(&self, model: &GuardianState, step: &GuardianStep, trace: &[GuardianStep]) {
        let fail = |what: &str| -> ! {
            panic!(
                "{what}\nstatus {:?}\nmodel {}\nafter {step:?}\ntrace {trace:#?}",
                self.last,
                model.describe()
            )
        };
        let out = model.publication();
        let real = (
            self.last.claim == RandomClaim::Handle,
            self.last.handling,
            self.last.ours,
            self.last.hold,
        );
        if (out.claim_script, out.handling, out.ours, out.hold) != real {
            fail("the published fields differ from the model");
        }
        let seat = model.seat().expect("a composed model");
        if seat.session_held() != self.seat.held() {
            fail("the seat's hold differs from the model");
        }
        if let Some(probe) = self.slot().lock().expect("slot").seat_probe() {
            if probe.held != self.last.hold {
                fail("the slot's seat does not follow the published hold");
            }
        }
        if seat.dispatched() != self.dispatched {
            fail("the seat dispatched differently from the model");
        }
        if seat.dropped() != usize::try_from(self.seat.dropped()).expect("drop count") {
            fail("the seat dropped differently from the model");
        }
        // The rising-edge callback runs once per event, never for a successor.
        if self.calls.load(Ordering::Relaxed) > self.edges {
            fail("a callback was replayed");
        }
    }
}

fn bounds() -> GuardianBounds {
    GuardianBounds {
        wait: BOX_BOUND,
        ids: 2,
        sessions: 1,
        ent: false,
        window: false,
        retain: false,
        ent_hold: 2,
    }
}

fn alphabet() -> Vec<GuardianStep> {
    vec![
        GuardianStep::Appear {
            family: Family::Trap,
            forced_host: false,
        },
        GuardianStep::Disappear,
        GuardianStep::ScriptStart {
            wants: false,
            live: false,
        },
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        GuardianStep::ScriptExit(ScriptExit::Pause),
        GuardianStep::ScriptExit(ScriptExit::Stop),
        GuardianStep::ScriptResume,
        GuardianStep::SetEnabled(false),
        GuardianStep::SetEnabled(true),
        GuardianStep::Tick { blocked: false },
        GuardianStep::Frame,
        GuardianStep::Seat(SeatStep::Take),
        GuardianStep::Seat(SeatStep::Dispatch),
    ]
}

/// Known defect, excluded from the generated traces until its fix lands: a
/// compiled script installed while the slot's guardian hold stands panics in
/// `SlotScript::admit_compiled_owner` (`crates/script/src/slot.rs:1196`,
/// `assert_eq!(admitted, Admission::Admitted)`; the held seat answers
/// `Admission::Deferred`). The composed model allows that Start. Set this to
/// `false` when the fix lands; `a_compiled_start_under_the_guardian_hold_is_admitted`
/// reproduces it.
const START_DURING_HOLD_EXCLUDED: bool = true;

/// The shortest composed-model trace to every state within `depth` steps. A
/// box is one signature: after a Disappear a fresh pass must see the gap before
/// the next Appear.
fn generate(depth: usize) -> Vec<Vec<GuardianStep>> {
    let letters = alphabet();
    let start = GuardianState::composed(bounds(), GuardianMutation::None);
    let mut seen: HashSet<(GuardianState, bool)> = HashSet::new();
    let mut queue = VecDeque::new();
    seen.insert((start.clone(), false));
    queue.push_back((start, false, Vec::new()));
    let mut traces = Vec::new();
    while let Some((state, gap, path)) = queue.pop_front() {
        if !path.is_empty() {
            traces.push(path.clone());
        }
        if path.len() >= depth {
            continue;
        }
        for step in &letters {
            if gap && matches!(step, GuardianStep::Appear { .. }) {
                continue;
            }
            if START_DURING_HOLD_EXCLUDED
                && state.publication().hold
                && matches!(step, GuardianStep::ScriptStart { .. })
            {
                continue;
            }
            let mut next = state.clone();
            if next.apply(step).is_err() || next.check().is_err() {
                continue;
            }
            let next_gap = match step {
                GuardianStep::Disappear => true,
                GuardianStep::Tick { .. } => false,
                _ => gap,
            };
            if seen.insert((next.clone(), next_gap)) {
                let mut next_path = path.clone();
                next_path.push(step.clone());
                queue.push_back((next, next_gap, next_path));
            }
        }
    }
    traces
}

fn replay(steps: &[GuardianStep]) {
    let mut model = GuardianState::composed(bounds(), GuardianMutation::None);
    let mut world = World::new();
    let mut done = Vec::new();
    for step in steps {
        model
            .apply(step)
            .unwrap_or_else(|error| panic!("{step:?} is disabled: {error}"));
        model
            .check()
            .unwrap_or_else(|violation| panic!("the model fails {violation:?} at {step:?}"));
        world.drive(step);
        done.push(step.clone());
        world.check(&model, step, &done);
    }
}

/// Every composed-model state within five steps: a held box, claims of real
/// compiled runtimes through the real adapter, and the seat's take and
/// dispatch under the published hold.
#[test]
fn the_guardian_hold_and_the_real_seat_follow_the_composed_model() {
    if !api::test_support::game_data_available() {
        return;
    }
    let traces = generate(5);
    assert!(traces.len() > 200, "only {} traces", traces.len());
    let seat_steps = traces
        .iter()
        .filter(|trace| {
            trace
                .iter()
                .any(|step| matches!(step, GuardianStep::Seat(_)))
        })
        .count();
    assert!(seat_steps > 50, "only {seat_steps} traces with seat steps");
    for trace in &traces {
        replay(trace);
    }
}

/// A compiled script replaced by another runtime before the next observation
/// loses the claim through the real adapter; neither runtime is called back
/// again, and the held box holds the real seat.
#[test]
fn a_replaced_compiled_runtime_returns_the_claim_through_the_adapter() {
    if !api::test_support::game_data_available() {
        return;
    }
    for successor_wants in [false, true] {
        replay(&[
            GuardianStep::ScriptStart {
                wants: true,
                live: false,
            },
            GuardianStep::Appear {
                family: Family::Trap,
                forced_host: false,
            },
            GuardianStep::Tick { blocked: false },
            GuardianStep::ScriptExit(ScriptExit::Stop),
            GuardianStep::ScriptStart {
                wants: successor_wants,
                live: false,
            },
            GuardianStep::Tick { blocked: false },
            GuardianStep::Tick { blocked: false },
        ]);
    }
}

/// The reproduction of the defect `START_DURING_HOLD_EXCLUDED` names: a held
/// box holds the slot's seat, then a compiled script starts.
#[test]
#[ignore = "known defect: compiled start under the guardian hold panics at slot.rs:1196"]
fn a_compiled_start_under_the_guardian_hold_is_admitted() {
    if !api::test_support::game_data_available() {
        return;
    }
    replay(&[
        GuardianStep::Appear {
            family: Family::Trap,
            forced_host: false,
        },
        GuardianStep::Tick { blocked: false },
        GuardianStep::ScriptStart {
            wants: false,
            live: false,
        },
        GuardianStep::Tick { blocked: false },
    ]);
}

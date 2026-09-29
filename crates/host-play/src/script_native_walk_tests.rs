//! Typed native walks through the real host pump: `script_observe`'s receipt
//! delivery and native drain, the off-pump route worker and `step_nav_bot`.
use super::*;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkEnd, WalkReceipt,
    WalkRequest,
};
use std::task::Poll;

#[derive(Default)]
struct Walker {
    /// Return `Blocked` from every tick while set.
    blocked: bool,
    /// On the next tick, cancel the walk and walk the same tile again.
    rewalk: bool,
    begun: usize,
    result: Option<Result<WalkReceipt, ActionError>>,
}

/// One native `Walk` to `(4, 0, 0)`, begun on the first eligible tick and
/// polled on every later one; the terminal lands in the shared cell.
struct WalkerScript {
    shared: Arc<parking_lot::Mutex<Walker>>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for WalkerScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let mut shared = self.shared.lock();
        if std::mem::take(&mut shared.rewalk) {
            if let Some(handle) = self.handle.take() {
                tick.actions.cancel(handle);
            }
            shared.begun = 0;
        }
        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                shared.result = Some(result);
                self.handle = None;
            }
        } else if shared.begun == 0 {
            shared.begun += 1;
            let request = WalkRequest {
                target: WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
                radius: 0,
                options: script::FindOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => shared.result = Some(Err(error)),
            }
        }
        if shared.blocked {
            return Ok(ScriptFlow::Blocked(ScriptFailure {
                code: "test-blocked".into(),
                message: "blocked by the card".into(),
                retryable: true,
            }));
        }
        Ok(ScriptFlow::Continue)
    }
}

struct Rig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<Arc<NavWorld>>,
    shared: Arc<parking_lot::Mutex<Walker>>,
    driver: NavRec,
    snapshot: GameSnapshot,
}

fn rig(world: Option<Arc<NavWorld>>, blocked: bool) -> Rig {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let shared = Arc::new(parking_lot::Mutex::new(Walker {
        blocked,
        ..Walker::default()
    }));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(WalkerScript {
                shared: Arc::clone(&shared),
                handle: None,
            }),
            None,
        )
        .unwrap();
    let mut client = nav_client();
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    Rig {
        scripts,
        cheats: Arc::new(Mutex::new(HashMap::new())),
        navs: Arc::new(Mutex::new(HashMap::new())),
        statuses: Arc::new(Mutex::new(vec![SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        }])),
        world,
        shared,
        driver: NavRec::default(),
        snapshot,
    }
}

fn open_rig(blocked: bool) -> Rig {
    rig(Some(Arc::new(open_world(40, 1))), blocked)
}

impl Rig {
    fn observe(&mut self, tick: u64) {
        script_observe(
            &mut self.driver,
            "alice",
            true,
            true,
            tick,
            Some((0, 0, 0)),
            None,
            None,
            Some(&self.snapshot),
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &self.world,
            false,
            false,
        );
    }

    fn step(&mut self) {
        step_nav_bot(
            &mut self.driver,
            "alice",
            Some((0, 0, 0)),
            &self.snapshot,
            &self.navs,
            &self.statuses,
            self.world.as_deref(),
            false,
            false,
            no_reach,
        );
    }

    fn walk_armed(&self) -> bool {
        self.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(NavBot::script_walk_armed)
    }

    fn wait_routed(&self) {
        assert!(
            wait_until(5_000, || queued(&self.navs).is_some()),
            "the worker armed the native route"
        );
    }

    fn end(&self) -> Option<Result<WalkEnd, ActionError>> {
        self.shared
            .lock()
            .result
            .clone()
            .map(|result| result.map(|receipt| receipt.end))
    }

    fn slot(&self) -> Arc<Mutex<script::SlotScript>> {
        script_slot(&self.scripts, "alice").unwrap()
    }
}

#[test]
fn a_blocked_tick_dispatches_no_native_walk() {
    let mut rig = open_rig(true);
    rig.observe(1);
    assert_eq!(rig.shared.lock().begun, 1, "the tick began a walk");
    assert_eq!(
        rig.slot().lock().unwrap().native_status().unwrap().phase,
        script::native::NativePhase::Blocked
    );
    assert!(
        !rig.walk_armed(),
        "Blocked closes dispatch: the tick's walk must not reach the navigator"
    );
    rig.step();
    assert_eq!(rig.driver.walked, None, "no hop is sent while Blocked");
}

#[test]
fn blocking_mid_walk_stops_the_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    rig.step();
    assert_eq!(rig.driver.walked, Some((4, 0)));
    rig.shared.lock().blocked = true;
    rig.observe(2);
    rig.driver.walked = None;
    rig.step();
    assert_eq!(queued(&rig.navs), None, "Blocked revokes the follow");
    assert_eq!(rig.driver.walked, None);
    assert!(rig
        .slot()
        .lock()
        .unwrap()
        .native_quiet_read(Instant::now())
        .is_none());
}

#[test]
fn a_native_walk_refused_for_lack_of_a_world_gets_a_refused_receipt() {
    let mut rig = rig(None, false);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
}

/// The host's own route arm, as the watchdog's recovery walk and legacy
/// script walks use it: no native authority.
fn host_walk(rig: &Rig, x: i32, retarget: bool) -> bool {
    crate::script_runtime::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".into(),
        state: None,
        bank: Vec::new(),
    }
    .queue_route_in_snapshot(
        &rig.snapshot,
        x,
        0,
        0,
        nav::router::FindOptions::default(),
        0,
        retarget,
        0,
    )
}

#[test]
fn a_native_walk_refused_by_an_in_flight_route_gets_a_refused_receipt() {
    let mut rig = open_rig(false);
    // A host walk to the same destination is already in flight under the
    // legacy request id; the distinct native id is refused, not coalesced.
    assert!(host_walk(&rig, 4, false));
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
}

#[test]
fn pause_and_resume_end_a_native_walk_with_a_cancelled_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let slot = rig.slot();
        let mut slot = slot.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
        slot.resume();
    }
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
    rig.step();
    assert_eq!(rig.driver.walked, None, "no orphaned follow after Resume");
}

#[test]
fn a_watchdog_walk_displacing_a_native_walk_cancels_it() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    // The watchdog's recovery ArmWalk: the host arm without native authority.
    assert!(host_walk(&rig, 8, true));
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
}

#[test]
fn an_unproven_crossing_reaches_the_native_owner_typed() {
    use api::selected::{FactKey, QuestGate, Truth};
    let gate = QuestGate::Complete(FactKey(Arc::from("tbwt")));
    let undecided: Arc<[QuestGate]> = Arc::from([gate]);
    for (verdict, unresolved, expected) in [
        (
            Truth::Unknown,
            Arc::clone(&undecided),
            WalkEnd::NeedsEvidence(Arc::clone(&undecided)),
        ),
        (Truth::False, Arc::from([]), WalkEnd::Blocked),
    ] {
        let mut rig = open_rig(false);
        rig.observe(1);
        rig.wait_routed();
        // The follow's pre-send recheck refused the crossing.
        crate::script_runtime::apply_nav_follow_outcome(
            rig.navs.lock().unwrap().get_mut("alice").unwrap(),
            Some(nav::traveller::TravelOutcome::EvidenceUnproven {
                at: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                leg: 0,
                verdict,
                unresolved,
            }),
            false,
        );
        rig.observe(2);
        assert_eq!(rig.end(), Some(Ok(expected)));
    }
}

#[test]
fn cancelling_and_rewalking_the_same_destination_arms_the_new_walk() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    // The owner cancels its walk and walks the same destination again
    // before the pump has stepped the abandoned follow away.
    rig.shared.lock().rewalk = true;
    rig.observe(2);
    assert!(
        rig.navs.lock().unwrap()["alice"]
            .native_walk
            .as_ref()
            .is_some_and(|owner| owner.live()),
        "the new walk owns the route, not a refusal beside the abandoned one"
    );
    rig.observe(3);
    assert_eq!(rig.end(), None, "the re-walk is in flight, not refused");
    rig.wait_routed();
}

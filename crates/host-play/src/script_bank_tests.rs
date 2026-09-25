use super::*;
use api::named_banks::NamedBank;
use client::dash3d::CollisionFlag;
use nav::collision::{pack_walk, WorldCollision};
use nav::transport::TransportGraph;
use parking_lot::{Condvar, Mutex as TestMutex};
use std::cell::RefCell;

thread_local! {
    static GATE: RefCell<Option<Arc<Gate>>> = const { RefCell::new(None) };
}

pub(super) fn capture_gate() -> Option<Arc<Gate>> {
    GATE.with(|gate| gate.borrow().clone())
}

#[derive(Default)]
pub(super) struct Gate {
    phase: TestMutex<u8>,
    cv: Condvar,
}
impl Gate {
    pub(super) fn enter(&self) {
        let mut phase = self.phase.lock();
        *phase = 1;
        self.cv.notify_all();
        while *phase < 2 { self.cv.wait(&mut phase); }
    }
    pub(super) fn finished(&self) {
        *self.phase.lock() = 3;
        self.cv.notify_all();
    }
    fn wait(&self, wanted: u8) {
        let mut phase = self.phase.lock();
        while *phase < wanted {
            assert!(!self.cv.wait_for(&mut phase, Duration::from_secs(10)).timed_out(), "worker phase {wanted}");
        }
    }
    fn release(&self) {
        *self.phase.lock() = 2;
        self.cv.notify_all();
    }
}
struct Controlled(Arc<Gate>);
impl Controlled {
    fn new() -> Self {
        let gate = Arc::new(Gate::default());
        GATE.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&gate)));
        Self(gate)
    }
}
impl Drop for Controlled {
    fn drop(&mut self) {
        GATE.with(|slot| *slot.borrow_mut() = None);
        self.0.release();
    }
}

fn tile(x: i32, z: i32) -> WorldTile { WorldTile { x, z, level: 0 } }
fn world(wall: bool) -> Arc<NavWorld> {
    let mut flags = vec![0; 4 * 16 * 16];
    if wall {
        for z in 0..16 { flags[z * 16 + 6] = CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32; }
    }
    let (walk, blocked) = pack_walk(&flags);
    Arc::new(NavWorld::from_parts(WorldCollision {
        origin: tile(0, 0), width: 16, height: 16, walk, blocked, flags: None,
    }, TransportGraph::default(), vec![]))
}
fn navs(banks: Vec<NamedBank>) -> Arc<Mutex<HashMap<String, super::super::NavBot>>> {
    let mut bot = super::super::NavBot::default();
    bot.bank_pick.facts = Some(Arc::new(NamedBankFacts::from_banks(banks)));
    Arc::new(Mutex::new(HashMap::from([("test".to_string(), bot)])))
}
fn bank(name: &'static str, x: i32, z: i32) -> NamedBank { NamedBank::new(name, tile(x,z)) }

#[test]
fn bank_pick_radius_four_skips_worker_but_five_and_other_plane_search() {
    let world = Some(world(false));
    let navs = navs(vec![bank("near", 8, 8)]);
    queue_bank_pick(&navs, "test", &world, None, tile(4,4), true, 1, BankPreferences::default(), None);
    {
        let all = navs.lock().unwrap();
        let bot = &all["test"];
        assert_eq!(bot.bank_pick.posted.kind, PickKind::NearShortcut as u8);
        assert!(bot.bank_pick.worker.is_none());
        assert!(bot.route.is_none());
    }
    for (id, from) in [(2, tile(3,3)), (3, WorldTile { level: 1, ..tile(8,8) })] {
        let gate = Controlled::new();
        queue_bank_pick(&navs, "test", &world, None, from, true, id, BankPreferences::default(), None);
        gate.0.wait(1);
        {
            let all = navs.lock().unwrap();
            assert_eq!(all["test"].bank_pick.last_id, id);
            assert!(all["test"].bank_pick.current.is_some());
            assert!(all["test"].route.is_none());
        }
        gate.0.release();
        gate.0.wait(3);
    }
}

#[test]
fn bank_pick_wall_changes_winner_and_all_unreachable_falls_back() {
    let world = Some(world(true));
    for (banks, expected, kind) in [
        (vec![bank("across wall", 7, 1), bank("same side", 0, 14)], 1, PickKind::Reachable),
        (vec![bank("across wall", 7, 1), bank("far across wall", 14, 14)], 0, PickKind::AirFallback),
    ] {
        let navs = navs(banks);
        let gate = Controlled::new();
        queue_bank_pick(&navs, "test", &world, None, tile(1,1), true, 1, BankPreferences::default(), None);
        gate.0.wait(1);
        gate.0.release();
        gate.0.wait(3);
        let all = navs.lock().unwrap();
        assert_eq!(all["test"].bank_pick.posted.bank_index, expected);
        assert_eq!(all["test"].bank_pick.posted.kind, kind as u8);
        assert!(all["test"].route.is_none());
    }
}

#[test]
fn bank_pick_timeout_and_reset_reject_late_completion_without_blocking_pump() {
    let world = Some(world(false));
    for reset in [false, true] {
        let navs = navs(vec![bank("bank", 8,8)]);
        let gate = Controlled::new();
        queue_bank_pick(&navs, "test", &world, None, tile(0,0), true, 1, BankPreferences::default(), None);
        gate.0.wait(1);
        // A live calculation is parked; the slot can still acquire state and
        // publish a timeout/reset. There is no timing-based concurrency claim.
        let expected = {
            let mut all = navs.lock().unwrap();
            let pick = &mut all.get_mut("test").unwrap().bank_pick;
            if reset { pick.reset(); } else {
                let deadline = pick.current.unwrap().0;
                pick.poll(deadline);
                assert_eq!(pick.posted.kind, PickKind::AirFallback as u8);
                assert_eq!(pick.posted.bank_index, 0);
            }
            pick.posted
        };
        gate.0.release();
        gate.0.wait(3);
        assert_eq!(navs.lock().unwrap()["test"].bank_pick.posted, expected);
    }
}

#[test]
fn bank_pick_same_id_is_idempotent_and_new_near_request_supersedes_worker() {
    let world = Some(world(false));
    let navs = navs(vec![bank("bank", 8,8)]);
    let gate = Controlled::new();
    queue_bank_pick(&navs, "test", &world, None, tile(0,0), true, 1, BankPreferences::default(), None);
    gate.0.wait(1);
    let generation = navs.lock().unwrap()["test"].bank_pick.generation;
    queue_bank_pick(&navs, "test", &world, None, tile(8,8), true, 1, BankPreferences::default(), None);
    assert_eq!(navs.lock().unwrap()["test"].bank_pick.generation, generation);
    queue_bank_pick(&navs, "test", &world, None, tile(8,8), true, 2, BankPreferences::default(), None);
    gate.0.release();
    gate.0.wait(3);
    let all = navs.lock().unwrap();
    assert_eq!(all["test"].bank_pick.posted.request_id, 2);
    assert_eq!(all["test"].bank_pick.posted.kind, PickKind::NearShortcut as u8);
    assert!(all["test"].route.is_none());
}

#[test]
fn bank_pick_gates_precede_the_near_shortcut() {
    static GATED: api::named_banks::BankDefinition = api::named_banks::BankDefinition {
        name: "Gated", tile: WorldTile { x: 4, z: 4, level: 0 },
        approach: None, skill: Some((10, 68)), quest: Some("Lost City"),
        setting: Some("useZanarisBank"), object: None, open_first: None, npc: None, choose: None,
    };
    let world = Some(world(false));
    for (fishing, quest, opt_in, expected) in [
        (67, true, true, -1), (68, false, true, -1), (68, true, false, -1), (68, true, true, 0),
    ] {
        let mut bank = bank("Gated", 4, 4);
        bank.definition = Some(&GATED);
        let navs = navs(vec![bank]);
        let mut state = WorldState::default();
        state.stats.insert(10, 99);
        if quest { state.quests.insert("Lost City".into()); }
        queue_bank_pick(&navs, "test", &world, Some(state), tile(0, 0), true, 1,
            BankPreferences { use_zanaris_bank: opt_in, ..Default::default() }, Some(fishing));
        let all = navs.lock().unwrap();
        assert_eq!(all["test"].bank_pick.posted.bank_index, expected);
        assert!(all["test"].bank_pick.worker.is_none());
    }
}

#[test]
fn bank_pick_ignores_unresolved_geometry_and_keeps_ties_in_catalog_order() {
    let world = Some(world(false));
    let mut unknown = bank("unknown access", 5, 5);
    unknown.routable = false;
    for (banks, expected, kind) in [
        (vec![unknown, bank("first reachable", 8, 0), bank("second reachable", 0, 8)], 1, PickKind::Reachable),
        (vec![unknown], 0, PickKind::AirFallback),
    ] {
        let navs = navs(banks);
        let gate = Controlled::new();
        queue_bank_pick(&navs, "test", &world, None, tile(0, 0), true, 1, BankPreferences::default(), None);
        gate.0.wait(1);
        gate.0.release();
        gate.0.wait(3);
        let all = navs.lock().unwrap();
        assert_eq!(all["test"].bank_pick.posted.bank_index, expected);
        assert_eq!(all["test"].bank_pick.posted.kind, kind as u8);
        assert!(all["test"].route.is_none());
    }
}

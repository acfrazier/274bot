use super::*;
use api::named_banks::NamedBank;
use client::dash3d::CollisionFlag;
use nav::collision::{pack_walk, WorldCollision};
use nav::transport::TransportGraph;
use parking_lot::{Condvar, Mutex as TestMutex};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    searches: AtomicUsize,
    single_routes: AtomicUsize,
    pub(super) fail_spawn: AtomicBool,
}
impl Gate {
    pub(super) fn enter(&self) {
        let mut phase = self.phase.lock();
        if *phase < 2 {
            *phase = 1;
            self.cv.notify_all();
            while *phase < 2 {
                self.cv.wait(&mut phase);
            }
        }
    }
    pub(super) fn finished(&self) {
        *self.phase.lock() = 3;
        self.cv.notify_all();
    }
    pub(super) fn idle(&self) {
        *self.phase.lock() = 4;
        self.cv.notify_all();
    }
    pub(super) fn bank_search(&self) {
        self.searches.fetch_add(1, Ordering::Relaxed);
    }
    pub(super) fn single_route(&self) {
        self.single_routes.fetch_add(1, Ordering::Relaxed);
    }
    fn wait(&self, wanted: u8) {
        let mut phase = self.phase.lock();
        while *phase < wanted {
            assert!(
                !self
                    .cv
                    .wait_for(&mut phase, Duration::from_secs(10))
                    .timed_out(),
                "worker phase {wanted}"
            );
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

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}
fn world(wall: bool) -> Arc<NavWorld> {
    let mut flags = vec![0; 4 * 16 * 16];
    if wall {
        for z in 0..16 {
            flags[z * 16 + 6] =
                CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;
        }
    }
    let (walk, blocked) = pack_walk(&flags);
    Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![],
    ))
}
fn navs(banks: Vec<NamedBank>) -> Arc<Mutex<HashMap<String, super::super::NavBot>>> {
    let mut bot = super::super::NavBot::default();
    bot.bank_pick.facts = Some(Arc::new(NamedBankFacts::from_banks(banks)));
    Arc::new(Mutex::new(HashMap::from([("test".to_string(), bot)])))
}
fn bank(name: &'static str, x: i32, z: i32) -> NamedBank {
    NamedBank::new(name, tile(x, z))
}

#[test]
fn bank_pick_radius_four_skips_worker_but_five_and_other_plane_search() {
    let world = Some(world(false));
    let navs = navs(vec![bank("near", 8, 8)]);
    queue_bank_pick(
        &navs,
        "test",
        &world,
        None,
        tile(4, 4),
        true,
        1,
        BankPreferences::default(),
        None,
    );
    {
        let all = navs.lock().unwrap();
        let bot = &all["test"];
        assert_eq!(bot.bank_pick.posted.kind, PickKind::NearShortcut as u8);
        assert!(bot.bank_pick.worker.is_none());
        assert!(bot.route.is_none());
    }
    for (id, from) in [
        (2, tile(3, 3)),
        (
            3,
            WorldTile {
                level: 1,
                ..tile(8, 8)
            },
        ),
    ] {
        let gate = Controlled::new();
        queue_bank_pick(
            &navs,
            "test",
            &world,
            None,
            from,
            true,
            id,
            BankPreferences::default(),
            None,
        );
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
        (
            vec![bank("across wall", 7, 1), bank("same side", 0, 14)],
            1,
            PickKind::Reachable,
        ),
        (
            vec![bank("across wall", 7, 1), bank("far across wall", 14, 14)],
            0,
            PickKind::AirFallback,
        ),
    ] {
        let navs = navs(banks);
        let gate = Controlled::new();
        queue_bank_pick(
            &navs,
            "test",
            &world,
            None,
            tile(1, 1),
            true,
            1,
            BankPreferences::default(),
            None,
        );
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
        let navs = navs(vec![bank("bank", 8, 8)]);
        let gate = Controlled::new();
        queue_bank_pick(
            &navs,
            "test",
            &world,
            None,
            tile(0, 0),
            true,
            1,
            BankPreferences::default(),
            None,
        );
        gate.0.wait(1);
        // A live calculation is parked; the slot can still acquire state and
        // publish a timeout/reset. There is no timing-based concurrency claim.
        let expected = {
            let mut all = navs.lock().unwrap();
            let pick = &mut all.get_mut("test").unwrap().bank_pick;
            if reset {
                pick.reset();
            } else {
                let deadline = pick.current.as_ref().unwrap().deadline.unwrap();
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
    let navs = navs(vec![bank("bank", 8, 8)]);
    let gate = Controlled::new();
    queue_bank_pick(
        &navs,
        "test",
        &world,
        None,
        tile(0, 0),
        true,
        1,
        BankPreferences::default(),
        None,
    );
    gate.0.wait(1);
    let generation = navs.lock().unwrap()["test"].bank_pick.generation;
    queue_bank_pick(
        &navs,
        "test",
        &world,
        None,
        tile(8, 8),
        true,
        1,
        BankPreferences::default(),
        None,
    );
    assert_eq!(
        navs.lock().unwrap()["test"].bank_pick.generation,
        generation
    );
    queue_bank_pick(
        &navs,
        "test",
        &world,
        None,
        tile(8, 8),
        true,
        2,
        BankPreferences::default(),
        None,
    );
    gate.0.release();
    gate.0.wait(3);
    let all = navs.lock().unwrap();
    assert_eq!(all["test"].bank_pick.posted.request_id, 2);
    assert_eq!(
        all["test"].bank_pick.posted.kind,
        PickKind::NearShortcut as u8
    );
    assert!(all["test"].route.is_none());
}

#[test]
fn bank_pick_gates_precede_the_near_shortcut() {
    static GATED: api::named_banks::BankDefinition = api::named_banks::BankDefinition {
        name: "Gated",
        tile: WorldTile {
            x: 4,
            z: 4,
            level: 0,
        },
        approach: None,
        skill: Some((10, 68)),
        quest: Some("Lost City"),
        setting: Some("useZanarisBank"),
        object: None,
        open_first: None,
        npc: None,
        choose: None,
    };
    let world = Some(world(false));
    for (fishing, quest, opt_in, expected) in [
        (67, true, true, -1),
        (68, false, true, -1),
        (68, true, false, -1),
        (68, true, true, 0),
    ] {
        let mut bank = bank("Gated", 4, 4);
        bank.definition = Some(&GATED);
        let navs = navs(vec![bank]);
        let mut state = WorldState::default();
        state.stats.insert(10, 99);
        if quest {
            state.quests.insert("Lost City".into());
        }
        queue_bank_pick(
            &navs,
            "test",
            &world,
            Some(state),
            tile(0, 0),
            true,
            1,
            BankPreferences {
                use_zanaris_bank: opt_in,
                ..Default::default()
            },
            Some(fishing),
        );
        let all = navs.lock().unwrap();
        assert_eq!(all["test"].bank_pick.posted.bank_index, expected);
        assert!(all["test"].bank_pick.worker.is_none());
    }
}

#[test]
fn bank_pick_ignores_unresolved_geometry_and_keeps_ties_in_catalog_order() {
    let world = Some(world(false));
    let surface = world.as_ref().unwrap();
    let targets = [tile(8, 0), tile(0, 8)];
    let routes = find_many_with_avoid_bounded(
        &surface.collision,
        &surface.graph,
        tile(0, 0),
        &targets,
        FindOptions::default(),
        &WorldState::empty(),
        &[],
        BANK_TARGET_BUDGET,
    );
    let a = routes.results()[0].as_ref().unwrap();
    let b = routes.results()[1].as_ref().unwrap();
    assert_eq!(a.ticks, b.ticks);
    assert!(
        a.settled_at < b.settled_at,
        "the reversed catalog row below must disagree with heap settlement order"
    );
    let mut unknown = bank("unknown access", 5, 5);
    unknown.routable = false;
    for (banks, expected, kind) in [
        (
            vec![
                unknown,
                bank("first reachable", 8, 0),
                bank("second reachable", 0, 8),
            ],
            1,
            PickKind::Reachable,
        ),
        (
            vec![
                unknown,
                bank("second reachable", 0, 8),
                bank("first reachable", 8, 0),
            ],
            1,
            PickKind::Reachable,
        ),
        (vec![unknown], 0, PickKind::AirFallback),
    ] {
        let navs = navs(banks);
        let gate = Controlled::new();
        queue_bank_pick(
            &navs,
            "test",
            &world,
            None,
            tile(0, 0),
            true,
            1,
            BankPreferences::default(),
            None,
        );
        gate.0.wait(1);
        gate.0.release();
        gate.0.wait(3);
        let all = navs.lock().unwrap();
        assert_eq!(all["test"].bank_pick.posted.bank_index, expected);
        assert_eq!(all["test"].bank_pick.posted.kind, kind as u8);
        assert!(all["test"].route.is_none());
    }
}

struct HeldDriver;
impl Driver for HeldDriver {
    fn set_menu(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32) {
        panic!("held bank walk sent a menu action");
    }
    fn do_action(&mut self, _: i32) -> bool {
        panic!("held bank walk dispatched an action");
    }
    fn try_move(
        &mut self,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: bool,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> bool {
        panic!("held bank walk sent movement");
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        panic!("held follow must not run");
    }
    fn build_base(&self) -> (i32, i32) {
        panic!("held follow must not run");
    }
    fn loc_typecode(&self, _: i32, _: i32) -> Option<i32> {
        panic!("held follow must not run");
    }
    fn out(&mut self) -> &mut dyn api::prot::Out {
        panic!("held bank walk wrote a packet");
    }
    fn login(&mut self, _: &str, _: &str, _: bool) -> bool {
        panic!("held bank walk attempted login");
    }
}

#[test]
fn bank_pick_walk_reuses_the_winning_route_and_hold_keeps_it_without_sends() {
    let world = Some(world(true));
    let navs = navs(vec![bank("across wall", 7, 1), bank("same side", 0, 14)]);
    let gate = Controlled::new();
    assert!(queue_bank_walk(
        &navs,
        "test",
        &world,
        None,
        tile(1, 1),
        None
    ));
    gate.0.wait(1);
    // Retransmitting the raw verb does not enqueue another ranking flood.
    assert!(queue_bank_walk(
        &navs,
        "test",
        &world,
        None,
        tile(1, 1),
        None
    ));
    gate.0.release();
    gate.0.wait(4);
    let expected = find_with(
        &world.as_ref().unwrap().collision,
        &world.as_ref().unwrap().graph,
        tile(1, 1),
        tile(0, 14),
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap();
    {
        let all = navs.lock().unwrap();
        let route = all["test"].route.as_ref().expect("winning route armed");
        assert_eq!((route.dest, route.ticks), (expected.dest, expected.ticks));
        assert_eq!(gate.0.searches.load(Ordering::Relaxed), 1);
        assert_eq!(
            gate.0.single_routes.load(Ordering::Relaxed),
            0,
            "reuse the winner rather than flood again"
        );
    }
    super::super::step_nav_bot(
        &mut HeldDriver,
        "test",
        Some((1, 1, 0)),
        &GameSnapshot::new(),
        &navs,
        &Arc::new(Mutex::new(vec![])),
        world.as_deref(),
        true,
        false,
        || panic!("a held slot must not enter the reach/follow path"),
    );
    assert_eq!(
        navs.lock().unwrap()["test"].route.as_ref().unwrap().dest,
        expected.dest
    );
}

#[test]
fn bank_pick_walk_timeout_discards_the_winner_and_attempts_only_the_air_fallback() {
    let world = Some(world(true));
    let navs = navs(vec![bank("across wall", 7, 1), bank("same side", 0, 14)]);
    let gate = Controlled::new();
    assert!(queue_bank_walk(
        &navs,
        "test",
        &world,
        None,
        tile(1, 1),
        None
    ));
    gate.0.wait(1);
    {
        let mut all = navs.lock().unwrap();
        let pick = &mut all.get_mut("test").unwrap().bank_pick;
        let deadline = pick.current.as_ref().unwrap().deadline.unwrap();
        pick.poll(deadline);
        assert!(pick.current.as_ref().unwrap().job.route_only);
    }
    gate.0.release();
    gate.0.wait(4);
    let all = navs.lock().unwrap();
    let bot = &all["test"];
    assert!(
        bot.route.is_none(),
        "the late reachable winner must not arm"
    );
    assert!(bot.walk_outcome_failed);
    assert_eq!(
        (bot.walk_outcome_x, bot.walk_outcome_z),
        (7, 1),
        "failure belongs to the air fallback"
    );
    assert_eq!(gate.0.searches.load(Ordering::Relaxed), 1);
    assert_eq!(gate.0.single_routes.load(Ordering::Relaxed), 1);
}

#[test]
fn bank_pick_walk_reset_abort_and_newer_route_lease_reject_late_results() {
    let world = Some(world(false));
    for replacement in 0..3 {
        let navs = navs(vec![bank("bank", 8, 8)]);
        let gate = Controlled::new();
        assert!(queue_bank_walk(
            &navs,
            "test",
            &world,
            None,
            tile(0, 0),
            None
        ));
        gate.0.wait(1);
        match replacement {
            0 => super::super::reset_script_nav(&navs, "test"),
            1 => super::super::abort_script_walk(&navs, "test"),
            _ => {
                // Publish another walk owner's refusal while the bank worker
                // is parked: its late success must not replace that outcome.
                let mut all = navs.lock().unwrap();
                let bot = all.get_mut("test").unwrap();
                bot.route_generation = bot.route_generation.wrapping_add(1);
                bot.walk_request_id = 91;
                bot.requested_route = Some((tile(15, 15), 2, false, false, false));
                bot.note_failure(bot.route_generation, 91, tile(15, 15), 2, false);
            }
        }
        let before = {
            let all = navs.lock().unwrap();
            let bot = &all["test"];
            (
                bot.requested_route,
                bot.walk_outcome_seq,
                bot.walk_outcome_request_id,
            )
        };
        gate.0.release();
        gate.0.wait(4);
        let all = navs.lock().unwrap();
        let bot = &all["test"];
        assert!(bot.route.is_none());
        assert_eq!(
            (
                bot.requested_route,
                bot.walk_outcome_seq,
                bot.walk_outcome_request_id
            ),
            before
        );
    }
}

#[test]
fn bank_pick_latest_pending_selection_preserves_an_armed_walk() {
    let world = Some(world(false));
    let navs = navs(vec![bank("bank", 8, 8)]);
    let route = find_with(
        &world.as_ref().unwrap().collision,
        &world.as_ref().unwrap().graph,
        tile(0, 0),
        tile(0, 3),
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap();
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("test").unwrap();
        bot.route = Some(route.clone());
        bot.requested_route = Some((route.dest, 0, false, false, false));
        bot.walk_request_id = 91;
        bot.route_generation = 12;
    }
    let gate = Controlled::new();
    queue_bank_pick(
        &navs,
        "test",
        &world,
        None,
        tile(0, 0),
        false,
        1,
        BankPreferences::default(),
        None,
    );
    gate.0.wait(1);
    for id in 2..=3 {
        queue_bank_pick(
            &navs,
            "test",
            &world,
            None,
            tile(0, 1),
            false,
            id,
            BankPreferences::default(),
            None,
        );
    }
    gate.0.release();
    gate.0.wait(4);
    let all = navs.lock().unwrap();
    let bot = &all["test"];
    assert_eq!(bot.bank_pick.posted.request_id, 3);
    assert_eq!(bot.bank_pick.posted.kind, PickKind::Reachable as u8);
    assert_eq!(
        gate.0.searches.load(Ordering::Relaxed),
        2,
        "only active and latest pending selection run"
    );
    assert_eq!(bot.route.as_ref(), Some(&route));
    assert_eq!((bot.route_generation, bot.walk_request_id), (12, 91));
}

#[test]
fn bank_pick_near_walk_skips_ranking_and_only_approaches_the_selected_stand() {
    let world = Some(world(false));
    let navs = navs(vec![bank("bank", 4, 4)]);
    let gate = Controlled::new();
    assert!(queue_bank_walk(
        &navs,
        "test",
        &world,
        None,
        tile(0, 0),
        None
    ));
    gate.0.wait(1);
    gate.0.release();
    gate.0.wait(4);
    assert_eq!(gate.0.searches.load(Ordering::Relaxed), 0);
    assert_eq!(gate.0.single_routes.load(Ordering::Relaxed), 1);
    assert_eq!(
        navs.lock().unwrap()["test"].route.as_ref().unwrap().dest,
        tile(4, 4)
    );
}

#[test]
fn bank_pick_v2_explicit_opt_ins_control_the_returned_bank_over_settings_defaults() {
    let catalog = api::named_banks::BANK_CATALOG;
    let mage = NamedBank {
        definition: catalog.iter().find(|b| b.name == "Mage Arena"),
        ..bank("Mage Arena", 3091, 3958)
    };
    let zanaris = NamedBank {
        definition: catalog.iter().find(|b| b.name == "Zanaris"),
        ..bank("Zanaris", 3092, 3958)
    };
    for (defaults, options, expected) in [
        (
            false,
            "use_mage_bank: true, use_zanaris_bank: false",
            "Mage Arena",
        ),
        (
            false,
            "use_mage_bank: false, use_zanaris_bank: true",
            "Zanaris",
        ),
        (
            true,
            "use_mage_bank: false, use_zanaris_bank: false",
            "Public",
        ),
        (true, "", "Mage Arena"),
    ] {
        let banks = vec![mage, zanaris, bank("Public", 3093, 3958)];
        let iso = script::LoadIsolate::spawn_with_content(format!(r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {{
  if (started) return;
  started = true;
  globalThis.result = await api.bankNearestReachable({{ from: {{x:3091,z:3958,level:0}}, {options} }});
}}
"#), script::LoadShape::NativeTick, vec![], None,
            Arc::new(NamedBankFacts::from_banks(banks.clone())),
            Arc::new(api::run_policy::RunPolicyOverrideCell::new())).unwrap();
        iso.post_settings_bag(
            serde_json::json!({"useMageBank": defaults, "useZanarisBank": defaults})
                .as_object()
                .unwrap(),
        );
        let post = |tick, selection| {
            iso.post_snapshot(super::super::with_script_snapshot_input(
                tick,
                Some((3091, 3958, 0)),
                true,
                None,
                None,
                None,
                None,
                None,
                false,
                false,
                false,
                0,
                false,
                0,
                false,
                0,
                false,
                None,
                Default::default(),
                Default::default(),
                |snapshot, mut native| {
                    native.bank_selection = selection;
                    script::isolate_fb::encode_snapshot_with_native(snapshot, native)
                },
            ));
            iso.on_game_tick(tick);
            iso.probe("true").unwrap();
        };
        post(1, BankSelectionInput::default());
        let navs = navs(banks);
        let mut state = WorldState::empty();
        state.quests.insert("Lost City".into());
        super::super::dispatch_script_interact_cached(
            &mut HeldDriver,
            &GameSnapshot::new(),
            None,
            Some((3091, 3958, 0)),
            &navs,
            &Some(world(false)),
            Some(state),
            "test",
            iso.drain_interacts(),
            None,
            None,
        );
        let result = navs.lock().unwrap()["test"].bank_pick.posted;
        post(2, result);
        assert_eq!(
            iso.probe("globalThis.result.ok && globalThis.result.value.name")
                .unwrap(),
            expected
        );
        assert!(
            navs.lock().unwrap()["test"].route.is_none(),
            "selection must not move"
        );
        iso.join();
    }
}

#[test]
fn bank_pick_stop_invalidates_pending_selection_and_walk_before_worker_publication() {
    let play = crate::Play::new(&crate::PlayOptions {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/bank-pick-empty-cache"
        )
        .into(),
        lowmem: true,
        mainland: false,
    });
    for walking in [false, true] {
        let fixture = navs(vec![bank("bank", 8, 8)]);
        *play.navs.lock().unwrap() = std::mem::take(&mut *fixture.lock().unwrap());
        let gate = Controlled::new();
        if walking {
            assert!(queue_bank_walk(
                &play.navs,
                "test",
                &Some(world(false)),
                None,
                tile(0, 0),
                None
            ));
        } else {
            queue_bank_pick(
                &play.navs,
                "test",
                &Some(world(false)),
                None,
                tile(0, 0),
                false,
                1,
                BankPreferences::default(),
                None,
            );
        }
        gate.0.wait(1);
        play.script_stop("test");
        gate.0.release();
        gate.0.wait(4);
        let all = play.navs.lock().unwrap();
        assert_eq!(all["test"].bank_pick.posted, BankSelectionInput::default());
        assert!(
            all["test"].route.is_none(),
            "Stop must prevent the bank worker from arming movement"
        );
    }
}

#[test]
fn bank_pick_spawn_failure_settles_selection_or_refuses_walk_without_arming() {
    for walking in [false, true] {
        let navs = navs(vec![bank("air fallback", 8, 8)]);
        let gate = Controlled::new();
        gate.0.fail_spawn.store(true, Ordering::Relaxed);
        if walking {
            assert!(!queue_bank_walk(
                &navs,
                "test",
                &Some(world(false)),
                None,
                tile(0, 0),
                None
            ));
        } else {
            queue_bank_pick(
                &navs,
                "test",
                &Some(world(false)),
                None,
                tile(0, 0),
                false,
                1,
                BankPreferences::default(),
                None,
            );
        }
        let all = navs.lock().unwrap();
        let bot = &all["test"];
        if walking {
            assert!(bot.walk_outcome_failed);
            assert_eq!((bot.walk_outcome_x, bot.walk_outcome_z), (8, 8));
        } else {
            assert_eq!(bot.bank_pick.posted.request_id, 1);
            assert_eq!(bot.bank_pick.posted.bank_index, 0);
            assert_eq!(bot.bank_pick.posted.kind, PickKind::AirFallback as u8);
        }
        assert!(bot.route.is_none());
        assert!(
            bot.bank_pick.worker.is_none()
                && bot.bank_pick.current.is_none()
                && bot.bank_pick.pending.is_none()
        );
    }
}

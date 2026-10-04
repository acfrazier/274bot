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
        world.as_ref(),
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
fn bank_pick_walk_reset_abort_takeover_and_newer_route_lease_reject_late_results() {
    let world = Some(world(false));
    for replacement in 0..4 {
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
            0 => super::super::reset_script_nav(&navs, "test", None),
            1 => super::super::abort_script_walk(&navs, "test"),
            2 => {
                // Publish another walk owner's refusal while the bank worker
                // is parked: its late success must not replace that outcome.
                let mut all = navs.lock().unwrap();
                let bot = all.get_mut("test").unwrap();
                bot.route_generation = bot.route_generation.wrapping_add(1);
                bot.walk_request_id = 91;
                bot.requested_route = Some((
                    tile(15, 15),
                    2,
                    false,
                    false,
                    false,
                    nav::zones::ZoneExempt::NONE,
                ));
                bot.note_failure(bot.route_generation, 91, tile(15, 15), 2, false);
            }
            _ => {
                let mut all = navs.lock().unwrap();
                let bot = all.get_mut("test").unwrap();
                assert!(bot.script_walk_armed());
                bot.cancel_for_manual_input();
                assert!(!bot.script_walk_armed());
                assert_eq!(bot.manual_takeover_watermark, bot.walk_outcome_seq);
                assert_eq!(
                    bot.walk_outcome_request_id, 0,
                    "a raw bank walk must not fabricate a request-id-zero cancellation receipt"
                );
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
        bot.requested_route = Some((
            route.dest,
            0,
            false,
            false,
            false,
            nav::zones::ZoneExempt::NONE,
        ));
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
            Arc::new(NamedBankFacts::from_banks(banks.clone()))).unwrap();
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
        transport: client::Transport::Tcp,
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

#[test]
fn native_bank_access_resolves_teller_and_object_metadata() {
    let flags = vec![0; 4 * 16 * 16];
    let (walk, blocked) = pack_walk(&flags);
    let teller = nav::pack::BankStand {
        name: "Gundai".to_owned(),
        tile: tile(8, 8),
        access: nav::pack::BankAccess::Npc {
            name: "Gundai".to_owned(),
            op: 4,
            choose: None,
        },
    };
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![
            nav::pack::BankStand {
                name: "Bank booth".to_owned(),
                tile: tile(7, 8),
                access: nav::pack::BankAccess::Booth { op: 2 },
            },
            teller,
        ],
    );
    let definition = api::named_banks::BANK_CATALOG
        .iter()
        .find(|definition| definition.name == "Mage Arena")
        .unwrap();
    let mage_bank = api::named_banks::NamedBank {
        name: definition.name,
        tile: tile(8, 8),
        definition: Some(definition),
        routable: true,
    };
    let (access_tiles, access) = native_bank_access(&world, mage_bank).unwrap();
    let access_tile = access_tiles[0];
    assert_ne!(access_tile, access.stand_tile);
    assert_eq!(access_tile.level, access.stand_tile.level);
    assert_eq!(access.kind, NativeAccessKind::Teller);
    assert_eq!(access.name.as_deref(), Some("Gundai"));
    assert_eq!(access.stand_op, 4);
    assert_eq!(
        access.choose.as_deref(),
        Some("I'd like to access my bank account")
    );

    let shantay = api::named_banks::BANK_CATALOG
        .iter()
        .find(|definition| definition.name == "Shantay Pass")
        .unwrap();
    let shantay_bank = api::named_banks::NamedBank {
        name: shantay.name,
        tile: shantay.tile,
        definition: Some(shantay),
        routable: true,
    };
    let (access_tiles, access) = native_bank_access(&world, shantay_bank).unwrap();
    assert_eq!(access_tiles, vec![shantay.tile]);
    assert_eq!(access.stand_tile, shantay.tile);
    assert_eq!(access.kind, NativeAccessKind::Booth);
    assert_eq!(access.name.as_deref(), Some("Shantay chest"));
    assert_eq!(access.stand_op, 0);
}

#[test]
fn native_bank_access_declared_teller_does_not_require_packed_geometry() {
    let definition = api::named_banks::BANK_CATALOG
        .iter()
        .find(|definition| definition.name == "Mage Arena")
        .unwrap();
    for stands in [
        Vec::new(),
        vec![
            nav::pack::BankStand {
                name: "Bank booth".to_owned(),
                tile: definition.tile,
                access: nav::pack::BankAccess::Booth { op: 2 },
            },
            nav::pack::BankStand {
                name: "Other teller".to_owned(),
                tile: definition.tile,
                access: nav::pack::BankAccess::Npc {
                    name: "Other teller".to_owned(),
                    op: 1,
                    choose: None,
                },
            },
        ],
    ] {
        let flags = vec![0; 4 * 16 * 16];
        let (walk, blocked) = pack_walk(&flags);
        let world = NavWorld::from_parts(
            WorldCollision {
                origin: tile(definition.tile.x - 8, definition.tile.z - 8),
                width: 16,
                height: 16,
                walk,
                blocked,
                flags: None,
            },
            TransportGraph::default(),
            stands,
        );
        let mage_bank = api::named_banks::NamedBank {
            name: definition.name,
            tile: definition.tile,
            definition: Some(definition),
            routable: true,
        };
        let (access_tiles, access) = native_bank_access(&world, mage_bank).unwrap();
        assert_eq!(access_tiles, vec![definition.tile]);
        assert_eq!(access.stand_tile, definition.tile);
        assert_eq!(access.kind, NativeAccessKind::Teller);
        assert_eq!(access.name.as_deref(), Some("Gundai"));
        assert_eq!(access.stand_op, 0);
        assert_eq!(
            access.choose.as_deref(),
            Some("I'd like to access my bank account")
        );
        assert!(native_bank_access(&world, bank("Unmapped", 8, 8)).is_none());
    }
}

fn native_pick_action(
    banks: Vec<NamedBank>,
    from: WorldTile,
    explicit: Option<&str>,
) -> (script::SlotScript, script::native::HostAction) {
    use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};
    use script::native_bank::{Select, SelectArgs};

    struct Selecting {
        args: Option<SelectArgs>,
        handle: Option<ActionHandle<Select>>,
    }
    impl Script for Selecting {
        fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            if let Some(args) = self.args.take() {
                self.handle = Some(tick.actions.begin::<Select>(args, &mut tick.cx).unwrap());
            }
            assert!(tick
                .actions
                .poll(self.handle.as_ref().unwrap(), &mut tick.cx)
                .is_pending());
            Ok(ScriptFlow::Continue)
        }
    }
    let mut slot = script::SlotScript::new();
    slot.start_test_script(
        Box::new(Selecting {
            args: Some(SelectArgs {
                facts: Arc::new(NamedBankFacts::from_banks(banks)),
                from,
                preferences: BankPreferences::default(),
                allow_wilderness: false,
                explicit: explicit.map(Arc::from),
            }),
            handle: None,
        }),
        None,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    slot.on_game_tick(&mut script::ScriptCtx {
        driver: &mut HeldDriver,
        tick: 1,
        here: Some((from.x, from.z, from.level)),
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(&snapshot),
        obj_names: None,
        compiled: script::CompiledTick::default(),
    });
    let action = slot
        .take_native_action()
        .expect("real native Select request");
    assert!(action.live());
    (slot, action)
}

fn native_world(wall: bool, banks: &[NamedBank], graph: TransportGraph) -> Arc<NavWorld> {
    let mut flags = vec![0; 4 * 48 * 48];
    if wall {
        for z in 0..48 {
            flags[z * 48 + 6] =
                CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;
        }
    }
    let (walk, blocked) = pack_walk(&flags);
    Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 48,
            height: 48,
            walk,
            blocked,
            flags: None,
        },
        graph,
        banks
            .iter()
            .map(|bank| nav::pack::BankStand {
                name: "Bank booth".into(),
                tile: bank.tile,
                access: nav::pack::BankAccess::Booth { op: 2 },
            })
            .collect(),
    ))
}

fn start_native_pick(
    navs: &Arc<Mutex<HashMap<String, super::super::NavBot>>>,
    world: Arc<NavWorld>,
    action: script::native::HostAction,
    opts: FindOptions,
    state: WorldState,
) {
    let authority = action.authority();
    let script::native::HostEffect::BankPick(request) = action.effect else {
        panic!("expected bank pick");
    };
    let evidence = EvidenceStamp {
        run: authority.run(),
        tick: 1,
        sequence: 1,
    };
    queue_bank(
        navs,
        "test",
        &Some(world),
        Some(state),
        request.from,
        opts,
        authority.request_id().get(),
        request.preferences,
        None,
        false,
        Some(NativeBankPickInput {
            request,
            authority,
            evidence,
        }),
    );
}

fn await_native_search(
    navs: &Arc<Mutex<HashMap<String, super::super::NavBot>>>,
    gate: &Controlled,
) {
    assert!(
        navs.lock().unwrap()["test"].bank_pick.worker.is_some(),
        "native selection must search even inside the air-near radius"
    );
    gate.0.wait(1);
}

#[test]
fn native_bank_pick_keeps_resolved_public_access_over_air_nearer_aisle() {
    let banks = vec![bank("local", 5, 8), bank("remote", 1, 14)];
    let mut flags = vec![0; 4 * 16 * 16];
    // The east tile is standable and adjacent to the booth, but enclosed in
    // the bankers' aisle. The catalog resolved the reachable public west side.
    for (x, z) in [(7, 7), (8, 8), (7, 9)] {
        flags[z * 16 + x] =
            CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;
    }
    flags[8 * 16 + 6] =
        CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::W_N as u32 | CollisionFlag::W_S as u32;
    let (walk, blocked) = pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![
            nav::pack::BankStand {
                name: "Bank booth".into(),
                tile: tile(6, 8),
                access: nav::pack::BankAccess::Booth { op: 2 },
            },
            nav::pack::BankStand {
                name: "Bank booth".into(),
                tile: tile(1, 15),
                access: nav::pack::BankAccess::Booth { op: 2 },
            },
        ],
    ));
    let (slot, action) = native_pick_action(banks.clone(), tile(10, 2), None);
    let navs = navs(banks);
    let gate = Controlled::new();
    start_native_pick(
        &navs,
        world,
        action,
        FindOptions::default(),
        WorldState::empty(),
    );
    await_native_search(&navs, &gate);
    gate.0.release();
    gate.0.wait(4);
    let mut all = navs.lock().unwrap();
    let selected = all
        .get_mut("test")
        .unwrap()
        .bank_pick
        .take_native_receipt()
        .unwrap()
        .1
        .selected;
    assert_eq!(selected.kind, NativePickKind::Reachable);
    assert_eq!(selected.bank_index, 0);
    assert_eq!(selected.access_tile, tile(5, 8));
    assert!(selected.access.is_some());
    assert!(all["test"].route.is_none(), "selection must not move");
    drop(slot);
}

#[test]
fn native_bank_pick_routes_non_catalog_booth_access_instead_of_enclosed_aisle() {
    // The catalog stand is diagonal to this booth, like Edgeville and Seers.
    // Neither InLine access tile equals it. Air-nearest picks the sealed east
    // aisle; selection must instead route to the public west access.
    let banks = vec![bank("local", 5, 7), bank("remote", 1, 14)];
    let mut flags = vec![0; 4 * 16 * 16];
    for (x, z) in [(7, 7), (8, 8), (7, 9)] {
        flags[z * 16 + x] =
            CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;
    }
    flags[8 * 16 + 6] =
        CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::W_N as u32 | CollisionFlag::W_S as u32;
    let (walk, blocked) = pack_walk(&flags);
    let stand = nav::pack::BankStand {
        name: "Bank booth".into(),
        tile: tile(6, 8),
        access: nav::pack::BankAccess::Booth { op: 2 },
    };
    let world = Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![stand.clone()],
    ));
    let from = tile(10, 2);
    let access_tiles: Vec<_> =
        nav::bank_fetch::bank_access_tiles(&world.collision, &stand).collect();
    assert!(!access_tiles.contains(&banks[0].tile));
    assert!(access_tiles.contains(&tile(7, 8)));
    assert!(find_with(
        &world.collision,
        &world.graph,
        from,
        tile(7, 8),
        FindOptions::default(),
        &WorldState::empty(),
    )
    .is_err());
    // Required and default selection must both keep this reachable local bank.
    for explicit in [None, Some("local")] {
        let (slot, action) = native_pick_action(banks.clone(), from, explicit);
        let navs = navs(banks.clone());
        let gate = Controlled::new();
        start_native_pick(
            &navs,
            Arc::clone(&world),
            action,
            FindOptions::default(),
            WorldState::empty(),
        );
        await_native_search(&navs, &gate);
        gate.0.release();
        gate.0.wait(4);
        let mut all = navs.lock().unwrap();
        let selected = all
            .get_mut("test")
            .unwrap()
            .bank_pick
            .take_native_receipt()
            .unwrap()
            .1
            .selected;
        assert_eq!(selected.kind, NativePickKind::Reachable);
        assert_eq!(selected.bank_index, 0);
        assert_eq!(selected.access_tile, tile(5, 8));
        assert!(selected.access.is_some());
        assert!(all["test"].route.is_none(), "selection must not move");
        drop(slot);
    }
}

#[test]
fn native_bank_pick_uses_cheapest_access_even_when_catalog_access_is_reachable() {
    let banks = vec![bank("local", 5, 8)];
    let mut flags = vec![0; 4 * 16 * 16];
    flags[8 * 16 + 6] =
        CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::W_N as u32 | CollisionFlag::W_S as u32;
    let (walk, blocked) = pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        WorldCollision {
            origin: tile(0, 0),
            width: 16,
            height: 16,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![nav::pack::BankStand {
            name: "Bank booth".into(),
            tile: tile(6, 8),
            access: nav::pack::BankAccess::Booth { op: 2 },
        }],
    ));
    let from = tile(10, 8);
    let public = find_with(
        &world.collision,
        &world.graph,
        from,
        banks[0].tile,
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap();
    let nearest = find_with(
        &world.collision,
        &world.graph,
        from,
        tile(7, 8),
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap();
    assert!(nearest.ticks < public.ticks);
    let (slot, action) = native_pick_action(banks.clone(), from, Some("local"));
    let navs = navs(banks);
    let gate = Controlled::new();
    start_native_pick(
        &navs,
        world,
        action,
        FindOptions::default(),
        WorldState::empty(),
    );
    await_native_search(&navs, &gate);
    gate.0.release();
    gate.0.wait(4);
    let selected = navs
        .lock()
        .unwrap()
        .get_mut("test")
        .unwrap()
        .bank_pick
        .take_native_receipt()
        .unwrap()
        .1
        .selected;
    assert_eq!(selected.kind, NativePickKind::Reachable);
    assert_eq!(selected.bank_index, 0);
    assert_eq!(selected.access_tile, tile(7, 8));
    drop(slot);
}

#[test]
#[ignore = "requires explicit WORLD_NAV_PACK for the real 289 bank geometry"]
fn native_bank_pick_edgeville_yews_real_pack() {
    let pack = std::env::var_os("WORLD_NAV_PACK").expect("explicit real pack path");
    let world = NavWorld::load_pack(std::path::Path::new(&pack)).unwrap();
    world
        .bind_named_bank_facts(
            &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
        )
        .unwrap();
    let banks = world.named_bank_facts().unwrap().banks().to_vec();
    let edgeville = banks
        .iter()
        .position(|bank| bank.name == "Edgeville")
        .unwrap();
    let from = tile(3080, 3472);
    let staff = tile(3095, 3490);
    let state = WorldState::default().with_map_members(true);
    assert!(find_with(
        &world.collision,
        &world.graph,
        from,
        staff,
        FindOptions::default(),
        &state
    )
    .is_err());
    let (slot, action) = native_pick_action(banks.clone(), from, Some("Edgeville"));
    let navs = navs(banks);
    let gate = Controlled::new();
    let world = Arc::new(world);
    start_native_pick(
        &navs,
        Arc::clone(&world),
        action,
        FindOptions::default(),
        state.clone(),
    );
    await_native_search(&navs, &gate);
    gate.0.release();
    gate.0.wait(4);
    let selected = navs
        .lock()
        .unwrap()
        .get_mut("test")
        .unwrap()
        .bank_pick
        .take_native_receipt()
        .unwrap()
        .1
        .selected;
    assert_eq!(selected.kind, NativePickKind::Reachable);
    assert_eq!(usize::from(selected.bank_index), edgeville);
    assert_eq!(selected.access_tile, tile(3094, 3491));
    assert!(find_with(
        &world.collision,
        &world.graph,
        from,
        selected.access_tile,
        FindOptions::default(),
        &state
    )
    .is_ok());
    assert_eq!(selected.access.unwrap().stand_tile, tile(3095, 3491));
    assert!(navs.lock().unwrap()["test"].route.is_none());
    drop(slot);
}

#[test]
fn native_bank_pick_routes_air_near_across_wall_and_required_refuses() {
    let banks = vec![bank("across wall", 8, 1), bank("local", 1, 30)];
    for (explicit, expected) in [(None, Some(1)), (Some("across wall"), None)] {
        let (slot, action) = native_pick_action(banks.clone(), tile(4, 1), explicit);
        let navs = navs(banks.clone());
        let gate = Controlled::new();
        start_native_pick(
            &navs,
            native_world(true, &banks, TransportGraph::default()),
            action,
            FindOptions::default(),
            WorldState::empty(),
        );
        await_native_search(&navs, &gate);
        gate.0.release();
        gate.0.wait(4);
        let mut all = navs.lock().unwrap();
        let selected = all
            .get_mut("test")
            .unwrap()
            .bank_pick
            .take_native_receipt()
            .unwrap()
            .1
            .selected;
        assert_eq!(
            selected.kind,
            if expected.is_some() {
                NativePickKind::Reachable
            } else {
                NativePickKind::NoCandidate
            }
        );
        assert_eq!(selected.bank_index, expected.unwrap_or(u16::MAX));
        assert!(all["test"].route.is_none(), "selection must not move");
        drop(slot);
    }
    // Frozen SelectBank still completes by four-tile air proximity; changing
    // native picks must not silently alter compatibility behavior.
    let navs = navs(banks.clone());
    queue_bank_pick(
        &navs,
        "test",
        &Some(native_world(true, &banks, TransportGraph::default())),
        None,
        tile(4, 1),
        false,
        1,
        BankPreferences::default(),
        None,
    );
    let all = navs.lock().unwrap();
    assert_eq!(all["test"].bank_pick.posted.bank_index, 0);
    assert_eq!(
        all["test"].bank_pick.posted.kind,
        PickKind::NearShortcut as u8
    );
    assert!(all["test"].bank_pick.worker.is_none());
}

#[test]
fn native_bank_pick_missing_access_is_not_even_a_timeout_candidate() {
    let banks = vec![bank("unmapped near", 4, 1), bank("local", 40, 40)];
    for explicit in [None, Some("unmapped near")] {
        let (slot, action) = native_pick_action(banks.clone(), tile(4, 1), explicit);
        let navs = navs(banks.clone());
        let gate = Controlled::new();
        start_native_pick(
            &navs,
            native_world(false, &banks[1..], TransportGraph::default()),
            action,
            FindOptions::default(),
            WorldState::empty(),
        );
        if explicit.is_none() {
            await_native_search(&navs, &gate);
            let mut all = navs.lock().unwrap();
            let pick = &mut all.get_mut("test").unwrap().bank_pick;
            let deadline = pick.current.as_ref().unwrap().deadline.unwrap();
            pick.poll(deadline);
            let selected = pick.take_native_receipt().unwrap().1.selected;
            assert_eq!(selected.bank_index, 1);
            assert_eq!(selected.kind, NativePickKind::AirFallback);
            assert!(selected.access.is_some());
            drop(all);
            gate.0.release();
            gate.0.wait(4);
        } else {
            let mut all = navs.lock().unwrap();
            let pick = &mut all.get_mut("test").unwrap().bank_pick;
            let selected = pick.take_native_receipt().unwrap().1.selected;
            assert_eq!(selected.kind, NativePickKind::NoCandidate);
            assert!(pick.worker.is_none());
        }
        drop(slot);
    }
}

#[test]
fn native_bank_pick_required_is_exclusive_and_default_uses_cheapest_route() {
    let banks = vec![bank("local", 9, 4), bank("authored distant", 40, 40)];
    for (explicit, expected) in [(None, 0), (Some("authored distant"), 1)] {
        let (slot, action) = native_pick_action(banks.clone(), tile(4, 4), explicit);
        let navs = navs(banks.clone());
        let gate = Controlled::new();
        start_native_pick(
            &navs,
            native_world(false, &banks, TransportGraph::default()),
            action,
            FindOptions::default(),
            WorldState::empty(),
        );
        await_native_search(&navs, &gate);
        gate.0.release();
        gate.0.wait(4);
        let selected = navs
            .lock()
            .unwrap()
            .get_mut("test")
            .unwrap()
            .bank_pick
            .take_native_receipt()
            .unwrap()
            .1
            .selected;
        assert_eq!(selected.bank_index, expected);
        assert_eq!(selected.kind, NativePickKind::Reachable);
        drop(slot);
    }
}

#[test]
fn native_bank_pick_forbids_granted_falador_teleport_with_runes_held() {
    use nav::transport::{TransportEdge, TransportKind};
    let banks = vec![bank("local", 12, 4), bank("Falador teleport bank", 40, 40)];
    let mut graph = TransportGraph::default();
    graph.teleports.push(TransportEdge { worn_all_req: Vec::new(), kind: TransportKind::Teleport,
    player_delta: None,
    at: tile(0, 0),
    to: tile(39, 39),
    loc_id: 0,
    option: 0,
    ticks: 1,
    dir: None,
    open_loc_id: None,
    skill_req: vec![(6, 37)],
    item_req: vec![(555, 1), (556, 3), (563, 1)],
    consumed_req: vec![],
    item_returns: vec![],
    quest_req: vec![],
    varp_req: vec![],
    worn_req: vec![],
    members_req: false,
    wildy_cap: None,
    quest_gates: None, });
    let world = native_world(false, &banks, graph);
    let mut state = WorldState::empty().with_map_members(true);
    state.stats.insert(6, 60);
    state.inv.extend([(555, 1), (556, 3), (563, 1)]);
    let opts = FindOptions {
        allow_teleports: true,
        ..FindOptions::default()
    };
    let local = native_bank_access(&world, banks[0]).unwrap().0[0];
    let remote = native_bank_access(&world, banks[1]).unwrap().0[0];
    let granted = nav::router::find_first_with(
        &world.collision,
        &world.graph,
        tile(4, 4),
        &[local, remote],
        opts,
        &state,
    )
    .into_route()
    .unwrap();
    assert_eq!(
        granted.dest, remote,
        "fixture must favor the granted teleport"
    );
    let (slot, action) = native_pick_action(banks.clone(), tile(4, 4), None);
    let navs = navs(banks);
    let gate = Controlled::new();
    start_native_pick(&navs, world, action, opts, state);
    await_native_search(&navs, &gate);
    gate.0.release();
    gate.0.wait(4);
    let selected = navs
        .lock()
        .unwrap()
        .get_mut("test")
        .unwrap()
        .bank_pick
        .take_native_receipt()
        .unwrap()
        .1
        .selected;
    assert_eq!(selected.bank_index, 0);
    assert_eq!(selected.kind, NativePickKind::Reachable);
    drop(slot);
}

#[test]
fn native_bank_pick_timeout_is_only_candidate_order_not_wall_arrival() {
    let banks = vec![bank("across wall", 8, 1), bank("local", 1, 30)];
    let from = tile(4, 1);
    let world = native_world(true, &banks, TransportGraph::default());
    let (slot, action) = native_pick_action(banks.clone(), from, None);
    let navs = navs(banks);
    let gate = Controlled::new();
    start_native_pick(
        &navs,
        Arc::clone(&world),
        action,
        FindOptions::default(),
        WorldState::empty(),
    );
    await_native_search(&navs, &gate);
    let selected = {
        let mut all = navs.lock().unwrap();
        let pick = &mut all.get_mut("test").unwrap().bank_pick;
        let deadline = pick.current.as_ref().unwrap().deadline.unwrap();
        pick.poll(deadline);
        let selected = pick.take_native_receipt().unwrap().1.selected;
        assert!(all["test"].route.is_none(), "a pick never arms movement");
        selected
    };
    assert_eq!(selected.kind, NativePickKind::AirFallback);
    assert_eq!(
        selected.bank_index, 0,
        "timeout keeps the captured air order"
    );
    assert!(
        find_with(
            &world.collision,
            &world.graph,
            from,
            selected.access_tile,
            FindOptions::default(),
            &WorldState::empty(),
        )
        .is_err(),
        "the ordinary subsequent walk must still route through collision"
    );
    gate.0.release();
    gate.0.wait(4);
    assert!(
        navs.lock()
            .unwrap()
            .get_mut("test")
            .unwrap()
            .bank_pick
            .take_native_receipt()
            .is_none(),
        "late reachable-bank result cannot replace the timeout candidate"
    );
    drop(slot);
}

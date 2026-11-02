use super::HoldWait;
use crate::admission::{self, Admission, RiskPolicy, WalkAllow, WalkRefusal};
use crate::script_runtime::NavBot;
use api::condition::{PoisonState, WindowReset};
use api::interact::Driver;
use api::snapshot::{
    GameSnapshot, HitmarkView, HitmarksView, LocalPlayerView, PlayerView, SceneView, StatView,
    WorldStateView, WorldTile,
};
use client::client::{Client, ClientConfig};
use client::io::{map_client_prot, ClientProt, ClientRevision};
use nav_planner::collision::{pack_walk, WorldCollision};
use nav_planner::router::{Leg, Route};
use nav_planner::transport::TransportGraph;
use nav_planner::world::NavWorld;
use nav_planner::zones::{Zone, ZoneClass, ZoneKind, ZoneTable};
use script::combat::risk::PoisonTest;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

const ALICE: &str = "alice";
const HP: i32 = 40;
const POISON_RATE: u8 = 11;

#[derive(Default)]
struct HoldDriver {
    current: (i32, i32),
    walks: Vec<(i32, i32)>,
    sink: Sink,
}

impl Driver for HoldDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}

    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }

    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        dx: i32,
        dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _ty: i32,
    ) -> bool {
        self.walks.push((dx, dz));
        true
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        Some(self.current)
    }

    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn send_packet(&mut self, packet: &api::prot::Send) -> Result<(), api::prot::PacketError> {
        packet.write_for_revision(self.revision(), &mut self.sink)
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

#[derive(Default)]
struct Sink;

impl api::prot::Out for Sink {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}

struct HoldRig {
    start: i32,
    route: Arc<Route>,
    world: Arc<NavWorld>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<crate::play_status::SlotStatus>>>,
    observer: Client,
    _listener: std::net::TcpListener,
    snapshot: GameSnapshot,
    driver: HoldDriver,
}

impl HoldRig {
    fn new(start: i32, safe_index: u16, accepted: bool) -> Self {
        let world = Arc::new(test_world(start, safe_index));
        let route = Arc::new(test_route(start));
        let (mut observer, listener) = outgoing_client();
        let snapshot = frame(&observer, 0, tile(start), None, None, world_width(start));
        let (combat, poison_rate) =
            admission::condition_tables(&world).expect("selected risk tables are available");
        assert_eq!(poison_rate, POISON_RATE);

        let mut bot = NavBot::default();
        bot.condition
            .session_started(0, 0, Some((combat, poison_rate)));
        bot.condition.observe_frame(&snapshot, &drain(false), false);
        if accepted {
            observe_dispatch(
                &mut bot,
                &mut observer,
                ClientProt::MOVE_GAMECLICK,
                &snapshot,
            );
        }

        let input = admission::capture(&snapshot, false, bot.condition.current(), false);
        assert!(
            !input.missing_facts(),
            "fixture must have complete live facts"
        );
        assert_eq!(input.poison, PoisonState::Unknown { since: 0 });
        let admission = Admission {
            policy: RiskPolicy::Avoid,
            enforce: true,
            grants: nav_planner::zones::ZoneExempt::NONE,
            allow: WalkAllow::default(),
            input,
            generation: 1,
            compat_v1: false,
            escape: None,
        };
        let assessment = admission::assess(&route, &world, &admission);
        assert!(
            matches!(
                &assessment.poison_test,
                PoisonTest::WaitForClear { until_index, .. } if *until_index == safe_index
            ),
            "hold fixture assessment: {assessment:?}"
        );

        bot.route_generation = 1;
        bot.walk_request_id = 1;
        bot.requested_route = Some((
            route.dest,
            0,
            false,
            false,
            false,
            nav_planner::zones::ZoneExempt::NONE,
        ));
        bot.publish_route(
            1,
            1,
            false,
            crate::walk_plan::RouteOutcome::Routed(route.as_ref().clone()),
        );
        bot.admission = Some(Box::new(admission));
        bot.assessment = Some(assessment);
        bot.admission_pending = true;

        let driver = HoldDriver {
            current: (start, 1),
            ..Default::default()
        };
        Self {
            start,
            route,
            world,
            navs: Arc::new(Mutex::new(HashMap::from([(ALICE.to_owned(), bot)]))),
            statuses: Arc::new(Mutex::new(vec![crate::play_status::SlotStatus {
                username: ALICE.to_owned(),
                ..Default::default()
            }])),
            observer,
            _listener: listener,
            snapshot,
            driver,
        }
    }

    fn observe(&mut self, tick: u32, here: WorldTile, modal: Option<i32>, hit: Option<(i32, i32)>) {
        self.snapshot = frame(
            &self.observer,
            tick,
            here,
            modal,
            hit,
            world_width(self.start),
        );
        self.navs
            .lock()
            .unwrap()
            .get_mut(ALICE)
            .expect("nav bot")
            .condition
            .observe_frame(&self.snapshot, &drain(true), false);
    }

    fn dispatch(&mut self, packet: ClientProt) {
        let mut navs = self.navs.lock().unwrap();
        let bot = navs.get_mut(ALICE).expect("nav bot");
        observe_dispatch(bot, &mut self.observer, packet, &self.snapshot);
    }

    fn pump(&mut self, here: WorldTile, world_available: bool) {
        self.driver.current = (here.x, here.z);
        let world = world_available.then_some(&self.world);
        crate::script_runtime::step_nav_bot(
            &mut self.driver,
            ALICE,
            Some((here.x, here.z, here.level)),
            &self.snapshot,
            &self.navs,
            &self.statuses,
            world,
            false,
            false,
            None,
            no_reach,
        );
    }

    fn rearm(&mut self) {
        let mut navs = self.navs.lock().unwrap();
        let bot = navs.get_mut(ALICE).expect("nav bot");
        let generation = bot.route_generation.wrapping_add(1);
        let request_id = bot.walk_request_id.wrapping_add(1);
        let input = admission::capture(&self.snapshot, false, bot.condition.current(), false);
        let admission = Admission {
            policy: RiskPolicy::Avoid,
            enforce: true,
            grants: nav_planner::zones::ZoneExempt::NONE,
            allow: WalkAllow::default(),
            input,
            generation,
            compat_v1: false,
            escape: None,
        };
        let assessment = admission::assess(&self.route, &self.world, &admission);
        assert!(matches!(
            &assessment.poison_test,
            PoisonTest::WaitForClear { .. }
        ));
        bot.route_generation = generation;
        bot.walk_request_id = request_id;
        bot.requested_route = Some((
            self.route.dest,
            0,
            false,
            false,
            false,
            nav_planner::zones::ZoneExempt::NONE,
        ));
        bot.publish_route(
            generation,
            request_id,
            false,
            crate::walk_plan::RouteOutcome::Routed(self.route.as_ref().clone()),
        );
        bot.admission = Some(Box::new(admission));
        bot.assessment = Some(assessment);
        bot.admission_pending = true;
        bot.risk_refusal = None;
    }

    fn bot(&self) -> MutexGuard<'_, HashMap<String, NavBot>> {
        self.navs.lock().unwrap()
    }
}

fn outgoing_client() -> (Client, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let addr = listener.local_addr().expect("fixture listener address");
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port())
        .expect("fixture client stream");
    let mut client = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        ClientRevision::R289,
    );
    client.stream = Some(stream);
    client.ingame = true;
    client.scene_state = 2;
    client.out.random = Some(client::io::Isaac::new(&[1, 2, 3, 4]));
    (client, listener)
}

fn observe_dispatch(
    bot: &mut NavBot,
    client: &mut Client,
    packet: ClientProt,
    snapshot: &GameSnapshot,
) {
    let checkpoint = bot
        .condition
        .dispatch_checkpoint(client)
        .expect("unknown condition permits packet observation");
    let selected = map_client_prot(client.revision(), packet);
    client.out.p1_enc(selected.id);
    let length = if selected.length == -1 {
        client.out.p1(5);
        5
    } else {
        selected.length
    };
    for _ in 0..length {
        client.out.p1(0);
    }
    bot.condition.observe_dispatch(client, checkpoint);
    bot.condition.observe_frame(snapshot, &drain(false), false);
}

fn no_reach() -> Arc<api::query::ReachQueryView> {
    Arc::new(api::query::ReachQueryView::unavailable())
}

fn drain(player_info: bool) -> host::DrainResult {
    host::DrainResult {
        player_info,
        npc_rows_coherent: true,
        session_changed: false,
        gens: Default::default(),
        dirty: Default::default(),
    }
}

fn tile(x: i32) -> WorldTile {
    WorldTile { x, z: 1, level: 0 }
}

fn world_width(start: i32) -> usize {
    usize::try_from(start + 101).expect("positive fixture width")
}

fn test_route(start: i32) -> Route {
    Route {
        legs: vec![Leg::Walk {
            tiles: (start..=start + 100).map(tile).collect(),
        }],
        dest: tile(start + 100),
        ticks: 100.0,
    }
}

fn test_world(start: i32, safe_index: u16) -> NavWorld {
    let width = world_width(start);
    let (walk, blocked) = pack_walk(&vec![0; width * 3 * 4]);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width,
        height: 3,
        walk,
        blocked,
        flags: None,
    };
    let combat = script::combat::guard::shared_tables().expect("revision 289 combat facts");
    let npc = combat
        .selected()
        .npc_by_config("icewarrior")
        .expect("revision 289 ice warrior facts");
    let kind = ZoneKind::new("ice-warrior", "Ice warrior", npc.id, 57, false, false);
    // The Ice warrior's six-tile backward envelope must end after the
    // requested retreat tile, not include that tile in the crossing.
    let zone = Zone::npc(
        tile(start + i32::from(safe_index) + 7),
        0,
        ZoneClass::Always,
        u16::MAX,
        0,
    );
    let mut graph = TransportGraph::default();
    graph.zones = Some(
        ZoneTable::from_parts(
            vec![zone],
            vec![kind],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            collision.origin,
            width as u32,
            3,
            &graph.wilderness,
        )
        .expect("valid one-zone corridor"),
    );
    NavWorld::from_parts(collision, graph, Vec::new())
}

fn frame(
    observer: &Client,
    tick: u32,
    here: WorldTile,
    modal: Option<i32>,
    hit: Option<(i32, i32)>,
    width: usize,
) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(observer);
    assert!(
        snapshot.attached(),
        "fixture frames must be backed by a connected observer"
    );
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView::default());
    snapshot.seed_tile(here);
    snapshot.seed_tick(tick);
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            actor: api::snapshot::ActorView {
                name: None,
                actions: Vec::new(),
                tile: here,
                distance: 0,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: HP,
                total_health: HP,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            network: here,
            combat_level: 3,
            skill_level: 1,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
    });
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_inventory_packet_generations(Some(0), None);
    snapshot.seed_stats(vec![
        StatView {
            index: 3,
            name: "Hitpoints".into(),
            effective: HP,
            base: HP,
            xp: 0,
            used: true,
        },
        StatView {
            index: 1,
            name: "Defence".into(),
            effective: 1,
            base: 1,
            xp: 0,
            used: true,
        },
        StatView {
            index: 5,
            name: "Prayer".into(),
            effective: 1,
            base: 1,
            xp: 0,
            used: true,
        },
    ]);
    snapshot.seed_varps(vec![api::snapshot::VarpView {
        index: 173,
        value: 0,
    }]);
    let mut marks = [HitmarkView {
        kind: 0,
        value: 0,
        cycle: -1,
    }; 4];
    let loop_cycle = tick as i32;
    if let Some((kind, value)) = hit {
        marks[0] = HitmarkView {
            kind,
            value,
            cycle: 100 + loop_cycle,
        };
    }
    snapshot.seed_hitmarks(HitmarksView { marks, loop_cycle });
    snapshot.seed_main_modal(modal.unwrap_or(-1), Vec::new());
    snapshot.seed_chat_modal(-1, Vec::new());
    snapshot.seed_scene(SceneView {
        available: true,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: width as i32,
        height: 3,
        collision_flags: vec![0; width * 3],
    });
    snapshot
}

#[test]
fn poison_hold_full_window_waits_31_and_resumes_only_after_clear() {
    let mut rig = HoldRig::new(0, 0, true);
    rig.pump(tile(0), true);
    {
        let navs = rig.bot();
        let hold = navs[ALICE].poison_hold.as_ref().expect("published hold");
        let wait = hold.wait.expect("at the retreat tile");
        assert_eq!(hold.until_index, 0);
        assert_eq!(wait.started, 0);
        assert_eq!(wait.deadline.wrapping_sub(wait.started), 31);
        assert!(
            rig.driver.walks.is_empty(),
            "accepted movement needs no stand click"
        );
    }

    for tick in 1..=30 {
        rig.observe(tick, tile(0), None, None);
        rig.pump(tile(0), true);
        let navs = rig.bot();
        let hold = navs[ALICE].poison_hold.as_ref().expect("hold until clear");
        assert_eq!(hold.wait.unwrap().deadline, 31);
        assert!(
            rig.driver.walks.is_empty(),
            "no hop while poison is unresolved"
        );
    }
    rig.observe(31, tile(0), None, None);
    assert_eq!(
        rig.bot()[ALICE].condition.current().unwrap().poison,
        PoisonState::Clear
    );
    rig.pump(tile(0), true);
    let navs = rig.bot();
    assert!(navs[ALICE].poison_hold.is_none());
    assert!(navs[ALICE].risk_refusal.is_none());
    assert!(matches!(
        &navs[ALICE].assessment.as_ref().unwrap().poison_test,
        PoisonTest::PassesCharged { .. } | PoisonTest::NotNeeded
    ));
    assert!(
        !rig.driver.walks.is_empty(),
        "follow resumes after the clear fact"
    );
}

#[test]
fn poison_hold_partial_window_waits_the_remainder() {
    let mut rig = HoldRig::new(0, 0, true);
    for tick in 1..=13 {
        rig.observe(tick, tile(0), None, None);
    }
    let window = rig.bot()[ALICE]
        .condition
        .current()
        .unwrap()
        .poison_window
        .unwrap();
    assert_eq!(window.remaining, 18);

    rig.pump(tile(0), true);
    let wait = rig.bot()[ALICE].poison_hold.as_ref().unwrap().wait.unwrap();
    assert_eq!(wait.started, 13);
    assert_eq!(wait.deadline, 31);
    for tick in 14..=31 {
        rig.observe(tick, tile(0), None, None);
        rig.pump(tile(0), true);
    }
    let navs = rig.bot();
    assert_eq!(
        navs[ALICE].condition.current().unwrap().poison,
        PoisonState::Clear
    );
    assert!(navs[ALICE].poison_hold.is_none());
    assert!(navs[ALICE].risk_refusal.is_none());
}

#[test]
fn poison_hold_sends_no_hop_into_exposure() {
    let mut rig = HoldRig::new(40, 0, true);
    rig.pump(tile(40), true);
    {
        let navs = rig.bot();
        let hold = navs[ALICE].poison_hold.as_ref().expect("published hold");
        assert_eq!(hold.until_index, 0);
        assert_eq!(hold.wait.unwrap().deadline, 31);
    }
    assert!(rig.driver.walks.is_empty());

    for tick in 1..=30 {
        rig.observe(tick, tile(40), None, None);
        rig.pump(tile(40), true);
        assert!(
            rig.driver.walks.is_empty(),
            "route cannot enter the zone during hold"
        );
    }
    assert!(rig.driver.walks.iter().all(|&(x, _)| x <= 40));
    rig.observe(31, tile(40), None, None);
    rig.pump(tile(40), true);
    assert!(rig.driver.walks.iter().any(|&(x, _)| x > 40));
}

#[test]
fn poison_long_approach_clears_without_a_stop() {
    let mut rig = HoldRig::new(0, 40, true);
    rig.pump(tile(0), true);
    {
        let navs = rig.bot();
        let hold = navs[ALICE]
            .poison_hold
            .as_ref()
            .expect("published approach hold");
        assert_eq!(hold.until_index, 40);
        assert!(hold.wait.is_none(), "approach does not start the wait");
    }

    for tick in 1..=30 {
        rig.observe(tick, tile(tick as i32), None, None);
        rig.pump(tile(tick as i32), true);
        if let Some(hold) = rig.bot()[ALICE].poison_hold.as_ref() {
            assert!(
                hold.wait.is_none(),
                "approach must not spend clear ticks waiting"
            );
        }
    }
    rig.observe(31, tile(31), None, None);
    rig.pump(tile(31), true);
    let navs = rig.bot();
    assert_eq!(
        navs[ALICE].condition.current().unwrap().poison,
        PoisonState::Clear
    );
    assert!(navs[ALICE].poison_hold.is_none());
    assert!(navs[ALICE].risk_refusal.is_none());
}

#[test]
fn poison_door_interaction_restarts_the_window_at_tick_20() {
    let mut rig = HoldRig::new(0, 40, true);
    rig.pump(tile(0), true);
    for tick in 1..=20 {
        rig.observe(tick, tile(tick as i32), None, None);
        if tick == 20 {
            rig.dispatch(ClientProt::OPLOC1);
            let window = rig.bot()[ALICE]
                .condition
                .current()
                .unwrap()
                .poison_window
                .unwrap();
            assert_eq!(window.remaining, 31);
            assert!(window.accepted);
            assert_eq!(window.last_reset, WindowReset::Interaction);
        }
        rig.pump(tile(tick as i32), true);
    }
    for tick in 21..=40 {
        rig.observe(tick, tile(tick as i32), None, None);
        rig.pump(tile(tick as i32), true);
    }
    let navs = rig.bot();
    let hold = navs[ALICE]
        .poison_hold
        .as_ref()
        .expect("still held at retreat");
    assert_eq!(hold.until_index, 40);
    assert_eq!(hold.wait.unwrap().started, 40);
    assert_eq!(hold.wait.unwrap().deadline, 51);
    assert_eq!(
        navs[ALICE]
            .condition
            .current()
            .unwrap()
            .poison_window
            .unwrap()
            .remaining,
        11
    );
}

#[test]
fn poison_hold_sends_one_stand_click_when_movement_was_not_accepted() {
    let mut rig = HoldRig::new(0, 0, false);
    rig.pump(tile(0), true);
    {
        let navs = rig.bot();
        let hold = navs[ALICE].poison_hold.as_ref().expect("published hold");
        assert!(hold.stand_sent);
        assert_eq!(hold.wait.unwrap().deadline, 31);
    }
    assert_eq!(rig.driver.walks.len(), 1);

    rig.dispatch(ClientProt::MOVE_GAMECLICK);
    rig.observe(1, tile(0), None, None);
    rig.pump(tile(0), true);
    assert_eq!(rig.driver.walks.len(), 1, "stand click is one-shot");
    assert!(rig.bot()[ALICE].poison_hold.as_ref().unwrap().stand_sent);
}

#[test]
fn poison_hold_reassesses_a_kind_two_hit_as_poisoned() {
    let mut rig = HoldRig::new(0, 0, true);
    rig.pump(tile(0), true);
    assert!(rig.bot()[ALICE]
        .poison_hold
        .as_ref()
        .unwrap()
        .wait
        .is_some());

    rig.observe(1, tile(0), None, Some((2, i32::from(POISON_RATE))));
    rig.pump(tile(0), true);
    let navs = rig.bot();
    assert!(matches!(
        navs[ALICE].condition.current().unwrap().poison,
        PoisonState::Poisoned {
            per_tick: POISON_RATE,
            ..
        }
    ));
    assert_eq!(
        navs[ALICE].assessment.as_ref().unwrap().input.poison,
        navs[ALICE].condition.current().unwrap().poison
    );
    assert!(navs[ALICE].poison_hold.is_none());
    assert!(matches!(
        navs[ALICE].risk_refusal.as_deref().map(|entry| entry.1),
        Some(WalkRefusal::Unsurvivable)
    ));
}

#[test]
fn poison_hold_reset_ends_at_deadline_and_retry_gets_a_new_hold() {
    let mut rig = HoldRig::new(0, 0, true);
    rig.pump(tile(0), true);
    for tick in 1..=31 {
        rig.observe(tick, tile(0), (tick == 5).then_some(123), None);
        rig.pump(tile(0), true);
    }
    {
        let navs = rig.bot();
        assert!(matches!(
            navs[ALICE].risk_refusal.as_deref().map(|entry| entry.1),
            Some(WalkRefusal::PoisonUnsettled { .. })
        ));
        assert!(navs[ALICE].poison_hold.is_none());
        assert!(navs[ALICE].route.is_none());
        assert_eq!(
            navs[ALICE]
                .condition
                .current()
                .unwrap()
                .poison_window
                .unwrap()
                .last_reset,
            WindowReset::Modal
        );
    }

    rig.observe(32, tile(0), None, None);
    assert_eq!(
        rig.bot()[ALICE]
            .condition
            .current()
            .unwrap()
            .poison_window
            .unwrap()
            .remaining,
        4
    );
    rig.rearm();
    rig.pump(tile(0), true);
    let navs = rig.bot();
    let hold = navs[ALICE]
        .poison_hold
        .as_ref()
        .expect("retry hold installed");
    let wait: HoldWait = hold.wait.expect("retry starts the new remainder");
    assert_eq!(wait.started, 32);
    assert_eq!(wait.deadline, 36);
}

#[test]
fn known_clear_or_poisoned_fact_with_no_world_never_becomes_unsettled() {
    let mut clear = HoldRig::new(0, 0, true);
    clear.pump(tile(0), true);
    for tick in 1..=30 {
        clear.observe(tick, tile(0), None, None);
        clear.pump(tile(0), true);
    }
    clear.observe(31, tile(0), None, None);
    assert_eq!(
        clear.bot()[ALICE].condition.current().unwrap().poison,
        PoisonState::Clear
    );
    assert!(clear.bot()[ALICE]
        .poison_hold
        .as_ref()
        .unwrap()
        .wait
        .is_some());
    clear.pump(tile(0), false);
    assert!(!matches!(
        clear.bot()[ALICE]
            .risk_refusal
            .as_deref()
            .map(|entry| entry.1),
        Some(WalkRefusal::PoisonUnsettled { .. })
    ));

    let mut poisoned = HoldRig::new(0, 0, true);
    poisoned.pump(tile(0), true);
    assert!(poisoned.bot()[ALICE]
        .poison_hold
        .as_ref()
        .unwrap()
        .wait
        .is_some());
    poisoned.observe(1, tile(0), None, Some((2, i32::from(POISON_RATE))));
    assert!(matches!(
        poisoned.bot()[ALICE].condition.current().unwrap().poison,
        PoisonState::Poisoned { .. }
    ));
    poisoned.pump(tile(0), false);
    assert!(!matches!(
        poisoned.bot()[ALICE]
            .risk_refusal
            .as_deref()
            .map(|entry| entry.1),
        Some(WalkRefusal::PoisonUnsettled { .. })
    ));
}
